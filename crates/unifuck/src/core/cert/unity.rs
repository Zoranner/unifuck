use anyhow::Context;
use std::path::Path;

use super::binary::patch_binary_file;

pub(super) const UNITY_ORIG_CERT_PEM: &str =
    include_str!("../../../keys/unity/orig/Unity.Licensing.EntitlementResolver.Unity.pem");
pub(super) const UNITY_MOD_CERT_PEM: &str =
    include_str!("../../../keys/unity/mod/Unity.Licensing.EntitlementResolver.Unity.pem");

/// Unity-only DLL patch: bails if the original cert was not found.
pub fn patch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let patched = patch_binary_file(
        dll_path,
        UNITY_ORIG_CERT_PEM,
        UNITY_MOD_CERT_PEM,
        false,
        0,
        false,
    )
    .context("patch Unity DLL failed")?;
    if !patched {
        anyhow::bail!("cert not found in DLL: {}", dll_path.display());
    }
    Ok(())
}
