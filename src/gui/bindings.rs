use crate::core;
use crate::gui::app::{sync_ui, AppState};
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
            let i = idx as usize;
            if i >= app.installs.len() {
                return;
            }
            let row_patched = *app.patched.get(i).unwrap_or(&false);
            let cur_sel = *app.selected.get(i).unwrap_or(&false);

            if cur_sel {
                // 取消选中
                app.selected[i] = false;
                if !app.selected.iter().any(|s| *s) {
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
        }
        sync_ui(&win, &app);
        if just_finished {
            let ok = !app.had_error;
            let is_unpatch = app.last_op_unpatch;
            let text: slint::SharedString = if ok {
                if is_unpatch {
                    if app.cleaned_globals {
                        "DLL restored, license and hosts cleaned.".into()
                    } else {
                        "DLL restored. License and hosts kept (other patched editors remain)."
                            .into()
                    }
                } else {
                    "License deployed and hosts configured.".into()
                }
            } else {
                app.first_error.clone().into()
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
        let (selected, mode, all_paths): (Vec<core::UnityInstall>, Option<bool>, Vec<PathBuf>) = {
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
            (sel, mode, all)
        };
        if selected.is_empty() || mode.is_none() {
            return;
        }
        let is_unpatch = mode == Some(true);

        let (tx, rx) = mpsc::channel();
        {
            let mut app = s.borrow_mut();
            app.running = true;
            app.done = false;
            app.had_error = false;
            app.first_error.clear();
            app.cleaned_globals = false;
            app.last_op_unpatch = is_unpatch;
            app.rx = Some(rx);
        }
        sync_ui(&win, &s.borrow());

        if is_unpatch {
            thread::spawn(move || run_unpatch(tx, selected, all_paths));
        } else {
            thread::spawn(move || run_patch(tx, selected));
        }
    });
}

fn run_patch(tx: mpsc::Sender<String>, selected: Vec<core::UnityInstall>) {
    let mut ok = false;
    for inst in &selected {
        if core::is_patched(&inst.dll_path) {
            tx.send(format!("OK  {}  already patched, skipped", inst.version))
                .ok();
            ok = true;
            continue;
        }
        tx.send(format!("..  {}  patching DLL...", inst.version))
            .ok();
        match core::patch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("OK  {}  success", inst.version)).ok();
                ok = true;
            }
            Err(e) => {
                tx.send(format!("ERR {}  failed: {}", inst.version, e)).ok();
            }
        }
    }
    if !ok {
        tx.send("ERR no version patched successfully".into()).ok();
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
) {
    let mut ok = false;
    for inst in &selected {
        tx.send(format!("..  {}  un-patching...", inst.version))
            .ok();
        match core::unpatch_dll(&inst.dll_path) {
            Ok(()) => {
                tx.send(format!("OK  {}  restored", inst.version)).ok();
                ok = true;
            }
            Err(e) => {
                tx.send(format!("ERR {}  failed: {}", inst.version, e)).ok();
            }
        }
    }
    if !ok {
        tx.send("ERR no version un-patched successfully".into())
            .ok();
    } else {
        // 检查是否还有任意 install 仍处于 patched 状态
        let any_left = all_paths.iter().any(|p| core::is_patched(p));
        if !any_left {
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
            tx.send("OK  other patched editors remain, keeping license and hosts".into())
                .ok();
        }
        tx.send("OK  un-patch complete".into()).ok();
    }
    tx.send(String::new()).ok();
}
