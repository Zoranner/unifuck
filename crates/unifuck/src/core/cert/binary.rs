//! Vendor-agnostic binary patch utilities: base64 cert replacement in
//! arbitrary binaries (DLL / exe / shared client) with automatic `.bak`
//! backup and restore.
//!
//! Unity and Tuanjie editors embed their certificates differently (Unity:
//! ASCII base64 lines; Tuanjie: UTF-16LE PEM), but the surrounding
//! read → backup → replace → write choreography is shared by all binary
//! patch targets, so it lives here instead of in a vendor module.

use anyhow::Context;
use std::path::{Path, PathBuf};

use super::base64_lines;

/// Compute the `.bak` backup path for a binary file.
///
/// Appends `.bak` to the *full* existing extension so `Unity.dll` →
/// `Unity.dll.bak` (not `Unity.bak`).
pub(crate) fn bak_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

/// Restore a binary file from its `.bak` backup. No-op when the backup
/// does not exist.
pub(crate) fn restore_from_backup(path: &Path) -> anyhow::Result<()> {
    let bak_path = bak_path(path);
    if !bak_path.exists() {
        return Ok(());
    }
    let data = std::fs::read(&bak_path)
        .with_context(|| format!("read backup failed: {}", bak_path.display()))?;
    std::fs::write(path, &data)
        .with_context(|| format!("restore write failed: {}", path.display()))?;
    std::fs::remove_file(&bak_path)
        .with_context(|| format!("remove backup failed: {}", bak_path.display()))?;
    Ok(())
}

/// In-place byte search-and-replace for equal-length patterns.
fn replace_bytes_in_place(data: &mut [u8], old: &[u8], new: &[u8]) {
    assert_eq!(
        old.len(),
        new.len(),
        "replace_bytes_in_place requires equal-length patterns"
    );
    if old.is_empty() || old == new {
        return;
    }
    let mut start = 0;
    while let Some(rel) = data[start..].windows(old.len()).position(|w| w == old) {
        let pos = start + rel;
        data[pos..pos + old.len()].copy_from_slice(new);
        start = pos + old.len();
    }
}

/// Replace the base64 body of an original cert PEM with the modified one,
/// inside an arbitrary binary blob.
///
/// Tries both storage shapes:
/// - multi-line: 76-char-per-line base64 PEM body (matches `base64_lines`
///   output, found in DLLs and line-wrapped exe resources)
/// - single-line: all base64 chars concatenated (monolithic exe resources)
///
/// Returns `true` if at least one replacement was made.
fn apply_cert_replacement(data: &mut [u8], orig_pem: &str, mod_pem: &str) -> anyhow::Result<bool> {
    let pem_orig = pem::parse(orig_pem).context("parse original cert failed")?;
    let pem_mod = pem::parse(mod_pem).context("parse modified cert failed")?;
    let der_orig = pem_orig.contents();
    let der_mod = pem_mod.contents();

    if der_orig.len() != der_mod.len() {
        anyhow::bail!(
            "cert DER length mismatch: original {} bytes, modified {} bytes",
            der_orig.len(),
            der_mod.len()
        );
    }

    let b64_orig = base64_lines(der_orig);
    let b64_mod = base64_lines(der_mod);

    // 先验证: 如果任何一行已经被替换过, 说明已经 patched
    for (_lo, lm) in b64_orig.lines().zip(b64_mod.lines()) {
        let mm = lm.as_bytes();
        if !mm.is_empty() && data.windows(mm.len()).any(|w| w == mm) {
            anyhow::bail!("already patched");
        }
    }

    let mut any_replaced = false;

    // 多行形态: 逐行替换 (DLL 及以 76 字符分行存储的 exe)
    for (lo, lm) in b64_orig.lines().zip(b64_mod.lines()) {
        let om = lo.as_bytes();
        let mm = lm.as_bytes();
        if om.is_empty() || om.len() != mm.len() {
            continue;
        }
        if data.windows(om.len()).any(|w| w == om) {
            replace_bytes_in_place(data, om, mm);
            any_replaced = true;
        }
    }

    // 单行形态: 整段 base64 (常见于 Unity 2021 单体 exe 中的嵌入证书)
    if !any_replaced {
        let single_orig: String = b64_orig.lines().collect();
        let single_mod: String = b64_mod.lines().collect();
        let om = single_orig.as_bytes();
        let mm = single_mod.as_bytes();
        if om.len() == mm.len() && !om.is_empty() && data.windows(om.len()).any(|w| w == om) {
            replace_bytes_in_place(data, om, mm);
            any_replaced = true;
        }
    }

    Ok(any_replaced)
}

/// Generic binary file cert replacement with automatic backup.
///
/// `skip_if_absent`: if true, return Ok silently when file doesn't exist (exe, shared client)
/// `min_size`: skip files smaller than this (exe launcher detection, 0 for DLL)
/// `skip_if_backup_exists`: if true, return Ok when .bak already exists (shared client)
///
/// Returns `Ok(false)` when the target was skipped or no cert was found;
/// `Ok(true)` when the file was patched.
pub(super) fn patch_binary_file(
    path: &Path,
    orig_pem: &str,
    mod_pem: &str,
    skip_if_absent: bool,
    min_size: u64,
    skip_if_backup_exists: bool,
) -> anyhow::Result<bool> {
    if skip_if_absent && !path.exists() {
        return Ok(false);
    }

    if min_size > 0 {
        let meta = std::fs::metadata(path)
            .with_context(|| format!("metadata failed: {}", path.display()))?;
        if meta.len() < min_size {
            return Ok(false);
        }
    }

    let bak_path = bak_path(path);
    if skip_if_backup_exists && bak_path.exists() {
        return Ok(false);
    }

    let mut data =
        std::fs::read(path).with_context(|| format!("read failed: {}", path.display()))?;
    std::fs::write(&bak_path, &data)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

    let replaced = apply_cert_replacement(&mut data, orig_pem, mod_pem)
        .with_context(|| format!("cert replacement failed: {}", path.display()));
    match replaced {
        Ok(true) => {}
        Ok(false) => {
            let _ = std::fs::remove_file(&bak_path);
            return Ok(false);
        }
        Err(e) => return Err(e),
    }

    std::fs::write(path, &data).with_context(|| format!("write failed: {}", path.display()))?;
    Ok(true)
}
