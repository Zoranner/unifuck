use super::PatchStrategy;
use crate::hub::asar::{self, AsarArchive};
use crate::hub::patcher::replace_fn_body;
use anyhow::Context;

/// Patch strategy for CommonJS separate files format
///
/// Characteristics:
/// - CommonJS format (`require()`, `module.exports`)
/// - Separate files for each service
/// - Uses `__awaiter` for async functions
/// - Files: `licenseService.js`, `licenseServiceCore.js`, `licensingSdk.js`, `AuthService.js`
pub(crate) struct CommonJsStrategy;

impl CommonJsStrategy {
    const TARGET_FILES: &'static [&'static str] = &[
        "build/main/services/licenseService/licenseService.js",
        "build/main/services/licenseService/licenseServiceCore.js",
        "build/main/services/licenseService/licensingSdk.js",
        "build/main/services/authService/AuthService.js",
    ];
}

impl PatchStrategy for CommonJsStrategy {
    fn version_name(&self) -> &'static str {
        "CommonJS (separate files)"
    }

    fn applies_to(&self, asar: &AsarArchive) -> bool {
        // Check for separate service files (CommonJS structure)
        for target in Self::TARGET_FILES {
            if asar::find_entry(&asar.header_json, target).is_some() {
                return true;
            }
        }
        false
    }

    fn apply_patches(&self, asar: &AsarArchive) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
        let mut modified = Vec::new();

        for target in Self::TARGET_FILES {
            let entry = match asar::find_entry(&asar.header_json, target) {
                Some(e) => e,
                None => continue,
            };

            let content_bytes = match asar::extract_file(asar, &entry) {
                Some(b) => b,
                None => continue,
            };
            let content = String::from_utf8(content_bytes)
                .with_context(|| format!("non-UTF8 content in {}", target))?;

            let patched = match *target {
                t if t.ends_with("licenseService.js") => patch_license_service_js(&content),
                t if t.ends_with("licenseServiceCore.js") => patch_license_core_js(&content),
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

// ── Patch Functions for 3.5/3.7 ──────────────────────────────

fn patch_license_service_js(content: &str) -> anyhow::Result<String> {
    let new_body = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
    replace_fn_body(content, "isLicenseValid(", new_body).context("patch isLicenseValid failed")
}

fn patch_license_core_js(content: &str) -> anyhow::Result<String> {
    // Return shape must match convertEntitlementGroupsIntoLicenses() which
    // reads: productName, entitlementGroupId, startDate, expirationDate,
    // updateDate, licenseType, validationErrors.
    // Missing entitlementGroupId → undefined.startsWith() crash in isEGL().
    let new_body = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return [{
                startDate: new Date('1970-01-01T00:00:00.000Z'),
                expirationDate: new Date('9999-01-01T08:00:00.000Z'),
                updateDate: new Date('2024-01-01T08:00:00.000Z'),
                productName: 'Unifuck Pro Max Ultra',
                licenseType: 'ULF',
                entitlementGroupId: 'U4-Unifuck-pro-001',
                validationErrors: [],
            }];
        });
    }"#;
    replace_fn_body(content, "getValidEntitlementGroups(", new_body)
        .context("patch getValidEntitlementGroups failed")
}

fn patch_licensing_sdk_js(content: &str) -> anyhow::Result<String> {
    let new_init = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
    let result = replace_fn_body(content, "init(externalCorrelationId", new_init)
        .context("patch init(externalCorrelationId) failed")?;

    // Return a stub instead of null — editorapp.js calls
    // `licensingSdk.getInstance().getConnectedPipeName()` without try/catch,
    // so returning null causes a TypeError that crashes project launch.
    let new_get_instance = r#"{
        return {
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
        };
    }"#;
    replace_fn_body(&result, "getInstance(", new_get_instance).context("patch getInstance failed")
}

fn patch_auth_service_js(content: &str) -> anyhow::Result<String> {
    // 1. Replace isUserSignedIn() → return true (3.7+ only)
    let new_signed_in = r#"{
        return true;
    }"#;
    let after_signed_in = match replace_fn_body(content, "isUserSignedIn(", new_signed_in) {
        Some(s) => s,
        None => content.to_string(),
    };

    // 2. Replace constructor() — inject fake userInfo
    let new_constructor = r#"{
        super();
        this.userInfo = undefined;
        this.tokenRefreshAttemptCount = -1;
        this.codeVerifier = '';
        this.authLogger = (0, logger_1.default)('Authentication Service');
        this.connectInfo = {
            error: false,
            initialized: false,
            loggedIn: false,
            maintenance: false,
            online: true,
            ready: false,
            showLoginWindow: false,
            workOffline: false,
        };
        this.tokenRefreshTimer = null;
        this.userInfoRefreshTimer = null;
        this.userInfo = {
            identifier: 'unifuck-bypass',
            foreignKey: 'unifuck-fk',
            name: 'Unifuck',
            email: 'bypass@local',
            primaryOrg: 'local',
            createdAt: '2024-01-01T00:00:00.000Z',
        };
    }"#;
    let after_constructor = match replace_fn_body(&after_signed_in, "constructor(", new_constructor)
    {
        Some(s) => s,
        None => after_signed_in,
    };

    // 3. Replace restoreAuthState() — set loggedIn and broadcast events
    let new_restore = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            this.connectInfo.ready = true;
            this.connectInfo.loggedIn = true;
            this.connectInfo.initialized = true;
            this.connectInfo.workOffline = true;
            this.connectInfo.online = false;
            this.userOrgs = null;
            if (windowManager_1 && windowManager_1.default) {
                windowManager_1.default.broadcastContent(PostalTopic_1.default.CONNECT_INFO_CHANGED, this.connectInfo);
            }
            postal_1.default.publish({
                channel: 'app',
                topic: PostalTopic_1.default.CONNECT_INFO_CHANGED,
                data: { connectInfo: this.connectInfo },
            });
            postal_1.default.publish({
                channel: 'app',
                topic: PostalTopic_1.default.USER_INFO_CHANGED,
                data: { userInfo: this.getFormattedUserInfo() },
            });
            postal_1.default.publish({
                channel: 'fsm.user',
                topic: PostalTopic_1.default.LOGGED_IN,
            });
        });
    }"#;
    match replace_fn_body(&after_constructor, "restoreAuthState(", new_restore) {
        Some(s) => Ok(s),
        None => Ok(after_constructor),
    }
}
