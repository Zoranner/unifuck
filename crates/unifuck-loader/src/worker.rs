//! Worker thread entry. Lives off the loader lock so heavy work (PDB parse,
//! file IO, polling) is safe.

use core::ffi::c_void;
use std::time::Duration;

use crate::{hook, log_init, panic_hook, pdb_resolve, unwrap, util};

const UNWRAP_TIMEOUT: Duration = Duration::from_secs(30);

enum LoaderError {
    NoHostBase,
    Unwrap(unwrap::UnwrapError),
    Pdb(pdb_resolve::PdbError),
    HookIsValidIpc(hook::HookError),
    HookValidateServer(hook::HookError),
}

impl core::fmt::Debug for LoaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoHostBase => f.write_str("host module handle is null"),
            Self::Unwrap(e) => write!(f, "unwrap: {e:?}"),
            Self::Pdb(e) => write!(f, "pdb: {e:?}"),
            Self::HookIsValidIpc(e) => write!(f, "hook IsValidIPCResponse: {e:?}"),
            Self::HookValidateServer(e) => write!(f, "hook ValidateServerProcess: {e:?}"),
        }
    }
}

/// Called by `CreateThread` from `DllMain`. The signature is fixed by the
/// Win32 API.
pub(crate) unsafe extern "system" fn worker_entry(_lp: *mut c_void) -> u32 {
    // Install the panic hook **before** anything else so even a logger init
    // failure can be reported without aborting the host.
    panic_hook::install();

    // Best-effort log init: failure means log:: macros become no-ops.
    let _ = log_init::init();
    log_init::banner();

    match run() {
        Ok(()) => log::info!("loader installed hooks successfully"),
        Err(e) => log::error!("loader aborted: {e:?}"),
    }
    0
}

fn run() -> Result<(), LoaderError> {
    let image_base = util::image_base();
    if image_base == 0 {
        return Err(LoaderError::NoHostBase);
    }
    log::info!("host image base = {:#x}", image_base);

    if let Some((text_start, text_size)) = util::text_section_range() {
        log::info!(".text @ {:#x} size={:#x}", text_start, text_size);
    } else {
        log::warn!(".text section header unreadable; proceeding anyway");
    }

    // Resolve symbols **first** so we know which VA to poll for plaintext.
    // PDB parse only touches our own process's filesystem; it does not
    // require Sentinel to have decrypted anything yet.
    let targets = pdb_resolve::resolve(image_base).map_err(LoaderError::Pdb)?;
    log::info!(
        "targets: IsValidIPCResponse @ {:#x}, ValidateServerProcess @ {:#x}",
        targets.is_valid_ipc_response,
        targets.validate_server_process
    );

    // Poll the IsValidIPCResponse prologue until Sentinel finishes decrypting
    // it. We only need to wait on one of the two; once `.text` is plaintext
    // both targets are decrypted.
    unwrap::wait_for_text_decryption(targets.is_valid_ipc_response, UNWRAP_TIMEOUT)
        .map_err(LoaderError::Unwrap)?;

    // SAFETY: targets came from a verified PDB; addresses sit inside the
    // host EXE's `.text` section which we have just confirmed is plaintext.
    unsafe {
        hook::install_return_true(targets.is_valid_ipc_response as *mut u8)
            .map_err(LoaderError::HookIsValidIpc)?;
        log::info!(
            "patched IsValidIPCResponse @ {:#x}",
            targets.is_valid_ipc_response
        );
        hook::install_return_true(targets.validate_server_process as *mut u8)
            .map_err(LoaderError::HookValidateServer)?;
        log::info!(
            "patched ValidateServerProcess @ {:#x}",
            targets.validate_server_process
        );
    }
    Ok(())
}
