#![cfg(windows)]
//! unifuck-loader: cdylib injected into Tuanjie.exe via VERSION.dll forwarder
//! hijack. Patches `Licensing::LicensingClient::IsValidIPCResponse` and
//! `Licensing::LicensingClient::ValidateServerProcess` to unconditionally
//! return `true` once Sentinel HASP envelope finishes decrypting `.text`.

mod hook;
mod log_init;
mod panic_hook;
mod pdb_resolve;
mod unwrap;
mod util;
mod worker;

use core::ffi::c_void;
use core::ptr;

use windows_sys::Win32::Foundation::{BOOL, HINSTANCE, TRUE};
use windows_sys::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows_sys::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows_sys::Win32::System::Threading::CreateThread;

use crate::worker::worker_entry;

/// # Safety
///
/// Invoked exclusively by the Windows loader on `DLL_PROCESS_ATTACH` and the
/// other DLL notification reasons. Must never be called from Rust code.
#[export_name = "DllMain"]
pub unsafe extern "system" fn dll_main(
    hinst: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        // Detach DLL_THREAD_ATTACH/DETACH notifications: loader has no use for
        // them and they incur loader-lock cost in every thread that spawns.
        DisableThreadLibraryCalls(hinst);

        // Spawn worker with raw Win32 CreateThread to avoid CRT / std::thread
        // initialisation running under the loader lock — std::thread relies on
        // the Rust thread registry which is unsafe to touch here.
        let handle = CreateThread(
            ptr::null_mut(),
            0,
            Some(worker_entry),
            ptr::null_mut(),
            0,
            ptr::null_mut(),
        );
        if !handle.is_null() {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
        // Even if CreateThread failed we must return TRUE so Tuanjie.exe keeps
        // starting; the loader is best-effort and must never abort the host.
    }
    TRUE
}

// =====================================================================
// VERSION.dll runtime trampoline forwarder.
// =====================================================================
//
// Tuanjie.exe statically imports `VerQueryValueW` from VERSION.dll. We
// hijack VERSION.dll by deploying this cdylib as `version.dll` in the
// Editor directory. Tuanjie's loader resolves all 17 VERSION.dll exports
// against us, so we must forward every one of them to the real
// `C:\Windows\System32\version.dll`. Doing this at runtime (rather than
// PE forward strings) avoids shipping `version_orig.dll` and stays
// resilient to Windows updates of the system DLL.

mod forwarder {
    use core::ffi::c_void;
    use core::mem;

    use once_cell::sync::OnceCell;
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    static REAL_DLL: OnceCell<usize> = OnceCell::new();

    /// Returns a handle to the real `version.dll` in System32. The handle is
    /// cached for the lifetime of the process. We store it as `usize` because
    /// `HMODULE` (a raw pointer) is not `Send`/`Sync`; the integer encoding is
    /// safe across threads.
    fn real_version_dll() -> HMODULE {
        let h = *REAL_DLL.get_or_init(|| {
            // UTF-16 literal: "C:\\Windows\\System32\\version.dll\0".
            // We hard-code the System32 path so the OS loader cannot redirect
            // us back to ourselves via search-order side effects.
            const PATH: &[u16] = &[
                b'C' as u16,
                b':' as u16,
                b'\\' as u16,
                b'W' as u16,
                b'i' as u16,
                b'n' as u16,
                b'd' as u16,
                b'o' as u16,
                b'w' as u16,
                b's' as u16,
                b'\\' as u16,
                b'S' as u16,
                b'y' as u16,
                b's' as u16,
                b't' as u16,
                b'e' as u16,
                b'm' as u16,
                b'3' as u16,
                b'2' as u16,
                b'\\' as u16,
                b'v' as u16,
                b'e' as u16,
                b'r' as u16,
                b's' as u16,
                b'i' as u16,
                b'o' as u16,
                b'n' as u16,
                b'.' as u16,
                b'd' as u16,
                b'l' as u16,
                b'l' as u16,
                0,
            ];
            let h = unsafe { LoadLibraryW(PATH.as_ptr()) };
            h as usize
        });
        h as HMODULE
    }

    /// Resolves a symbol from the real `version.dll`. Returns `None` if the
    /// system DLL is missing or the symbol is absent — both are real
    /// "this should never happen" conditions on healthy Windows.
    pub(super) fn resolve(name: &[u8]) -> Option<unsafe extern "system" fn() -> isize> {
        // Caller passes a null-terminated ASCII byte slice.
        let h = real_version_dll();
        if h.is_null() {
            return None;
        }
        // GetProcAddress returns FARPROC, which is `Option<unsafe extern
        // "system" fn() -> isize>` in windows-sys. Callers transmute the raw
        // pointer to the actual signature.
        unsafe { GetProcAddress(h, name.as_ptr()) }
    }

    /// Helper macro to declare a forwarder export. Each export caches the
    /// resolved real function pointer in a per-export `OnceCell` so the
    /// `GetProcAddress` lookup only happens once.
    macro_rules! forward {
        ($fn_name:ident, $sym:literal, ($($arg:ident: $ty:ty),*$(,)?) -> $ret:ty, $fallback:expr) => {
            #[doc = concat!("Forwards `", $sym, "` to the real System32 version.dll.")]
            ///
            /// # Safety
            ///
            /// The Windows loader and host process call this export with the
            /// exact ABI and pointer validity contract of the corresponding
            /// System32 version.dll function.
            #[export_name = $sym]
            pub unsafe extern "system" fn $fn_name($($arg: $ty),*) -> $ret {
                type Fn = unsafe extern "system" fn($($ty),*) -> $ret;
                static SLOT: OnceCell<usize> = OnceCell::new();
                let raw = *SLOT.get_or_init(|| {
                    match resolve(concat!($sym, "\0").as_bytes()) {
                        Some(f) => f as usize,
                        None => 0,
                    }
                });
                if raw == 0 {
                    // System32\version.dll missing or symbol absent — return a
                    // safe failure value so the caller treats it as "no info".
                    return $fallback;
                }
                let f: Fn = mem::transmute(raw);
                f($($arg),*)
            }
        };
    }

    // Pull in the Win32 types we need for the signatures.
    use windows_sys::core::{PCSTR, PCWSTR};
    use windows_sys::Win32::Foundation::{BOOL, HANDLE};

    // BOOL return = 0 (FALSE), DWORD/u32 return = 0, UINT return = 0.

    forward!(get_file_version_info_a, "GetFileVersionInfoA",
        (filename: PCSTR, handle: u32, len: u32, data: *mut c_void) -> BOOL, 0);

    forward!(get_file_version_info_by_handle, "GetFileVersionInfoByHandle",
        (handle: HANDLE, filename: PCWSTR, dummy: u32, len: u32) -> BOOL, 0);

    forward!(get_file_version_info_ex_a, "GetFileVersionInfoExA",
        (flags: u32, filename: PCSTR, handle: u32, len: u32, data: *mut c_void) -> BOOL, 0);

    forward!(get_file_version_info_ex_w, "GetFileVersionInfoExW",
        (flags: u32, filename: PCWSTR, handle: u32, len: u32, data: *mut c_void) -> BOOL, 0);

    forward!(get_file_version_info_size_a, "GetFileVersionInfoSizeA",
        (filename: PCSTR, handle: *mut u32) -> u32, 0);

    forward!(get_file_version_info_size_ex_a, "GetFileVersionInfoSizeExA",
        (flags: u32, filename: PCSTR, handle: *mut u32) -> u32, 0);

    forward!(get_file_version_info_size_ex_w, "GetFileVersionInfoSizeExW",
        (flags: u32, filename: PCWSTR, handle: *mut u32) -> u32, 0);

    forward!(get_file_version_info_size_w, "GetFileVersionInfoSizeW",
        (filename: PCWSTR, handle: *mut u32) -> u32, 0);

    forward!(get_file_version_info_w, "GetFileVersionInfoW",
        (filename: PCWSTR, handle: u32, len: u32, data: *mut c_void) -> BOOL, 0);

    forward!(ver_find_file_a, "VerFindFileA",
        (flags: u32, filename: PCSTR, win_dir: PCSTR, app_dir: PCSTR,
         cur_dir: *mut u8, cur_dir_len: *mut u32, dest_dir: *mut u8,
         dest_dir_len: *mut u32) -> u32, 0);

    forward!(ver_find_file_w, "VerFindFileW",
        (flags: u32, filename: PCWSTR, win_dir: PCWSTR, app_dir: PCWSTR,
         cur_dir: *mut u16, cur_dir_len: *mut u32, dest_dir: *mut u16,
         dest_dir_len: *mut u32) -> u32, 0);

    forward!(ver_install_file_a, "VerInstallFileA",
        (flags: u32, src: PCSTR, dst: PCSTR, src_dir: PCSTR, dst_dir: PCSTR,
         cur_dir: PCSTR, tmp_file: *mut u8, tmp_file_len: *mut u32) -> u32, 0);

    forward!(ver_install_file_w, "VerInstallFileW",
        (flags: u32, src: PCWSTR, dst: PCWSTR, src_dir: PCWSTR, dst_dir: PCWSTR,
         cur_dir: PCWSTR, tmp_file: *mut u16, tmp_file_len: *mut u32) -> u32, 0);

    forward!(ver_language_name_a, "VerLanguageNameA",
        (lang: u32, buf: *mut u8, buf_len: u32) -> u32, 0);

    forward!(ver_language_name_w, "VerLanguageNameW",
        (lang: u32, buf: *mut u16, buf_len: u32) -> u32, 0);

    forward!(ver_query_value_a, "VerQueryValueA",
        (block: *const c_void, sub_block: PCSTR,
         lplp_buffer: *mut *mut c_void, pu_len: *mut u32) -> BOOL, 0);

    forward!(ver_query_value_w, "VerQueryValueW",
        (block: *const c_void, sub_block: PCWSTR,
         lplp_buffer: *mut *mut c_void, pu_len: *mut u32) -> BOOL, 0);
}
