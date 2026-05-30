use crate::core::{self, UnityInstall};
use crate::hub::{self, HubInfo};
use slint::{ModelRc, VecModel};
use std::sync::mpsc;

pub struct AppState {
    pub installs: Vec<UnityInstall>,
    pub hub_info: Option<HubInfo>,
    pub hub_patched: bool,
    pub hub_selected: bool,
    pub running: bool,
    pub done: bool,
    pub success_count: usize,
    pub error_count: usize,
    pub first_error: String,
    pub rx: Option<mpsc::Receiver<String>>,
    /// 当前选择模式: `Some(true)` = 选中均为已 patched (UN-PATCH 模式),
    /// `Some(false)` = 选中均为 unpatched (PATCH 模式), `None` = 无选择
    pub selection_mode: Option<bool>,
    /// 行的 selected 状态 (与 installs 等长)
    pub selected: Vec<bool>,
    /// 行的 patched 状态缓存 (与 installs 等长)
    pub patched: Vec<bool>,
    /// 上次操作类型: false=PATCH, true=UN-PATCH (用于结果 dialog 文案)
    pub last_op_unpatch: bool,
    /// UN-PATCH 完成后是否同时清掉了 license / hosts
    pub cleaned_globals: bool,
}

impl AppState {
    pub fn new() -> Self {
        let installs = core::scan_installations();
        let patched: Vec<bool> = installs
            .iter()
            .map(|i| core::is_patched(&i.dll_path))
            .collect();
        let selected = vec![false; installs.len()];
        let hub_info = hub::find_hub();
        let hub_patched = hub_info
            .as_ref()
            .map(|h| hub::is_hub_patched(&h.asar_path))
            .unwrap_or(false);
        Self {
            installs,
            hub_info,
            hub_patched,
            hub_selected: false,
            running: false,
            done: false,
            success_count: 0,
            error_count: 0,
            first_error: String::new(),
            rx: None,
            selection_mode: None,
            selected,
            patched,
            last_op_unpatch: false,
            cleaned_globals: false,
        }
    }

    pub fn total_count(&self) -> usize {
        self.installs.len() + if self.hub_info.is_some() { 1 } else { 0 }
    }

    pub fn status_text(&self) -> String {
        if self.running {
            return if self.last_op_unpatch {
                "UN-PATCHING...".into()
            } else {
                "PATCHING...".into()
            };
        }
        let n = self.total_count();
        if n == 0 {
            return "NO INSTALL".into();
        }
        let hub_patched_count = if self.hub_info.is_some() && self.hub_patched {
            1
        } else {
            0
        };
        let editor_patched_count = self.patched.iter().filter(|p| **p).count();
        let patched = hub_patched_count + editor_patched_count;
        if patched == n {
            "ALL PATCHED".into()
        } else if patched == 0 {
            "READY".into()
        } else {
            "MIXED".into()
        }
    }

    pub fn has_selection(&self) -> bool {
        self.hub_selected || self.selected.iter().any(|s| *s)
    }

    /// 重新扫描 patched 状态 (基于 .bak 是否存在)
    pub fn refresh_patched(&mut self) {
        self.patched = self
            .installs
            .iter()
            .map(|i| core::is_patched(&i.dll_path))
            .collect();
        if let Some(ref hub) = self.hub_info {
            self.hub_patched = hub::is_hub_patched(&hub.asar_path);
        }
        if self.patched.len() != self.selected.len() {
            self.selected = vec![false; self.patched.len()];
        }
        self.hub_selected = false;
    }

    // Returns (updated, just_finished)
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
                    // 线程内传递的元数据 (非用户可见日志)
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
    window.set_is_unpatch_mode(state.selection_mode == Some(true));
    window.set_has_selection(state.has_selection());

    let mode = state.selection_mode;
    let mut items: Vec<super::InstallItem> = Vec::new();

    // Hub 行 (始终作为第一行)
    if let Some(ref hub) = state.hub_info {
        let p = state.hub_patched;
        let sel = state.hub_selected;
        let disabled = match mode {
            Some(m) => p != m,
            None => false,
        };
        items.push(super::InstallItem {
            version: format!("Hub {}", hub.version).into(),
            path: hub.asar_path.display().to_string().into(),
            status: (if p { "PATCHED" } else { "UNPATCHED" }).into(),
            st_color: if p {
                slint::Color::from_rgb_u8(0x34, 0xd3, 0x99)
            } else {
                slint::Color::from_rgb_u8(0xf8, 0x71, 0x71)
            },
            selected: sel,
            disabled,
            patched: p,
        });
    }

    // Editor 行
    for (idx, i) in state.installs.iter().enumerate() {
        let p = *state.patched.get(idx).unwrap_or(&false);
        let sel = *state.selected.get(idx).unwrap_or(&false);
        let disabled = match mode {
            Some(m) => p != m,
            None => false,
        };
        items.push(super::InstallItem {
            version: format!("Editor {}", i.version).into(),
            path: i.dll_path.display().to_string().into(),
            status: (if p { "PATCHED" } else { "UNPATCHED" }).into(),
            st_color: if p {
                slint::Color::from_rgb_u8(0x34, 0xd3, 0x99)
            } else {
                slint::Color::from_rgb_u8(0xf8, 0x71, 0x71)
            },
            selected: sel,
            disabled,
            patched: p,
        });
    }

    window.set_installs(ModelRc::new(VecModel::from(items)));
}
