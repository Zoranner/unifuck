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
    pub name: String,
    pub version: String,
    pub asar_path: PathBuf,
}

// ── Hub Detection ──────────────────────────────────────────────

/// Find all Hub installations (Unity Hub and Tuanjie Hub).
pub(crate) fn find_hubs() -> Vec<HubInfo> {
    let mut hubs = Vec::new();

    // Try Unity Hub first (registry then Program Files on all drives),
    // then Tuanjie Hub with the same strategy.
    for hub_name in ["Unity Hub", "Tuanjie Hub"] {
        for hub_root in crate::discovery::find_hub_roots(hub_name) {
            let asar = hub_root.join(HUB_ASAR_REL);
            if let Some(info) = hub_info_from_path(hub_name, &hub_root, &asar) {
                hubs.push(info);
            }
        }
    }

    hubs
}

/// Find first Hub installation (legacy API for backward compatibility).
#[allow(dead_code)]
pub(crate) fn find_hub() -> Option<HubInfo> {
    find_hubs().into_iter().next()
}

fn hub_info_from_path(hub_name: &str, hub_path: &Path, asar_path: &Path) -> Option<HubInfo> {
    let version = read_hub_version(hub_path, asar_path).unwrap_or_else(|| "unknown".to_string());
    Some(HubInfo {
        name: hub_name.to_string(),
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
