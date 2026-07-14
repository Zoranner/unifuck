use anyhow::Context;
use std::path::Path;

use super::super::base64_lines;

pub(super) const UNITY_ORIG_CERT_PEM: &[u8] =
    include_bytes!("../../../keys/orig/Unity.Licensing.EntitlementResolver.Unity.cer");
pub(super) const UNITY_MOD_CERT_PEM: &[u8] =
    include_bytes!("../../../keys/mod/Unity.Licensing.EntitlementResolver.Unity.cer");

/// 在字节数组中搜索 `old` 并原地替换为 `new`。
///
/// `old.len()` 必须等于 `new.len()`，否则替换会破坏后续偏移量。
/// 对于 73 MB 的二进制文件，31 次 76 字节的扫描总计约 2.2 GB 的顺序读，
/// 在现代 CPU 上几十毫秒即可完毕。
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

/// 对二进制数据 (DLL / exe) 执行证书 base64 替换。
///
/// 同时尝试两种匹配形态:
/// - 多行: 按 76 字符一行的 base64 PEM 体 (与 `base64_lines` 输出一致)
/// - 单行: 全部 base64 字符拼接 (无换行, 常见于单体 exe 中的嵌入资源)
///
/// 返回 `true` 表示至少替换了一处, `false` 表示数据中未发现原始证书。
pub(super) fn apply_cert_replacement(data: &mut [u8]) -> anyhow::Result<bool> {
    let pem_orig = pem::parse(UNITY_ORIG_CERT_PEM).context("parse original cert failed")?;
    let pem_mod = pem::parse(UNITY_MOD_CERT_PEM).context("parse modified cert failed")?;
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
pub(super) fn patch_binary_file(
    path: &Path,
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

    let bak_path = path.with_extension(
        path.extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .to_string()
            + ".bak",
    );
    if skip_if_backup_exists && bak_path.exists() {
        return Ok(false);
    }

    let mut data =
        std::fs::read(path).with_context(|| format!("read failed: {}", path.display()))?;
    std::fs::write(&bak_path, &data)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

    let replaced = apply_cert_replacement(&mut data)
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

/// Unity-only DLL patch: bails if the original cert was not found.
pub fn patch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let patched = patch_binary_file(dll_path, false, 0, false)?;
    if !patched {
        anyhow::bail!("cert not found in DLL: {}", dll_path.display());
    }
    Ok(())
}
