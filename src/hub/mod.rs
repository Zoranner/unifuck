pub(crate) mod asar;
pub(crate) mod patcher;
pub(crate) mod versions;

use anyhow::Context;
use serde_json::Value;
use std::path::{Path, PathBuf};

// ── Constants ──────────────────────────────────────────────────

pub(crate) const HUB_ASAR_REL: &str = r"resources\app.asar";

// ── HubInfo ────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct HubInfo {
    pub version: String,
    pub asar_path: PathBuf,
}

// ── Hub Detection ──────────────────────────────────────────────

pub(crate) fn find_hub() -> Option<HubInfo> {
    let candidates = vec![
        PathBuf::from(r"C:\Program Files\Unity Hub"),
        PathBuf::from(r"D:\Program Files\Unity Hub"),
        PathBuf::from(r"C:\Program Files\Tuanjie Hub"),
        PathBuf::from(r"D:\Program Files\Tuanjie Hub"),
    ];

    for path in candidates {
        let asar = path.join(HUB_ASAR_REL);
        if asar.exists() {
            if let Some(info) = hub_info_from_path(&path, &asar) {
                return Some(info);
            }
        }
    }

    if let Some(path) = hub_path_from_registry() {
        let asar = path.join(HUB_ASAR_REL);
        if asar.exists() {
            return hub_info_from_path(&path, &asar);
        }
    }

    None
}

fn hub_info_from_path(hub_path: &Path, asar_path: &Path) -> Option<HubInfo> {
    let version = read_hub_version(hub_path, asar_path).unwrap_or_else(|| "unknown".to_string());
    Some(HubInfo {
        version,
        asar_path: asar_path.to_path_buf(),
    })
}

pub(crate) fn is_hub_patched(asar_path: &Path) -> bool {
    asar_path.with_extension("asar.bak").exists()
}

// ── Version Reading ────────────────────────────────────────────

fn read_hub_version(hub_path: &Path, asar_path: &Path) -> Option<String> {
    let unpacked_pkg = hub_path.join(r"resources\app.asar.unpacked\package.json");
    if let Ok(content) = std::fs::read_to_string(&unpacked_pkg) {
        if let Some(v) = extract_version_from_json_str(&content) {
            return Some(v);
        }
    }

    if let Ok(asar) = asar::read_asar(asar_path) {
        if let Some(entry) = asar::find_entry(&asar.header_json, "package.json") {
            if let Some(data) = asar::extract_file(&asar, &entry) {
                if let Ok(content) = String::from_utf8(data) {
                    if let Some(v) = extract_version_from_json_str(&content) {
                        return Some(v);
                    }
                }
            }
        }
    }

    None
}

fn extract_version_from_json_str(s: &str) -> Option<String> {
    let v: Value = serde_json::from_str(s).ok()?;
    v.get("version")?.as_str().map(|s| s.to_string())
}

// ── Registry Lookup ────────────────────────────────────────────

#[cfg(windows)]
fn hub_path_from_registry() -> Option<PathBuf> {
    // 优先查找 Unity Hub，找不到再尝试 Tuanjie Hub
    query_registry_path(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Unity Hub").or_else(
        || query_registry_path(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Tuanjie Hub"),
    )
}

#[cfg(windows)]
fn query_registry_path(key_path: &str) -> Option<PathBuf> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::Registry::*;

    unsafe {
        let key_path_wide: Vec<u16> = OsStr::new(key_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut hkey: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            key_path_wide.as_ptr(),
            0,
            KEY_READ,
            &mut hkey,
        ) != 0
        {
            return None;
        }

        let value_name: Vec<u16> = OsStr::new("InstallLocation")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut buf = vec![0u8; 1024];
        let mut size = buf.len() as u32;
        let mut kind = 0u32;

        let result = RegQueryValueExW(
            hkey,
            value_name.as_ptr(),
            std::ptr::null_mut(),
            &mut kind,
            buf.as_mut_ptr(),
            &mut size,
        );

        RegCloseKey(hkey);

        if result != 0 || (kind != REG_SZ && kind != REG_EXPAND_SZ) {
            return None;
        }

        let wide: &[u16] =
            std::slice::from_raw_parts(buf.as_ptr() as *const u16, size as usize / 2);
        let len = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        let path = String::from_utf16_lossy(&wide[..len]);
        Some(PathBuf::from(path.trim()))
    }
}

#[cfg(not(windows))]
fn hub_path_from_registry() -> Option<PathBuf> {
    None
}

// ── Main Patch / Unpatch ───────────────────────────────────────

pub(crate) fn patch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = &info.asar_path;

    // Backup
    let bak_path = asar_path.with_extension("asar.bak");
    std::fs::copy(asar_path, &bak_path).context("backup asar failed")?;

    // Read
    let asar = asar::read_asar(asar_path)?;

    // Detect version and get strategy
    let strategy = versions::detect_strategy(&asar)?;

    // Apply patches
    let modified = strategy.apply_patches(&asar)?;

    if modified.is_empty() {
        anyhow::bail!("no patches applied for version {}", strategy.version_name());
    }

    // Write patched asar
    asar::write_asar(asar_path, &modified, &asar)?;

    Ok(())
}

pub(crate) fn unpatch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let bak_path = info.asar_path.with_extension("asar.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    std::fs::copy(&bak_path, &info.asar_path).context("restore asar failed")?;
    std::fs::remove_file(&bak_path).context("remove backup failed")?;
    Ok(())
}
