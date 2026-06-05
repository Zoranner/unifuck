//! Resolves the IsValidIPCResponse and ValidateServerProcess symbols from
//! `Tuanjie_x64.pdb` located next to the host EXE.
//!
//! The PDB GUID/age **must** match the EXE's `IMAGE_DEBUG_DIRECTORY` RSDS
//! record, otherwise the RVAs we read are pointing at a different build and
//! we must refuse to install the hooks.

use std::fs::File;
use std::path::PathBuf;

use pdb::{FallibleIterator, SymbolData, PDB};
use uuid::Uuid;

use crate::util;

/// PDB mangled name for `Licensing::LicensingClient::IsValidIPCResponse`.
const MANGLED_IS_VALID_IPC: &str = "?IsValidIPCResponse@LicensingClient@Licensing@@QEAA_NAEBV?$basic_string@DV?$StringStorageDefault@D@core@@@core@@0@Z";

/// Candidate mangled names for `ValidateServerProcess`. Visibility encoded in
/// the access character (`Q` public / `A` private / `I` protected).
const MANGLED_VALIDATE_SERVER_CANDIDATES: &[&str] = &[
    "?ValidateServerProcess@LicensingClient@Licensing@@QEAA_NXZ",
    "?ValidateServerProcess@LicensingClient@Licensing@@AEAA_NXZ",
    "?ValidateServerProcess@LicensingClient@Licensing@@IEAA_NXZ",
];

pub enum PdbError {
    HostExeUnknown,
    PdbMissing(PathBuf),
    PdbOpen(String),
    SymbolMissing(&'static str),
    AddressMap(&'static str),
    GuidMismatch,
}

impl core::fmt::Debug for PdbError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HostExeUnknown => f.write_str("could not derive host exe directory"),
            Self::PdbMissing(p) => write!(f, "PDB missing at {}", p.display()),
            Self::PdbOpen(msg) => write!(f, "PDB open failed: {msg}"),
            Self::SymbolMissing(name) => write!(f, "symbol not in PDB: {name}"),
            Self::AddressMap(what) => write!(f, "PDB address resolution failed at {what}"),
            Self::GuidMismatch => f.write_str("PDB GUID/age does not match host EXE"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Targets {
    pub is_valid_ipc_response: usize,
    pub validate_server_process: usize,
}

/// Reads PDB next to the host EXE, verifies GUID/age against the EXE
/// `DEBUG_DIRECTORY`, and returns the absolute VAs of both target functions.
pub fn resolve(image_base: usize) -> Result<Targets, PdbError> {
    let editor = util::editor_dir().ok_or(PdbError::HostExeUnknown)?;
    let pdb_path = editor.join("Tuanjie_x64.pdb");
    if !pdb_path.exists() {
        return Err(PdbError::PdbMissing(pdb_path));
    }
    let file =
        File::open(&pdb_path).map_err(|e| PdbError::PdbOpen(format!("open {pdb_path:?}: {e}")))?;
    let mut pdb = PDB::open(file).map_err(|e| PdbError::PdbOpen(format!("PDB::open: {e}")))?;

    // GUID/age check: read the EXE's RSDS record and compare with the PDB
    // header. Mismatched PDBs would resolve to wrong RVAs and a guaranteed
    // crash on hook install, so we hard-fail.
    if !verify_guid(&mut pdb, image_base) {
        return Err(PdbError::GuidMismatch);
    }

    let address_map = pdb
        .address_map()
        .map_err(|_| PdbError::AddressMap("address_map"))?;
    let symbol_table = pdb
        .global_symbols()
        .map_err(|_| PdbError::AddressMap("global_symbols"))?;

    let mut is_valid: Option<u32> = None;
    let mut validate_srv: Option<u32> = None;
    let mut validate_hit: Option<&str> = None;

    let mut iter = symbol_table.iter();
    while let Some(symbol) = iter
        .next()
        .map_err(|_| PdbError::AddressMap("symbol iter"))?
    {
        let Ok(data) = symbol.parse() else { continue };
        let SymbolData::Public(p) = data else {
            continue;
        };
        let name = p.name.to_string();
        let name = name.as_ref();

        if is_valid.is_none() && name == MANGLED_IS_VALID_IPC {
            if let Some(rva) = p.offset.to_rva(&address_map) {
                is_valid = Some(rva.0);
            }
        }
        if validate_srv.is_none() {
            for cand in MANGLED_VALIDATE_SERVER_CANDIDATES {
                if name == *cand {
                    if let Some(rva) = p.offset.to_rva(&address_map) {
                        validate_srv = Some(rva.0);
                        validate_hit = Some(cand);
                    }
                    break;
                }
            }
        }
        if is_valid.is_some() && validate_srv.is_some() {
            break;
        }
    }

    let is_valid_rva = is_valid.ok_or(PdbError::SymbolMissing("IsValidIPCResponse"))?;
    let validate_rva = validate_srv.ok_or(PdbError::SymbolMissing("ValidateServerProcess"))?;
    log::info!(
        "pdb resolved: IsValidIPCResponse rva={:#x}, ValidateServerProcess rva={:#x} (variant={})",
        is_valid_rva,
        validate_rva,
        validate_hit.unwrap_or("?")
    );

    Ok(Targets {
        is_valid_ipc_response: image_base + is_valid_rva as usize,
        validate_server_process: image_base + validate_rva as usize,
    })
}

/// Compare PDB header GUID/age against the EXE's IMAGE_DEBUG_DIRECTORY RSDS
/// record. Returns true if they match (or if we cannot read the EXE record
/// at all, in which case we conservatively trust the PDB rather than block
/// hook install — Sentinel envelope sometimes mangles the debug directory).
fn verify_guid(pdb: &mut PDB<File>, image_base: usize) -> bool {
    let pdb_info = match pdb.pdb_information() {
        Ok(info) => info,
        Err(e) => {
            log::warn!("pdb_information failed: {e}; skipping GUID check");
            return true;
        }
    };

    match read_rsds(image_base) {
        Some(rsds) => {
            let match_guid = rsds.guid == pdb_info.guid;
            let match_age = rsds.age == pdb_info.age;
            if !match_guid || !match_age {
                log::error!(
                    "PDB GUID/age mismatch: exe={}/{} pdb={}/{}",
                    rsds.guid,
                    rsds.age,
                    pdb_info.guid,
                    pdb_info.age
                );
                return false;
            }
            log::info!("PDB GUID/age verified");
            true
        }
        None => {
            log::warn!("DEBUG_DIRECTORY RSDS unreadable; skipping GUID check");
            true
        }
    }
}

struct Rsds {
    guid: Uuid,
    age: u32,
}

/// Reads the first CodeView (RSDS) entry from the in-memory EXE's
/// IMAGE_DEBUG_DIRECTORY. Returns None on any parsing failure.
fn read_rsds(image_base: usize) -> Option<Rsds> {
    if image_base == 0 {
        return None;
    }
    let base = image_base as *const u8;
    unsafe {
        let e_lfanew = (base.add(0x3C) as *const i32).read_unaligned();
        if e_lfanew <= 0 || e_lfanew > 0x1000 {
            return None;
        }
        let nt = base.add(e_lfanew as usize);
        if nt.read() != b'P' || nt.add(1).read() != b'E' {
            return None;
        }
        // OptionalHeader starts at nt + 24. For PE32+ Magic == 0x20B.
        let opt = nt.add(24);
        let magic = (opt as *const u16).read_unaligned();
        if magic != 0x20B {
            return None;
        }
        // In OptionalHeader (PE32+), DataDirectory begins at offset 112.
        // Debug directory is index 6 => offset 112 + 6*8 = 160.
        let dd_debug_va = (opt.add(160) as *const u32).read_unaligned() as usize;
        let dd_debug_size = (opt.add(164) as *const u32).read_unaligned() as usize;
        if dd_debug_va == 0 || dd_debug_size < 28 {
            return None;
        }
        let debug_dir = base.add(dd_debug_va);
        let entry_count = dd_debug_size / 28;
        for i in 0..entry_count {
            let entry = debug_dir.add(i * 28);
            let kind = (entry.add(12) as *const u32).read_unaligned();
            // IMAGE_DEBUG_TYPE_CODEVIEW == 2.
            if kind != 2 {
                continue;
            }
            let cv_rva = (entry.add(20) as *const u32).read_unaligned() as usize;
            if cv_rva == 0 {
                continue;
            }
            let cv = base.add(cv_rva);
            // RSDS layout:
            //   'R','S','D','S' (4 bytes signature)
            //   GUID (16 bytes; DWORD/WORD/WORD/BYTE[8] little-endian)
            //   DWORD age
            //   ASCIIZ PDB path
            if cv.read() != b'R'
                || cv.add(1).read() != b'S'
                || cv.add(2).read() != b'D'
                || cv.add(3).read() != b'S'
            {
                continue;
            }
            let d1 = u32::from_le_bytes([
                cv.add(4).read(),
                cv.add(5).read(),
                cv.add(6).read(),
                cv.add(7).read(),
            ]);
            let d2 = u16::from_le_bytes([cv.add(8).read(), cv.add(9).read()]);
            let d3 = u16::from_le_bytes([cv.add(10).read(), cv.add(11).read()]);
            let mut d4 = [0u8; 8];
            for (i, b) in d4.iter_mut().enumerate() {
                *b = cv.add(12 + i).read();
            }
            let guid = Uuid::from_fields(d1, d2, d3, &d4);
            let age = (cv.add(20) as *const u32).read_unaligned();
            return Some(Rsds { guid, age });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_from_fields_layout() {
        // Ensure the RSDS field order parsing matches uuid::from_fields.
        let bytes: [u8; 16] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
            0x00, 0x11,
        ];
        let d1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let d2 = u16::from_le_bytes([bytes[4], bytes[5]]);
        let d3 = u16::from_le_bytes([bytes[6], bytes[7]]);
        let mut d4 = [0u8; 8];
        d4.copy_from_slice(&bytes[8..16]);
        let g = Uuid::from_fields(d1, d2, d3, &d4);
        assert_eq!(g.as_fields().0, 0x44332211);
        assert_eq!(g.as_fields().1, 0x6655);
        assert_eq!(g.as_fields().2, 0x8877);
    }
}
