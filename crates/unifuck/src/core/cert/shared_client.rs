use crate::vendor::Vendor;

use super::binary::{bak_path, patch_binary_file, restore_from_backup};
use super::tuanjie::{TUANJIE_MOD_CERT_PEM, TUANJIE_ORIG_CERT_PEM};
use super::unity::{UNITY_MOD_CERT_PEM, UNITY_ORIG_CERT_PEM};

/// 补丁 Hub 安装的全局共享 Licensing Client DLL。
///
/// Unity 2021 Editor 通过 IPC 连接到此共享客户端 (位于
/// `<Hub>\UnityLicensingClient_V1\Unity.Licensing.EntitlementResolver.dll`)
/// 进行 ULF 许可证签名验证; 内含与 Editor DLL 相同的原始证书。
/// 若仅补丁 Editor 本地副本而未补丁此共享客户端, 2021 Editor 会报
/// "digital signature is invalid"。
///
/// 自动扫描所有可访问磁盘上的 Hub 安装目录, 对每个 vendor 派发到对应实现。
///
/// 跳过条件 (返回 Ok):
/// - Hub 未安装 (目录或 DLL 不存在)
/// - 已存在 `.dll.bak` 备份 (说明已补丁过)
/// - DLL 中未发现原始证书
pub fn patch_shared_licensing_client() -> anyhow::Result<()> {
    for vendor in Vendor::all().iter().copied() {
        for hub_root in crate::discovery::find_hub_roots(vendor.hub_name()) {
            let dll_path = hub_root
                .join(vendor.licensing_client_dir())
                .join(vendor.dll_name());
            if !dll_path.exists() || bak_path(&dll_path).exists() {
                continue;
            }
            match vendor {
                Vendor::Unity => {
                    patch_binary_file(
                        &dll_path,
                        UNITY_ORIG_CERT_PEM,
                        UNITY_MOD_CERT_PEM,
                        true,
                        0,
                        true,
                    )?;
                }
                Vendor::Tuanjie => {
                    patch_binary_file(
                        &dll_path,
                        TUANJIE_ORIG_CERT_PEM,
                        TUANJIE_MOD_CERT_PEM,
                        true,
                        0,
                        true,
                    )?;
                }
            }
        }
    }
    Ok(())
}

/// 还原 Hub 安装的全局共享 Licensing Client DLL 至原始状态。
///
/// 自动扫描所有可访问磁盘上的 Hub 安装目录。
/// 若从未补丁过 (.dll.bak 不存在), 静默成功。
pub fn unpatch_shared_licensing_client() -> anyhow::Result<()> {
    for vendor in Vendor::all().iter().copied() {
        for hub_root in crate::discovery::find_hub_roots(vendor.hub_name()) {
            let dll_path = hub_root
                .join(vendor.licensing_client_dir())
                .join(vendor.dll_name());
            restore_from_backup(&dll_path)?;
        }
    }
    Ok(())
}

/// 检查 Hub 安装的全局共享 Licensing Client DLL 是否处于已补丁状态。
///
/// 扫描所有可访问磁盘上的 Hub 安装目录; 任一位置存在备份即视为已补丁。
pub fn is_shared_licensing_client_patched() -> bool {
    for vendor in Vendor::all().iter().copied() {
        for hub_root in crate::discovery::find_hub_roots(vendor.hub_name()) {
            let dll_path = hub_root
                .join(vendor.licensing_client_dir())
                .join(vendor.dll_name());
            if bak_path(&dll_path).exists() {
                return true;
            }
        }
    }
    false
}
