use crate::hub::asar::{self, AsarArchive};
use crate::hub::patcher::replace_fn_body;
use crate::hub::payloads::{wrap_async_body, FAKE_USER_FIELDS};
use crate::hub::strategy::PatchStrategy;
use anyhow::Context;

/// Patch strategy for Unity Hub ESM bundled format.
///
/// Characteristics:
/// - ESM format (`import`/`export`)
/// - Bundled into `build/main/app-*.js` (single file with hash in name)
/// - Native `async`/`await` (no `__awaiter` wrapper)
/// - Dependency injection via constructor parameters
/// - Private fields (`#logger`, `#licensingSdk`, etc.)
/// - Event broadcasting via `getChannel("app").publish()` and `mainContainer.get(IEventBroadcasterId)`
pub(crate) struct EsmStrategy;

impl EsmStrategy {
    /// Find the bundled app file in the asar archive.
    fn find_app_bundle(asar: &AsarArchive) -> Option<String> {
        // Walk the header to find build/main/app-*.js
        // read_asar wraps header in {"files": {...}}, so root is already "files"
        let root = asar.header_json.get("files")?;
        let build = root.get("build")?;
        let main = build.get("files")?.get("main")?;
        let main_files = main.get("files")?.as_object()?;

        for (name, entry) in main_files {
            if name.starts_with("app-") && name.ends_with(".js") {
                // Check it has size/offset (is a file, not a directory)
                if entry.get("size").is_some() && entry.get("offset").is_some() {
                    return Some(format!("build/main/{}", name));
                }
            }
        }
        None
    }
}

impl PatchStrategy for EsmStrategy {
    fn version_name(&self) -> &'static str {
        "ESM (bundled)"
    }

    fn applies_to(&self, asar: &AsarArchive) -> bool {
        Self::find_app_bundle(asar).is_some()
    }

    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let bundle_path =
            Self::find_app_bundle(asar).context("could not find app bundle in asar")?;

        let entry =
            asar::find_entry(&asar.header_json, &bundle_path).context("bundle entry not found")?;
        let content_bytes = asar::extract_file(asar, &entry).context("could not extract bundle")?;
        let content = String::from_utf8(content_bytes).context("bundle is not valid UTF-8")?;

        let patched = patch_bundled_app(&content)?;
        Ok(vec![(bundle_path, patched.into_bytes())])
    }
}

fn patch_bundled_app(content: &str) -> anyhow::Result<String> {
    let mut result = content.to_string();

    // 1. Patch isLicenseValid() → return true
    let new_is_valid = wrap_async_body("        return true;");
    result = replace_fn_body(&result, "isLicenseValid(", &new_is_valid)
        .context("patch isLicenseValid failed")?;

    // 2. Patch isUserSignedIn() → return true
    // In 3.18: checks `this.userInfo?.foreignKey`
    let new_signed_in = wrap_async_body("        return true;");
    result = replace_fn_body(&result, "isUserSignedIn(", &new_signed_in)
        .context("patch isUserSignedIn failed")?;

    // 3. Patch getValidEntitlementGroups() → return fake Pro license
    let new_entitlements = wrap_async_body(
        "        return [{
            startDate: new Date('1970-01-01T00:00:00.000Z'),
            expirationDate: new Date('9999-01-01T08:00:00.000Z'),
            productName: 'Pro Max Ultra',
            licenseType: 'ULF',
        }];",
    );
    result = replace_fn_body(&result, "getValidEntitlementGroups(", &new_entitlements)
        .context("patch getValidEntitlementGroups failed")?;

    // 4. Patch constructor() — inject fake userInfo
    // In 3.18, constructor has many DI params: (loggerFactory, windowActions, runtimePowerMonitor, gitCredentialManager, tokenManager)
    let new_constructor = wrap_async_body(&format!(
        "        super();
        this.#logger = loggerFactory.createLogger(\"AuthService\");
        this.#windowActions = windowActions;
        this.#runtimePowerMonitor = runtimePowerMonitor;
        this.#gitCredentialManager = gitCredentialManager;
        this.#tokenManager = tokenManager;
        this.codeVerifier = \"\";
        this.connectInfo = {{
            error: false,
            initialized: false,
            loggedIn: false,
            maintenance: false,
            online: true,
            ready: false,
            showLoginWindow: false,
            workOffline: false
        }};
        this.tokenRefreshTimer = null;
        this.userInfoRefreshTimer = null;
        this.userInfo = {{
{FAKE_USER_FIELDS}
        }};"
    ));
    result = replace_fn_body(
        &result,
        "constructor(loggerFactory, windowActions, runtimePowerMonitor",
        &new_constructor,
    )
    .context("patch constructor failed")?;

    // 5. Patch restoreAuthState() — set loggedIn and broadcast events
    // In 3.18: uses mainContainer.get(IEventBroadcasterId).broadcast() and getChannel("app").publish()
    let new_restore = wrap_async_body(
        "        this.connectInfo.ready = true;
        this.connectInfo.loggedIn = true;
        this.connectInfo.initialized = true;
        this.connectInfo.workOffline = true;
        this.connectInfo.online = false;
        this.userOrgs = null;
        mainContainer.get(IEventBroadcasterId).broadcast(POSTAL_TOPICS.CONNECT_INFO_CHANGED, this.connectInfo);
        getChannel(\"app\").publish(POSTAL_TOPICS.CONNECT_INFO_CHANGED, { connectInfo: this.connectInfo });
        getChannel(\"app\").publish(POSTAL_TOPICS.USER_INFO_CHANGED, { userInfo: this.getFormattedUserInfo() });
        getChannel(\"fsm.user\").publish(POSTAL_TOPICS.LOGGED_IN);",
    );
    result = replace_fn_body(&result, "restoreAuthState(", &new_restore)
        .context("patch restoreAuthState failed")?;

    // 6. Patch getInstance() on LicensingSdk → return null
    // This prevents the actual licensing SDK from being used
    let new_get_instance = wrap_async_body("        return null;");
    result = replace_fn_body(&result, "getInstance(", &new_get_instance)
        .context("patch getInstance failed")?;

    // 7. Patch fetchVariants() to skip network call in airgapped environments
    // FeatureFlagsService.fetchVariants() makes HTTP request to Amplitude Experiment API
    // which times out in airgapped/intranet-only environments (15s timeout + retries = minutes)
    let new_fetch_variants = wrap_async_body(
        "        this.#logger.info(\"Fetching feature flags (skipped - offline mode)\");
        const user = await this.getExperimentUser();
        if (!user.user_id && !user.device_id) {
            this.#logger.info(\"User info and licensing info not complete, skipping fetching feature flags for now.\");
            return;
        }
        // Use cached variants only, skip network call
        const { amplitude: cachedAmplitudeVariants } = this.#cache.cachedSchema;
        if (cachedAmplitudeVariants && Object.keys(cachedAmplitudeVariants).length > 0) {
            this.handleNewVariants({
                previous: this.variants,
                next: cachedAmplitudeVariants
            });
            this.#logger.info(\"Using cached feature flags\");
        }",
    );
    result = replace_fn_body(&result, "fetchVariants(", &new_fetch_variants)
        .context("patch fetchVariants failed")?;

    // 8. Patch AuthService.getOrganizations() to return empty array (prevent hanging in airgapped environments)
    // AuthService.getOrganizations() calls cloudCore.getOrganizations() which makes axios call without timeout
    // In airgapped environments, this hangs waiting for TCP connection timeout (~18 seconds)
    let new_get_organizations = wrap_async_body(
        "        this.#logger.debug(\"Organization data has been requested from AuthService\");
        if (!this.connectInfo.loggedIn) throw new Error(\"User is not logged in\");
        if (this.userOrgs !== null) {
            this.#logger.debug(\"Returning cached userOrg data\");
            return this.userOrgs;
        }
        // Skip network call in airgapped environments, return empty array
        this.#logger.debug(\"Returning empty organization list (offline mode)\");
        this.userOrgs = [];
        return this.userOrgs;",
    );
    result = replace_fn_body(&result, "getOrganizations(", &new_get_organizations)
        .context("patch getOrganizations failed")?;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_bundle() -> String {
        r#"
class AuthService extends BaseAuthService {
    constructor(loggerFactory, windowActions, runtimePowerMonitor, gitCredentialManager, tokenManager) {
        super();
        this.#logger = loggerFactory.createLogger("AuthService");
        this.#windowActions = windowActions;
        this.#runtimePowerMonitor = runtimePowerMonitor;
        this.#gitCredentialManager = gitCredentialManager;
        this.#tokenManager = tokenManager;
        this.connectInfo = {};
        this.userInfo = null;
    }

    isLicenseValid() {
        return Boolean(this.license);
    }

    isUserSignedIn() {
        return Boolean(this.userInfo?.foreignKey);
    }

    getValidEntitlementGroups() {
        return this.entitlementGroups.filter((group) => group.valid);
    }

    restoreAuthState() {
        this.connectInfo.ready = false;
        mainContainer.get(IEventBroadcasterId).broadcast(POSTAL_TOPICS.CONNECT_INFO_CHANGED, this.connectInfo);
        getChannel("app").publish(POSTAL_TOPICS.CONNECT_INFO_CHANGED, { connectInfo: this.connectInfo });
    }

    async getOrganizations() {
        this.#logger.debug("Organization data has been requested from AuthService");
        if (!this.connectInfo.loggedIn) throw new Error("User is not logged in");
        if (this.userOrgs !== null) {
            this.#logger.debug("Returning cached userOrg data");
            return this.userOrgs;
        }
        this.#logger.debug("userOrg cache miss - requesting from the server.");
        this.userOrgs = await cloudCore.getOrganizations();
        return this.userOrgs;
    }
}

class LicensingSdk {
    getInstance() {
        return this.instance;
    }
}

class FeatureFlagsService {
    async fetchVariants() {
        try {
            this.#logger.info("Fetching feature flags");
            const user = await this.getExperimentUser();
            if (!user.user_id && !user.device_id) {
                this.#logger.info("User info and licensing info not complete");
                return;
            }
            const newVariants = await this.client.fetchV2(user);
        } catch (error) {
            this.#logger.error("Error fetching feature flags", error);
        }
    }
}

class CloudCoreApiClient {
    getOrganizations(accessToken) {
        const url = `${mainContainer.get(ICloudConfigId).urls.core}/api/users/me?include=orgs`;
        this.#logger.debug(`GET ${url}`);
        return axios.get(url, this._getCommonConfig(accessToken));
    }
}
"#
        .to_string()
    }

    #[test]
    fn patch_is_license_valid() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "isLicenseValid(", "{ return true; }");
        assert!(result.is_some(), "should find isLicenseValid definition");
    }

    #[test]
    fn patch_is_user_signed_in() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "isUserSignedIn(", "{ return true; }");
        assert!(result.is_some(), "should find isUserSignedIn definition");
    }

    #[test]
    fn patch_get_valid_entitlement_groups() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "getValidEntitlementGroups(", "{ return []; }");
        assert!(
            result.is_some(),
            "should find getValidEntitlementGroups definition"
        );
    }

    #[test]
    fn patch_constructor() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "constructor(loggerFactory", "{ super(); }");
        assert!(result.is_some(), "should find AuthService constructor");
    }

    #[test]
    fn patch_restore_auth_state() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "restoreAuthState(", "{ /* noop */ }");
        assert!(result.is_some(), "should find restoreAuthState definition");
    }

    #[test]
    fn patch_get_instance() {
        let content = load_bundle();
        let result = replace_fn_body(&content, "getInstance(", "{ return null; }");
        assert!(result.is_some(), "should find getInstance definition");
    }

    #[test]
    fn full_patch_succeeds() {
        let content = load_bundle();
        let result = patch_bundled_app(&content);
        assert!(
            result.is_ok(),
            "full patch should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn find_app_bundle_detects_correct_file() {
        // Test with actual 3.18.0 asar file
        let asar_path = r"C:\Users\Zoranner\AppData\Local\Temp\hub-test\app.asar";
        if !std::path::Path::new(asar_path).exists() {
            eprintln!("SKIP: {} not found", asar_path);
            return;
        }

        let asar = crate::hub::asar::read_asar(std::path::Path::new(asar_path))
            .expect("Failed to read asar");

        let bundle = EsmStrategy::find_app_bundle(&asar);
        assert!(bundle.is_some(), "Should find app-*.js bundle");

        let bundle_name = bundle.unwrap();
        assert!(
            bundle_name.starts_with("build/main/app-"),
            "Bundle path should start with build/main/app-"
        );
        assert!(bundle_name.ends_with(".js"), "Bundle should be a .js file");

        eprintln!("✓ Found bundle: {}", bundle_name);
    }

    /// Verify the new `wrap_async_body` + inline-userInfo emit byte-identical
    /// output to the legacy hardcoded constructor body. Guards against drift
    /// during refactors.
    #[test]
    fn esm_constructor_body_matches_legacy() {
        let legacy = r#"{
        super();
        this.#logger = loggerFactory.createLogger("AuthService");
        this.#windowActions = windowActions;
        this.#runtimePowerMonitor = runtimePowerMonitor;
        this.#gitCredentialManager = gitCredentialManager;
        this.#tokenManager = tokenManager;
        this.codeVerifier = "";
        this.connectInfo = {
            error: false,
            initialized: false,
            loggedIn: false,
            maintenance: false,
            online: true,
            ready: false,
            showLoginWindow: false,
            workOffline: false
        };
        this.tokenRefreshTimer = null;
        this.userInfoRefreshTimer = null;
        this.userInfo = {
            foreignKey: "unifuck-fk",
            name: "Unifuck",
            email: "bypass@local",
            primaryOrg: "local",
            createdAt: "1970-01-01T00:00:00.000Z",
        };
    }"#;
        let built = wrap_async_body(&format!(
            "        super();
        this.#logger = loggerFactory.createLogger(\"AuthService\");
        this.#windowActions = windowActions;
        this.#runtimePowerMonitor = runtimePowerMonitor;
        this.#gitCredentialManager = gitCredentialManager;
        this.#tokenManager = tokenManager;
        this.codeVerifier = \"\";
        this.connectInfo = {{
            error: false,
            initialized: false,
            loggedIn: false,
            maintenance: false,
            online: true,
            ready: false,
            showLoginWindow: false,
            workOffline: false
        }};
        this.tokenRefreshTimer = null;
        this.userInfoRefreshTimer = null;
        this.userInfo = {{
{FAKE_USER_FIELDS}
        }};"
        ));
        assert_eq!(built, legacy);
    }
}
