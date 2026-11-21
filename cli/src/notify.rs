//! Desktop notifications for the background helper: the notification daemon on Linux,
//! toast notifications on Windows.

use setu_core::watch::Notice;

/// Windows shows toasts only for an app ID a Start-menu shortcut carries; the installer sets it.
#[cfg(windows)]
const APP_ID: &str = "dev.setu.app";

pub fn show(n: &Notice) {
    let mut note = notify_rust::Notification::new();
    note.summary(&n.title).body(&n.body).appname("Setu");
    #[cfg(windows)]
    note.app_id(APP_ID);
    if let Err(e) = note.show() {
        eprintln!("setu: could not show a notification: {e}");
    }
}
