use anyhow::Context;
use std::path::Path;

use super::{base64_lines, MOD_CERT_PEM};

const ORIG_CERT_PEM: &[u8] =
    include_bytes!("../../keys/orig/Unity.Licensing.EntitlementResolver.Unity.cer");
const TUANJIE_ORIG_CERT_PEM: &str =
    include_str!("../../keys/tuanjie/orig/Tuanjie.Licensing.EntitlementResolver.Tuanjie.pem");
const TUANJIE_MOD_CERT_PEM: &str =
    include_str!("../../keys/tuanjie/mod/Tuanjie.Licensing.EntitlementResolver.Tuanjie.pem");

/// Candidate paths for the Unity shared licensing client directory (C: and D: drives).
fn shared_licensing_client_candidates() -> &'static [&'static str] {
    &[
        r"C:\Program Files\Unity Hub\UnityLicensingClient_V1",
        r"D:\Program Files\Unity Hub\UnityLicensingClient_V1",
    ]
}

/// Candidate paths for the Tuanjie shared licensing client directory (C: and D: drives).
fn tuanjie_shared_licensing_client_candidates() -> &'static [&'static str] {
    &[
        r"C:\Program Files\Tuanjie Hub\TuanjieLicensingClient_V1",
        r"D:\Program Files\Tuanjie Hub\TuanjieLicensingClient_V1",
    ]
}

/// 将 ASCII/UTF-8 字符串转换为 UTF-16LE 字节序列。
fn to_utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|c| c.to_le_bytes()).collect()
}

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
fn apply_cert_replacement(data: &mut [u8]) -> anyhow::Result<bool> {
    let pem_orig = pem::parse(ORIG_CERT_PEM).context("parse original cert failed")?;
    let pem_mod = pem::parse(MOD_CERT_PEM).context("parse modified cert failed")?;
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
fn patch_binary_file(
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

pub fn patch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let filename = dll_path.file_name().unwrap_or_default().to_string_lossy();

    if filename.starts_with("Tuanjie.") {
        patch_tuanjie_dll(dll_path)
    } else {
        let patched = patch_binary_file(dll_path, false, 0, false)?;
        if !patched {
            anyhow::bail!("cert not found in DLL: {}", dll_path.display());
        }
        Ok(())
    }
}

/// 补丁 Tuanjie Editor DLL 中 UTF-16LE 编码的 PEM 证书字符串。
///
/// Tuanjie DLL 将证书以 C# 字符串字面量形式存储在 .NET #US 堆中,
/// 编码为 UTF-16LE。此函数通过字节替换实现证书替换,
/// 无需修改 PE 头或 .NET 元数据。
pub fn patch_tuanjie_dll(dll_path: &Path) -> anyhow::Result<()> {
    let orig_utf16 = to_utf16le(TUANJIE_ORIG_CERT_PEM.trim_end());
    let mod_utf16 = to_utf16le(TUANJIE_MOD_CERT_PEM.trim_end());

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
        anyhow::bail!(
            "Tuanjie cert not found in DLL (UTF-16LE PEM): {}",
            dll_path.display()
        );
    };

    // 备份
    let bak_path = dll_path.with_extension("dll.bak");
    std::fs::write(&bak_path, &data)
        .with_context(|| format!("backup write failed: {}", bak_path.display()))?;

    // 替换
    data[pos..pos + orig_utf16.len()].copy_from_slice(&mod_utf16);

    // 写入
    std::fs::write(dll_path, &data)
        .with_context(|| format!("write failed: {}", dll_path.display()))?;

    Ok(())
}

fn restore_from_backup(path: &Path) -> anyhow::Result<()> {
    let bak_path = path.with_extension(
        path.extension()
            .unwrap_or_default()
            .to_str()
            .unwrap_or("")
            .to_string()
            + ".bak",
    );
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

pub fn unpatch_dll(dll_path: &Path) -> anyhow::Result<()> {
    let bak_path = dll_path.with_extension("dll.bak");
    if !bak_path.exists() {
        anyhow::bail!("no backup: {}", bak_path.display());
    }
    restore_from_backup(dll_path)
}

/// 补丁 Unity Editor 主程序 (Unity.exe) 中内嵌的许可证书。
///
/// 仅对 Unity 2021 等单体二进制版本有效: 这类 exe 约 73 MB, 内含 LicensingLegacy
/// 模块与同一份 X.509 证书; Unity 6000+ 的 exe 只是约 282 KB 的启动器, 不含证书。
///
/// 跳过条件 (返回 Ok 但不生成备份):
/// - exe 不存在
/// - exe 体积小于 1 MB (判定为启动器)
/// - exe 中未发现原始证书 base64 (既不以多行也不以单行形态出现)
pub fn patch_exe(exe_path: &Path) -> anyhow::Result<()> {
    patch_binary_file(exe_path, true, 1024 * 1024, false)?;
    Ok(())
}

pub fn unpatch_exe(exe_path: &Path) -> anyhow::Result<()> {
    restore_from_backup(exe_path)
}

pub fn is_exe_patched(exe_path: &Path) -> bool {
    exe_path.with_extension("exe.bak").exists()
}

/// 补丁 Hub 安装的全局共享 Licensing Client DLL。
///
/// Unity 2021 Editor 通过 IPC 连接到此共享客户端 (位于
/// `<Program Files>\Unity Hub\UnityLicensingClient_V1\Unity.Licensing.EntitlementResolver.dll`)
/// 进行 ULF 许可证签名验证; 内含与 Editor DLL 相同的原始证书。
/// 若仅补丁 Editor 本地副本而未补丁此共享客户端, 2021 Editor 会报
/// "digital signature is invalid"。
///
/// 自动扫描 C: 和 D: 盘的 Program Files 目录。
///
/// 跳过条件 (返回 Ok):
/// - Hub 未安装 (目录或 DLL 不存在)
/// - 已存在 `.dll.bak` 备份 (说明已补丁过)
/// - DLL 中未发现原始证书
pub fn patch_shared_licensing_client() -> anyhow::Result<()> {
    // Unity shared client — try all candidate paths
    for dir in shared_licensing_client_candidates() {
        let dll_path = Path::new(dir).join("Unity.Licensing.EntitlementResolver.dll");
        patch_binary_file(&dll_path, true, 0, true)?;
    }

    // Tuanjie shared client (same UTF-16LE PEM replacement as Editor DLL)
    for dir in tuanjie_shared_licensing_client_candidates() {
        let tuanjie_dll = Path::new(dir).join("Tuanjie.Licensing.EntitlementResolver.dll");
        if tuanjie_dll.exists() {
            let bak = tuanjie_dll.with_extension("dll.bak");
            if !bak.exists() {
                if let Err(e) = patch_tuanjie_dll(&tuanjie_dll) {
                    eprintln!("warning: Tuanjie shared client patch failed: {}", e);
                }
            }
        }
    }

    Ok(())
}

/// 还原 Hub 安装的全局共享 Licensing Client DLL 至原始状态。
///
/// 自动扫描 C: 和 D: 盘的 Program Files 目录。
/// 若从未补丁过 (.dll.bak 不存在), 静默成功。
pub fn unpatch_shared_licensing_client() -> anyhow::Result<()> {
    // Unity shared client — try all candidate paths
    for dir in shared_licensing_client_candidates() {
        let dll_path = Path::new(dir).join("Unity.Licensing.EntitlementResolver.dll");
        restore_from_backup(&dll_path)?;
    }

    // Tuanjie shared client
    for dir in tuanjie_shared_licensing_client_candidates() {
        let tuanjie_dll = Path::new(dir).join("Tuanjie.Licensing.EntitlementResolver.dll");
        restore_from_backup(&tuanjie_dll)?;
    }

    Ok(())
}

/// 检查 Hub 安装的全局共享 Licensing Client DLL 是否处于已补丁状态。
///
/// 扫描 C: 和 D: 盘的 Program Files 目录; 任一位置存在备份即视为已补丁。
pub fn is_shared_licensing_client_patched() -> bool {
    // Unity shared client
    for dir in shared_licensing_client_candidates() {
        let dll_path = Path::new(dir).join("Unity.Licensing.EntitlementResolver.dll");
        if dll_path.with_extension("dll.bak").exists() {
            return true;
        }
    }

    // Tuanjie shared client
    for dir in tuanjie_shared_licensing_client_candidates() {
        let tuanjie_dll = Path::new(dir).join("Tuanjie.Licensing.EntitlementResolver.dll");
        if tuanjie_dll.with_extension("dll.bak").exists() {
            return true;
        }
    }

    false
}
