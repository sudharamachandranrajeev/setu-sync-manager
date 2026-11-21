//! Tray icon: Setu's state at a glance and a quick way back in. On Windows, closing the window
//! keeps Setu in the tray; on Linux set SETU_TRAY=1 for that (many Linux bars have no tray).

use std::time::Duration;

use setu_core::control::now_secs;
use setu_core::local::config_dir;
use setu_core::watch;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, WindowEvent};

const TRAY_ID: &str = "setu";
const REFRESH: Duration = Duration::from_secs(30);

fn keep_in_tray() -> bool {
    cfg!(windows) || std::env::var_os("SETU_TRAY").is_some()
}

/// One line for the tooltip, from the helper's state file.
pub fn status_line() -> String {
    let w = watch::read_state(&config_dir());
    if w.alive_at + 120 < now_secs() {
        return "Setu: background helper not running".into();
    }
    if !w.holds.is_empty() {
        return format!("Setu: {} folder(s) paused after many deletions", w.holds.len());
    }
    if !w.gate.open {
        return format!("Setu: on hold ({})", w.gate.reason.as_deref().unwrap_or("network"));
    }
    "Setu: running".into()
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &App) -> tauri::Result<()> {
    let main = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
    if keep_in_tray() {
        let w = main.clone();
        main.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = w.hide();
            }
        });
    }

    let open = MenuItem::with_id(app, "open", "Open Setu", true, None::<&str>)?;
    let conflicts = MenuItem::with_id(app, "conflicts", "Conflicts…", true, None::<&str>)?;
    let versions = MenuItem::with_id(app, "versions", "Old versions…", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Progress…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Setu", true, None::<&str>)?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(status_line())
        .menu(&Menu::with_items(app, &[&open, &conflicts, &versions, &check, &sep, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            tab @ ("conflicts" | "versions" | "check") => {
                show_main(app);
                let _ = app.emit("open-trust", tab);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    let app = app.handle().clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(REFRESH);
        if let Some(t) = app.tray_by_id(TRAY_ID) {
            let _ = t.set_tooltip(Some(status_line()));
        }
    });
    Ok(())
}
