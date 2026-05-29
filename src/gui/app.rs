use crate::core::{self, UnityInstall};
use slint::{ModelRc, VecModel};
use std::sync::mpsc;

pub struct AppState {
    pub installs: Vec<UnityInstall>,
    pub running: bool,
    pub done: bool,
    pub had_error: bool,
    pub first_error: String,
    pub rx: Option<mpsc::Receiver<String>>,
}

impl AppState {
    pub fn new() -> Self {
        let installs = core::scan_installations();
        Self {
            installs,
            running: false,
            done: false,
            had_error: false,
            first_error: String::new(),
            rx: None,
        }
    }

    pub fn status_text(&self) -> String {
        if self.running {
            return "PATCHING...".into();
        }
        if self.done {
            return "PATCHED".into();
        }
        let n = self.installs.len();
        if n == 0 {
            return "NO INSTALL".into();
        }
        let patched = self
            .installs
            .iter()
            .filter(|i| core::is_patched(&i.dll_path))
            .count();
        if patched == n && n > 0 {
            "ALL PATCHED".into()
        } else {
            "READY".into()
        }
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
                } else if msg.starts_with("ERR") && !self.had_error {
                    self.had_error = true;
                    self.first_error = msg.strip_prefix("ERR").unwrap_or(&msg).trim().to_string();
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
    window.set_version_count(state.installs.len() as i32);

    let items: Vec<super::InstallItem> = state
        .installs
        .iter()
        .map(|i| {
            let p = core::is_patched(&i.dll_path);
            super::InstallItem {
                version: i.version.clone().into(),
                path: i.dll_path.display().to_string().into(),
                status: (if p { "PATCHED" } else { "UNPATCHED" }).into(),
                st_color: if p {
                    slint::Color::from_rgb_u8(0x34, 0xd3, 0x99)
                } else {
                    slint::Color::from_rgb_u8(0xf8, 0x71, 0x71)
                },
                selected: true,
            }
        })
        .collect();
    window.set_installs(ModelRc::new(VecModel::from(items)));
}
