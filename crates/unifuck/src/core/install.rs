use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::vendor::Vendor;

#[derive(Clone)]
pub struct EditorInstall {
    pub version: String,
    pub vendor: Vendor,
    pub dll_path: PathBuf,
    pub exe_path: PathBuf,
}

impl EditorInstall {
    /// Whether this editor is currently a supported patch target. Always true
    /// now that the Tuanjie native runtime hook (`unifuck-loader`) covers the
    /// IPC AuthorizationCheck that previously blocked Tuanjie 2022.3+ rows.
    pub fn is_supported(&self) -> bool {
        true
    }
}

// ── Scan Helpers ───────────────────────────────────────────────

/// Try adding any vendor's editor install at `root` to the list.
fn add_if_valid_install(
    root: &Path,
    version: &str,
    installs: &mut Vec<EditorInstall>,
    seen: &mut HashSet<PathBuf>,
) {
    for vendor in Vendor::all().iter().copied() {
        if let Some(install) = try_find_install_for(root, version, vendor) {
            if seen.insert(install.dll_path.clone()) {
                installs.push(install);
            }
        }
    }
}

/// Scan a Program Files directory for standalone vendor editor folders.
fn scan_pf_for_editors(
    pf_path: &Path,
    installs: &mut Vec<EditorInstall>,
    seen: &mut HashSet<PathBuf>,
) {
    if let Ok(dir) = std::fs::read_dir(pf_path) {
        for entry in dir.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if path.is_dir() {
                for vendor in Vendor::all().iter().copied() {
                    if let Some(version) = name.strip_prefix(vendor.editor_dir_prefix()) {
                        add_if_valid_install(&path, version, installs, seen);
                        break;
                    }
                }
            }
        }
    }
}

/// Scan a Hub-managed Editor directory (e.g. `<Hub>\Editor\`) for version folders.
fn scan_hub_editor_dir(
    editor_dir: &Path,
    installs: &mut Vec<EditorInstall>,
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

fn try_find_install_for(root: &Path, version: &str, vendor: Vendor) -> Option<EditorInstall> {
    let dll_path = root
        .join(r"Editor\Data\Resources\Licensing\Client")
        .join(vendor.dll_name());
    if dll_path.exists() {
        Some(EditorInstall {
            version: version.to_string(),
            vendor,
            dll_path,
            exe_path: root.join("Editor").join(vendor.exe_name()),
        })
    } else {
        None
    }
}

pub fn scan_installations() -> Vec<EditorInstall> {
    let mut installs = Vec::new();
    let mut seen = HashSet::new();

    // 1. Standalone: scan Program Files on all drives
    for pf in crate::discovery::program_files_dirs() {
        scan_pf_for_editors(&pf, &mut installs, &mut seen);
    }

    // 2. Hub-managed: derive from discovered Hub roots per vendor
    for vendor in Vendor::all().iter().copied() {
        for hub_root in crate::discovery::find_hub_roots(vendor.hub_name()) {
            let editor_dir = hub_root.join("Editor");
            scan_hub_editor_dir(&editor_dir, &mut installs, &mut seen);
        }
    }

    // 3. Custom paths from editors.json
    for editor_path in crate::discovery::read_editor_paths_from_config() {
        let version = editor_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "UNKNOWN".to_string());
        add_if_valid_install(&editor_path, &version, &mut installs, &mut seen);
    }

    installs.sort_by(|a, b| {
        a.vendor
            .sort_order()
            .cmp(&b.vendor.sort_order())
            .then_with(|| b.version.cmp(&a.version))
    });
    installs
}
