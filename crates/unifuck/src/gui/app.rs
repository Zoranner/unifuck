use crate::core::{self, InstallGroup};
use crate::discovery;
use slint::{ModelRc, VecModel};
use std::collections::HashMap;
use std::sync::mpsc;

pub struct AppState {
    pub groups: Vec<InstallGroup>,
    pub group_selected: Vec<bool>,
    pub group_patched: Vec<bool>,
    pub custom_hubs: HashMap<String, String>,
    pub running: bool,
    pub done: bool,
    pub success_count: usize,
    pub error_count: usize,
    pub first_error: String,
    pub rx: Option<mpsc::Receiver<String>>,
    pub selection_mode: Option<bool>,
    pub last_op_unpatch: bool,
    pub cleaned_globals: bool,
}

impl AppState {
    pub fn new() -> Self {
        let groups = core::scan_groups();
        let n = groups.len();

        let group_selected = vec![false; n];
        let group_patched: Vec<bool> = groups
            .iter()
            .map(|g| crate::hub::is_hub_patched(&g.hub.root))
            .collect();

        let config = discovery::load_config();
        let custom_hub_paths = config.custom_hubs;

        Self {
            groups,
            group_selected,
            group_patched,
            custom_hubs: custom_hub_paths,
            running: false,
            done: false,
            success_count: 0,
            error_count: 0,
            first_error: String::new(),
            rx: None,
            selection_mode: None,
            last_op_unpatch: false,
            cleaned_globals: false,
        }
    }

    pub fn add_custom_hub_path(&mut self, path: String) {
        let normalized = path.trim_end_matches(['\\', '/']).to_lowercase();
        if self
            .custom_hubs
            .values()
            .any(|v| v.trim_end_matches(['\\', '/']).to_lowercase() == normalized)
        {
            return;
        }
        let hub_name = crate::vendor::Vendor::all()
            .iter()
            .map(|v| v.hub_name().to_string())
            .find(|name| {
                std::path::Path::new(&path)
                    .join(format!("{}.exe", name))
                    .exists()
            })
            .unwrap_or_else(|| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            });
        self.custom_hubs.insert(hub_name, path);
        let config = discovery::Config {
            custom_hubs: self.custom_hubs.clone(),
        };
        let _ = discovery::save_config(&config);

        let groups = core::scan_groups();
        self.groups = groups;
        self.group_selected = vec![false; self.groups.len()];
        self.group_patched = self
            .groups
            .iter()
            .map(|g| crate::hub::is_hub_patched(&g.hub.root))
            .collect();
        self.selection_mode = None;
    }

    pub fn total_count(&self) -> usize {
        self.groups.iter().map(|g| 1usize + g.editors.len()).sum()
    }

    pub fn status_text(&self) -> String {
        if self.running {
            return if self.last_op_unpatch {
                "UN-PATCHING...".into()
            } else {
                "PATCHING...".into()
            };
        }
        let mut total_patched = 0usize;
        let mut total_items = 0usize;
        for (gi, group) in self.groups.iter().enumerate() {
            total_items += 1;
            if *self.group_patched.get(gi).unwrap_or(&false) {
                total_patched += 1;
            }
            for editor in &group.editors {
                total_items += 1;
                if core::is_patched(editor) {
                    total_patched += 1;
                }
            }
        }
        if total_items == 0 {
            return "NO INSTALL".into();
        }
        if total_patched == total_items {
            "ALL PATCHED".into()
        } else if total_patched == 0 {
            "READY".into()
        } else {
            "MIXED".into()
        }
    }

    pub fn has_selection(&self) -> bool {
        self.group_selected.iter().any(|s| *s)
    }

    pub fn refresh_patched(&mut self) {
        self.group_patched = self
            .groups
            .iter()
            .map(|g| crate::hub::is_hub_patched(&g.hub.root))
            .collect();
        if self.group_selected.len() != self.groups.len() {
            self.group_selected = vec![false; self.groups.len()];
        }
    }

    pub fn poll(&mut self) -> (bool, bool) {
        let mut updated = false;
        let mut done = false;
        if let Some(ref rx) = self.rx {
            while let Ok(msg) = rx.try_recv() {
                updated = true;
                if msg.is_empty() {
                    done = true;
                } else if msg.starts_with("TOK") {
                    self.success_count += 1;
                } else if msg.starts_with("TERR") {
                    self.error_count += 1;
                    if self.first_error.is_empty() {
                        self.first_error =
                            msg.strip_prefix("TERR").unwrap_or(&msg).trim().to_string();
                    }
                } else if let Some(rest) = msg.strip_prefix("META ") {
                    if rest == "cleaned-globals" {
                        self.cleaned_globals = true;
                    }
                }
            }
        }
        let just_finished = done && self.running;
        if done {
            self.running = false;
            self.done = true;
            self.rx = None;
        }
        (updated, just_finished)
    }
}

pub fn sync_ui(window: &super::AppWindow, state: &AppState) {
    window.set_status_text(state.status_text().into());
    window.set_working(state.running);
    window.set_done(state.done);
    window.set_version_count(state.total_count() as i32);
    window.set_is_unpatch(state.selection_mode == Some(true));
    window.set_has_selection(state.has_selection());

    let mode = state.selection_mode;
    let mut items: Vec<super::RowData> = Vec::new();

    for (group_idx, group) in state.groups.iter().enumerate() {
        let gp = *state.group_patched.get(group_idx).unwrap_or(&false);
        let gs = *state.group_selected.get(group_idx).unwrap_or(&false);
        let g_disabled = match mode {
            Some(m) => gp != m,
            None => false,
        };

        // Group header 行
        items.push(super::RowData {
            version: format!("{} {}", group.hub.vendor.hub_name(), group.hub.version).into(),
            path: group.hub.root.display().to_string().into(),
            status: (if gp { "PATCHED" } else { "UNPATCHED" }).into(),
            status_color: if gp {
                slint::Color::from_rgb_u8(0x34, 0xd3, 0x99)
            } else {
                slint::Color::from_rgb_u8(0xf8, 0x71, 0x71)
            },
            selected: gs,
            disabled: g_disabled,
            is_group_header: true,
        });

        // Editor 子行
        for editor in &group.editors {
            let ep = core::is_patched(editor);
            items.push(super::RowData {
                version: editor.version.clone().into(),
                path: editor.root.display().to_string().into(),
                status: (if ep { "PATCHED" } else { "UNPATCHED" }).into(),
                status_color: if ep {
                    slint::Color::from_rgb_u8(0x34, 0xd3, 0x99)
                } else {
                    slint::Color::from_rgb_u8(0xf8, 0x71, 0x71)
                },
                selected: gs,
                disabled: g_disabled,
                is_group_header: false,
            });
        }
    }

    window.set_installs(ModelRc::new(VecModel::from(items)));
}
