use std::path::Path;

use super::binary::patch_binary_file;
use super::unity::{UNITY_MOD_CERT_PEM, UNITY_ORIG_CERT_PEM};

/// 补丁 Editor 主程序 exe 中内嵌的许可证书。
///
/// 仅对 Unity 2021 等单体二进制版本有效: 这类 exe 约 73 MB, 内含 LicensingLegacy
/// 模块与同一份 X.509 证书; Unity 6000+ 的 exe 只是约 282 KB 的启动器, 不含证书。
/// Tuanjie exe 走 native hook 路线 (jobs.rs 的 `supports_native_hook` 门控),
/// 不内嵌 Unity 证书, 因此这里匹配不到任何 base64 时静默跳过 ——
/// 调用方对所有 vendor 无条件调用本函数是安全的。
///
/// 跳过条件 (返回 Ok 但不生成备份):
/// - exe 不存在
/// - exe 体积小于 1 MB (判定为启动器)
/// - exe 中未发现原始证书 base64 (既不以多行也不以单行形态出现)
pub fn patch_exe(exe_path: &Path) -> anyhow::Result<()> {
    patch_binary_file(
        exe_path,
        UNITY_ORIG_CERT_PEM,
        UNITY_MOD_CERT_PEM,
        true,
        1024 * 1024,
        false,
    )?;
    Ok(())
}

pub fn unpatch_exe(exe_path: &Path) -> anyhow::Result<()> {
    super::binary::restore_from_backup(exe_path)
}

pub fn is_exe_patched(exe_path: &Path) -> bool {
    super::binary::bak_path(exe_path).exists()
}
