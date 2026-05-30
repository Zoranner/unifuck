use crate::core;
use crate::gui::app::{sync_ui, AppState};
use crate::hub::{self, HubInfo};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

pub fn connect(window: &super::AppWindow, state: Rc<RefCell<AppState>>) {
    let s = state.clone();
    let w = window.as_weak();
    window.on_toggle_select(move |idx| {
        let Some(win) = w.upgrade() else {
            return;
        };
        {
            let mut app = s.borrow_mut();
            if app.running {
                return;
            }

            let has_hub = app.hub_info.is_some();
            let is_hub_row = has_hub && idx == 0;
            // Hub 存在时 Editor 行从 UI index 1 开始, 映射到 installs index 0
            let editor_idx = if has_hub {
                idx as usize - 1
            } else {
                idx as usize
            };

            if is_hub_row {
                let hub_patched = app.hub_patched;
                if app.hub_selected {
                    app.hub_selected = false;
                    if !app.selected.iter().any(|s| *s) {
                        app.selection_mode = None;
                    }
                } else {
                    match app.selection_mode {
                        None => {
                            app.selection_mode = Some(hub_patched);
                            app.hub_selected = true;
                        }
                        Some(m) if m == hub_patched => {
                            app.hub_selected = true;
                        }
                        _ => return,
                    }
                }
            } else {
                let i = editor_idx;
                if i >= app.installs.len() {
                    return;
                }
                let row_patched = *app.patched.get(i).unwrap_or(&false);
                let cur_sel = *app.selected.get(i).unwrap_or(&false);

                if cur_sel {
                    // 取消选中
                    app.selected[i] = false;
                    if !app.selected.iter().any(|s| *s) && !app.hub_selected {
                        app.selection_mode = None;
                    }
                } else {
                    // 选中
                    match app.selection_mode {
                        None => {
                            app.selection_mode = Some(row_patched);
                            app.selected[i] = true;
                        }
                        Some(m) if m == row_patched => {
                            app.selected[i] = true;
                        }
                        _ => {
                            // 状态不匹配, 忽略
                            return;
                        }
                    }
                }
            }
        }
        sync_ui(&win, &s.borrow());
    });

    let s = state.clone();
    let w = window.as_weak();
    window.on_poll(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        let mut app = s.borrow_mut();
        let (updated, just_finished) = app.poll();
        if !updated {
            return;
        }
        if just_finished {
            // 重扫 patched, 重置选择
            app.refresh_patched();
            app.selection_mode = None;
            for s in app.selected.iter_mut() {
                *s = false;
            }
            app.hub_selected = false;
        }
        sync_ui(&win, &app);
        if just_finished {
            let total = app.success_count + app.error_count;
            let ok = app.error_count == 0;
            let partial = app.success_count > 0 && app.error_count > 0;
            let is_unpatch = app.last_op_unpatch;

            let text: slint::SharedString = if ok {
                // 全部成功
                if is_unpatch {
                    if app.cleaned_globals {
                        format!("{} restored, license and hosts cleaned up.", total).into()
                    } else {
                        format!(
                            "{} restored. License and hosts kept (other targets still patched).",
                            total
                        )
                        .into()
                    }
                } else {
                    format!("{} patched, license deployed.", total).into()
                }
            } else if partial {
                // 部分成功
                let summary = format!(
                    "{} selected, {} succeeded, {} failed",
                    total, app.success_count, app.error_count
                );
                let err_info = truncate_error(&app.first_error, 40);
                format!("{}\n{}", summary, err_info).into()
            } else {
                // 全部失败
                let summary = format!(
                    "{} selected, 0 succeeded, {} failed",
                    total, app.error_count
                );
                let err_info = truncate_error(&app.first_error, 40);
                format!("{}\n{}", summary, err_info).into()
            };

            win.set_result_ok(if ok { 1 } else { 0 });
            win.set_result_mode(if is_unpatch { 1 } else { 0 });
            win.set_result_text(text);
            win.set_show_result_dialog(true);
        }
    });

    let w = window.as_weak();
    window.on_dismiss_result(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        win.set_show_result_dialog(false);
    });

    let s = state.clone();
    let w = window.as_weak();
    window.on_activate(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        // 准备数据
        #[allow(clippy::type_complexity)]
        let (selected, mode, all_paths, all_exe_paths, hub_info, hub_selected): (
            Vec<core::UnityInstall>,
            Option<bool>,
            Vec<PathBuf>,
            Vec<PathBuf>,
            Option<HubInfo>,
            bool,
        ) = {
            let app = s.borrow();
            if app.running {
                return;
            }
            let mode = app.selection_mode;
            if mode.is_none() {
                return;
            }
            let mut sel = Vec::new();
            for (idx, inst) in app.installs.iter().enumerate() {
                if *app.selected.get(idx).unwrap_or(&false) {
                    sel.push(inst.clone());
                }
            }
            let all: Vec<PathBuf> = app.installs.iter().map(|i| i.dll_path.clone()).collect();
            let all_exe: Vec<PathBuf> = app.installs.iter().map(|i| i.exe_path.clone()).collect();
            let hub = app.hub_info.clone();
            let hub_sel = app.hub_selected;
            (sel, mode, all, all_exe, hub, hub_sel)
        };
        if (selected.is_empty() && !hub_selected) || mode.is_none() {
            return;
        }
        let is_unpatch = mode == Some(true);

        let (tx, rx) = mpsc::channel();
        {
            let mut app = s.borrow_mut();
            app.running = true;
            app.done = false;
            app.success_count = 0;
            app.error_count = 0;
            app.first_error.clear();
            app.cleaned_globals = false;
            app.last_op_unpatch = is_unpatch;
            app.rx = Some(rx);
        }
        sync_ui(&win, &s.borrow());

        if is_unpatch {
            thread::spawn(move || {
                run_unpatch(
                    tx,
                    selected,
                    all_paths,
                    all_exe_paths,
                    hub_info,
                    hub_selected,
                );
            });
        } else {
            thread::spawn(move || run_patch(tx, selected, hub_info, hub_selected));
        }
    });
}

fn run_patch(
    tx: mpsc::Sender<String>,
    selected: Vec<core::UnityInstall>,
    hub_info: Option<HubInfo>,
    hub_selected: bool,
) {
    let mut ok = false;

    // Editor DLL + exe patching
    for inst in &selected {
        if core::is_patched(&inst.dll_path) {
            tx.send(format!("TOK  {}  already patched, skipped", inst.version))
                .ok();
            ok = true;
            continue;
        }
        tx.send(format!("..  {}  patching DLL...", inst.version))
            .ok();
        match core::patch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL patched", inst.version)).ok();
                // 同时补丁 exe (仅对 Unity 2021 等单体二进制生效)
                tx.send(format!("..  {}  patching exe...", inst.version))
                    .ok();
                match core::patch_exe(&inst.exe_path) {
                    Ok(()) => {
                        if core::is_exe_patched(&inst.exe_path) {
                            tx.send(format!("..  {}  exe patched", inst.version)).ok();
                        } else {
                            tx.send(format!("..  {}  exe skipped (no cert)", inst.version))
                                .ok();
                        }
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe patch failed: {}", inst.version, e))
                            .ok();
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", inst.version, e))
                    .ok();
            }
        }
    }

    // Shared Licensing Client (Hub-installed, used by 2021 Editors via IPC)
    tx.send("..  patching shared licensing client...".into())
        .ok();
    match core::patch_shared_licensing_client() {
        Ok(()) => {
            if core::is_shared_licensing_client_patched() {
                tx.send("OK  shared licensing client patched".into()).ok();
            } else {
                tx.send("..  shared licensing client skipped (not found)".into())
                    .ok();
            }
        }
        Err(e) => {
            tx.send(format!("..  shared licensing client failed: {}", e))
                .ok();
        }
    }

    // Hub patching
    if hub_selected {
        if let Some(ref hub) = hub_info {
            if hub::is_hub_patched(&hub.asar_path) {
                tx.send(format!(
                    "TOK  Hub {}  already patched, skipped",
                    hub.version
                ))
                .ok();
                ok = true;
            } else {
                tx.send(format!("..  Hub {}  patching...", hub.version))
                    .ok();
                match hub::patch_hub(hub) {
                    Ok(()) => {
                        tx.send(format!("TOK  Hub {}  success", hub.version)).ok();
                        ok = true;
                    }
                    Err(e) => {
                        tx.send(format!("TERR Hub {}: {}", hub.version, e)).ok();
                    }
                }
            }
        }
    }

    if !ok {
        tx.send("ERR no target patched successfully".into()).ok();
    } else {
        tx.send("..  generating license...".into()).ok();
        match core::sign_license() {
            Ok(xml) => match core::deploy_license(&xml) {
                Ok(()) => {
                    tx.send("OK  license deployed".into()).ok();
                }
                Err(e) => {
                    tx.send(format!("ERR deploy: {}", e)).ok();
                }
            },
            Err(e) => {
                tx.send(format!("ERR sign: {}", e)).ok();
            }
        }
        tx.send("..  writing hosts...".into()).ok();
        match core::write_hosts_if_needed() {
            Ok(()) => {
                tx.send("OK  hosts updated".into()).ok();
            }
            Err(e) => {
                tx.send(format!("ERR hosts: {}", e)).ok();
            }
        }
        tx.send("OK  patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}

fn run_unpatch(
    tx: mpsc::Sender<String>,
    selected: Vec<core::UnityInstall>,
    all_paths: Vec<PathBuf>,
    all_exe_paths: Vec<PathBuf>,
    hub_info: Option<HubInfo>,
    hub_selected: bool,
) {
    let mut ok = false;

    // Editor DLL + exe unpatching
    for inst in &selected {
        tx.send(format!("..  {}  un-patching...", inst.version))
            .ok();
        match core::unpatch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("TOK  {}  DLL restored", inst.version)).ok();
                // 同时还原 exe (若从未补丁过则静默跳过)
                match core::unpatch_exe(&inst.exe_path) {
                    Ok(()) => {
                        tx.send(format!("..  {}  exe restored/skipped", inst.version))
                            .ok();
                    }
                    Err(e) => {
                        tx.send(format!("..  {}  exe restore failed: {}", inst.version, e))
                            .ok();
                    }
                }
                ok = true;
            }
            Err(e) => {
                tx.send(format!("TERR {}  failed: {}", inst.version, e))
                    .ok();
            }
        }
    }

    // Shared Licensing Client
    tx.send("..  un-patching shared licensing client...".into())
        .ok();
    match core::unpatch_shared_licensing_client() {
        Ok(()) => {
            tx.send("OK  shared licensing client restored".into()).ok();
        }
        Err(e) => {
            tx.send(format!("..  shared licensing client restore failed: {}", e))
                .ok();
        }
    }

    // Hub unpatching
    if hub_selected {
        if let Some(ref hub) = hub_info {
            tx.send(format!("..  Hub {}  un-patching...", hub.version))
                .ok();
            match hub::unpatch_hub(hub) {
                Ok(()) => {
                    tx.send(format!("TOK  Hub {}  restored", hub.version)).ok();
                    ok = true;
                }
                Err(e) => {
                    tx.send(format!("TERR Hub {}: {}", hub.version, e)).ok();
                }
            }
        }
    }

    if !ok {
        tx.send("ERR no target un-patched successfully".into()).ok();
    } else {
        // 检查是否还有任意 install 仍处于 patched 状态
        let any_editor_left = all_paths.iter().any(|p| core::is_patched(p))
            || all_exe_paths.iter().any(|p| core::is_exe_patched(p))
            || core::is_shared_licensing_client_patched();
        let any_hub_left = hub_info
            .as_ref()
            .map(|h| hub::is_hub_patched(&h.asar_path))
            .unwrap_or(false);

        if !any_editor_left && !any_hub_left {
            tx.send("..  removing license...".into()).ok();
            match core::remove_license() {
                Ok(()) => {
                    tx.send("OK  license removed".into()).ok();
                }
                Err(e) => {
                    tx.send(format!("ERR remove license: {}", e)).ok();
                }
            }
            tx.send("..  cleaning hosts...".into()).ok();
            match core::clean_hosts() {
                Ok(()) => {
                    tx.send("OK  hosts cleaned".into()).ok();
                    tx.send("META cleaned-globals".into()).ok();
                }
                Err(e) => {
                    tx.send(format!("ERR clean hosts: {}", e)).ok();
                }
            }
        } else {
            tx.send("OK  other patched targets remain, keeping license and hosts".into())
                .ok();
        }
        tx.send("OK  un-patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}

fn truncate_error(err: &str, max_chars: usize) -> String {
    if err.len() <= max_chars {
        err.to_string()
    } else {
        format!("{}...", &err[..max_chars])
    }
}
