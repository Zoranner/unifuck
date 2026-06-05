#![cfg(windows)]

use windows_sys::Win32::Foundation::{BOOL, HINSTANCE, TRUE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;

/// # Safety
///
/// Invoked exclusively by the Windows loader on `DLL_PROCESS_ATTACH` and the
/// other DLL notification reasons. Must never be called from Rust code.
#[export_name = "DllMain"]
pub unsafe extern "system" fn dll_main(
    hinst: HINSTANCE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DisableThreadLibraryCalls(hinst);
    }
    TRUE
}
