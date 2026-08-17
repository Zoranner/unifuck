use crate::hub::asar::{self, AsarArchive};
use crate::hub::patcher::replace_fn_body;
use crate::hub::payloads::{wrap_async_body, FAKE_USER_FIELDS};
use crate::hub::strategy::PatchStrategy;
use anyhow::Context;

/// Patch strategy for Unity Hub 3.20+ (Electron Forge Vite bundle format).
///
/// Characteristics:
/// - Code lives under `/.vite/build/` with content-hash file names
///   (e.g. `licenseQueryService-C2g--8Il.js`), so targets are matched by
///   file-name prefix, never exact names.
/// - Each service is its own chunk; the same method may exist in several
///   chunks (e.g. `isLicenseValid` in both LicenseQueryService and
///   LicenseService), so patches are applied per chunk with no ambiguity.
/// - `build/main` does not exist in this format (that marker belongs to the
///   older ESM/CommonJS layouts and keeps the strategies mutually exclusive).
pub(crate) struct ViteStrategy;

impl ViteStrategy {
    /// Find the first file under `.vite/build` whose base name starts with
    /// `prefix` (e.g. `licenseQueryService-`). Hash suffixes make exact names
    /// drift between Hub patch releases, so prefix matching is required.
    fn find_chunk(asar: &AsarArchive, prefix: &str) -> Option<(String, serde_json::Value)> {
        asar::walk_asar(&asar.header_json)
            .into_iter()
            .find(|(path, _)| {
                path.strip_prefix(".vite/build/")
                    .and_then(|rest| rest.split('/').next())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".js"))
            })
    }

    fn chunk_content(asar: &AsarArchive, path: &str) -> anyhow::Result<String> {
        let entry = asar::find_entry(&asar.header_json, path)
            .with_context(|| format!("chunk entry not found: {path}"))?;
        let bytes = asar::extract_file(asar, &entry)
            .with_context(|| format!("chunk extract failed: {path}"))?;
        String::from_utf8(bytes).with_context(|| format!("chunk is not valid UTF-8: {path}"))
    }

    fn patch_chunk(
        asar: &AsarArchive,
        prefix: &str,
        patch_fn: impl Fn(&str) -> anyhow::Result<String>,
    ) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let (path, _) = Self::find_chunk(asar, prefix)
            .with_context(|| format!("vite chunk not found: .vite/build/{prefix}*.js"))?;
        let content = Self::chunk_content(asar, &path)?;
        let patched =
            patch_fn(&content).with_context(|| format!("patch failed for chunk {path}"))?;
        Ok(vec![(path, patched.into_bytes())])
    }
}

impl PatchStrategy for ViteStrategy {
    fn version_name(&self) -> &'static str {
        "Vite (3.20+)"
    }

    fn applies_to(&self, asar: &AsarArchive) -> bool {
        // Vite marker: `.vite/build` exists and the older `build/main` layout
        // does not. Both conditions keep this disjoint from ESM/CommonJS.
        let has_vite = asar::walk_asar(&asar.header_json)
            .iter()
            .any(|(path, _)| path.starts_with(".vite/build/") && path.ends_with(".js"));
        let has_legacy_main = asar::find_entry(&asar.header_json, "build/main").is_some();
        has_vite && !has_legacy_main
    }

    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let mut modified = Vec::new();

        // ── LicenseQueryService (the real license gate) ──────────────
        // `isLicenseValid()` consults `licensingSdk.getInstance()`; with the
        // SDK not initialized it returns false, blocking all projects.
        // `getValidEntitlementGroups()` feeds getLicense(); with the SDK not
        // initialized it returns null and no licenses are listed. Both live in
        // the same chunk, so they are patched in one pass to avoid clobbering.
        let patch_license_query = |content: &str| -> anyhow::Result<String> {
            let mut patched = replace_fn_body(
                content,
                "isLicenseValid(",
                &wrap_async_body("        return true;"),
            )
            .context("patch isLicenseValid failed (licenseQueryService)")?;

            let fake = wrap_async_body(
                "        return [{
                    productName: 'Unity Pro',
                    entitlementGroupId: 'unifuck-pro',
                    licenseType: import_lib.LicenseType.ULF,
                    startDate: new Date('1970-01-01T00:00:00.000Z'),
                    expirationDate: '',
                    updateDate: new Date('1970-01-01T00:00:00.000Z'),
                    type: 'ULF',
                    organizationName: 'local',
                    validationErrors: [],
                }];",
            );
            patched = replace_fn_body(&patched, "getValidEntitlementGroups(", &fake)
                .context("patch getValidEntitlementGroups failed (licenseQueryService)")?;
            Ok(patched)
        };
        modified.extend(Self::patch_chunk(
            asar,
            "licenseQueryService-",
            patch_license_query,
        )?);

        // ── LicenseService (delegates to LicenseQueryService) ────────
        // Short-circuit the outer `isLicenseValid()` so the initial-activation
        // settle path is never awaited.
        let patch_license_svc = |content: &str| -> anyhow::Result<String> {
            replace_fn_body(
                content,
                "isLicenseValid(",
                &wrap_async_body("        return true;"),
            )
            .context("patch isLicenseValid failed (licenseService)")
        };
        modified.extend(Self::patch_chunk(
            asar,
            "licenseService-",
            patch_license_svc,
        )?);

        // ── AuthService (sign-in state) ─────────────────────────────
        // `isUserSignedIn()` gates everything user-related; force true so the
        // offline flags below are not immediately contradicted. All four
        // patches below target the same chunk and must run in one pass:
        // each `replace_fn_body` consumes the previous output, otherwise the
        // second pass reads the original bytes and the first patch is lost.
        let patch_auth = |content: &str| -> anyhow::Result<String> {
            let mut patched = replace_fn_body(content, "isUserSignedIn(", "{ return true; }")
                .context("patch isUserSignedIn failed (AuthService)")?;

            // `restoreAuthState()` validates tokens and logs out when they are
            // missing. Reuse the class's own `setOfflineLoggedInFlags()` which
            // sets the connect info and broadcasts the same events the legacy
            // ESM patch used to emit by hand.
            patched = replace_fn_body(
                &patched,
                "restoreAuthState(",
                &wrap_async_body("        this.setOfflineLoggedInFlags();"),
            )
            .context("patch restoreAuthState failed (AuthService)")?;

            // `getOrganizations()` calls cloudCore without a timeout and hangs
            // in airgapped environments; return an empty list instead.
            let new_orgs = wrap_async_body(
                "        this.#logger.debug(\"Organization data has been requested from AuthService\");
        if (!this.connectInfo.loggedIn) throw new Error(\"User is not logged in\");
        this.#logger.debug(\"Returning empty organization list (offline mode)\");
        this.userOrgs = [];
        return this.userOrgs;",
            );
            patched = replace_fn_body(&patched, "getOrganizations(", &new_orgs)
                .context("patch getOrganizations failed (AuthService)")?;

            // Inject fake userInfo into the constructor so `isUserSignedIn()`
            // and `getFormattedUserInfo()` have consistent data. The 3.20.1
            // constructor is minified into `super(), this.x = ...,` chains and
            // its body also carries initialization for many private fields, so
            // rebuilding it wholesale would drop those assignments. Instead we
            // insert the fake userInfo before the constructor's last
            // initializer line, keeping every original statement intact.
            //
            // The anchor is whitespace-agnostic: the bundle indents with tabs,
            // the test fixtures with spaces. `userInfoRefreshTimer = null;`
            // first occurs in the constructor (later occurrences are timer
            // helpers), and the immediately following
            // `tokenRefreshBackOffTimer = null;` line is the constructor tail.
            let user_info = format!(
                "\t\tthis.userInfo = {{
{FAKE_USER_FIELDS}
\t\t\tcached: true,
\t\t\thasValidUnityToken: false,
\t\t}};\n"
            );
            let refresh_idx = patched
                .find("this.userInfoRefreshTimer = null;")
                .context("constructor userInfoRefreshTimer anchor not found (AuthService)")?;
            let tail_idx = patched[refresh_idx..]
                .find("this.tokenRefreshBackOffTimer = null;")
                .map(|i| refresh_idx + i)
                .context("constructor tokenRefreshBackOffTimer anchor not found (AuthService)")?;
            // Insert at the start of the tokenRefreshBackOffTimer line.
            let line_start = patched[..tail_idx]
                .rfind('\n')
                .map(|i| i + 1)
                .unwrap_or(tail_idx);
            patched.insert_str(line_start, &user_info);

            Ok(patched)
        };
        modified.extend(Self::patch_chunk(asar, "AuthService-", patch_auth)?);

        // ── MainWindow (early startup route) ──────────────────────
        // 3.20.1 picks the initial window route synchronously at window
        // construction via `readIsSignedInSync()` (accounts.db + keyring
        // lookup, run before AuthService exists). A bypassed install has no
        // real tokens, so the first frame lands on the WELCOME sign-in screen
        // and only flips to the projects route seconds later once
        // `initializeUI()` sees the patched `isUserSignedIn()`. Force the
        // snapshot read to true so the very first frame already resolves the
        // signed-in route; the inserted early return leaves the rest of the
        // body intact (unreachable but harmless).
        let patch_main_window = |content: &str| -> anyhow::Result<String> {
            let anchor = "var readIsSignedInSync = () => {";
            let idx = content
                .find(anchor)
                .context("readIsSignedInSync anchor not found (mainWindow)")?;
            let mut patched = String::with_capacity(content.len() + 16);
            patched.push_str(&content[..idx + anchor.len()]);
            patched.push_str("\n\t\treturn true;");
            patched.push_str(&content[idx + anchor.len()..]);
            Ok(patched)
        };
        modified.extend(Self::patch_chunk(asar, "mainWindow-", patch_main_window)?);

        // ── FeatureFlagsService (Amplitude network call) ────────────
        // `fetchVariants()` performs an HTTP round-trip with a 15s timeout +
        // retries, stalling startup in airgapped environments. Mirror the ESM
        // offline strategy: resolve from cache only.
        let patch_flags = |content: &str| -> anyhow::Result<String> {
            let new_fetch = wrap_async_body(
                "        this.#logger.info(\"Fetching feature flags (skipped - offline mode)\");
        const user = await this.getExperimentUser();
        if (!user.user_id && !user.device_id) {
            this.#logger.info(\"User info and licensing info not complete, skipping fetching feature flags for now.\");
            return;
        }
        const { amplitude: cachedAmplitudeVariants } = this.#cache.cachedSchema;
        if (cachedAmplitudeVariants && Object.keys(cachedAmplitudeVariants).length > 0) {
            this.handleNewVariants({
                previous: this.variants,
                next: cachedAmplitudeVariants
            });
            this.#logger.info(\"Using cached feature flags\");
        }",
            );
            replace_fn_body(content, "fetchVariants(", &new_fetch)
                .context("patch fetchVariants failed (FeatureFlagsService)")
        };
        modified.extend(Self::patch_chunk(
            asar,
            "FeatureFlagsService-",
            patch_flags,
        )?);

        Ok(modified)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造含任意数量 chunk 的 Vite 布局 asar。每个 chunk 内容按给定顺序
    /// 拼进 data 区, offset 依次累加(与 write_asar 输出格式一致)。
    fn make_archive(chunks: &[(&str, &str)]) -> AsarArchive {
        let mut data = Vec::new();
        let mut files = serde_json::Map::new();
        for (name, content) in chunks {
            let offset = data.len();
            data.extend_from_slice(content.as_bytes());
            files.insert(
                (*name).to_string(),
                json!({
                    "size": content.len(),
                    "offset": offset.to_string(),
                    "integrity": {}
                }),
            );
        }
        AsarArchive {
            header_json: json!({
                "files": {
                    ".vite": {
                        "files": {
                            "build": { "files": files }
                        }
                    }
                }
            }),
            data,
        }
    }

    #[test]
    fn find_chunk_matches_by_prefix() {
        let asar = make_archive(&[(
            "licenseQueryService-C2g--8Il.js",
            "async isLicenseValid() {}",
        )]);
        let (path, _) = ViteStrategy::find_chunk(&asar, "licenseQueryService-").unwrap();
        assert_eq!(path, ".vite/build/licenseQueryService-C2g--8Il.js");
        assert!(ViteStrategy::find_chunk(&asar, "noSuchChunk-").is_none());
    }

    #[test]
    fn applies_to_requires_vite_and_no_legacy_main() {
        let asar = make_archive(&[(
            "licenseQueryService-C2g--8Il.js",
            "async isLicenseValid() {}",
        )]);
        assert!(ViteStrategy.applies_to(&asar));

        // legacy main present → 不命中 (ESM/CommonJS 布局)
        let legacy = json!({
            "files": {
                ".vite": { "files": { "build": { "files": {
                    "x.js": { "size": 1, "offset": "0" }
                }}}},
                "build": { "files": { "main": { "files": {} } } }
            }
        });
        let legacy_asar = AsarArchive {
            header_json: legacy,
            data: vec![b'x'],
        };
        assert!(!ViteStrategy.applies_to(&legacy_asar));
    }

    /// 其余 chunk 的最小可 patch fixture, 与真实 3.20.1 形态一致。
    fn other_chunks() -> Vec<(&'static str, &'static str)> {
        vec![
            (
                "licenseService-DzjY7Re0.js",
                r#"class LicenseService {
    async isLicenseValid() {
        if (await this.#licenseQueryService.isLicenseValid()) return true;
        await this.ensureInitialActivationSettled();
        return this.#licenseQueryService.isLicenseValid();
    }
}"#,
            ),
            (
                "AuthService-C6DcJIUT.js",
                r#"class AuthService {
    constructor(loggerFactory, windowActions, runtimePowerMonitor, gitCredentialManager, tokenManager, licenseEulaManager, translate, cloudConfig, localConfig, eventBroadcaster, localSettings, userSettings, cloudCore) {
        super();
        this.#logger = loggerFactory.createLogger("AuthService");
        this.#windowActions = windowActions;
        this.#runtimePowerMonitor = runtimePowerMonitor;
        this.#gitCredentialManager = gitCredentialManager;
        this.#tokenManager = tokenManager;
        this.#licenseEulaManager = licenseEulaManager;
        this.#translate = translate;
        this.#cloudConfig = cloudConfig;
        this.#localConfig = localConfig;
        this.#eventBroadcaster = eventBroadcaster;
        this.#localSettings = localSettings;
        this.#userSettings = userSettings;
        this.#cloudCore = cloudCore;
        this.codeVerifier = "";
        this.connectInfo = { error: false, initialized: false, loggedIn: false };
        this.tokenRefreshTimer = null;
        this.userInfoRefreshTimer = null;
        this.tokenRefreshBackOffTimer = null;
    }
    isUserSignedIn() {
        return Boolean(this.userInfo?.foreignKey);
    }
    async restoreAuthState() {
        const { accessToken, refreshToken } = this.#tokenManager.getTokens();
        if (accessToken && refreshToken) {
            return;
        }
        await this.logout({ skipGitPATClear: true });
    }
    async getOrganizations() {
        this.#logger.debug("Organization data has been requested from AuthService");
        if (!this.connectInfo.loggedIn) throw new Error("User is not logged in");
        if (this.userOrgs !== null) return this.userOrgs;
        this.userOrgs = await this.#cloudCore.getOrganizations();
        return this.userOrgs;
    }
}"#,
            ),
            (
                "mainWindow-Dn-9pF3L.js",
                r#"var readIsSignedInSync = () => {
	try {
		const store = createCredentialStore(logger);
		const raw = store.get(KEYRING_SERVICE, authTokensAccount(activeForeignKey));
		if (!raw) return false;
		return true;
	} catch {
		return false;
	}
};
"#,
            ),
            (
                "FeatureFlagsService-C0yCzF8k.js",
                r#"class FeatureFlagsService {
    async fetchVariants() {
        this.#logger.info("Fetching feature flags");
        const user = await this.getExperimentUser();
        if (!user.user_id && !user.device_id) {
            return;
        }
        const newVariants = await this.client.fetchV2(user);
        this.handleNewVariants({ previous: this.variants, next: newVariants });
    }
}"#,
            ),
        ]
    }

    #[test]
    fn patch_license_query_service() {
        let content = r#"class LicenseQueryService {
    async isLicenseValid() {
        this.#logger.debug("isLicenseValid");
        if (this.isFloatingEnabled()) return true;
        try {
            const instance = this.#licensingSdk.getInstance();
            if (!instance) return false;
            return await instance.checkEntitlement(useEditorUi);
        } catch (err) {
            return false;
        }
    }
    async getValidEntitlementGroups() {
        const instance = this.#licensingSdk.getInstance();
        if (!instance) return null;
        return instance.getAllEntitlementGroups().then((response) => response);
    }
}"#;
        let mut chunks = other_chunks();
        chunks.insert(0, ("licenseQueryService-C2g--8Il.js", content));
        let asar = make_archive(&chunks);
        let modified = ViteStrategy.apply_patches(&asar).unwrap();
        // licenseQueryService + licenseService + AuthService + mainWindow + FeatureFlagsService
        assert_eq!(modified.len(), 5);
        let query = modified
            .iter()
            .find(|(path, _)| path.contains("licenseQueryService-"))
            .unwrap();
        let patched = String::from_utf8(query.1.clone()).unwrap();
        assert!(
            patched.contains("return true;"),
            "isLicenseValid should return true"
        );
        assert!(patched.contains("entitlementGroupId: 'unifuck-pro'"));
    }

    #[test]
    fn patch_auth_service() {
        // 与真实 3.20.1 一致: constructor 用 tab 缩进 + minified 逗号链
        // 初始化; 断言 patch 后这些初始化语句全部存活 (旧实现重写整个
        // body 会丢光它们, 导致 Hub 启动即崩)。
        let content = "class AuthService {\n\tconstructor(loggerFactory, windowActions, runtimePowerMonitor, gitCredentialManager, tokenManager, licenseEulaManager, translate, cloudConfig, localConfig, eventBroadcaster, localSettings, userSettings, cloudCore) {\n\t\tsuper(), this.tokenRefreshAttemptCount = -1, this.authCodeVerifiersByState = new Map();\n\t\tthis.#logger = loggerFactory.createLogger(\"AuthService\");\n\t\tthis.#windowActions = windowActions;\n\t\tthis.codeVerifier = \"\";\n\t\tthis.connectInfo = { error: false, loggedIn: false };\n\t\tthis.tokenRefreshTimer = null;\n\t\tthis.userInfoRefreshTimer = null;\n\t\tthis.tokenRefreshBackOffTimer = null;\n\t}\n\tisUserSignedIn() {\n\t\treturn Boolean(this.userInfo?.foreignKey);\n\t}\n\tasync restoreAuthState() {\n\t\tconst { accessToken, refreshToken } = this.#tokenManager.getTokens();\n\t\tif (accessToken && refreshToken) {\n\t\t\treturn;\n\t\t}\n\t\tawait this.logout({ skipGitPATClear: true });\n\t}\n\tasync getOrganizations() {\n\t\tthis.#logger.debug(\"Organization data has been requested from AuthService\");\n\t\tif (!this.connectInfo.loggedIn) throw new Error(\"User is not logged in\");\n\t\tif (this.userOrgs !== null) return this.userOrgs;\n\t\tthis.userOrgs = await this.#cloudCore.getOrganizations();\n\t\treturn this.userOrgs;\n\t}\n}";
        let mut chunks = other_chunks();
        // AuthService 用本测试的完整 fixture 替换 other_chunks 里的最小版
        chunks.retain(|(name, _)| !name.starts_with("AuthService-"));
        chunks.push(("AuthService-C6DcJIUT.js", content));
        // apply_patches 需要全部 4 个 chunk; licenseQueryService 在
        // other_chunks 中没有, 补一个最小可 patch 版
        chunks.push((
            "licenseQueryService-C2g--8Il.js",
            r#"class LicenseQueryService {
    async isLicenseValid() {
        if (this.isFloatingEnabled()) return true;
        return false;
    }
    async getValidEntitlementGroups() {
        return null;
    }
}"#,
        ));
        let asar = make_archive(&chunks);
        let modified = ViteStrategy.apply_patches(&asar).unwrap();
        // licenseQueryService + licenseService + AuthService + mainWindow + FeatureFlagsService
        assert_eq!(modified.len(), 5);
        let auth = modified
            .iter()
            .find(|(path, _)| path.contains("AuthService-"))
            .unwrap();
        let patched = String::from_utf8(auth.1.clone()).unwrap();
        assert!(patched.contains("foreignKey: \"unifuck-fk\""));
        assert!(patched.contains("setOfflineLoggedInFlags()"));
        assert!(
            patched.contains("Returning empty organization list (offline mode)"),
            "getOrganizations should be offline"
        );
        // 回归: minified 逗号链初始化必须存活, 不能随 constructor 重写丢失
        assert!(
            patched.contains("super(), this.tokenRefreshAttemptCount = -1"),
            "constructor minified init chain must survive"
        );
        assert!(
            patched.contains("this.tokenRefreshBackOffTimer = null;"),
            "constructor tail assignment must survive"
        );
    }

    #[test]
    fn patch_main_window_read_is_signed_in_snapshot() {
        let mut chunks = other_chunks();
        chunks.push((
            "licenseQueryService-C2g--8Il.js",
            r#"class LicenseQueryService {
    async isLicenseValid() {
        if (this.isFloatingEnabled()) return true;
        return false;
    }
    async getValidEntitlementGroups() {
        return null;
    }
}"#,
        ));
        let asar = make_archive(&chunks);
        let modified = ViteStrategy.apply_patches(&asar).unwrap();
        let main_window = modified
            .iter()
            .find(|(path, _)| path.contains("mainWindow-"))
            .unwrap();
        let patched = String::from_utf8(main_window.1.clone()).unwrap();
        // 首帧路由快照: readIsSignedInSync 开头立即返回 true, 剩余原逻辑保留
        assert!(
            patched.contains("var readIsSignedInSync = () => {\n\t\treturn true;"),
            "early return must be injected right after the arrow body opens"
        );
        assert!(
            patched.contains("createCredentialStore(logger)"),
            "original snapshot logic must survive"
        );
    }
}
