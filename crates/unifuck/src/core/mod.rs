mod cert;
mod group;
mod hosts;
mod license;
mod loader;

use base64::Engine;

pub use cert::{
    is_exe_patched, is_patched, is_shared_licensing_client_patched, patch_dll, patch_exe,
    patch_shared_licensing_client, unpatch_dll, unpatch_exe, unpatch_shared_licensing_client,
};
pub use group::{scan_groups, EditorInfo, InstallGroup};
pub use hosts::{deploy_hosts, remove_hosts};
pub use license::{deploy_license, remove_license, sign_license};
pub use loader::{deploy as deploy_loader, revert as revert_loader};

pub(super) fn base64_lines(der: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::with_capacity(b64.len() + b64.len() / 76 + 1);
    for (i, chunk) in b64.as_bytes().chunks(76).enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(std::str::from_utf8(chunk).unwrap());
    }
    out.push('\n');
    out
}
