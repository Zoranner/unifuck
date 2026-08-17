use anyhow::Context;
use std::path::Path;

use super::binary::bak_path;

pub(super) const TUANJIE_ORIG_CERT_PEM: &str =
    include_str!("../../../keys/tuanjie/orig/Tuanjie.Licensing.EntitlementResolver.Tuanjie.pem");
pub(super) const TUANJIE_MOD_CERT_PEM: &str =
    include_str!("../../../keys/tuanjie/mod/Tuanjie.Licensing.EntitlementResolver.Tuanjie.pem");

/// 将 ASCII/UTF-8 字符串转换为 CRLF UTF-16LE 字节序列。
///
/// Tuanjie DLL 中的 PEM 字符串以 CRLF 行尾存储 (.NET #US 堆在 Windows 上
/// 保留编译器原始行尾)。PEM 源文件行尾可能是 LF 或 CRLF,
/// 因此先统一为 LF 再转 CRLF, 避免双重转换。
pub(super) fn to_utf16le_crlf(s: &str) -> Vec<u8> {
    let normalized = s.replace("\r\n", "\n");
    let crlf = normalized.replace('\n', "\r\n");
    crlf.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

/// 补丁 Tuanjie DLL 中 UTF-16LE 编码的 PEM 证书字符串。
///
/// Tuanjie DLL 将证书以 C# 字符串字面量形式存储在 .NET #US 堆中,
/// 编码为 UTF-16LE。此函数通过字节替换实现证书替换,
/// 无需修改 PE 头或 .NET 元数据。
///
/// 返回 `Ok(true)` 表示已替换; `Ok(false)` 表示 DLL 中未发现原始证书
/// (shared client 场景允许静默跳过, Editor DLL 场景由调用方决定是否报错)。
pub fn patch_dll(dll_path: &Path) -> anyhow::Result<bool> {
    let orig_utf16 = to_utf16le_crlf(TUANJIE_ORIG_CERT_PEM.trim_end());
    let mod_utf16 = to_utf16le_crlf(TUANJIE_MOD_CERT_PEM.trim_end());

    if orig_utf16.len() != mod_utf16.len() {
        anyhow::bail!(
            "Tuanjie cert UTF-16LE length mismatch: original {} bytes, modified {} bytes",
            orig_utf16.len(),
            mod_utf16.len()
        );
    }

    let mut data =
        std::fs::read(dll_path).with_context(|| format!("read failed: {}", dll_path.display()))?;

    // 检查是否已补丁: 搜索修改版 UTF-16LE PEM
    if data
        .windows(mod_utf16.len())
        .any(|w| w == mod_utf16.as_slice())
    {
        anyhow::bail!("already patched: {}", dll_path.display());
    }

    // 搜索原始 UTF-16LE PEM
    let Some(pos) = data
        .windows(orig_utf16.len())
        .position(|w| w == orig_utf16.as_slice())
    else {
        return Ok(false);
    };

    // 备份
    let bak_path = bak_path(dll_path);
    std::fs::write(&bak_path, &data)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

    // 替换
    data[pos..pos + orig_utf16.len()].copy_from_slice(&mod_utf16);

    // 写入
    std::fs::write(dll_path, &data)
        .with_context(|| format!("write failed: {}", dll_path.display()))?;

    Ok(true)
}
