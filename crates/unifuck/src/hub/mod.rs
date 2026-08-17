pub(crate) mod asar;
pub(crate) mod patcher;
pub(crate) mod payloads;
pub(crate) mod strategy;
pub(crate) mod tuanjie;
pub(crate) mod unity;

use crate::hub::strategy::PatchStrategy;
use crate::vendor::Vendor;
use anyhow::Context;
use serde_json::Value;
use sha2::Digest;
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

// ── Embedded Asar Hash Fix ──────────────────────────────────────

/// Unity Hub 3.20+ enables Electron's `EnableEmbeddedAsarIntegrityValidation`
/// fuse: the Hub executable embeds the SHA-256 of the asar header JSON
/// (e.g. `{"resources\app.asar","alg":"SHA256","value":"<hex>"}`) and aborts
/// at startup when the on-disk header no longer matches. Patching rewrites
/// the header, so the embedded hash must be patched in place afterwards.
///
/// The embedded value is exactly 64 hex chars, so the replacement is
/// byte-length-preserving and the surrounding JSON stays valid.
///
/// Computes the SHA-256 hex of the asar header JSON.
fn asar_header_hash(path: &std::path::Path) -> anyhow::Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read asar: {}", path.display()))?;
    anyhow::ensure!(bytes.len() >= 16, "asar file too small");
    let header_len = u32::from_le_bytes(bytes[12..16].try_into()?) as usize;
    anyhow::ensure!(
        16 + header_len <= bytes.len(),
        "asar header exceeds file size"
    );
    let hash = sha2::Sha256::digest(&bytes[16..16 + header_len]);
    Ok(hash.iter().map(|b| format!("{:02x}", b)).collect())
}

/// Rewrite the asar header hash embedded in the Hub exe, replacing
/// `old_hash_hex` with `new_hash_hex` (both 64 hex chars, so the exe size
/// and surrounding JSON layout are preserved).
///
/// Hubs without the integrity fuse (3.5/3.18) do not embed the hash; the
/// lookup misses and the function skips silently.
fn patch_embedded_asar_hash(
    info: &HubInfo,
    old_hash_hex: &str,
    new_hash_hex: &str,
) -> anyhow::Result<()> {
    let exe_path = info.root.join(format!("{}.exe", info.vendor.hub_name()));
    let data = std::fs::read(&exe_path)
        .with_context(|| format!("read hub exe: {}", exe_path.display()))?;
    let Some(pos) = data.windows(64).position(|w| w == old_hash_hex.as_bytes()) else {
        crate::diag::log("embedded asar hash not present in hub exe, skipping (no fuse)");
        return Ok(());
    };
    let mut patched_exe = data;
    patched_exe[pos..pos + 64].copy_from_slice(new_hash_hex.as_bytes());
    std::fs::write(&exe_path, &patched_exe)
        .with_context(|| format!("write hub exe: {}", exe_path.display()))?;
    crate::diag::log(&format!(
        "embedded asar hash updated: {} -> {}",
        old_hash_hex, new_hash_hex
    ));
    Ok(())
}

// ── Main Patch / Unpatch ───────────────────────────────────────

pub(crate) fn patch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = info.root.join(r"resources\app.asar");

    // Read
    let asar = asar::read_asar(&asar_path)?;

    // Dispatch to the per-vendor strategy detector.
    let strategy = match info.vendor {
        Vendor::Unity => unity::detect_strategy(&asar)?,
        Vendor::Tuanjie => tuanjie::detect_strategy(&asar)?,
    };

    let old_hash = asar_header_hash(&asar_path)?;
    patch_asar_with_strategy(&asar_path, &asar, strategy.as_ref())?;
    let new_hash = asar_header_hash(&asar_path)?;
    patch_embedded_asar_hash(info, &old_hash, &new_hash)?;
    Ok(())
}

/// Patch the asar at `asar_path` using the given strategy.
///
/// Backup handling: the original `app.asar` is first copied to a temporary
/// `app.asar.bak.tmp` (same directory as the final `.bak`, so the later rename
/// stays on the same volume and is atomic). Only after the patched asar has
/// been fully written do we rename the temp file to the official
/// `app.asar.bak`. Any mid-way failure therefore leaves no `app.asar.bak`
/// behind, so `is_hub_patched` (which only checks for `.bak`) stays correct.
///
/// Note: `write_asar` overwrites `app.asar` in place, so the backup copy must
/// be taken from the original bytes *before* writing — only the final rename
/// is deferred to the end.
fn patch_asar_with_strategy(
    asar_path: &Path,
    asar: &asar::AsarArchive,
    strategy: &dyn PatchStrategy,
) -> anyhow::Result<()> {
    let bak_path = asar_path.with_extension("asar.bak");
    let tmp_bak_path = asar_path.with_extension("asar.bak.tmp");

    // Copy the original bytes to the temp backup before anything overwrites
    // the source asar. Clean up the temp file on any failure path.
    let result = (|| -> anyhow::Result<()> {
        std::fs::copy(asar_path, &tmp_bak_path).context("backup asar failed")?;

        // Apply patches
        let modified = strategy.apply_patches(asar)?;

        if modified.is_empty() {
            anyhow::bail!("no patches applied for version {}", strategy.version_name());
        }

        // Write patched asar (overwrites `asar_path` in place)
        asar::write_asar(asar_path, &modified, asar)?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            // All steps succeeded: promote the temp backup to the official
            // `.bak` so that `is_hub_patched` reports PATCHED.
            std::fs::rename(&tmp_bak_path, &bak_path).context("finalize backup failed")?;
            Ok(())
        }
        Err(err) => {
            // Best-effort cleanup; a leftover `.tmp` does not affect
            // `is_hub_patched`, which only recognises `.asar.bak`.
            let _ = std::fs::remove_file(&tmp_bak_path);
            Err(err)
        }
    }
}

pub(crate) fn unpatch_hub(info: &HubInfo) -> anyhow::Result<()> {
    let asar_path = info.root.join(r"resources\app.asar");
    let bak_path = asar_path.with_extension("asar.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    let old_hash = asar_header_hash(&asar_path)?;
    std::fs::copy(&bak_path, &asar_path).context("restore asar failed")?;
    std::fs::remove_file(&bak_path).context("remove backup failed")?;
    let new_hash = asar_header_hash(&asar_path)?;
    patch_embedded_asar_hash(info, &old_hash, &new_hash)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Build a minimal in-memory asar archive with a single patchable file,
    /// then return its raw bytes (the "original" app.asar content).
    fn original_asar_bytes() -> Vec<u8> {
        let target_js = b"original app bundle";
        let archive = asar::AsarArchive {
            header_json: json!({
                "files": {
                    "build": {
                        "files": {
                            "main": {
                                "files": {
                                    "app-test.js": {
                                        "size": target_js.len(),
                                        "offset": "0",
                                        "integrity": {}
                                    }
                                }
                            }
                        }
                    }
                }
            }),
            data: target_js.to_vec(),
        };

        // Serialise via write_asar into a temp file, then read the bytes back
        // so the on-disk original is a well-formed asar we can round-trip.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seed.asar");
        asar::write_asar(&path, &[], &archive).unwrap();
        std::fs::read(&path).unwrap()
    }

    /// Write `bytes` to a fresh `resources\app.asar` under a temp hub root and
    /// return (root guard, asar path).
    fn setup_hub(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("resources");
        std::fs::create_dir_all(&resources).unwrap();
        let asar_path = resources.join("app.asar");
        std::fs::write(&asar_path, bytes).unwrap();
        (dir, asar_path)
    }

    /// Strategy whose `apply_patches` always errors.
    struct FailingStrategy;
    impl PatchStrategy for FailingStrategy {
        fn version_name(&self) -> &'static str {
            "failing"
        }
        fn applies_to(&self, _asar: &asar::AsarArchive) -> bool {
            true
        }
        fn apply_patches(
            &self,
            _asar: &asar::AsarArchive,
        ) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
            anyhow::bail!("simulated apply failure")
        }
    }

    /// Strategy that applies no patches (empty `modified`).
    struct EmptyStrategy;
    impl PatchStrategy for EmptyStrategy {
        fn version_name(&self) -> &'static str {
            "empty"
        }
        fn applies_to(&self, _asar: &asar::AsarArchive) -> bool {
            true
        }
        fn apply_patches(
            &self,
            _asar: &asar::AsarArchive,
        ) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
            Ok(Vec::new())
        }
    }

    /// Strategy that patches the single target file.
    struct OkStrategy;
    impl PatchStrategy for OkStrategy {
        fn version_name(&self) -> &'static str {
            "ok"
        }
        fn applies_to(&self, _asar: &asar::AsarArchive) -> bool {
            true
        }
        fn apply_patches(
            &self,
            _asar: &asar::AsarArchive,
        ) -> anyhow::Result<Vec<(String, Vec<u8>)>> {
            Ok(vec![(
                "build/main/app-test.js".to_string(),
                b"patched app bundle".to_vec(),
            )])
        }
    }

    #[test]
    fn failed_apply_leaves_no_backup_and_does_not_touch_asar() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let asar = asar::read_asar(&asar_path).unwrap();
        let bak_path = asar_path.with_extension("asar.bak");

        let result = patch_asar_with_strategy(&asar_path, &asar, &FailingStrategy);

        assert!(result.is_err());
        // The bug: a failed patch must not leave a `.bak` behind, otherwise
        // `is_hub_patched` would wrongly report PATCHED.
        assert!(!bak_path.exists());
        assert!(!is_hub_patched(
            asar_path.parent().unwrap().parent().unwrap()
        ));
        // The source asar must be untouched on failure.
        assert_eq!(std::fs::read(&asar_path).unwrap(), original);
    }

    #[test]
    fn empty_modified_leaves_no_backup() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let asar = asar::read_asar(&asar_path).unwrap();
        let bak_path = asar_path.with_extension("asar.bak");

        let result = patch_asar_with_strategy(&asar_path, &asar, &EmptyStrategy);

        assert!(result.is_err());
        assert!(!bak_path.exists());
        assert_eq!(std::fs::read(&asar_path).unwrap(), original);
    }

    #[test]
    fn successful_patch_writes_backup_with_original_bytes_and_rewrites_asar() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let asar = asar::read_asar(&asar_path).unwrap();
        let bak_path = asar_path.with_extension("asar.bak");
        let tmp_bak_path = asar_path.with_extension("asar.bak.tmp");

        patch_asar_with_strategy(&asar_path, &asar, &OkStrategy).unwrap();

        // `.bak` exists and holds the original (pre-patch) bytes.
        assert!(bak_path.exists());
        assert_eq!(std::fs::read(&bak_path).unwrap(), original);
        // Temp backup is gone (renamed), not left behind.
        assert!(!tmp_bak_path.exists());

        // The live asar now contains the patched content.
        let patched = asar::read_asar(&asar_path).unwrap();
        let entry = asar::find_entry(&patched.header_json, "build/main/app-test.js").unwrap();
        assert_eq!(
            asar::extract_file(&patched, &entry).unwrap(),
            b"patched app bundle"
        );
        assert_ne!(std::fs::read(&asar_path).unwrap(), original);

        // `is_hub_patched` agrees that the hub is now patched.
        assert!(is_hub_patched(
            asar_path.parent().unwrap().parent().unwrap()
        ));
    }

    #[test]
    fn asar_header_hash_matches_sha256_of_header_json() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let hash = asar_header_hash(&asar_path).unwrap();
        assert_eq!(hash.len(), 64);
        // 与原始字节的 header JSON 段 sha256 一致
        let bytes = std::fs::read(&asar_path).unwrap();
        let header_len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let expect: String = sha2::Sha256::digest(&bytes[16..16 + header_len])
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        assert_eq!(hash, expect);
    }

    #[test]
    fn patch_embedded_asar_hash_rewrites_in_place() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let root = asar_path.parent().unwrap().parent().unwrap();
        let exe_path = root.join("Unity Hub.exe");
        // 模拟 fuse 布局: exe 内嵌 old hash + 周围 JSON 文本
        let old_hash = asar_header_hash(&asar_path).unwrap();
        let new_hash = "9".repeat(64);
        let exe_content = format!(
            "{{\"resources\\\\app.asar\",\"alg\":\"SHA256\",\"value\":\"{}\"}}",
            old_hash
        );
        std::fs::write(&exe_path, &exe_content).unwrap();
        let info = HubInfo {
            version: "3.20.1".to_string(),
            vendor: Vendor::Unity,
            root: root.to_path_buf(),
        };
        patch_embedded_asar_hash(&info, &old_hash, &new_hash).unwrap();
        let patched = std::fs::read_to_string(&exe_path).unwrap();
        assert!(patched.contains(&new_hash));
        assert!(!patched.contains(&old_hash));
        assert_eq!(patched.len(), exe_content.len());
    }

    #[test]
    fn patch_embedded_asar_hash_skips_when_no_fuse() {
        let original = original_asar_bytes();
        let (_guard, asar_path) = setup_hub(&original);
        let root = asar_path.parent().unwrap().parent().unwrap();
        let exe_path = root.join("Unity Hub.exe");
        std::fs::write(&exe_path, b"no embedded hash here").unwrap();
        let info = HubInfo {
            version: "3.5.0".to_string(),
            vendor: Vendor::Unity,
            root: root.to_path_buf(),
        };
        // 无 fuse 的 Hub 找不到 old hash → 静默跳过, 不报错
        let old_hash = "a".repeat(64);
        let new_hash = "b".repeat(64);
        patch_embedded_asar_hash(&info, &old_hash, &new_hash).unwrap();
        assert_eq!(
            std::fs::read(&exe_path).unwrap(),
            b"no embedded hash here".to_vec()
        );
    }
}
