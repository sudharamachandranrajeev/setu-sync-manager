//! Desktop notifications for the background helper: the notification daemon on Linux,
//! toast notifications on Windows. A notice with a progress bar updates one notification in
//! place for as long as its round of copying runs.

use setu_core::watch::{Bar, Notice};

/// Windows shows toasts only for an app ID a Start-menu shortcut carries; the installer sets it.
#[cfg(windows)]
const APP_ID: &str = "dev.setu.app";

pub fn show(n: &Notice) {
    match &n.bar {
        Some(bar) => progress(n, bar),
        None => plain(n),
    }
}

fn plain(n: &Notice) {
    let mut note = notify_rust::Notification::new();
    note.summary(&n.title).body(&n.body).appname("Setu");
    #[cfg(windows)]
    note.app_id(APP_ID);
    if let Err(e) = note.show() {
        eprintln!("setu: could not show a notification: {e}");
    }
}

/// A toast with a real progress bar, updated through its tag. If the person dismisses it, it
/// stays away until the round ends; then a plain "done" notice says how it went.
#[cfg(windows)]
fn progress(n: &Notice, bar: &Bar) {
    use std::collections::BTreeSet;
    use std::sync::Mutex;
    use tauri_winrt_notification::{NotificationUpdateResult, Progress, Toast};

    static SHOWN: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    let mut shown = SHOWN.lock().unwrap_or_else(|e| e.into_inner());
    let p = Progress {
        tag: bar.tag.clone(),
        title: n.title.clone(),
        status: bar.status.clone(),
        value: f32::from(bar.permille) / 1000.0,
        value_string: bar.value.clone(),
    };
    if shown.contains(&bar.tag) {
        let updated = matches!(Toast::new(APP_ID).set_progress(&p), Ok(r) if r == NotificationUpdateResult::Succeeded);
        if bar.done {
            shown.remove(&bar.tag);
            if !updated {
                plain(n);
            }
        }
        return;
    }
    if bar.done {
        plain(n);
        return;
    }
    match Toast::new(APP_ID).title(&n.title).progress(&p).sound(None).show() {
        Ok(()) => {
            shown.insert(bar.tag.clone());
        }
        Err(e) => eprintln!("setu: could not show a progress notification: {e}"),
    }
}

/// One notification per tag, replaced in place (its id is kept). The `value` hint draws a bar
/// where the desktop supports it (KDE, and others); the body always carries the numbers.
/// Updates come at most every 15 s, so the desktop is not flooded.
#[cfg(not(windows))]
fn progress(n: &Notice, bar: &Bar) {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const EVERY: Duration = Duration::from_secs(15);
    static IDS: Mutex<BTreeMap<String, (u32, Instant)>> = Mutex::new(BTreeMap::new());
    let mut ids = IDS.lock().unwrap_or_else(|e| e.into_inner());
    let known = ids.get(&bar.tag).copied();
    if !bar.done && known.is_some_and(|(_, at)| at.elapsed() < EVERY) {
        return;
    }
    let mut note = notify_rust::Notification::new();
    note.summary(&n.title)
        .body(&n.body)
        .appname("Setu")
        .hint(notify_rust::Hint::CustomInt("value".into(), i32::from(bar.permille / 10)))
        .hint(notify_rust::Hint::SuppressSound(!bar.done))
        .urgency(notify_rust::Urgency::Low);
    if let Some((id, _)) = known {
        note.id(id);
    }
    match note.show() {
        Ok(h) if !bar.done => {
            ids.insert(bar.tag.clone(), (h.id(), Instant::now()));
        }
        Ok(_) => {
            ids.remove(&bar.tag);
        }
        Err(e) => eprintln!("setu: could not show a progress notification: {e}"),
    }
}
