pub(crate) mod asar;
pub(crate) mod patcher;
pub(crate) mod payloads;
pub(crate) mod strategy;
pub(crate) mod tuanjie;
pub(crate) mod unity;

use crate::vendor::Vendor;
use anyhow::Context;
use serde_json::Value;
use std::path::{Path, PathBuf};

// ── Constants ──────────────────────────────────────────────────

pub(crate) const HUB_ASAR_REL: &str = r"resources\app.asar";

// ── HubInfo ────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct HubInfo {
    pub version: String,
    pub vendor: Vendor,
    /// Hub root directory (e.g. `C:\Program Files\Unity Hub`).
    /// All sub-paths (asar, licensing client) derive from this.
    pub root: PathBuf,
}

// ── Hub Detection ──────────────────────────────────────────────

pub(crate) fn hub_info_from_path(
    vendor: Vendor,
    hub_path: &Path,
    asar_path: &Path,
) -> Option<HubInfo> {
    let version = read_hub_version(hub_path, asar_path).unwrap_or_else(|| "unknown".to_string());
    Some(HubInfo {
        version,
        vendor,
        root: hub_path.to_path_buf(),
    })
}

pub(crate) fn is_hub_patched(root: &Path) -> bool {
    root.join(r"resources\app.asar.bak").exists()
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

// ── Main Patch / Unpatch ───────────────────────────────────────

pub(crate) fn patch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = info.root.join(r"resources\app.asar");

    // Backup
    let bak_path = asar_path.with_extension("asar.bak");
    std::fs::copy(&asar_path, &bak_path).context("backup asar failed")?;

    // Read
    let asar = asar::read_asar(&asar_path)?;

    // Dispatch to the per-vendor strategy detector.
    let strategy = match info.vendor {
        Vendor::Unity => unity::detect_strategy(&asar)?,
        Vendor::Tuanjie => tuanjie::detect_strategy(&asar)?,
    };
    // Apply patches
    let modified = strategy.apply_patches(&asar)?;

    if modified.is_empty() {
        anyhow::bail!("no patches applied for version {}", strategy.version_name());
    }

    // Write patched asar
    asar::write_asar(&asar_path, &modified, &asar)?;

    Ok(())
}

pub(crate) fn unpatch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = info.root.join(r"resources\app.asar");
    let bak_path = asar_path.with_extension("asar.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    std::fs::copy(&bak_path, &asar_path).context("restore asar failed")?;
    std::fs::remove_file(&bak_path).context("remove backup failed")?;
    Ok(())
}
