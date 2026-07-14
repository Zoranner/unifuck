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
/// 1. Check registry for exact install path
/// 2. Enumerate `<drive>:\Program Files\<hub_name>` for each available drive
/// 3. User-configured custom hub paths from %APPDATA%\unifuck\config.json
///
/// Results are deduplicated and verified by checking `resources\app.asar`.
pub(crate) fn find_hub_roots(hub_name: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    let asar_rel = r"resources\app.asar";

    // 1. Registry first (exact path)
    if let Some(path) = hub_path_from_registry(hub_name) {
        let canonical = normalize_path(&path);
        if seen.insert(canonical) && path.join(asar_rel).exists() {
            roots.push(path);
        }
    }

    // 2. Program Files on all drives
    for drive in available_drive_roots() {
        let candidate = drive.join("Program Files").join(hub_name);
        let canonical = normalize_path(&candidate);
        if seen.insert(canonical) && candidate.join(asar_rel).exists() {
            roots.push(candidate);
        }
    }

    // 3. Custom paths from user config
    let config = load_config();
    for (name, path) in &config.custom_hubs {
        if name != hub_name {
            continue;
        }
        let candidate = PathBuf::from(path);
        let canonical = normalize_path(&candidate);
        if seen.insert(canonical) && candidate.join(asar_rel).exists() {
            roots.push(candidate);
        }
    }

    roots
}

// ── Editor Discovery ───────────────────────────────────────────

/// Read editor paths from Hub config for a specific vendor.
pub(crate) fn read_editor_paths(vendor: crate::vendor::Vendor) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    let appdata = match std::env::var("APPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => return paths,
    };

    for v in std::iter::once(vendor) {
        let hub_dir = appdata.join(v.config_dir_name());

        // editors-v2.json (current Hub format)
        let v2_path = hub_dir.join("editors-v2.json");
        if let Ok(content) = std::fs::read_to_string(&v2_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                extract_v2_editor_paths(&value, &mut paths, &mut seen);
            }
        }

        // editors.json (legacy format)
        let v1_path = hub_dir.join("editors.json");
        if let Ok(content) = std::fs::read_to_string(&v1_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                extract_v1_editor_paths(&value, &mut paths, &mut seen);
            }
        }
    }

    paths
}

// ── Internal Helpers ───────────────────────────────────────────

/// Normalize a path for deduplication: strip trailing separators and
/// lowercase on Windows (case-insensitive filesystem).
fn normalize_path(path: &std::path::Path) -> PathBuf {
    let s = path
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_string();
    if cfg!(windows) {
        PathBuf::from(s.to_lowercase())
    } else {
        PathBuf::from(s)
    }
}

/// Add `root` to `paths` if it is a directory and hasn't been seen.
fn try_add_path(root: &Path, paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let canonical = normalize_path(root);
    if seen.insert(canonical) && root.is_dir() {
        paths.push(root.to_path_buf());
    }
}

/// Parse editors-v2.json: `{"data": [{"location": ["C:\\...\\Editor\\Unity.exe"]}]}`
///
/// `location` points to the .exe; we derive the Editor root as the parent of
/// the "Editor" directory containing the .exe.
fn extract_v2_editor_paths(
    value: &serde_json::Value,
    paths: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    let data = match value.get("data").and_then(|d| d.as_array()) {
        Some(arr) => arr,
        None => return,
    };
    for item in data {
        let loc = match item.get("location") {
            Some(serde_json::Value::Array(arr)) => {
                arr.first().and_then(|v| v.as_str()).map(|s| s.to_string())
            }
            Some(serde_json::Value::String(s)) => Some(s.clone()),
            _ => None,
        };
        if let Some(loc) = loc {
            // exe path e.g. "C:\Program Files\Unity 6000.4.8f1\Editor\Unity.exe"
            // root = parent of "Editor" folder
            let p = PathBuf::from(&loc);
            let root = p
                .parent()
                .and_then(|editor| editor.parent())
                .map(|r| r.to_path_buf())
                .unwrap_or(p);
            try_add_path(&root, paths, seen);
        }
    }
}

/// Parse legacy editors.json formats:
/// - `{"version": {"location": "path"}}`
/// - `{"version": "path"}`
/// - `[{"location": "path"}]`
fn extract_v1_editor_paths(
    value: &serde_json::Value,
    paths: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (_key, val) in map {
                if let Some(loc) = val.get("location").and_then(|l| l.as_str()) {
                    try_add_path(&PathBuf::from(loc), paths, seen);
                } else if let Some(loc) = val.as_str() {
                    try_add_path(&PathBuf::from(loc), paths, seen);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(loc) = item.get("location").and_then(|l| l.as_str()) {
                    try_add_path(&PathBuf::from(loc), paths, seen);
                }
            }
        }
        _ => {}
    }
}

#[cfg(windows)]
fn hub_path_from_registry(hub_name: &str) -> Option<PathBuf> {
    let key_path = format!(
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{}",
        hub_name
    );
    query_registry_path(&key_path)
}

#[cfg(windows)]
fn query_registry_path(key_path: &str) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;

    unsafe {
        let key_path_wide: Vec<u16> = OsStr::new(key_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_path_wide.as_ptr(),
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
fn hub_path_from_registry(_hub_name: &str) -> Option<PathBuf> {
    None
}
