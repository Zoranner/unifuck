//! Waits for Sentinel HASP envelope to finish decrypting `.text` before any
//! hook attempt. Strategy: poll the IsValidIPCResponse prologue bytes every
//! 50 ms, accept once they look like a real x64 MSVC function prologue.

use core::mem::MaybeUninit;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Memory::{
    VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS,
};

pub enum UnwrapError {
    Timeout,
}

impl core::fmt::Debug for UnwrapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Timeout => f.write_str("wait_for_text_decryption timed out"),
        }
    }
}

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const READ_LEN: usize = 16;

/// Blocks until the bytes at `target_va` look like a decrypted x64 function
/// prologue, or `timeout` elapses.
pub fn wait_for_text_decryption(target_va: usize, timeout: Duration) -> Result<(), UnwrapError> {
    let deadline = Instant::now() + timeout;
    let mut polls: u64 = 0;
    loop {
        if Instant::now() >= deadline {
            log::error!("unwrap timeout after {polls} polls @ {target_va:#x}");
            return Err(UnwrapError::Timeout);
        }
        match safe_read(target_va, READ_LEN) {
            Ok(buf) => {
                if looks_like_x64_prologue(&buf) {
                    log::info!(
                        "decryption detected @ {:#x} after {} polls, prologue={:02x?}",
                        target_va,
                        polls,
                        &buf[..8]
                    );
                    return Ok(());
                }
            }
            Err(e) => {
                // VirtualQuery may legitimately fail (page not yet committed
                // while envelope is still mapping). Treat as "wait and retry".
                if polls.is_multiple_of(20) {
                    log::debug!("safe_read err @ {target_va:#x}: {e:?}");
                }
            }
        }
        polls += 1;
        thread::sleep(POLL_INTERVAL);
    }
}

enum ReadError {
    QueryFailed,
    NotCommitted,
    NoAccess,
}

impl core::fmt::Debug for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::QueryFailed => "VirtualQuery returned 0",
            Self::NotCommitted => "page not committed",
            Self::NoAccess => "page is NOACCESS or GUARD",
        })
    }
}

fn safe_read(va: usize, n: usize) -> Result<Vec<u8>, ReadError> {
    let mut mbi = MaybeUninit::<MEMORY_BASIC_INFORMATION>::zeroed();
    let written = unsafe {
        VirtualQuery(
            va as *const _,
            mbi.as_mut_ptr(),
            core::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    if written == 0 {
        return Err(ReadError::QueryFailed);
    }
    let mbi = unsafe { mbi.assume_init() };
    if mbi.State != MEM_COMMIT {
        return Err(ReadError::NotCommitted);
    }
    if mbi.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0 {
        return Err(ReadError::NoAccess);
    }
    let mut buf = vec![0u8; n];
    unsafe {
        core::ptr::copy_nonoverlapping(va as *const u8, buf.as_mut_ptr(), n);
    }
    Ok(buf)
}

/// Heuristic: does this byte slice look like a real MSVC x64 function
/// prologue? Strategy:
/// 1. Reject all-zero, all-0xCC, or high-entropy ciphertext.
/// 2. First byte must be a REX/push/sub opcode commonly used in prologues.
/// 3. Within the first 8 bytes, expect either `sub rsp, imm` or a home-area
///    register spill (`mov [rsp+x], rbx/rsi/rdi/rcx`).
fn looks_like_x64_prologue(b: &[u8]) -> bool {
    if b.len() < 8 {
        return false;
    }
    if b.iter().all(|&x| x == 0) {
        return false;
    }
    if b.iter().all(|&x| x == 0xCC) {
        return false;
    }
    if shannon_entropy(b) > 7.5 {
        return false;
    }
    if !matches!(b[0], 0x40 | 0x48 | 0x4C | 0x55 | 0x53 | 0x57) {
        return false;
    }
    let head = &b[..8];
    let has_sub_rsp = head
        .windows(3)
        .any(|w| w == [0x48, 0x83, 0xEC] || w == [0x48, 0x81, 0xEC]);
    let has_home_spill = head.windows(4).any(|w| {
        w[0] == 0x48 && w[1] == 0x89 && matches!(w[2], 0x5C | 0x74 | 0x7C | 0x4C) && w[3] == 0x24
    });
    has_sub_rsp || has_home_spill
}

fn shannon_entropy(buf: &[u8]) -> f64 {
    let mut counts = [0u32; 256];
    for &b in buf {
        counts[b as usize] += 1;
    }
    let n = buf.len() as f64;
    let mut h = 0.0f64;
    for &c in counts.iter() {
        if c == 0 {
            continue;
        }
        let p = c as f64 / n;
        h -= p * p.log2();
    }
    h
}

// VirtualQuery wrapper kept private; integration tests live in the smoke
// harness, not here.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_all_zero() {
        assert!(!looks_like_x64_prologue(&[0u8; 16]));
    }

    #[test]
    fn rejects_all_int3() {
        assert!(!looks_like_x64_prologue(&[0xCCu8; 16]));
    }

    #[test]
    fn accepts_sub_rsp_prologue() {
        // 48 83 EC 28 ... — sub rsp, 0x28 (very common)
        let bytes = [
            0x48, 0x83, 0xEC, 0x28, 0x48, 0x8B, 0xD9, 0xE8, 0x00, 0x00, 0x00, 0x00, 0x90, 0x90,
            0x90, 0x90,
        ];
        assert!(looks_like_x64_prologue(&bytes));
    }

    #[test]
    fn accepts_home_spill_prologue() {
        // 48 89 5C 24 08 57 48 83 EC 30 — mov [rsp+8], rbx; push rdi; sub rsp, 0x30
        let bytes = [
            0x48, 0x89, 0x5C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x30, 0x48, 0x8B, 0xD9, 0x33,
            0xC0, 0x90,
        ];
        assert!(looks_like_x64_prologue(&bytes));
    }
}
