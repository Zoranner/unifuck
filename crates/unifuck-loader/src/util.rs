//! Small helpers around the host process image and module paths.

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr;

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};

/// Returns the HMODULE of the host EXE (Tuanjie.exe in production).
pub fn host_module() -> HMODULE {
    // GetModuleHandleW(NULL) returns the handle of the EXE that loaded us.
    unsafe { GetModuleHandleW(ptr::null()) }
}

/// Returns the on-disk path of the host EXE.
pub fn host_exe_path() -> Option<PathBuf> {
    let h = host_module();
    if h.is_null() {
        return None;
    }
    let mut buf = vec![0u16; 32768];
    let len = unsafe { GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) };
    if len == 0 || len as usize >= buf.len() {
        return None;
    }
    buf.truncate(len as usize);
    Some(PathBuf::from(OsString::from_wide(&buf)))
}

/// Returns the directory containing the host EXE.
pub fn editor_dir() -> Option<PathBuf> {
    host_exe_path().and_then(|p| p.parent().map(PathBuf::from))
}

/// Returns the host image base as a `usize`. Convenient for RVA arithmetic.
pub fn image_base() -> usize {
    host_module() as usize
}

/// Reads the `.text` section virtual range `(start_va, size)` from the
/// in-memory PE header of the host EXE. Returns `None` if the headers are
/// unreadable or the `.text` section cannot be located.
pub fn text_section_range() -> Option<(usize, usize)> {
    let base = host_module() as *const u8;
    if base.is_null() {
        return None;
    }
    unsafe {
        // IMAGE_DOS_HEADER.e_lfanew is at offset 0x3C.
        let dos_e_lfanew = base.add(0x3C) as *const i32;
        let nt_offset = dos_e_lfanew.read_unaligned();
        if nt_offset <= 0 || nt_offset > 0x1000 {
            return None;
        }
        let nt = base.add(nt_offset as usize);
        // Signature "PE\0\0" check.
        if nt.read() != b'P' || nt.add(1).read() != b'E' {
            return None;
        }
        // IMAGE_FILE_HEADER starts at nt+4. Fields:
        //   WORD Machine; WORD NumberOfSections; DWORD TimeDateStamp;
        //   DWORD PointerToSymbolTable; DWORD NumberOfSymbols;
        //   WORD SizeOfOptionalHeader; WORD Characteristics;
        let file_header = nt.add(4);
        let number_of_sections = (file_header.add(2) as *const u16).read_unaligned() as usize;
        let size_of_optional = (file_header.add(16) as *const u16).read_unaligned() as usize;
        // First section header starts at: file_header + 20 + SizeOfOptionalHeader.
        let mut section = file_header.add(20 + size_of_optional);
        for _ in 0..number_of_sections {
            // IMAGE_SECTION_HEADER:
            //   BYTE Name[8];
            //   DWORD VirtualSize;        // +8
            //   DWORD VirtualAddress;     // +12
            // (rest irrelevant for us)
            let name = core::slice::from_raw_parts(section, 8);
            if &name[..5] == b".text" {
                let virtual_size = (section.add(8) as *const u32).read_unaligned() as usize;
                let virtual_addr = (section.add(12) as *const u32).read_unaligned() as usize;
                return Some((base as usize + virtual_addr, virtual_size));
            }
            section = section.add(40); // sizeof(IMAGE_SECTION_HEADER) == 40
        }
        None
    }
}
