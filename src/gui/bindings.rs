use crate::gui::app::{sync_ui, AppState};
use crate::gui::jobs;
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

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
                let err_info = truncate_error(&app.first_error, 80);
                format!("{}\n{}", summary, err_info).into()
            } else {
                // 全部失败
                let summary = format!(
                    "{} selected, 0 succeeded, {} failed",
                    total, app.error_count
                );
                let err_info = truncate_error(&app.first_error, 80);
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
        let started = {
            let mut app = s.borrow_mut();
            jobs::start_selected(&mut app)
        };
        if started {
            sync_ui(&win, &s.borrow());
        }
    });
}

fn truncate_error(err: &str, max_chars: usize) -> String {
    if err.len() <= max_chars {
        err.to_string()
    } else {
        format!("{}...", &err[..max_chars])
    }
}
