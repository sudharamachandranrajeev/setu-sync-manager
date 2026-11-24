//! Tray icon: Setu's state at a glance and a quick way back in. Closing the window only hides
//! it: Setu holds the sync engine, so it keeps running until "Quit Setu". Where a Linux desktop
//! shows no tray, starting Setu again from the menu brings the window back.

use setu_core::control::now_secs;
use setu_core::local::config_dir;
use setu_core::progress::{bytes, duration, Flow, Snapshot};
use setu_core::watch;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, WindowEvent};

const TRAY_ID: &str = "setu";

/// One line about a direction that is copying, e.g. "↓ 45% · 12 MB/s · 3 min left".
fn flow_line(arrow: &str, f: &Flow, paused: bool) -> String {
    let left = match (paused, f.eta_secs) {
        (true, _) => "paused".to_string(),
        (false, Some(t)) => format!("{}/s · {} left", bytes(f.speed), duration(t)),
        (false, None) => format!("{}/s", bytes(f.speed)),
    };
    format!("{arrow} {:.0}% of {} · {left}", f.percent.floor(), bytes(f.total_bytes))
}

/// The tooltip: copy progress while copying, otherwise the helper's state.
pub fn status_line(snap: Option<&Snapshot>) -> String {
    if let Some(s) = snap.filter(|s| s.active()) {
        let mut parts = Vec::new();
        if s.down.active {
            parts.push(flow_line("↓", &s.down, s.paused));
        }
        if s.up.active {
            parts.push(flow_line("↑", &s.up, s.paused));
        }
        return format!("Setu: {}", parts.join("  "));
    }
    let w = watch::read_state(&config_dir());
    if w.alive_at + 120 < now_secs() {
        return "Setu: starting".into();
    }
    if !w.holds.is_empty() {
        return format!("Setu: {} folder(s) paused after many deletions", w.holds.len());
    }
    if !w.gate.open {
        return format!("Setu: on hold ({})", w.gate.reason.as_deref().unwrap_or("network"));
    }
    "Setu: all copied".into()
}

pub fn show_status(app: &AppHandle, snap: Option<&Snapshot>) {
    if let Some(t) = app.tray_by_id(TRAY_ID) {
        let _ = t.set_tooltip(Some(status_line(snap)));
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &App) -> tauri::Result<()> {
    let main = app.get_webview_window("main").expect("main window is declared in tauri.conf.json");
    let w = main.clone();
    main.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = w.hide();
        }
    });

    let open = MenuItem::with_id(app, "open", "Open Setu", true, None::<&str>)?;
    let progress = MenuItem::with_id(app, "progress", "Progress…", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause syncing for 1 hour", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume syncing", true, None::<&str>)?;
    let conflicts = MenuItem::with_id(app, "conflicts", "Conflicts…", true, None::<&str>)?;
    let versions = MenuItem::with_id(app, "versions", "Old versions…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Setu", true, None::<&str>)?;
    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(status_line(None))
        .menu(&Menu::with_items(app, &[&open, &progress, &sep, &pause, &resume, &sep2, &conflicts, &versions, &quit])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "progress" => {
                show_main(app);
                let _ = app.emit("open-progress", ());
            }
            what @ ("pause" | "resume") => {
                let until = (what == "pause").then(|| now_secs() + 3600);
                if let Err(e) = crate::pause_until(until) {
                    eprintln!("setu: {e}");
                }
                // The window shows the new state.
                let _ = app.emit("view-changed", ());
            }
            tab @ ("conflicts" | "versions") => {
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
    Ok(())
}
