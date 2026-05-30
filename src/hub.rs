use anyhow::Context;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ── Constants ──────────────────────────────────────────────────

pub(crate) const HUB_ASAR_REL: &str = r"resources\app.asar";

const TARGET_FILES: &[&str] = &[
    "build/main/services/licenseService/licenseService.js",
    "build/main/services/licenseService/licenseServiceCore.js",
    "build/main/services/licenseService/licensingSdk.js",
    "build/main/services/authService/AuthService.js",
];

// ── HubInfo ────────────────────────────────────────────────────

pub(crate) struct HubInfo {
    pub hub_path: PathBuf,
    pub version: String,
    pub asar_path: PathBuf,
}

impl Clone for HubInfo {
    fn clone(&self) -> Self {
        Self {
            hub_path: self.hub_path.clone(),
            version: self.version.clone(),
            asar_path: self.asar_path.clone(),
        }
    }
}

// ── Hub Detection ──────────────────────────────────────────────

pub(crate) fn find_hub() -> Option<HubInfo> {
    let candidates = vec![
        PathBuf::from(r"C:\Program Files\Unity Hub"),
        PathBuf::from(r"D:\Program Files\Unity Hub"),
    ];

    for path in candidates {
        let asar = path.join(HUB_ASAR_REL);
        if asar.exists() {
            if let Some(info) = hub_info_from_path(&path, &asar) {
                return Some(info);
            }
        }
    }

    if let Some(path) = hub_path_from_registry() {
        let asar = path.join(HUB_ASAR_REL);
        if asar.exists() {
            return hub_info_from_path(&path, &asar);
        }
    }

    None
}

fn hub_info_from_path(hub_path: &Path, asar_path: &Path) -> Option<HubInfo> {
    let version = read_hub_version(hub_path, asar_path).unwrap_or_else(|| "unknown".to_string());
    Some(HubInfo {
        hub_path: hub_path.to_path_buf(),
        version,
        asar_path: asar_path.to_path_buf(),
    })
}

pub(crate) fn is_hub_patched(asar_path: &Path) -> bool {
    asar_path.with_extension("asar.bak").exists()
}

// ── Version Reading ────────────────────────────────────────────

fn read_hub_version(hub_path: &Path, asar_path: &Path) -> Option<String> {
    let unpacked_pkg = hub_path.join(r"resources\app.asar.unpacked\package.json");
    if let Ok(content) = std::fs::read_to_string(&unpacked_pkg) {
        if let Some(v) = extract_version_from_json_str(&content) {
            return Some(v);
        }
    }

    if let Ok(asar) = read_asar(asar_path) {
        if let Some(entry) = find_entry(&asar.header_json, "package.json") {
            if let Some(data) = extract_file(&asar, &entry) {
                if let Ok(content) = String::from_utf8(data) {
                    if let Some(v) = extract_version_from_json_str(&content) {
                        return Some(v);
                    }
                }
            }
        }
    }

    None
}

fn extract_version_from_json_str(s: &str) -> Option<String> {
    let v: Value = serde_json::from_str(s).ok()?;
    v.get("version")?.as_str().map(|s| s.to_string())
}

// ── Registry Lookup ────────────────────────────────────────────

#[cfg(windows)]
fn hub_path_from_registry() -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;

    unsafe {
        let key_path: Vec<u16> =
            OsStr::new(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Unity Hub")
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_path.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        ) != 0
        {
            return None;
        }

        let value_name: Vec<u16> = OsStr::new("InstallLocation")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut buf = vec![0u8; 1024];
        let mut size = buf.len() as u32;
        let mut kind = 0u32;

        let result = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr(),
            &mut size,
        );

        RegCloseKey(hkey);

        if result != 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
            return None;
        }

        let wide: &[u16] =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, size as usize / 2);
        let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        let path = String::from_utf16_lossy(&wide[..len]);
        Some(PathBuf::from(path.trim()))
    }
}

#[cfg(not(windows))]
fn hub_path_from_registry() -> Option<PathBuf> {
    None
}

// ── asar Reading ───────────────────────────────────────────────

struct AsarArchive {
    header_json: Value,
    data: Vec<u8>,
}

fn read_asar(path: &Path) -> anyhow::Result<AsarArchive> {
    let bytes = std::fs::read(path).with_context(|| format!("read asar: {}", path.display()))?;

    anyhow::ensure!(bytes.len() >= 16, "asar file too small");

    let _outer_size = u32::from_le_bytes(bytes[0..4].try_into()?) as usize;
    let header_payload_size = u32::from_le_bytes(bytes[4..8].try_into()?) as usize;
    let _inner_type = u32::from_le_bytes(bytes[8..12].try_into()?);
    let json_len = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;

    anyhow::ensure!(
        16 + json_len <= bytes.len(),
        "asar header exceeds file size"
    );

    let json_str =
        std::str::from_utf8(&bytes[16..16 + json_len]).context("asar header is not valid UTF-8")?;
    let raw_header: Value = serde_json::from_str(json_str).context("parse asar JSON header")?;

    let header_total = 8 + header_payload_size;
    let data = if header_total < bytes.len() {
        bytes[header_total..].to_vec()
    } else {
        Vec::new()
    };

    // Normalise: some asar versions wrap the tree under a "files" key at root level.
    let header_json = if raw_header.get("files").is_some()
        && raw_header.get("files").map(|f| f.is_object()) == Some(true)
        && raw_header.get("size").is_none()
        && raw_header.get("offset").is_none()
    {
        raw_header
    } else {
        json!({ "files": raw_header })
    };

    Ok(AsarArchive { header_json, data })
}

// ── Header Navigation ──────────────────────────────────────────

fn walk_asar(header: &Value) -> Vec<(String, Value)> {
    let mut result = Vec::new();
    walk_recursive(header, "", &mut result);
    result
}

fn walk_recursive(node: &Value, path: &str, result: &mut Vec<(String, Value)>) {
    let Some(files) = node.get("files").and_then(|f| f.as_object()) else {
        if node.get("size").is_some() && node.get("offset").is_some() && !path.is_empty() {
            result.push((path.to_string(), node.clone()));
        }
        return;
    };
    for (name, child) in files {
        let child_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", path, name)
        };
        walk_recursive(child, &child_path, result);
    }
}

fn find_entry(header: &Value, path: &str) -> Option<Value> {
    let segments: Vec<&str> = path.split('/').collect();
    let mut current = header;
    for seg in &segments {
        current = current.get("files")?.get(seg)?;
    }
    Some(current.clone())
}

fn extract_file(asar: &AsarArchive, entry: &Value) -> Option<Vec<u8>> {
    let size = entry.get("size")?.as_u64()? as usize;
    let offset = entry.get("offset")?.as_str()?.parse::<usize>().ok()?;
    if offset.saturating_add(size) > asar.data.len() {
        return None;
    }
    Some(asar.data[offset..offset + size].to_vec())
}

// ── asar Writing ───────────────────────────────────────────────

fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex_encode(&hash)
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn compute_integrity(data: &[u8]) -> Value {
    const BLOCK_SIZE: usize = 4 * 1024 * 1024; // 4 MB
    let hash = sha256_hex(data);
    let blocks: Vec<Value> = if data.is_empty() {
        vec![json!(sha256_hex(&[]))]
    } else {
        data.chunks(BLOCK_SIZE)
            .map(|chunk| json!(sha256_hex(chunk)))
            .collect()
    };
    json!({
        "algorithm": "SHA256",
        "hash": hash,
        "blockSize": BLOCK_SIZE,
        "blocks": blocks,
    })
}

fn write_asar(
    path: &Path,
    modified: &[(String, Vec<u8>)],
    original: &AsarArchive,
) -> anyhow::Result<()> {
    let entries = walk_asar(&original.header_json);
    let mod_set: std::collections::HashMap<&str, &[u8]> = modified
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();

    // Clone the header so we can mutate offsets/sizes/integrity.
    let mut new_header = original.header_json.clone();

    let mut new_data = Vec::new();
    let mut offset: u64 = 0;

    for (entry_path, _) in &entries {
        let content = match mod_set.get(entry_path.as_str()) {
            Some(data) => data.to_vec(),
            None => match find_entry(&original.header_json, entry_path) {
                Some(e) => extract_file(original, &e).unwrap_or_default(),
                None => Vec::new(),
            },
        };

        let size = content.len() as u64;
        let integrity = compute_integrity(&content);
        new_data.extend_from_slice(&content);

        let segments: Vec<&str> = entry_path.split('/').collect();
        let mut pointer = String::new();
        for seg in &segments {
            pointer.push_str("/files/");
            pointer.push_str(seg);
        }

        if let Some(obj) = new_header
            .pointer_mut(&pointer)
            .and_then(|v| v.as_object_mut())
        {
            obj.insert("offset".into(), json!(offset.to_string()));
            obj.insert("size".into(), json!(size));
            obj.insert("integrity".into(), integrity);
        }

        offset += size;
    }

    let json_str = serde_json::to_string(&new_header)?;
    let json_bytes = json_str.as_bytes();
    let json_len = json_bytes.len();
    let inner_payload_size = (4 + json_len) as u32;
    let outer_payload_size = 4 + inner_payload_size;

    let mut output = Vec::with_capacity(16 + json_len + new_data.len());
    output.extend_from_slice(&4u32.to_le_bytes());
    output.extend_from_slice(&outer_payload_size.to_le_bytes());
    output.extend_from_slice(&inner_payload_size.to_le_bytes());
    output.extend_from_slice(&(json_len as u32).to_le_bytes());
    output.extend_from_slice(json_bytes);
    output.extend_from_slice(&new_data);

    std::fs::write(path, &output).with_context(|| format!("write asar: {}", path.display()))?;
    Ok(())
}

// ── JS Function Body Replacement ───────────────────────────────

/// Replace the body of a JS function identified by `signature` (e.g. `"isLicenseValid("`
/// or `"init(externalCorrelationId)"`). Handles `async` prefix, skips comments and string
/// literals while searching, and counts braces while skipping comments/strings to locate
/// the closing brace.
fn replace_fn_body(source: &str, signature: &str, new_body: &str) -> Option<String> {
    let src = source.as_bytes();
    let pat = signature.as_bytes();
    let mut search_start = 0;

    loop {
        let start = source[search_start..].find(signature)? + search_start;
        let after_pat = start + pat.len();

        // Reject if preceded by an identifier character (partial match).
        if start > 0 {
            let prev = src[start - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                search_start = after_pat;
                continue;
            }
        }

        // Skip past ')' to find the opening brace.
        let mut pos = after_pat;
        let mut paren_depth: i32 = 1;
        while pos < src.len() && paren_depth > 0 {
            match src[pos] {
                b'(' => paren_depth += 1,
                b')' => paren_depth -= 1,
                _ => {}
            }
            if paren_depth > 0 {
                pos += 1;
            }
        }
        if paren_depth != 0 {
            search_start = after_pat;
            continue;
        }
        pos += 1; // move past ')'

        // Skip whitespace / `async` / `=>` until '{'.
        while pos < src.len() {
            let b = src[pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                pos += 1;
            } else if src[pos..].starts_with(b"=>") {
                pos += 2;
            } else if b == b'{' {
                break;
            } else {
                // Unexpected character before '{' – skip ahead and retry.
                pos += 1;
            }
        }
        if pos >= src.len() || src[pos] != b'{' {
            search_start = after_pat;
            continue;
        }

        // Match braces to find function body end, skipping comments and strings.
        let brace_start = pos;
        let mut depth: i32 = 0;
        let mut end = pos;
        while pos < src.len() {
            match src[pos] {
                // Line comment
                b'/' if pos + 1 < src.len() && src[pos + 1] == b'/' => {
                    pos += 2;
                    while pos < src.len() && src[pos] != b'\n' {
                        pos += 1;
                    }
                }
                // Block comment
                b'/' if pos + 1 < src.len() && src[pos + 1] == b'*' => {
                    pos += 2;
                    while pos + 1 < src.len() && !(src[pos] == b'*' && src[pos + 1] == b'/') {
                        pos += 1;
                    }
                    if pos + 1 < src.len() {
                        pos += 2;
                    }
                }
                // String literals
                b'"' | b'\'' | b'`' => {
                    let quote = src[pos];
                    pos += 1;
                    while pos < src.len() && src[pos] != quote {
                        if src[pos] == b'\\' && pos + 1 < src.len() {
                            pos += 2;
                        } else {
                            pos += 1;
                        }
                    }
                    if pos < src.len() {
                        pos += 1; // skip closing quote
                    }
                }
                b'{' => {
                    depth += 1;
                    pos += 1;
                }
                b'}' => {
                    depth -= 1;
                    pos += 1;
                    if depth == 0 {
                        end = pos;
                        break;
                    }
                }
                _ => {
                    pos += 1;
                }
            }
        }

        if end <= brace_start {
            search_start = after_pat;
            continue;
        }

        let mut result = String::with_capacity(source.len());
        result.push_str(&source[..brace_start]);
        result.push_str(new_body);
        result.push_str(&source[end..]);
        return Some(result);
    }
}

// ── Patch Definitions ──────────────────────────────────────────

fn patch_license_service_js(content: &str) -> anyhow::Result<String> {
    let new_body = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return true;
        });
    }"#;
    replace_fn_body(content, "isLicenseValid(", new_body).context("patch isLicenseValid failed")
}

fn patch_license_core_js(content: &str) -> anyhow::Result<String> {
    let new_body = r#"{
        return __awaiter(this, void 0, void 0, function* () {
            return [{
                startDate: new Date('1993-01-01T08:00:00.000Z'),
                expirationDate: new Date('9999-01-01T08:00:00.000Z'),
                productName: __importDefault(require("../../i18nHelper")).default.i18n.translate('license-management:TYPE_EDUCATION'),
                licenseType: 'ULF',
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

    let new_get_instance = r#"{
        return null;
    }"#;
    replace_fn_body(&result, "getInstance(", new_get_instance).context("patch getInstance failed")
}

fn patch_auth_service_js(content: &str) -> anyhow::Result<String> {
    // 1. 替换 isUserSignedIn() → return true
    let new_signed_in = r#"{
        return true;
    }"#;
    let after_signed_in = match replace_fn_body(content, "isUserSignedIn(", new_signed_in) {
        Some(s) => s,
        None => {
            // isUserSignedIn only exists in Hub 3.7+, skip gracefully.
            content.to_string()
        }
    };

    // 2. 替换 constructor() — 注入假 userInfo，使 getFormattedUserInfo() 返回有效用户
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
    let after_constructor =
        match replace_fn_body(&after_signed_in, "constructor(", new_constructor) {
            Some(s) => s,
            None => after_signed_in,
        };

    // 3. 替换 restoreAuthState() — 设置 loggedIn 并广播事件，让 renderer 显示主界面
    //    注意：windowManager 在 authService.init() 之后才初始化，必须加 null 保护
    //    事件数据格式必须与 setOfflineLoggedInFlags/setLoggedInFlags 一致
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

// ── Main Patch / Unpatch ───────────────────────────────────────

pub(crate) fn patch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = &info.asar_path;

    // Backup
    let bak_path = asar_path.with_extension("asar.bak");
    std::fs::copy(asar_path, &bak_path).context("backup asar failed")?;

    // Read
    let asar = read_asar(asar_path)?;

    // Extract and patch target files
    let mut modified = Vec::new();

    for target in TARGET_FILES {
        let entry = match find_entry(&asar.header_json, target) {
            Some(e) => e,
            None => continue, // file not present in this Hub version
        };

        let content_bytes = match extract_file(&asar, &entry) {
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

    // Write patched asar
    write_asar(asar_path, &modified, &asar)?;

    Ok(())
}

pub(crate) fn unpatch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let bak_path = info.asar_path.with_extension("asar.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    std::fs::copy(&bak_path, &info.asar_path).context("restore asar failed")?;
    std::fs::remove_file(&bak_path).context("remove backup failed")?;
    Ok(())
}
