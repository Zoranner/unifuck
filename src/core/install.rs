use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::{base64_lines, MOD_CERT_PEM};

const LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll";
const EDITOR_EXE_SUBPATH: &str = r"Editor\Unity.exe";

const TUANJIE_LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Tuanjie.Licensing.EntitlementResolver.dll";
const TUANJIE_EDITOR_EXE_SUBPATH: &str = r"Editor\Tuanjie.exe";

#[derive(Clone)]
pub struct UnityInstall {
    pub version: String,
    pub display_name: &'static str,
    pub dll_path: PathBuf,
    pub exe_path: PathBuf,
}

pub fn is_patched(dll_path: &Path) -> bool {
    if dll_path.with_extension("dll.bak").exists() {
        return true;
    }

    // 内容检查仅适用于 Unity DLL (base64 文本); Tuanjie DLL 使用 UTF-16LE PEM,
    // 无法通过 base64 行匹配检测, 仅依赖上方 .bak 检查。
    let filename = dll_path.file_name().unwrap_or_default().to_string_lossy();
    if filename.starts_with("Tuanjie.") {
        return false;
    }

    if let Ok(data) = std::fs::read(dll_path) {
        if let Ok(pem0) = pem::parse(MOD_CERT_PEM) {
            let b64 = base64_lines(pem0.contents());
            if let Some(line) = b64.lines().next() {
                return data.windows(line.len()).any(|w| w == line.as_bytes());
            }
        }
    }
    false
}

// ── Scan Helpers ───────────────────────────────────────────────

/// Try adding a Unity or Tuanjie editor install at `root` to the list.
/// Checks both Unity and Tuanjie DLL locations.
fn add_if_valid_install(
    root: &Path,
    version: &str,
    installs: &mut Vec<UnityInstall>,
    seen: &mut HashSet<PathBuf>,
) {
    if let Some(install) = try_find_install(root, version) {
        if seen.insert(install.dll_path.clone()) {
            installs.push(install);
        }
    }
    if let Some(install) = try_find_tuanjie_install(root, version) {
        if seen.insert(install.dll_path.clone()) {
            installs.push(install);
        }
    }
}

/// Scan a Program Files directory for standalone Unity/Tuanjie editor folders.
fn scan_pf_for_editors(
    pf_path: &Path,
    installs: &mut Vec<UnityInstall>,
    seen: &mut HashSet<PathBuf>,
) {
    if let Ok(dir) = std::fs::read_dir(pf_path) {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if path.is_dir() {
                if let Some(version) = name.strip_prefix("Unity ") {
                    add_if_valid_install(&path, version, installs, seen);
                } else if let Some(version) = name.strip_prefix("Tuanjie ") {
                    add_if_valid_install(&path, version, installs, seen);
                }
            }
        }
    }
}

/// Scan a Hub-managed Editor directory (e.g. `<Hub>\Editor\`) for version folders.
fn scan_hub_editor_dir(
    editor_dir: &Path,
    installs: &mut Vec<UnityInstall>,
    seen: &mut HashSet<PathBuf>,
) {
    if let Ok(dir) = std::fs::read_dir(editor_dir) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let version = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                add_if_valid_install(&path, &version, installs, seen);
            }
        }
    }
}

fn try_find_install(root: &Path, version: &str) -> Option<UnityInstall> {
    let dll_path = root.join(LICENSING_DLL_SUBPATH);
    if dll_path.exists() {
        Some(UnityInstall {
            version: version.to_string(),
            display_name: "Unity",
            dll_path,
            exe_path: root.join(EDITOR_EXE_SUBPATH),
        })
    } else {
        None
    }
}

fn try_find_tuanjie_install(root: &Path, version: &str) -> Option<UnityInstall> {
    let dll_path = root.join(TUANJIE_LICENSING_DLL_SUBPATH);
    if dll_path.exists() {
        Some(UnityInstall {
            version: version.to_string(),
            display_name: "Tuanjie",
            dll_path,
            exe_path: root.join(TUANJIE_EDITOR_EXE_SUBPATH),
        })
    } else {
        None
    }
}

/// Sort key: Unity before Tuanjie, any unknown product last.
fn sort_order(name: &str) -> u8 {
    match name {
        "Unity" => 0,
        "Tuanjie" => 1,
        _ => 2,
    }
}

pub fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();
    let mut seen = HashSet::new();

    // 1. Standalone: scan Program Files on all drives
    for pf in crate::discovery::program_files_dirs() {
        scan_pf_for_editors(&pf, &mut installs, &mut seen);
    }

    // 2. Hub-managed: derive from discovered Hub roots
    for hub_name in ["Unity Hub", "Tuanjie Hub"] {
        for hub_root in crate::discovery::find_hub_roots(hub_name) {
            let editor_dir = hub_root.join("Editor");
            scan_hub_editor_dir(&editor_dir, &mut installs, &mut seen);
        }
    }

    // 3. Custom paths from editors.json
    for editor_path in crate::discovery::read_editor_paths_from_config() {
        let version = editor_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        add_if_valid_install(&editor_path, &version, &mut installs, &mut seen);
    }

    installs.sort_by(|a, b| {
        sort_order(a.display_name)
            .cmp(&sort_order(b.display_name))
            .then_with(|| b.version.cmp(&a.version))
    });
    installs
}
