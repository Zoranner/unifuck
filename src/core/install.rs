use std::path::{Path, PathBuf};

use super::{base64_lines, MOD_CERT_PEM};

const LICENSING_DLL_SUBPATH: &str =
    r"Editor\Data\Resources\Licensing\Client\Unity.Licensing.EntitlementResolver.dll";
const EDITOR_EXE_SUBPATH: &str = r"Editor\Unity.exe";

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

pub fn scan_installations() -> Vec<UnityInstall> {
    let mut installs = Vec::new();

    if let Ok(dir) = std::fs::read_dir(r"C:\Program Files") {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if path.is_dir() && name.starts_with("Unity ") {
                let version = name.strip_prefix("Unity ").unwrap_or(&name);
                if let Some(install) = try_find_install(&path, version) {
                    installs.push(install);
                }
            }
        }
    }

    if let Ok(hub_dir) = std::fs::read_dir(r"C:\Program Files\Unity Hub\Editor") {
        for entry in hub_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let version = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                if let Some(install) = try_find_install(&path, &version) {
                    installs.push(install);
                }
            }
        }
    }

    installs.sort_by(|a, b| b.version.cmp(&a.version));
    installs
}
