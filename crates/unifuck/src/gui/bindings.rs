use crate::gui::app::{sync_ui, AppState};
use crate::gui::jobs;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

fn group_idx_from_flat(idx: usize, state: &AppState) -> Option<usize> {
    let mut cursor = 0usize;
    for (gi, group) in state.groups.iter().enumerate() {
        if idx == cursor {
            return Some(gi);
        }
        cursor += 1 + group.editors.len();
    }
    None
}

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

            let Some(group_idx) = group_idx_from_flat(idx as usize, &app) else {
                return;
            };

            let row_patched = *app.group_patched.get(group_idx).unwrap_or(&false);
            let cur_sel = *app.group_selected.get(group_idx).unwrap_or(&false);

            if cur_sel {
                if let Some(v) = app.group_selected.get_mut(group_idx) {
                    *v = false;
                }
                if !app.group_selected.iter().any(|s| *s) {
                    app.selection_mode = None;
                }
            } else {
                match app.selection_mode {
                    None => app.selection_mode = Some(row_patched),
                    Some(m) if m == row_patched => {}
                    _ => return,
                }
                if let Some(v) = app.group_selected.get_mut(group_idx) {
                    *v = true;
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
            app.refresh_patched();
            app.selection_mode = None;
            for v in app.group_selected.iter_mut() {
                *v = false;
            }
        }
        sync_ui(&win, &app);
        if just_finished {
            let total = app.success_count + app.error_count;
            let ok = app.error_count == 0;
            let partial = app.success_count > 0 && app.error_count > 0;
            let is_unpatch = app.last_op_unpatch;

            let text: slint::SharedString = if ok {
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
                let summary = format!(
                    "{} selected, {} succeeded, {} failed",
                    total, app.success_count, app.error_count
                );
                let err_info = truncate_error(&app.first_error, 80);
                format!("{}\n{}", summary, err_info).into()
            } else {
                let summary = format!(
                    "{} selected, 0 succeeded, {} failed",
                    total, app.error_count
                );
                let err_info = truncate_error(&app.first_error, 80);
                format!("{}\n{}", summary, err_info).into()
            };

            win.set_success_code(if ok { 1 } else { 0 });
            win.set_operation_type(if is_unpatch { 1 } else { 0 });
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

    let s_clone = state.clone();
    let w = window.as_weak();
    window.on_activate(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        let started = {
            let mut app = s_clone.borrow_mut();
            jobs::start_selected(&mut app)
        };
        if started {
            sync_ui(&win, &s_clone.borrow());
        }
    });

    let s_clone = state.clone();
    let w = window.as_weak();
    window.on_add_hub_folder(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        let path: String = pick_folder_dialog();
        if path.is_empty() {
            return;
        }
        {
            let mut app = s_clone.borrow_mut();
            app.add_custom_hub_path(path);
        }
        sync_ui(&win, &s_clone.borrow());
    });
}

fn truncate_error(err: &str, max_chars: usize) -> String {
    if err.len() <= max_chars {
        err.to_string()
    } else {
        format!("{}...", &err[..max_chars])
    }
}

fn pick_folder_dialog() -> String {
    rfd::FileDialog::new()
        .pick_folder()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}
