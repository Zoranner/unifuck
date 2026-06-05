use std::path::Path;

use super::restore_from_backup;
use super::unity;

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
    unity::patch_binary_file(exe_path, true, 1024 * 1024, false)?;
    Ok(())
}

pub fn unpatch_exe(exe_path: &Path) -> anyhow::Result<()> {
    restore_from_backup(exe_path)
}

pub fn is_exe_patched(exe_path: &Path) -> bool {
    exe_path.with_extension("exe.bak").exists()
}
