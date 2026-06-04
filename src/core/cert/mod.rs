mod exe;
mod shared_client;
mod tuanjie;
mod unity;

use anyhow::Context;
use std::path::Path;

use crate::vendor::Vendor;

use super::base64_lines;
use super::install::EditorInstall;

pub use exe::{is_exe_patched, patch_exe, unpatch_exe};
pub use shared_client::{
    is_shared_licensing_client_patched, patch_shared_licensing_client,
    unpatch_shared_licensing_client,
};

/// Top-level vendor-aware DLL patch entry point.
pub fn patch_dll(vendor: Vendor, dll_path: &Path) -> anyhow::Result<()> {
    match vendor {
        Vendor::Unity => unity::patch_dll(dll_path),
        Vendor::Tuanjie => tuanjie::patch_dll(dll_path),
    }
}

/// Restore a DLL from its .bak backup. Vendor-agnostic.
pub fn unpatch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let bak_path = dll_path.with_extension("dll.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    restore_from_backup(dll_path)
}

/// Check whether an editor install is currently patched.
///
/// Unity: `.dll.bak` existence OR base64-encoded modified cert line found
/// in the DLL contents.
/// Tuanjie: `.dll.bak` existence only (UTF-16LE PEM contents are not
/// detectable via base64 line matching).
pub fn is_patched(install: &EditorInstall) -> bool {
    let dll_path = install.dll_path.as_path();
    if dll_path.with_extension("dll.bak").exists() {
        return true;
    }

    match install.vendor {
        Vendor::Tuanjie => false,
        Vendor::Unity => {
            if let Ok(data) = std::fs::read(dll_path) {
                if let Ok(pem0) = pem::parse(unity::MOD_CERT_PEM) {
                    let b64 = base64_lines(pem0.contents());
                    if let Some(line) = b64.lines().next() {
                        return data.windows(line.len()).any(|w| w == line.as_bytes());
                    }
                }
            }
            false
        }
    }
}

/// Shared backup-restore helper used by all unpatch entry points.
pub(super) fn restore_from_backup(path: &Path) -> anyhow::Result<()> {
    let bak_path = path.with_extension(
        path.extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .to_string()
            + ".bak",
    );
    if !bak_path.exists() {
        return Ok(());
    }
    let data = std::fs::read(&bak_path)
        .with_context(|| format!("read backup failed: {}", bak_path.display()))?;
    std::fs::write(path, &data)
        .with_context(|| format!("restore write failed: {}", path.display()))?;
    std::fs::remove_file(&bak_path)
        .with_context(|| format!("remove backup failed: {}", path.display()))?;
    Ok(())
}
