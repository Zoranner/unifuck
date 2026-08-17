use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ── Drive Discovery ────────────────────────────────────────────

/// Returns root paths for all accessible drive letters (A:–Z:).
///
/// Uses `std::fs::read_dir` to quickly probe each letter; non-existent
/// or inaccessible drives fail fast and are silently skipped.
pub(crate) fn available_drive_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for letter in b'A'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if std::fs::read_dir(&root).is_ok() {
            roots.push(root);
        }
    }
    roots
}

// ── Config Persistence ─────────────────────────────────────────

/// User configuration stored at `%APPDATA%\unifuck\config.json`.
pub(crate) struct Config {
    pub custom_hubs: std::collections::HashMap<String, String>,
}

/// Load config from `%APPDATA%\unifuck\config.json`.
/// Returns default (empty) config if the file does not exist or cannot be parsed.
pub(crate) fn load_config() -> Config {
    use std::collections::HashMap;

    let empty = Config {
        custom_hubs: HashMap::new(),
    };
    let appdata = match std::env::var("APPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => return empty,
    };
    let config_path = appdata.join("unifuck").join("config.json");
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return empty,
    };
    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return empty,
    };
    let map = value
        .get("custom_hub_paths")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Config { custom_hubs: map }
}

/// Save config to `%APPDATA%\unifuck\config.json`.
/// Creates the directory if it does not exist.
pub(crate) fn save_config(config: &Config) -> std::io::Result<()> {
    let appdata = match std::env::var("APPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "APPDATA not set",
            ));
        }
    };
    let dir = appdata.join("unifuck");
    std::fs::create_dir_all(&dir)?;
    let config_path = dir.join("config.json");
    let value = serde_json::json!({
        "custom_hub_paths": config.custom_hubs,
    });
    let content = serde_json::to_string_pretty(&value)?;
    std::fs::write(&config_path, content)
}

// ── Hub Discovery ──────────────────────────────────────────────

/// Finds all installations of a hub (e.g., "Unity Hub" or "Tuanjie Hub")
/// across all drives.
///
/// Strategy:
/// 1. Registry: enumerate Uninstall keys, match by DisplayName prefix
/// 2. Enumerate `<drive>:\Program Files\<hub_name>` for each available drive
///
/// Results are deduplicated and verified by checking `resources\app.asar`.
pub(crate) fn find_hub_roots(hub_name: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    let asar_rel = r"resources\app.asar";

    // 1. Registry first (exact install path)
    if let Some(path) = hub_path_from_registry(hub_name) {
        let canonical = normalize_path(&path);
        if seen.insert(canonical) && path.join(asar_rel).exists() {
            crate::diag::log(&format!("hub found via registry: {}", path.display()));
            roots.push(path);
        }
    }

    // 2. Program Files on all drives
    for drive in available_drive_roots() {
        let candidate = drive.join("Program Files").join(hub_name);
        let canonical = normalize_path(&candidate);
        if seen.insert(canonical) && candidate.join(asar_rel).exists() {
            crate::diag::log(&format!(
                "hub found via Program Files: {}",
                candidate.display()
            ));
            roots.push(candidate);
        }
    }

    if roots.is_empty() {
        crate::diag::log(&format!("hub NOT found: {}", hub_name));
    }
    roots
}

// ── Editor Discovery ───────────────────────────────────────────

/// Hub 配置 (editors-v2.json / editors.json) 中记录的一个 Editor。
pub(crate) struct EditorRecord {
    /// Hub 记录里的版本号 (可能缺失, 调用方回退到目录名)。
    pub version: Option<String>,
    /// Editor 安装根目录 (包含 "Editor" 子目录的那一层)。
    pub root: PathBuf,
}

/// Read editor records from Hub config for a specific vendor.
pub(crate) fn read_editor_paths(vendor: crate::vendor::Vendor) -> Vec<EditorRecord> {
    let mut records = Vec::new();
    let mut seen = HashSet::new();

    let appdata = match std::env::var("APPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            crate::diag::log("APPDATA not set, no editor records");
            return records;
        }
    };

    let hub_dir = appdata.join(vendor.config_dir_name());

    // editors-v2.json (current Hub format)
    let v2_path = hub_dir.join("editors-v2.json");
    match std::fs::read_to_string(&v2_path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => {
                let parsed = parse_v2_locations(&value);
                crate::diag::log(&format!(
                    "{}: {} record(s)",
                    v2_path.display(),
                    parsed.len()
                ));
                for (version, loc) in parsed {
                    let root = editor_root_from_location(&loc);
                    try_add_record(version, root, &mut records, &mut seen);
                }
            }
            Err(_) => crate::diag::log(&format!("{}: JSON parse failed", v2_path.display())),
        },
        Err(_) => crate::diag::log(&format!("{}: not found", v2_path.display())),
    }

    // editors.json (legacy Hub 2.x format, usually absent)
    let v1_path = hub_dir.join("editors.json");
    if let Ok(content) = std::fs::read_to_string(&v1_path) {
        match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(value) => {
                let parsed = parse_v1_locations(&value);
                crate::diag::log(&format!(
                    "{}: {} record(s)",
                    v1_path.display(),
                    parsed.len()
                ));
                for (version, loc) in parsed {
                    let root = editor_root_from_location(&loc);
                    try_add_record(version, root, &mut records, &mut seen);
                }
            }
            Err(_) => crate::diag::log(&format!("{}: JSON parse failed", v1_path.display())),
        }
    }

    records
}

/// 扫描 Hub 默认安装目录下的 Editor,补 Hub 配置未记录的官方安装。
///
/// Hub 官方安装的 Editor 位于 `<前缀>\<Vendor>\Hub\Editor\<version>\Editor\<exe>`,
/// 不写入 editors-v2.json / editors.json,需直接遍历默认安装根。
/// 前缀取并集: secondaryInstallPath.json、ProgramFiles 环境变量、`C:\Program Files`
/// 三者各自尝试 (不再短路),按规范化路径去重后逐个扫描,合并所有发现的版本根。
///
/// 返回若干 Editor 安装根 (包含 `Editor` 子目录的 `<version>` 目录)。
pub(crate) fn scan_default_editor_paths(vendor: crate::vendor::Vendor) -> Vec<PathBuf> {
    let prefixes = default_install_prefixes(vendor);
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    for prefix in &prefixes {
        let editor_root = default_editor_root(prefix, vendor);
        for dir in scan_editor_root(&editor_root) {
            // 不同前缀可能解析到同一物理安装 (如软链接/大小写差异),
            // 按规范化路径去重,避免同一 Editor 重复出现。
            if seen.insert(normalize_path(&dir)) {
                roots.push(dir);
            }
        }
    }
    roots
}

// ── Internal Helpers ───────────────────────────────────────────

/// 收集去重后的默认安装前缀候选集,按顺序扫描:
/// secondaryInstallPath.json → ProgramFiles 环境变量 → `C:\Program Files`。
///
/// 三者都尝试 (不短路);空串跳过,按规范化路径去重 (同一物理前缀只扫一次)。
fn default_install_prefixes(vendor: crate::vendor::Vendor) -> Vec<PathBuf> {
    let secondary = secondary_install_prefix(vendor);
    let program_files = std::env::var("ProgramFiles").ok();
    collect_prefixes(secondary.as_deref(), program_files.as_deref())
}

/// 读取 secondaryInstallPath.json 中的前缀 (纯 JSON string, 如 `"C:\Program Files"`)。
/// 文件缺失、解析失败或值为空串时返回 None。
fn secondary_install_prefix(vendor: crate::vendor::Vendor) -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let path = PathBuf::from(appdata)
        .join(vendor.config_dir_name())
        .join("secondaryInstallPath.json");
    let prefix = std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str::<String>(&content).ok())?;
    if prefix.trim().is_empty() {
        return None;
    }
    crate::diag::log(&format!(
        "default install prefix from {}: {}",
        path.display(),
        prefix
    ));
    Some(prefix)
}

/// 合并前缀候选并去重。`secondary` 与 `program_files` 可注入,便于测试;
/// `C:\Program Files` 恒为最后兜底。空串跳过,按规范化路径去重,保持来源顺序。
fn collect_prefixes(secondary: Option<&str>, program_files: Option<&str>) -> Vec<PathBuf> {
    let mut prefixes = Vec::new();
    let mut seen = HashSet::new();
    // (来源, 值) 按扫描顺序排列; None 表示该来源不可用。
    let candidates = [
        ("secondaryInstallPath.json", secondary),
        ("ProgramFiles", program_files),
        ("fallback", Some(r"C:\Program Files")),
    ];
    for (source, raw) in candidates {
        let Some(value) = raw else { continue };
        if value.trim().is_empty() {
            crate::diag::log(&format!(
                "default install prefix from {} skipped (empty)",
                source
            ));
            continue;
        }
        let prefix = PathBuf::from(value);
        if seen.insert(normalize_path(&prefix)) {
            crate::diag::log(&format!(
                "default install prefix from {}: {}",
                source,
                prefix.display()
            ));
            prefixes.push(prefix);
        } else {
            crate::diag::log(&format!(
                "default install prefix from {} deduped: {}",
                source,
                prefix.display()
            ));
        }
    }
    prefixes
}

/// 拼默认 Editor 根: `<前缀>\<Vendor>\Hub\Editor`。可注入前缀,便于测试。
fn default_editor_root(prefix: &Path, vendor: crate::vendor::Vendor) -> PathBuf {
    prefix
        .join(vendor.display_name())
        .join("Hub")
        .join("Editor")
}

/// 遍历默认 Editor 根的子目录,收录含 `Editor` 子目录的 `<version>` 目录。
fn scan_editor_root(editor_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let entries = match std::fs::read_dir(editor_root) {
        Ok(e) => e,
        Err(_) => {
            crate::diag::log(&format!(
                "default editor root missing: {}",
                editor_root.display()
            ));
            return roots;
        }
    };
    crate::diag::log(&format!(
        "scanning default editor root: {}",
        editor_root.display()
    ));
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        if dir.join("Editor").is_dir() {
            crate::diag::log(&format!("default editor found: {}", dir.display()));
            roots.push(dir);
        } else {
            crate::diag::log(&format!(
                "default editor skipped (no Editor dir): {}",
                dir.display()
            ));
        }
    }
    roots
}

/// Normalize a path for deduplication.
///
/// Existing paths are first resolved so `..`, 8.3 paths, junctions, and
/// symbolic links share one key. Missing paths fall back to lexical cleanup,
/// which keeps discovery probes non-fatal. `pub(crate)` allows `core::group`
/// to share this key while retaining original paths for display and operations.
pub(crate) fn normalize_path(path: &std::path::Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = resolved
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    if cfg!(windows) {
        PathBuf::from(s.to_lowercase())
    } else {
        PathBuf::from(s)
    }
}

/// Add a record if `root` is an existing directory and hasn't been seen.
fn try_add_record(
    version: Option<String>,
    root: PathBuf,
    records: &mut Vec<EditorRecord>,
    seen: &mut HashSet<PathBuf>,
) {
    let canonical = normalize_path(&root);
    if !seen.insert(canonical) {
        return;
    }
    if root.is_dir() {
        records.push(EditorRecord { version, root });
    } else {
        crate::diag::log(&format!(
            "editor record rejected (dir missing): {}",
            root.display()
        ));
    }
}

/// Derive the install root from a Hub `location` value.
///
/// `location` usually points to the exe (`<root>\Editor\Unity.exe`), but
/// manually added entries may already be the install root. Both forms
/// are accepted.
fn editor_root_from_location(loc: &str) -> PathBuf {
    let p = PathBuf::from(loc);
    if p.is_dir() && p.join("Editor").is_dir() {
        return p;
    }
    p.parent()
        .and_then(|editor| editor.parent())
        .map(|r| r.to_path_buf())
        .unwrap_or(p)
}

/// Parse editors-v2.json: `{"data": [{"version": "...", "location": ["C:\\...\\Editor\\Unity.exe"]}]}`
///
/// Returns (version, raw location) pairs. `location` may be an array
/// (first element wins) or a plain string.
fn parse_v2_locations(value: &serde_json::Value) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let Some(data) = value.get("data").and_then(|d| d.as_array()) else {
        return out;
    };
    for item in data {
        let version = item
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let loc = match item.get("location") {
            Some(serde_json::Value::Array(arr)) => arr.first().and_then(|v| v.as_str()),
            Some(serde_json::Value::String(s)) => Some(s.as_str()),
            _ => None,
        };
        if let Some(loc) = loc {
            out.push((version, loc.to_string()));
        }
    }
    out
}

/// Parse legacy editors.json formats:
/// - `{"version": {"location": "path"}}`
/// - `{"version": "path"}`
/// - `[{"version": "...", "location": "path"}]`
///
/// As with v2, `location` points to the exe; root derivation is shared
/// via `editor_root_from_location`.
fn parse_v1_locations(value: &serde_json::Value) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                let version = Some(key.clone());
                if let Some(loc) = val.get("location").and_then(|l| l.as_str()) {
                    out.push((version, loc.to_string()));
                } else if let Some(loc) = val.as_str() {
                    out.push((version, loc.to_string()));
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                let version = item
                    .get("version")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if let Some(loc) = item.get("location").and_then(|l| l.as_str()) {
                    out.push((version, loc.to_string()));
                }
            }
        }
        _ => {}
    }
    out
}

// ── Registry (Windows) ─────────────────────────────────────────

/// Enumerate Uninstall keys under HKLM / HKCU, match by DisplayName
/// prefix (e.g. "Unity Hub 3.5.0" matches "Unity Hub"). Install path
/// comes from InstallLocation, falling back to the DisplayIcon parent.
#[cfg(windows)]
fn hub_path_from_registry(hub_name: &str) -> Option<PathBuf> {
    use windows_sys::Win32::System::Registry::*;

    const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let Some(uninstall) = reg_open_read(root, UNINSTALL) else {
            continue;
        };
        let mut found = None;
        let mut idx = 0u32;
        let mut name_buf = vec![0u16; 256];
        loop {
            let mut len = name_buf.len() as u32;
            let ret = unsafe {
                RegEnumKeyExW(
                    uninstall,
                    idx,
                    name_buf.as_mut_ptr(),
                    &mut len,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if ret != 0 {
                break;
            }
            idx += 1;
            let sub_name = String::from_utf16_lossy(&name_buf[..len as usize]);
            let sub_path = format!(r"{}\{}", UNINSTALL, sub_name);
            let Some(sub) = reg_open_read(root, &sub_path) else {
                continue;
            };
            if let Some(display) = reg_query_string(sub, "DisplayName") {
                if display
                    .to_ascii_lowercase()
                    .starts_with(&hub_name.to_ascii_lowercase())
                {
                    found = reg_query_string(sub, "InstallLocation")
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| {
                            reg_query_string(sub, "DisplayIcon")
                                .and_then(|icon| parent_dir_of(&icon))
                        })
                        .map(PathBuf::from);
                }
            }
            unsafe { RegCloseKey(sub) };
            if found.is_some() {
                break;
            }
        }
        unsafe { RegCloseKey(uninstall) };
        if found.is_some() {
            return found;
        }
    }
    None
}

/// Strip a trailing `,<index>` from DisplayIcon and return the parent
/// directory, e.g. `"C:\Program Files\Unity Hub\Unity Hub.exe,0"` →
/// `C:\Program Files\Unity Hub`.
#[cfg(windows)]
fn parent_dir_of(icon: &str) -> Option<String> {
    let cleaned = icon.trim().trim_matches('"');
    let exe_path = match cleaned.rsplit_once(',') {
        Some((path, idx)) if !path.is_empty() && idx.chars().all(|c| c.is_ascii_digit()) => path,
        _ => cleaned,
    };
    Path::new(exe_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg(windows)]
fn reg_open_read(
    root: windows_sys::Win32::System::Registry::HKEY,
    path: &str,
) -> Option<windows_sys::Win32::System::Registry::HKEY> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;

    let wide: Vec<u16> = OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut hkey: HKEY = std::ptr::null_mut();
    let ret = unsafe { RegOpenKeyExW(root, wide.as_ptr(), 0, KEY_READ, &mut hkey) };
    if ret != 0 {
        None
    } else {
        Some(hkey)
    }
}

#[cfg(windows)]
fn reg_query_string(
    hkey: windows_sys::Win32::System::Registry::HKEY,
    name: &str,
) -> Option<String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;

    let name_wide: Vec<u16> = OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut buf = vec![0u8; 2048];
    let mut size = buf.len() as u32;
    let mut kind = 0u32;
    let ret = unsafe {
        RegQueryValueExW(
            hkey,
            name_wide.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr(),
            &mut size,
        )
    };
    if ret != 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
        return None;
    }
    let wide: &[u16] =
        unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u16, size as usize / 2) };
    let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
    Some(String::from_utf16_lossy(&wide[..len]).trim().to_string())
}

#[cfg(not(windows))]
fn hub_path_from_registry(_hub_name: &str) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_v2_array_and_string_location() {
        let v = serde_json::json!({
            "schema_version": "2",
            "data": [
                {"version": "6000.4.8f1", "location": ["C:\\Program Files\\Unity 6000.4.8f1\\Editor\\Unity.exe"], "manual": true},
                {"version": "2021.3.58f1", "location": "D:\\Editors\\2021.3.58f1\\Editor\\Unity.exe"}
            ]
        });
        let parsed = parse_v2_locations(&v);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0.as_deref(), Some("6000.4.8f1"));
        assert!(parsed[0].1.ends_with("Unity.exe"));
        assert_eq!(parsed[1].0.as_deref(), Some("2021.3.58f1"));
    }

    #[test]
    fn parse_v2_missing_data() {
        assert!(parse_v2_locations(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn parse_v1_object_form() {
        let v = serde_json::json!({
            "2022.3.50f1": {"location": "C:\\Editors\\2022.3.50f1\\Editor\\Unity.exe"},
            "2019.4.40f1": "C:\\Editors\\2019.4.40f1\\Editor\\Unity.exe"
        });
        let parsed = parse_v1_locations(&v);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().all(|(ver, _)| ver.is_some()));
    }

    #[test]
    fn parse_v1_array_form() {
        let v = serde_json::json!([{"version": "2022.3.50f1", "location": "C:\\Editors\\2022.3.50f1\\Editor\\Unity.exe"}]);
        let parsed = parse_v1_locations(&v);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0.as_deref(), Some("2022.3.50f1"));
    }

    #[cfg(windows)]
    #[test]
    fn parent_dir_strips_icon_index() {
        assert_eq!(
            parent_dir_of(r"C:\Program Files\Unity Hub\Unity Hub.exe,0").as_deref(),
            Some(r"C:\Program Files\Unity Hub")
        );
        assert_eq!(
            parent_dir_of(r"C:\Program Files\Unity Hub\Unity Hub.exe").as_deref(),
            Some(r"C:\Program Files\Unity Hub")
        );
    }

    #[test]
    fn default_editor_root_joins_vendor_hub_editor() {
        let prefix = Path::new(r"C:\Program Files");
        let root = default_editor_root(prefix, crate::vendor::Vendor::Unity);
        assert_eq!(root, Path::new(r"C:\Program Files\Unity\Hub\Editor"));
        let root = default_editor_root(prefix, crate::vendor::Vendor::Tuanjie);
        assert_eq!(root, Path::new(r"C:\Program Files\Tuanjie\Hub\Editor"));
    }

    #[test]
    fn scan_editor_root_collects_versions_with_editor_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // 合法版本: <tmp>\Unity\Hub\Editor\2022.3.62f3\Editor\Unity.exe
        let good = tmp.path().join(r"Unity\Hub\Editor\2022.3.62f3\Editor");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("Unity.exe"), b"").unwrap();
        // 缺 Editor 子目录的版本: <tmp>\Unity\Hub\Editor\2019.4.40f1
        let bad = tmp.path().join(r"Unity\Hub\Editor\2019.4.40f1");
        std::fs::create_dir_all(&bad).unwrap();

        let editor_root = default_editor_root(tmp.path(), crate::vendor::Vendor::Unity);
        let roots = scan_editor_root(&editor_root);

        let expected = tmp.path().join(r"Unity\Hub\Editor\2022.3.62f3");
        assert!(roots.contains(&expected), "应收录含 Editor 子目录的版本根");
        assert!(!roots.contains(&bad), "缺 Editor 子目录的版本应被跳过");
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn scan_editor_root_collects_tuanjie_versions() {
        let tmp = tempfile::tempdir().unwrap();
        // Tuanjie 合法版本: <tmp>\Tuanjie\Hub\Editor\2022.3.62t5\Editor\Tuanjie.exe
        let good = tmp.path().join(r"Tuanjie\Hub\Editor\2022.3.62t5\Editor");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("Tuanjie.exe"), b"").unwrap();

        let editor_root = default_editor_root(tmp.path(), crate::vendor::Vendor::Tuanjie);
        let roots = scan_editor_root(&editor_root);

        let expected = tmp.path().join(r"Tuanjie\Hub\Editor\2022.3.62t5");
        assert!(roots.contains(&expected), "应收录 Tuanjie 版本根");
        assert_eq!(roots.len(), 1);
    }

    #[test]
    fn scan_editor_root_missing_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join(r"Unity\Hub\Editor");
        assert!(scan_editor_root(&missing).is_empty());
    }

    /// 构造 `<prefix>\Unity\Hub\Editor\<ver>\Editor\Unity.exe`。
    fn make_default_install(prefix: &Path, version: &str) -> PathBuf {
        let editor_dir = prefix
            .join(r"Unity\Hub\Editor")
            .join(version)
            .join("Editor");
        std::fs::create_dir_all(&editor_dir).unwrap();
        std::fs::write(editor_dir.join("Unity.exe"), b"").unwrap();
        prefix.join(r"Unity\Hub\Editor").join(version)
    }

    /// 对一组候选前缀做并集扫描 (与 scan_default_editor_paths 内部逻辑一致)。
    fn union_scan(prefixes: &[PathBuf]) -> Vec<PathBuf> {
        let mut roots = Vec::new();
        let mut seen = HashSet::new();
        for prefix in prefixes {
            let editor_root = default_editor_root(prefix, crate::vendor::Vendor::Unity);
            for dir in scan_editor_root(&editor_root) {
                if seen.insert(normalize_path(&dir)) {
                    roots.push(dir);
                }
            }
        }
        roots
    }

    #[test]
    fn collect_prefixes_unions_all_sources() {
        // secondary 与 ProgramFiles 不同: 两者都保留; 兜底与 ProgramFiles 相同被去重。
        let prefixes = collect_prefixes(Some(r"D:\CustomInstall"), Some(r"C:\Program Files"));
        assert_eq!(prefixes.len(), 2, "secondary 与 ProgramFiles 应都尝试");
        assert_eq!(prefixes[0], Path::new(r"D:\CustomInstall"));
        assert_eq!(prefixes[1], Path::new(r"C:\Program Files"));
    }

    #[test]
    fn collect_prefixes_keeps_distinct_fallback() {
        // secondary 缺失, ProgramFiles 与兜底不同: 两个都保留。
        let prefixes = collect_prefixes(None, Some(r"D:\PF"));
        assert_eq!(prefixes.len(), 2);
        assert_eq!(prefixes[0], Path::new(r"D:\PF"));
        assert_eq!(prefixes[1], Path::new(r"C:\Program Files"));
    }

    #[cfg(windows)]
    #[test]
    fn collect_prefixes_dedupes_same_physical_prefix() {
        // secondary 与 ProgramFiles 指向同一物理前缀 (仅大小写差异),
        // 且与兜底相同: 三者规范化为同一键, 只扫描一次。
        let prefixes = collect_prefixes(Some(r"C:\Program Files"), Some(r"C:\PROGRAM FILES"));
        assert_eq!(prefixes.len(), 1, "同一物理前缀应只扫描一次");
        assert_eq!(prefixes[0], Path::new(r"C:\Program Files"));
    }

    #[test]
    fn collect_prefixes_skips_empty_and_missing() {
        // secondary 空串跳过; ProgramFiles 缺失 (None) 跳过; 仅兜底生效。
        let prefixes = collect_prefixes(Some("   "), None);
        assert_eq!(prefixes, vec![PathBuf::from(r"C:\Program Files")]);
    }

    #[test]
    fn union_scan_finds_installs_across_prefixes() {
        // 两个候选前缀各自都有合法安装, 并集扫描应两个都发现。
        let tmp_a = tempfile::tempdir().unwrap();
        let tmp_b = tempfile::tempdir().unwrap();
        let expected_a = make_default_install(tmp_a.path(), "2022.3.62f3");
        let expected_b = make_default_install(tmp_b.path(), "6000.4.8f1");

        let roots = union_scan(&[tmp_a.path().to_path_buf(), tmp_b.path().to_path_buf()]);

        assert_eq!(roots.len(), 2, "两个前缀下的安装都应被发现");
        assert!(roots.contains(&expected_a));
        assert!(roots.contains(&expected_b));
    }
}
