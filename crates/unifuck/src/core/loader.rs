use std::path::Path;

use anyhow::Context;

/// Forwarder DLL name our cdylib must masquerade as. Tuanjie.exe statically
/// imports `VerQueryValueW` from VERSION.dll, so Windows resolves that name
/// against the Editor directory before falling through to System32.
pub const FORWARDER_NAME: &str = "version.dll";

const LOADER_DLL_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/unifuck_loader.dll"));

fn target_paths(editor_dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let target = editor_dir.join(FORWARDER_NAME);
    let bak = editor_dir.join(format!("{FORWARDER_NAME}.bak"));
    (target, bak)
}

/// Deploy the loader cdylib as `version.dll` next to Tuanjie.exe. If a real
/// `version.dll` already exists in the Editor directory it is preserved as
/// `version.dll.bak`. Idempotent: re-deploying just overwrites our DLL while
/// leaving the existing `.bak` intact.
pub fn deploy(editor_dir: &Path) -> anyhow::Result<()> {
    let (target, bak) = target_paths(editor_dir);
    if target.exists() && !bak.exists() {
        std::fs::copy(&target, &bak)
            .with_context(|| format!("backup write failed: {}", bak.display()))?;
    }
    std::fs::write(&target, LOADER_DLL_BYTES)
        .with_context(|| format!("loader write failed: {}", target.display()))?;
    Ok(())
}

/// Revert the loader deployment. If a `.bak` exists it is restored over the
/// forwarder file; otherwise the forwarder file (if any) is removed.
pub fn revert(editor_dir: &Path) -> anyhow::Result<()> {
    let (target, bak) = target_paths(editor_dir);
    if bak.exists() {
        std::fs::copy(&bak, &target)
            .with_context(|| format!("restore from bak failed: {}", target.display()))?;
        std::fs::remove_file(&bak)
            .with_context(|| format!("remove bak failed: {}", bak.display()))?;
    } else if target.exists() {
        std::fs::remove_file(&target)
            .with_context(|| format!("remove loader failed: {}", target.display()))?;
    }
    Ok(())
}
