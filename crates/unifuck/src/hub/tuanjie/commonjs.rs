use crate::hub::asar::{self, AsarArchive};
use crate::hub::patcher::replace_fn_body;
use crate::hub::payloads::{wrap_async_body, wrap_awaiter, FAKE_USER_FIELDS};
use crate::hub::strategy::PatchStrategy;
use anyhow::Context;
use serde_json::Value;

/// Tuanjie Hub patch strategy.
///
/// Bootstrap path in real Tuanjie 1.4.2 AuthService:
///   `init()` → `initNetworkInterceptors()` + `onNetworkUp()`;
///   `onNetworkUp()` short-circuits to `logout()` when no tokens exist,
///   so `setLoggedInFlags()` is never reached on an airgapped install.
///   Therefore `init()` itself must be replaced to call
///   `initNetworkInterceptors()` and then `setLoggedInFlags()` directly.
pub(crate) struct CommonJsStrategy;

impl CommonJsStrategy {
    const TARGET_FILES: &'static [&'static str] = &[
        "build/main/services/licenseService/licenseService.js",
        "build/main/services/licenseService/licensingSdk.js",
        "build/main/services/authService/AuthService.js",
    ];

    const TUANJIE_VERSION_MARKER: &'static str =
        "build/main/services/versionMapping/tuanjieVersionMapping.js";

    /// Check whether `package.json`'s `name` field is `"TuanjieHub"`.
    fn is_tuanjie_hub(asar: &AsarArchive) -> bool {
        let entry = match asar::find_entry(&asar.header_json, "package.json") {
            Some(e) => e,
            None => return false,
        };
        let data = match asar::extract_file(asar, &entry) {
            Some(d) => d,
            None => return false,
        };
        let content = match String::from_utf8(data) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let v: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return false,
        };
        v.get("name").and_then(|n| n.as_str()) == Some("TuanjieHub")
    }

    /// Fallback: check for the Tuanjie-specific version-mapping file.
    fn has_tuanjie_marker(asar: &AsarArchive) -> bool {
        asar::find_entry(&asar.header_json, Self::TUANJIE_VERSION_MARKER).is_some()
    }
}

impl PatchStrategy for CommonJsStrategy {
    fn version_name(&self) -> &'static str {
        "Tuanjie Hub"
    }

    fn applies_to(&self, asar: &AsarArchive) -> bool {
        if Self::is_tuanjie_hub(asar) {
            return true;
        }
        Self::has_tuanjie_marker(asar)
    }

    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let mut modified = Vec::new();

        for target in Self::TARGET_FILES {
            let entry = match asar::find_entry(&asar.header_json, target) {
                Some(e) => e,
                None => {
                    eprintln!("warning: target not found in asar: {}", target);
                    continue;
                }
            };

            let content_bytes = match asar::extract_file(asar, &entry) {
                Some(b) => b,
                None => {
                    eprintln!("warning: failed to extract {}", target);
                    continue;
                }
            };
            let content = String::from_utf8(content_bytes)
                .with_context(|| format!("non-UTF8 content in {}", target))?;

            let patched = match *target {
                t if t.ends_with("licenseService.js") => patch_license_service_js(&content),
                t if t.ends_with("licensingSdk.js") => patch_licensing_sdk_js(&content),
                t if t.ends_with("AuthService.js") => patch_auth_service_js(&content),
                _ => continue,
            };

            match patched {
                Ok(p) => modified.push((target.to_string(), p.into_bytes())),
                Err(e) => {
                    eprintln!("warning: skip {}: {}", target, e);
                }
            }
        }

        Ok(modified)
    }
}

// ── Patch Functions ────────────────────────────────────────────

/// Patches to `licenseService.js`:
/// 1. `isLicenseValid()` → return true.
/// 2. `getValidEntitlementGroups()` → return a fake entitlement array shaped
///    for `convertEntitlementGroupsIntoLicenses` (productName, licenseType,
///    startDate, expirationDate, updateDate, entitlementGroupId,
///    validationErrors). The original delegates to
///    `licensingSdk.getInstance().getAllEntitlementGroups()` which our SDK
///    stub returns empty, so Hub UI would show no license without this.
///
/// Each patch is applied independently.
fn patch_license_service_js(content: &str) -> anyhow::Result<String> {
    let mut current = content.to_string();
    let mut applied = 0usize;

    let new_is_valid = wrap_awaiter("            return true;");
    if let Some(s) = replace_fn_body(&current, "isLicenseValid(", &new_is_valid) {
        current = s;
        applied += 1;
    } else {
        eprintln!("warning: licenseService isLicenseValid signature missed");
    }

    let new_groups = wrap_awaiter(
        "            return [{
                startDate: new Date('1970-01-01T00:00:00.000Z'),
                expirationDate: new Date('9999-01-01T08:00:00.000Z'),
                updateDate: new Date('2024-01-01T08:00:00.000Z'),
                productName: 'Pro Max Ultra',
                licenseType: 'ULF',
                entitlementGroupId: 'U4-Unifuck-pro-001',
                validationErrors: [],
            }];",
    );
    if let Some(s) = replace_fn_body(&current, "getValidEntitlementGroups(", &new_groups) {
        current = s;
        applied += 1;
    } else {
        eprintln!("warning: licenseService getValidEntitlementGroups signature missed");
    }

    if applied == 0 {
        anyhow::bail!("no licenseService patches applied");
    }
    Ok(current)
}

/// Patches to `licensingSdk.js` are applied independently. Tuanjie does not
/// expose a zero-argument `getValidEntitlementGroups`, only
/// `getValidEntitlementGroupsByType(licenseTypes)`, so we skip it here.
///
/// `getInstance()` returns a fat stub rather than `null` because
/// `getValidEntitlementGroupsByType` (and possibly other call sites)
/// chains `.getAllEntitlementGroups()` without optional chaining, which
/// would throw on a null return.
fn patch_licensing_sdk_js(content: &str) -> anyhow::Result<String> {
    let mut current = content.to_string();
    let mut applied = 0usize;

    let new_init = wrap_awaiter("            return true;");
    match replace_fn_body(&current, "init(externalCorrelationId", &new_init) {
        Some(s) => {
            current = s;
            applied += 1;
        }
        None => eprintln!("warning: licensingSdk init(externalCorrelationId) signature missed"),
    }

    let stub = licensing_sdk_get_instance_stub();
    match replace_fn_body(&current, "getInstance(", &stub) {
        Some(s) => {
            current = s;
            applied += 1;
        }
        None => eprintln!("warning: licensingSdk getInstance signature missed"),
    }

    if applied == 0 {
        anyhow::bail!("no licensingSdk patches applied");
    }
    Ok(current)
}

/// Fat stub mirroring the Unity CommonJS strategy — safe defaults so that
/// `getInstance().getAllEntitlementGroups()` and similar chains do not throw.
fn licensing_sdk_get_instance_stub() -> String {
    wrap_async_body(
        "        return {
            getConnectedPipeName() { return ''; },
            connect() { return Promise.resolve(false); },
            dispose() { return Promise.resolve(); },
            getMachineId() { return Promise.resolve(''); },
            getCorrelationId() { return Promise.resolve(''); },
            getAllEntitlementGroups() { return Promise.resolve({ entitlementGroupsDetails: [] }); },
            checkEntitlement() { return Promise.resolve(true); },
            checkFeatureStatus() { return Promise.resolve({ featureStatus: 0 }); },
            importLicense() { return Promise.resolve({ isSuccessful: () => false, responseCode: 0, responseStatus: 'stub' }); },
            generateUnityAlf() { return Promise.resolve({ isSuccessful: () => false, responseCode: 0, responseStatus: 'stub' }); },
            activateUlfLicense() { return Promise.resolve({ isSuccessful: () => false, responseCode: 0, responseStatus: 'stub' }); },
            returnEntitlementGroup() { return Promise.resolve({ isSuccessful: () => false, responseCode: 0, responseStatus: 'stub', returnEntitlementGroupResults: [{ statusCode: 500 }] }); },
            borrowLicense() { return Promise.resolve({ licenseError: 'Generic' }); },
            returnBorrow() { return Promise.resolve({ isSuccessful: () => false, responseCode: 0, responseStatus: 'stub' }); },
            activateAllEntitlementBasedLicenses() { return Promise.resolve({ isSuccessful: () => true }); },
            updateLicenses() { return Promise.resolve({ isSuccessful: () => true, responseCode: 0, responseStatus: 'stub' }); },
            setAccessToken() { return Promise.resolve(); },
        };",
    )
}

/// Patches to `AuthService.js`:
/// 1. `constructor()` — preserve original field init, append fake userInfo
///    (with `identifier` so any caller that bypasses the patched
///    `isUserSignedIn` still sees a signed-in user).
/// 2. `isUserSignedIn()` — return true (real Tuanjie checks
///    `Boolean(userInfo?.identifier)`).
/// 3. `init()` — replace body so the token-check branch in
///    `onNetworkUp()` (which calls `logout()` on airgapped installs) is
///    skipped; we still call `initNetworkInterceptors()` so the rest of
///    the app sees network state events, then call the ORIGINAL
///    `setLoggedInFlags()` (we do not patch it — Tuanjie's own body
///    already emits the full 3× windowManager + cloudAnalytics + 3×
///    postal_1 broadcast set the renderer subscribes to).
///
/// Each patch is applied independently — a missed signature on one
/// function does not roll back the others.
fn patch_auth_service_js(content: &str) -> anyhow::Result<String> {
    let mut current = content.to_string();
    let mut applied = 0usize;

    let new_constructor = wrap_async_body(&format!(
        "        super();
        this.userInfo = undefined;
        this.codeVerifier = '';
        this.authLogger = (0, logger_1.default)('Authentification Service');
        this.connectInfo = {{
            error: false,
            initialized: false,
            loggedIn: false,
            maintenance: false,
            online: true,
            ready: false,
            showLoginWindow: false,
            workOffline: false,
        }};
        this.tokenWatcher = null;
        this.userInfo = {{
{FAKE_USER_FIELDS}
            identifier: \"unifuck-id\",
        }};"
    ));
    if let Some(s) = replace_fn_body(&current, "constructor(", &new_constructor) {
        current = s;
        applied += 1;
    } else {
        eprintln!("warning: AuthService constructor signature missed");
    }

    let new_is_signed_in = wrap_async_body("        return true;");
    if let Some(s) = replace_fn_body(&current, "isUserSignedIn(", &new_is_signed_in) {
        current = s;
        applied += 1;
    } else {
        eprintln!("warning: AuthService isUserSignedIn signature missed");
    }

    let new_init = wrap_awaiter(
        "            this.authLogger.info('Initializing the auth service');
            this.initNetworkInterceptors();
            this.setLoggedInFlags();
            return this.userInfo;",
    );
    if let Some(s) = replace_fn_body(&current, "init(", &new_init) {
        current = s;
        applied += 1;
    } else {
        eprintln!("warning: AuthService init() signature missed");
    }

    if applied == 0 {
        anyhow::bail!("no AuthService patches applied");
    }
    Ok(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_service_is_license_valid_wrap() {
        let built = wrap_awaiter("            return true;");
        assert!(built.contains("return __awaiter"));
        assert!(built.contains("return true;"));
    }

    #[test]
    fn licensing_sdk_get_instance_stub_has_safe_defaults() {
        let stub = licensing_sdk_get_instance_stub();
        // Critical: getAllEntitlementGroups is called without optional chaining
        // in Tuanjie's getValidEntitlementGroupsByType, must not return null.
        assert!(stub.contains("getAllEntitlementGroups()"));
        assert!(stub.contains("entitlementGroupsDetails: []"));
    }

    #[test]
    fn auth_service_constructor_injects_identifier() {
        // identifier field is required because real Tuanjie's isUserSignedIn
        // checks Boolean(userInfo?.identifier).
        let built = wrap_async_body(&format!(
            "        super();
        this.userInfo = {{
{FAKE_USER_FIELDS}
            identifier: \"unifuck-id\",
        }};"
        ));
        assert!(built.contains("identifier: \"unifuck-id\""));
        assert!(built.contains("foreignKey: \"unifuck-fk\""));
    }

    #[test]
    fn auth_service_init_calls_set_logged_in_flags() {
        let built = wrap_awaiter(
            "            this.authLogger.info('Initializing the auth service');
            this.initNetworkInterceptors();
            this.setLoggedInFlags();
            return this.userInfo;",
        );
        assert!(built.contains("this.initNetworkInterceptors();"));
        assert!(built.contains("this.setLoggedInFlags();"));
    }
}
