use std::path::{Path, PathBuf};

use super::{base64_lines, MOD_CERT_PEM};

const LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll";
const EDITOR_EXE_SUBPATH: &str = r"Editor\Unity.exe";

const TUANJIE_LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Tuanjie.Licensing.EntitlementResolver.dll";
const TUANJIE_EDITOR_EXE_SUBPATH: &str = r"Editor\Tuanjie.exe";

/// Program Files roots to scan for Editor installations (covers C: and D: drives).
const PROGRAM_FILES_ROOTS: &[&str] = &[r"C:\Program Files", r"D:\Program Files"];

#[derive(Clone)]
pub struct UnityInstall {
    pub version: String,
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

fn try_find_install(root: &Path, version: &str) -> Option<UnityInstall> {
    let dll_path = root.join(LICENSING_DLL_SUBPATH);
    if dll_path.exists() {
        Some(UnityInstall {
            version: version.to_string(),
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
            version: format!("Tuanjie {}", version),
            dll_path,
            exe_path: root.join(TUANJIE_EDITOR_EXE_SUBPATH),
        })
    } else {
        None
    }
}

pub fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();

    // Scan standalone installations under each Program Files root
    for pf_root in PROGRAM_FILES_ROOTS {
        let pf_path = Path::new(pf_root);
        if let Ok(dir) = std::fs::read_dir(pf_path) {
            for entry in dir.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if path.is_dir() {
                    if let Some(version) = name.strip_prefix("Unity ") {
                        if let Some(install) = try_find_install(&path, version) {
                            installs.push(install);
                        }
                    } else if let Some(version) = name.strip_prefix("Tuanjie ") {
                        if let Some(install) = try_find_tuanjie_install(&path, version) {
                            installs.push(install);
                        }
                    }
                }
            }
        }
    }

    // Scan Hub-managed Editor installations under each Program Files root
    for pf_root in PROGRAM_FILES_ROOTS {
        let hub_editor_dirs: &[(String, bool)] = &[
            (format!(r"{}\Unity Hub\Editor", pf_root), false),
            (format!(r"{}\Tuanjie Hub\Editor", pf_root), true),
        ];
        for (hub_editor_dir, is_tuanjie) in hub_editor_dirs {
            if let Ok(hub_dir) = std::fs::read_dir(hub_editor_dir) {
                for entry in hub_dir.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        let version = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| "UNKNOWN".to_string());
                        let install = if *is_tuanjie {
                            try_find_tuanjie_install(&path, &version)
                        } else {
                            try_find_install(&path, &version)
                        };
                        if let Some(install) = install {
                            installs.push(install);
                        }
                    }
                }
            }
        }
    }

    installs.sort_by(|a, b| b.version.cmp(&a.version));
    installs
}
