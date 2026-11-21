//! Desktop notifications for the background helper: the notification daemon on Linux,
//! toast notifications on Windows.

use setu_core::watch::Notice;

pub fn show(n: &Notice) {
    let mut note = notify_rust::Notification::new();
    note.summary(&n.title).body(&n.body).appname("Setu");
    #[cfg(windows)]
    note.app_id("Setu");
    if let Err(e) = note.show() {
        eprintln!("setu: could not show a notification: {e}");
    }
}
