use std::collections::HashSet;
use std::path::PathBuf;

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

// ── Hub Discovery ──────────────────────────────────────────────

/// Finds all installations of a hub (e.g., "Unity Hub" or "Tuanjie Hub")
/// across all drives.
///
/// Strategy:
/// 1. Check registry for exact install path
/// 2. Enumerate `<drive>:\Program Files\<hub_name>` for each available drive
///
/// Results are deduplicated and verified by checking `resources\app.asar`.
pub(crate) fn find_hub_roots(hub_name: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut roots = Vec::new();
    let asar_rel = r"resources\app.asar";

    // Registry first (exact path)
    if let Some(path) = hub_path_from_registry(hub_name) {
        let canonical = normalize_path(&path);
        if seen.insert(canonical) && path.join(asar_rel).exists() {
            roots.push(path);
        }
    }

    // Program Files on all drives
    for drive in available_drive_roots() {
        let candidate = drive.join("Program Files").join(hub_name);
        let canonical = normalize_path(&candidate);
        if seen.insert(canonical) && candidate.join(asar_rel).exists() {
            roots.push(candidate);
        }
    }

    roots
}

// ── Editor Discovery ───────────────────────────────────────────

/// Reads editor install paths from Hub `editors.json` config files.
///
/// Checks both `%APPDATA%\UnityHub\editors.json` and
/// `%APPDATA%\TuanjieHub\editors.json`.  The JSON format varies:
/// - `{"version": {"location": "path"}}`
/// - `{"version": "path"}`
/// - `[{"location": "path"}]`
///
/// Only existing directories are returned.
pub(crate) fn read_editor_paths_from_config() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();

    let appdata = match std::env::var("APPDATA") {
        Ok(v) => PathBuf::from(v),
        Err(_) => return paths,
    };

    for vendor in crate::vendor::Vendor::all() {
        let config_path = appdata.join(vendor.config_dir_name()).join("editors.json");
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
                extract_editor_paths(&value, &mut paths, &mut seen);
            }
        }
    }

    paths
}

/// Returns `<drive>:\Program Files` for each available drive.
pub(crate) fn program_files_dirs() -> Vec<PathBuf> {
    available_drive_roots()
        .into_iter()
        .map(|root| root.join("Program Files"))
        .collect()
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

fn extract_editor_paths(
    value: &serde_json::Value,
    paths: &mut Vec<PathBuf>,
    seen: &mut HashSet<PathBuf>,
) {
    match value {
        // Object: {"version": {"location": "path"}} or {"version": "path"}
        serde_json::Value::Object(map) => {
            for (_key, val) in map {
                if let Some(loc) = val.get("location").and_then(|l| l.as_str()) {
                    add_editor_path(loc, paths, seen);
                } else if let Some(loc) = val.as_str() {
                    add_editor_path(loc, paths, seen);
                }
            }
        }
        // Array: [{"location": "path"}]
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(loc) = item.get("location").and_then(|l| l.as_str()) {
                    add_editor_path(loc, paths, seen);
                }
            }
        }
        _ => {}
    }
}

fn add_editor_path(path_str: &str, paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) {
    let p = PathBuf::from(path_str);
    let canonical = normalize_path(&p);
    if seen.insert(canonical) && p.is_dir() {
        paths.push(p);
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
