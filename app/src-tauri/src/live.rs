//! Live copy progress for the window and the tray. A thread reads the engine every two seconds
//! while something is copying (every six when not), keeps the latest snapshot for the window,
//! sends it as a `progress` event and puts a one-line summary on the tray icon.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use setu_core::control::now_secs;
use setu_core::local::{config_dir, LocalConfig};
use setu_core::progress::{self, Meter, Snapshot};
use setu_core::syncthing::Client;
use setu_core::watch;
use tauri::{AppHandle, Emitter};

const BUSY: Duration = Duration::from_secs(2);
const IDLE: Duration = Duration::from_secs(6);
const RETRY: Duration = Duration::from_secs(10);

/// The latest snapshot, shared with the `progress` command.
#[derive(Default, Clone)]
pub struct Live(pub Arc<Mutex<Option<Snapshot>>>);

pub fn start(app: AppHandle, live: Live) {
    std::thread::spawn(move || {
        let mut meter = Meter::default();
        let mut conn: Option<(Client, String)> = None;
        loop {
            let dir = config_dir();
            let cfg = LocalConfig::load(&dir).ok();
            if conn.is_none() {
                // Until the engine runs (moving an old setup, waiting for Tailscale) this fails.
                conn = Client::connect().ok().and_then(|client| client.my_id().ok().map(|me| (client, me)));
            }
            let Some((client, me)) = &conn else {
                crate::tray::show_status(&app, None);
                std::thread::sleep(RETRY);
                continue;
            };
            let wait = match progress::read(client, me, watch::now_ms()) {
                Ok(r) => {
                    let w = watch::read_state(&dir);
                    let helper_alive = w.alive_at + 120 >= now_secs();
                    let hold = (helper_alive && !w.gate.open) || cfg.as_ref().is_some_and(|c| c.user_paused(now_secs()));
                    let (mut snap, _) = meter.observe(r, hold);
                    if let Some(c) = &cfg {
                        progress::mark_pauses(&mut snap, &w, c);
                    }
                    crate::tray::show_status(&app, Some(&snap));
                    let _ = app.emit("progress", &snap);
                    let busy = snap.active();
                    *live.0.lock().unwrap_or_else(|e| e.into_inner()) = Some(snap);
                    if busy { BUSY } else { IDLE }
                }
                Err(_) => {
                    // The engine stopped: connect again next time.
                    conn = None;
                    RETRY
                }
            };
            std::thread::sleep(wait);
        }
    });
}
