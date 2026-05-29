use crate::core;
use crate::gui::app::{sync_ui, AppState};
use slint::{ComponentHandle, Model};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;

pub fn connect(window: &super::AppWindow, state: Rc<RefCell<AppState>>) {
    let w = window.as_weak();
    window.on_toggle_select(move |idx| {
        let Some(win) = w.upgrade() else {
            return;
        };
        let rc = win.get_installs();
        if let Some(model) = rc
            .as_any()
            .downcast_ref::<slint::VecModel<super::InstallItem>>()
        {
            let i = idx as usize;
            if i < model.row_count() {
                if let Some(mut item) = model.row_data(i) {
                    item.selected = !item.selected;
                    model.set_row_data(i, item);
                }
            }
        }
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
        sync_ui(&win, &app);
        if just_finished {
            let ok = !app.had_error;
            let text: slint::SharedString = if ok {
                "License deployed and hosts configured.".into()
            } else {
                app.first_error.clone().into()
            };
            win.set_result_ok(if ok { 1 } else { 0 });
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
        let selected = {
            let app = s.borrow();
            if app.running {
                return;
            }
            let rc = win.get_installs();
            let model = rc
                .as_any()
                .downcast_ref::<slint::VecModel<super::InstallItem>>();
            let mut sel = Vec::new();
            if let Some(m) = model {
                for i in 0..m.row_count() {
                    if m.row_data(i).is_some_and(|item| item.selected) {
                        if let Some(inst) = app.installs.get(i) {
                            sel.push(inst.clone());
                        }
                    }
                }
            }
            sel
        };
        if selected.is_empty() {
            return;
        }

        let (tx, rx) = mpsc::channel();
        {
            let mut app = s.borrow_mut();
            app.running = true;
            app.done = false;
            app.had_error = false;
            app.first_error.clear();
            app.rx = Some(rx);
        }
        sync_ui(&win, &s.borrow());

        thread::spawn(move || {
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
        });
    });
}
