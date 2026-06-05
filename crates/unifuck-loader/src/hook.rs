//! Raw inline patch: writes 4 bytes (`mov al, 1; ret; nop`) at the target so
//! the function returns `true` unconditionally. We deliberately do not use
//! MinHook — the Sentinel HASP envelope hardening notes (see
//! `.work/inject/sentinel-self-integrity.md`) argue that a bare 4-byte stub
//! has a smaller signature surface than a 5-byte JMP trampoline.

use core::ptr;

use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    VirtualProtect, PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const STUB: [u8; 4] = [0xB0, 0x01, 0xC3, 0x90]; // mov al,1 ; ret ; nop

pub enum HookError {
    VirtualProtect(u32),
    FlushCache(u32),
}

impl core::fmt::Debug for HookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::VirtualProtect(code) => write!(f, "VirtualProtect failed (GetLastError={code})"),
            Self::FlushCache(code) => {
                write!(f, "FlushInstructionCache failed (GetLastError={code})")
            }
        }
    }
}

/// Overwrites the first 4 bytes at `target` with a `return true` stub.
///
/// # Safety
///
/// `target` must point to executable host code that we are allowed to
/// rewrite (i.e. inside the host EXE's `.text` section, with the page
/// currently committed). Concurrent execution of those 4 bytes during the
/// write is benign on x64 (single-quadword aligned writes are atomic if the
/// store does not cross a cache line), but the caller should still try to
/// install hooks before the target is invoked.
pub unsafe fn install_return_true(target: *mut u8) -> Result<(), HookError> {
    let mut old_protect: PAGE_PROTECTION_FLAGS = PAGE_EXECUTE_READ;
    if VirtualProtect(
        target as _,
        STUB.len(),
        PAGE_EXECUTE_READWRITE,
        &mut old_protect,
    ) == 0
    {
        return Err(HookError::VirtualProtect(last_error()));
    }

    ptr::copy_nonoverlapping(STUB.as_ptr(), target, STUB.len());

    let mut tmp: PAGE_PROTECTION_FLAGS = 0;
    // We intentionally ignore the restore-protection failure: at this point
    // the bytes are already patched and FlushInstructionCache will commit
    // them to the icache. A failed restore leaves the page as RWX which is a
    // minor hardening regression but not a functional problem.
    let _ = VirtualProtect(target as _, STUB.len(), old_protect, &mut tmp);

    if FlushInstructionCache(GetCurrentProcess(), target as _, STUB.len()) == 0 {
        return Err(HookError::FlushCache(last_error()));
    }
    Ok(())
}

fn last_error() -> u32 {
    unsafe { windows_sys::Win32::Foundation::GetLastError() }
}
