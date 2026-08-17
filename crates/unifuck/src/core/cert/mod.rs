mod binary;
mod exe;
mod shared_client;
mod tuanjie;
mod unity;

use std::path::Path;

use crate::vendor::Vendor;

use super::base64_lines;
use super::group::EditorInfo;

use self::binary::{bak_path, restore_from_backup};

pub use exe::{is_exe_patched, patch_exe, unpatch_exe};
pub use shared_client::{
    is_shared_licensing_client_patched, patch_shared_licensing_client,
    unpatch_shared_licensing_client,
};

/// Top-level vendor-aware DLL patch entry point.
///
/// Both vendors return `Ok(())` only after a real patch; "cert not found"
/// is an error for Editor DLLs (the DLL must contain the vendor cert).
pub fn patch_dll(vendor: Vendor, dll_path: &Path) -> anyhow::Result<()> {
    match vendor {
        Vendor::Unity => unity::patch_dll(dll_path),
        Vendor::Tuanjie => {
            let patched = tuanjie::patch_dll(dll_path)?;
            if !patched {
                anyhow::bail!("cert not found in DLL: {}", dll_path.display());
            }
            Ok(())
        }
    }
}

/// Restore a DLL from its .bak backup. Vendor-agnostic.
pub fn unpatch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let bak_path = bak_path(dll_path);
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    restore_from_backup(dll_path)
}

/// Check whether an editor install is currently patched.
///
/// Primary signal: `.dll.bak` backup exists. Fallback: content-level check
/// (modified cert material present in the DLL), which stays correct when a
/// user deletes the backup after patching.
pub fn is_patched(install: &EditorInfo) -> bool {
    let dll_path = install.dll_path();
    if bak_path(&dll_path).exists() {
        return true;
    }

    let Ok(data) = std::fs::read(dll_path) else {
        return false;
    };
    match install.vendor {
        Vendor::Unity => {
            let Ok(pem0) = pem::parse(unity::UNITY_MOD_CERT_PEM) else {
                return false;
            };
            let b64 = base64_lines(pem0.contents());
            b64.lines().any(|line| {
                !line.is_empty() && data.windows(line.len()).any(|w| w == line.as_bytes())
            })
        }
        Vendor::Tuanjie => {
            let mod_utf16 = tuanjie::to_utf16le_crlf(tuanjie::TUANJIE_MOD_CERT_PEM.trim_end());
            data.windows(mod_utf16.len())
                .any(|w| w == mod_utf16.as_slice())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor::Vendor;

    /// 构造一个 EditorInfo, root 下按 Editor 布局放 DLL 文件。
    /// 返回 TempDir 供调用方持有, 否则目录在函数返回时被删除。
    fn editor_with_dll(vendor: Vendor, dll_bytes: &[u8]) -> (EditorInfo, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let client_dir = dir.path().join(r"Data\Resources\Licensing\Client");
        std::fs::create_dir_all(&client_dir).unwrap();
        std::fs::write(client_dir.join(vendor.dll_name()), dll_bytes).unwrap();
        let info = EditorInfo {
            version: "2022.3".to_string(),
            vendor,
            root: dir.path().to_path_buf(),
        };
        (info, dir)
    }

    #[test]
    fn is_patched_tuanjie_detects_utf16le_cert_without_bak() {
        // DLL 内容含修改版证书的 UTF-16LE 编码 (丢失 .bak 后的状态)
        let mod_utf16 = tuanjie::to_utf16le_crlf(tuanjie::TUANJIE_MOD_CERT_PEM.trim_end());
        let mut dll = b"prefix garbage".to_vec();
        dll.extend_from_slice(&mod_utf16);
        dll.extend_from_slice(b"suffix garbage");
        let (info, _dir) = editor_with_dll(Vendor::Tuanjie, &dll);
        assert!(
            is_patched(&info),
            "Tuanjie DLL containing the modified cert must report patched without .bak"
        );
    }

    #[test]
    fn is_patched_tuanjie_clean_dll_is_not_patched() {
        let orig_utf16 = tuanjie::to_utf16le_crlf(tuanjie::TUANJIE_ORIG_CERT_PEM.trim_end());
        let mut dll = b"prefix garbage".to_vec();
        dll.extend_from_slice(&orig_utf16);
        let (info, _dir) = editor_with_dll(Vendor::Tuanjie, &dll);
        assert!(!is_patched(&info));
    }

    #[test]
    fn is_patched_unity_detects_mod_cert_line_without_bak() {
        // Unity DLL: 修改版证书的 base64 行 (76 字符/行) 直接嵌入
        let pem_mod = pem::parse(unity::UNITY_MOD_CERT_PEM).unwrap();
        let b64 = base64_lines(pem_mod.contents());
        let first_line = b64.lines().next().unwrap();
        let dll = format!("junk\n{}\njunk", first_line);
        let (info, _dir) = editor_with_dll(Vendor::Unity, dll.as_bytes());
        assert!(
            is_patched(&info),
            "Unity DLL containing a modified cert base64 line must report patched without .bak"
        );
    }
}
