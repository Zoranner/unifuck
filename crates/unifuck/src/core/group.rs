use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::hub;
use crate::hub::HubInfo;
use crate::vendor::Vendor;

// ── InstallGroup ───────────────────────────────────────────────

pub struct InstallGroup {
    pub hub: HubInfo,
    pub editors: Vec<EditorInfo>,
}

// ── EditorInfo ─────────────────────────────────────────────────

/// Detected Editor installation.
///
/// `root` is the Editor directory (e.g. `...\Unity 6000.4.8f1\Editor`).
/// All other paths derive from `root`.
#[derive(Clone)]
pub struct EditorInfo {
    pub version: String,
    pub vendor: Vendor,
    pub root: PathBuf,
}

impl EditorInfo {
    pub fn dll_path(&self) -> PathBuf {
        self.root
            .join(r"Data\Resources\Licensing\Client")
            .join(self.vendor.dll_name())
    }

    pub fn exe_path(&self) -> PathBuf {
        self.root.join(self.vendor.exe_name())
    }
}

// ── Scan helpers ───────────────────────────────────────────────

/// 收录条件: `root\Editor\<vendor>.exe` 存在。
/// DLL 是否存在不在此检查 —— 那是 patch 阶段的事, 缺 DLL 应在 patch 时报错,
/// 而不是发现时把 Editor 隐藏掉。
fn add_if_valid_install(
    root: &Path,
    version: &str,
    installs: &mut Vec<EditorInfo>,
    seen: &mut HashSet<PathBuf>,
) {
    for vendor in Vendor::all().iter().copied() {
        if let Some(info) = try_find_install_for(root, version, vendor) {
            // seen 键用规范化路径,避免同一物理安装因大小写/尾分隔差异被重复收录;
            // info.root 本身保留原始路径,用于展示与后续 patch 操作。
            if seen.insert(crate::discovery::normalize_path(&info.root)) {
                crate::diag::log(&format!(
                    "editor accepted: {} {} at {}",
                    vendor.display_name(),
                    version,
                    info.root.display()
                ));
                installs.push(info);
            }
            return;
        }
    }
    crate::diag::log(&format!(
        "editor rejected (no exe under {}\\Editor)",
        root.display()
    ));
}

fn try_find_install_for(root: &Path, version: &str, vendor: Vendor) -> Option<EditorInfo> {
    let editor_dir = root.join("Editor");
    if editor_dir.join(vendor.exe_name()).is_file() {
        Some(EditorInfo {
            version: version.to_string(),
            vendor,
            root: editor_dir,
        })
    } else {
        None
    }
}

// ── Entry point ────────────────────────────────────────────────

/// 扫描所有 Hub 安装，每个 vendor 最多一个 group。
///
/// Editor 来自两类来源，按规范化路径去重、共享一个 seen 集合：
/// - Hub 配置 JSON 记录 (editors-v2.json / editors.json)，优先收录；
/// - 厂商默认安装目录扫描 (`<前缀>\<Vendor>\Hub\Editor`)，补 JSON 未记录的官方安装。
///
/// JSON 记录优先、默认目录补漏；同一物理安装只收录一次。
///
/// 优先级：自动扫描（注册表 > Program Files）> 自定义路径兜底。
/// `custom_hubs` 的 key 是 hub_name（如 "Unity Hub"），value 是自定义路径。
pub fn scan_groups(custom_hubs: &HashMap<String, String>) -> Vec<InstallGroup> {
    crate::diag::begin_session();
    let mut groups = Vec::new();

    for vendor in Vendor::all().iter().copied() {
        let hub_name = vendor.hub_name();
        let asar_rel = std::path::Path::new(hub::HUB_ASAR_REL);

        // 1. 自动扫描优先：find_hub_roots 内部已按 注册表 → Program Files 排序
        let auto_roots = crate::discovery::find_hub_roots(hub_name);
        if let Some(root) = auto_roots.first() {
            let asar_path = root.join(asar_rel);
            if let Some(group) = build_hub_group(vendor, root, &asar_path) {
                groups.push(group);
                continue; // 已命中，跳过自定义兜底
            }
        }

        // 2. 自定义路径兜底：自动扫描无结果时尝试
        if let Some(custom_path) = custom_hubs.get(hub_name) {
            let custom_root = std::path::PathBuf::from(custom_path);
            let hub_exe = custom_root.join(format!("{}.exe", hub_name));
            if hub_exe.exists() {
                crate::diag::log(&format!(
                    "hub found via custom path: {}",
                    custom_root.display()
                ));
                let asar_path = custom_root.join(asar_rel);
                if let Some(group) = build_hub_group(vendor, &custom_root, &asar_path) {
                    groups.push(group);
                }
            } else {
                crate::diag::log(&format!(
                    "custom hub path invalid (exe missing): {}",
                    hub_exe.display()
                ));
            }
        }
    }

    for group in &mut groups {
        group.editors.sort_by(|a, b| b.version.cmp(&a.version));
    }
    groups.sort_by(|a, b| {
        a.hub
            .vendor
            .sort_order()
            .cmp(&b.hub.vendor.sort_order())
            .then_with(|| b.hub.version.cmp(&a.hub.version))
    });

    groups
}

fn build_hub_group(vendor: Vendor, hub_root: &Path, asar_path: &Path) -> Option<InstallGroup> {
    let hub_info = hub::hub_info_from_path(vendor, hub_root, asar_path)?;

    let mut editors = Vec::new();
    let mut seen = HashSet::new();

    // Hub 配置记录 (editors-v2.json / editors.json) 中的 Editor
    for record in crate::discovery::read_editor_paths(vendor) {
        let version = record.version.unwrap_or_else(|| {
            record
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "UNKNOWN".to_string())
        });
        add_if_valid_install(&record.root, &version, &mut editors, &mut seen);
    }

    // 默认安装目录补漏: Hub 官方安装的 Editor 不写入 Hub 配置 JSON,
    // 需扫描默认目录。version 直接用目录名 (同上面的目录名回退)。
    // 与 JSON 记录共享 seen 去重, JSON 记录优先, 此处仅补漏。
    for root in crate::discovery::scan_default_editor_paths(vendor) {
        let version = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        add_if_valid_install(&root, &version, &mut editors, &mut seen);
    }

    crate::diag::log(&format!(
        "group built: {} hub {} at {}, editors={}",
        vendor.display_name(),
        hub_info.version,
        hub_root.display(),
        editors.len()
    ));

    Some(InstallGroup {
        hub: hub_info,
        editors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 `<tmp>\<version>\Editor\<vendor>.exe`,返回 `<tmp>\<version>` 这一层。
    fn make_install(tmp: &Path, version: &str, vendor: Vendor) -> PathBuf {
        let editor_dir = tmp.join(version).join("Editor");
        std::fs::create_dir_all(&editor_dir).unwrap();
        std::fs::write(editor_dir.join(vendor.exe_name()), b"").unwrap();
        tmp.join(version)
    }

    #[test]
    fn add_if_valid_install_collects_tuanjie() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_install(tmp.path(), "2022.3.62t5", Vendor::Tuanjie);

        let mut installs = Vec::new();
        let mut seen = HashSet::new();
        add_if_valid_install(&root, "2022.3.62t5", &mut installs, &mut seen);

        assert_eq!(installs.len(), 1);
        assert_eq!(installs[0].vendor, Vendor::Tuanjie);
    }

    #[cfg(windows)]
    #[test]
    fn add_if_valid_install_dedupes_by_case() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_install(tmp.path(), "2022.3.62f3", Vendor::Unity);
        // 同一物理安装根的另一种大小写形式 (Windows 文件系统不区分大小写,
        // 两种写法都指向同一目录,均能命中 exe 检查)。
        let upper = root.to_string_lossy().to_uppercase();
        let root_upper = PathBuf::from(upper);

        let mut installs = Vec::new();
        let mut seen = HashSet::new();
        add_if_valid_install(&root, "2022.3.62f3", &mut installs, &mut seen);
        add_if_valid_install(&root_upper, "2022.3.62f3", &mut installs, &mut seen);

        assert_eq!(installs.len(), 1, "同一安装按规范化路径只收录一次");
        // 收录的 info.root 保留首次进入的原始路径,不被改写。
        assert_eq!(installs[0].root, root.join("Editor"));
    }

    #[test]
    fn add_if_valid_install_dedupes_canonical_alias() {
        let tmp = tempfile::tempdir().unwrap();
        let root = make_install(tmp.path(), "2022.3.62f3", Vendor::Unity);
        let alias = root.join("..").join("2022.3.62f3");

        let mut installs = Vec::new();
        let mut seen = HashSet::new();
        add_if_valid_install(&root, "2022.3.62f3", &mut installs, &mut seen);
        add_if_valid_install(&alias, "2022.3.62f3", &mut installs, &mut seen);

        assert_eq!(installs.len(), 1, "同一安装的规范路径别名只收录一次");
    }

    #[test]
    fn add_if_valid_install_collects_distinct_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let root_a = make_install(tmp.path(), "2022.3.62f3", Vendor::Unity);
        let root_b = make_install(tmp.path(), "6000.4.8f1", Vendor::Unity);

        let mut installs = Vec::new();
        let mut seen = HashSet::new();
        add_if_valid_install(&root_a, "2022.3.62f3", &mut installs, &mut seen);
        add_if_valid_install(&root_b, "6000.4.8f1", &mut installs, &mut seen);

        assert_eq!(installs.len(), 2, "不同安装根应分别收录");
    }
}
