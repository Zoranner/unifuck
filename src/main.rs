#![windows_subsystem = "windows"]

mod c14n;
mod core;
mod discovery;
mod gui;
mod hub;

use gui::app::{sync_ui, AppState};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;

fn main() -> anyhow::Result<()> {
    let window = gui::AppWindow::new()?;
    let state = Rc::new(RefCell::new(AppState::new()));

    window.set_app_version(env!("CARGO_PKG_VERSION").into());
    sync_ui(&window, &state.borrow());
    gui::bindings::connect(&window, state.clone());

    // 窗口居中 (Windows API)
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN,
        };
        unsafe {
            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let win_w = 440;
            let win_h = 320;
            let x = (screen_w - win_w) / 2;
            let y = (screen_h - win_h) / 2;
            window
                .window()
                .set_position(slint::PhysicalPosition::new(x, y));
        }
    }

    // 关闭按钮
    window.on_close_window(move || {
        let _ = slint::quit_event_loop();
    });

    // 最小化按钮
    let w = window.as_weak();
    window.on_minimize_window(move || {
        let Some(win) = w.upgrade() else {
            return;
        };
        win.window().set_minimized(true);
    });

    // 拖动窗口 (按下记录起点光标 + 窗口位置, 移动时按 delta 调整 set_position)
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

        #[derive(Clone, Copy)]
        struct DragInit {
            mouse_x: i32,
            mouse_y: i32,
            win_x: i32,
            win_y: i32,
        }

        let drag_state: Rc<RefCell<Option<DragInit>>> = Rc::new(RefCell::new(None));

        let ds = drag_state.clone();
        let w = window.as_weak();
        window.on_start_window_drag(move || {
            let Some(win) = w.upgrade() else {
                return;
            };
            let pos = win.window().position();
            let mut cursor = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut cursor) };
            *ds.borrow_mut() = Some(DragInit {
                mouse_x: cursor.x,
                mouse_y: cursor.y,
                win_x: pos.x,
                win_y: pos.y,
            });
        });

        let ds = drag_state.clone();
        let w = window.as_weak();
        window.on_drag_window(move || {
            let Some(win) = w.upgrade() else {
                return;
            };
            let Some(init) = *ds.borrow() else {
                return;
            };
            let mut cursor = POINT { x: 0, y: 0 };
            unsafe { GetCursorPos(&mut cursor) };
            let new_x = init.win_x + (cursor.x - init.mouse_x);
            let new_y = init.win_y + (cursor.y - init.mouse_y);
            win.window()
                .set_position(slint::PhysicalPosition::new(new_x, new_y));
        });
    }

    window.run()?;
    Ok(())
}
