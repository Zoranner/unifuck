mod cert;
mod hosts;
mod install;
mod license;

use base64::Engine;

pub use cert::{
    is_exe_patched, is_patched, is_shared_licensing_client_patched, patch_dll, patch_exe,
    patch_shared_licensing_client, unpatch_dll, unpatch_exe, unpatch_shared_licensing_client,
};
pub use hosts::{clean_hosts, write_hosts_if_needed};
pub use install::{scan_installations, EditorInstall};
pub use license::{deploy_license, remove_license, sign_license};

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
