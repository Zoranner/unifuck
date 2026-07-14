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

fn add_if_valid_install(
    root: &Path,
    version: &str,
    installs: &mut Vec<EditorInfo>,
    seen: &mut HashSet<PathBuf>,
) {
    for vendor in Vendor::all().iter().copied() {
        if let Some(info) = try_find_install_for(root, version, vendor) {
            if seen.insert(info.root.clone()) {
                installs.push(info);
            }
        }
    }
}

fn scan_hub_editor_dir(
    editor_dir: &Path,
    installs: &mut Vec<EditorInfo>,
    seen: &mut HashSet<PathBuf>,
) {
    if let Ok(dir) = std::fs::read_dir(editor_dir) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let version = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                add_if_valid_install(&path, &version, installs, seen);
            }
        }
    }
}

fn try_find_install_for(root: &Path, version: &str, vendor: Vendor) -> Option<EditorInfo> {
    let editor_dir = root.join("Editor");
    let dll = editor_dir
        .join(r"Data\Resources\Licensing\Client")
        .join(vendor.dll_name());
    if dll.exists() {
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
/// 优先级：自动扫描（注册表 > Program Files）> 自定义路径兜底。
/// `custom_hubs` 的 key 是 hub_name（如 "Unity Hub"），value 是自定义路径。
pub fn scan_groups(custom_hubs: &HashMap<String, String>) -> Vec<InstallGroup> {
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
                let asar_path = custom_root.join(asar_rel);
                if let Some(group) = build_hub_group(vendor, &custom_root, &asar_path) {
                    groups.push(group);
                }
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
    let editor_dir = hub_root.join("Editor");
    scan_hub_editor_dir(&editor_dir, &mut editors, &mut seen);

    // editors.json for this vendor
    for editor_path in crate::discovery::read_editor_paths(vendor) {
        let version = editor_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        add_if_valid_install(&editor_path, &version, &mut editors, &mut seen);
    }

    Some(InstallGroup {
        hub: hub_info,
        editors,
    })
}
