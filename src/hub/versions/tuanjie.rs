use super::PatchStrategy;
use crate::hub::asar::{self, AsarArchive};
use crate::hub::patcher::replace_fn_body;
use anyhow::Context;
use serde_json::Value;

/// Tuanjie Hub 补丁策略
///
/// 特征:
/// - 基于 Unity Hub CommonJS 架构，但 package.json name 为 "TuanjieHub"
/// - 文件结构与 CommonJS 策略类似，但 AuthService 构造函数和认证流程有差异
/// - 使用 `setLoggedInFlags()` 替代 Unity Hub 的 `restoreAuthState()`
pub(crate) struct TuanjieStrategy;

impl TuanjieStrategy {
    const TARGET_FILES: &'static [&'static str] = &[
        "build/main/services/licenseService/licenseService.js",
        "build/main/services/licenseService/licensingSdk.js",
        "build/main/services/authService/AuthService.js",
    ];

    const TUANJIE_VERSION_MARKER: &'static str =
        "build/main/services/versionMapping/tuanjieVersionMapping.js";

    /// 检查 package.json 中的 name 字段是否为 "TuanjieHub"
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

    /// Fallback: 检查是否存在 Tuanjie 特有的版本映射文件
    fn has_tuanjie_marker(asar: &AsarArchive) -> bool {
        asar::find_entry(&asar.header_json, Self::TUANJIE_VERSION_MARKER).is_some()
    }
}

impl PatchStrategy for TuanjieStrategy {
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

fn patch_license_service_js(content: &str) -> anyhow::Result<String> {
    let new_body = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
    replace_fn_body(content, "isLicenseValid(", new_body).context("patch isLicenseValid failed")
}

fn patch_licensing_sdk_js(content: &str) -> anyhow::Result<String> {
    // 1. init(externalCorrelationId) → return true
    let new_init = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
    let after_init = replace_fn_body(content, "init(externalCorrelationId", new_init)
        .context("patch init(externalCorrelationId) failed")?;

    // 2. getInstance() → return null
    let new_get_instance = r#"{
        return null;
    }"#;
    let after_instance = replace_fn_body(&after_init, "getInstance(", new_get_instance)
        .context("patch getInstance failed")?;

    // 3. getValidEntitlementGroups() → 返回伪造的教育许可证
    let new_groups = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return [{
                startDate: new Date('1970-01-01T00:00:00.000Z'),
                expirationDate: new Date('9999-01-01T00:00:00.000Z'),
                productName: 'Education',
                licenseType: 'ULF',
            }];
        });
    }"#;
    replace_fn_body(&after_instance, "getValidEntitlementGroups(", new_groups)
        .context("patch getValidEntitlementGroups failed")
}

fn patch_auth_service_js(content: &str) -> anyhow::Result<String> {
    // 1. constructor() — 保留原始初始化逻辑，末尾注入 fake userInfo
    let new_constructor = r#"{
        super();
        this.userInfo = undefined;
        this.codeVerifier = '';
        this.authLogger = (0, logger_1.default)('Authentification Service');
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
        this.tokenWatcher = null;
        this.userInfo = {
            foreignKey: "unifuck-fk",
            name: "Unifuck",
            email: "bypass@local",
            primaryOrg: "local",
            createdAt: "1970-01-01T00:00:00.000Z",
        };
    }"#;
    let after_constructor = match replace_fn_body(content, "constructor(", new_constructor) {
        Some(s) => s,
        None => content.to_string(),
    };

    // 2. setLoggedInFlags() — 设置 loggedIn/ready 为 true 并广播事件
    let new_flags = r#"{
        this.connectInfo.ready = true;
        this.connectInfo.loggedIn = true;
        this.connectInfo.initialized = true;
        this.connectInfo.workOffline = false;
        this.userOrgs = null;
        windowManager_1.default.broadcastContent(PostalTopic_1.default.CONNECT_INFO_CHANGED, this.connectInfo);
        windowManager_1.default.broadcastContent(PostalTopic_1.default.USER_INFO_CHANGED, this.getFormattedUserInfo());
        windowManager_1.default.broadcastContent(RendererEvents_1.default.USER_SIGN_IN, this.userInfo);
    }"#;
    match replace_fn_body(&after_constructor, "setLoggedInFlags(", new_flags) {
        Some(s) => Ok(s),
        None => Ok(after_constructor),
    }
}
