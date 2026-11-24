//! Setu hosts the sync engine (Syncthing's code, built in) and the background helper in its own
//! process. There is no separate Syncthing program, no local web port and no password file.
//!
//! Start-up, on a thread so the window opens at once:
//! 1. If an old setup exists (Syncthing running on its own), wait until the person agrees to
//!    move it (`move_engine`). Starting the engine first would give this computer a new
//!    identity, and the other computers would no longer know it.
//! 2. Wait for a Tailscale address: the engine only ever listens there.
//! 3. Move the old setup if agreed: stop the old Syncthing, copy its identity, config and index.
//! 4. Start the engine. After a move, check it has the old identity and every Setu folder, and
//!    only then remove the old start-up pieces. If anything fails, the copy is removed and the
//!    old Syncthing started again.
//! 5. Run the helper (network gate, delete guard, Brahma rounds, notices) and follow changes of
//!    the Tailscale address.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use setu_core::local::{LocalConfig, config_dir, engine_home};
use setu_core::migrate::{self, OldSetup};
use setu_core::syncthing::{Client, Transport, set_local};
use setu_core::{Error, mesh, watch};
use tauri::{AppHandle, Emitter};

/// What the engine is doing, for the window (`engine` command, `engine-changed` event).
#[derive(Clone, Serialize, Default, Debug)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Status {
    #[default]
    Starting,
    /// No Tailscale address on this computer yet.
    Tailnet,
    /// An old setup can be moved into Setu; waits for the person to agree.
    #[serde(rename_all = "camelCase")]
    Move { steps: Vec<String>, other_folders: Vec<String> },
    Moving,
    /// Running. `left`: old start-up pieces that could not be removed after a move.
    Running { left: Vec<String> },
    Failed { message: String },
}

#[derive(Default)]
pub struct Host {
    status: Mutex<Status>,
    agree: Mutex<Option<Sender<()>>>,
}

impl Host {
    pub fn status(&self) -> Status {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The person agreed to move the old setup.
    pub fn agree(&self) -> Result<(), String> {
        let tx = self.agree.lock().unwrap_or_else(|e| e.into_inner()).take();
        tx.ok_or("nothing to move")?.send(()).map_err(|_| "Setu is no longer waiting".to_string())
    }
}

/// The engine in this process.
struct Engine;

impl Transport for Engine {
    fn call(&self, method: &str, args: &Value) -> Result<Value, Error> {
        setu_engine::call(method, args).map_err(Error::Syncthing)
    }
}

/// This computer's name, given to the engine the first time it starts.
fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Setu".into())
}

const TAILNET_WAIT: Duration = Duration::from_secs(10);
const FOLLOW_EVERY: Duration = Duration::from_secs(60);
const HELPER_RETRY: Duration = Duration::from_secs(30);

pub fn start(app: AppHandle, host: Arc<Host>) {
    // At once, so nothing in this process looks for the engine over the pipe (which would reach
    // this same process). Calls fail with "not running" until the engine has started.
    set_local(Arc::new(Engine));
    std::thread::spawn(move || {
        let set = |s: Status| {
            *host.status.lock().unwrap_or_else(|e| e.into_inner()) = s.clone();
            let _ = app.emit("engine-changed", &s);
        };
        let home = engine_home();
        let old_home = LocalConfig::load(&config_dir()).ok().and_then(|c| c.syncthing_home);
        let mut old = migrate::find(&home, old_home.as_deref());

        if let Some(o) = &old {
            let (tx, rx) = channel();
            *host.agree.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
            set(Status::Move { steps: o.steps(), other_folders: o.other_folders.clone() });
            // Started in the background at sign-in: the person still has to see this.
            crate::tray::show_main(&app);
            if rx.recv().is_err() {
                return;
            }
            set(Status::Moving);
        }

        let ips = loop {
            let ips = mesh::tailnet_ips();
            if !ips.is_empty() {
                break ips;
            }
            if !matches!(host.status(), Status::Tailnet) {
                set(Status::Tailnet);
            }
            std::thread::sleep(TAILNET_WAIT);
        };
        if old.is_some() {
            set(Status::Moving);
        }

        if let Some(o) = old.as_mut()
            && let Err(e) = o.stop_old().and_then(|()| o.copy_into(&home)) {
                undo_move(o, &home, false);
                set(Status::Failed { message: format!("Moving the old setup did not work, so it is back as it was: {e}. Restart Setu to try again.") });
                return;
            }

        let args = setu_engine::StartArgs { home: home.clone(), listen: mesh::addresses(&ips), name: computer_name() };
        if let Err(e) = setu_engine::start(&args) {
            if let Some(o) = &old {
                undo_move(o, &home, false);
            }
            set(Status::Failed { message: format!("The sync engine did not start: {e}") });
            return;
        }
        let client = Client::connect().expect("the engine is registered");

        let mut left = Vec::new();
        if let Some(o) = &old {
            if let Err(e) = o.check(&client, &home) {
                undo_move(o, &home, true);
                set(Status::Failed { message: format!("The moved setup did not check out, so the old one is back as it was: {e}. Restart Setu to try again.") });
                return;
            }
            left = o.clean_up();
        }
        set(Status::Running { left });

        let helper = client.clone();
        std::thread::spawn(move || {
            loop {
                let dir = config_dir();
                // Before `init` there is no local.json yet: try again later.
                if let Err(e) = watch::run(&dir, || LocalConfig::load(&config_dir()), &helper, &crate::notify::show) {
                    eprintln!("setu: helper: {e}");
                }
                std::thread::sleep(HELPER_RETRY);
            }
        });

        // A new Tailscale address (another network, Tailscale restarted): listen there instead.
        loop {
            std::thread::sleep(FOLLOW_EVERY);
            let ips = mesh::tailnet_ips();
            if !ips.is_empty()
                && let Err(e) = mesh::harden(&client, &ips) {
                    eprintln!("setu: listen addresses: {e}");
                }
        }
    });
}

/// Put the old setup back: stop the engine if it started, remove the copy (it holds nothing
/// that is not in the old home) and start the old Syncthing again.
fn undo_move(old: &OldSetup, home: &Path, engine_started: bool) {
    if engine_started {
        let _ = setu_engine::stop();
    }
    let _ = std::fs::remove_dir_all(home);
    old.restore();
}

/// The program to start at sign-in: the AppImage itself when running from one.
fn exe() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(p));
    }
    std::env::current_exe().map_err(|e| e.to_string())
}

/// "Start Setu when I sign in": `HKCU\…\Run\Setu` on Windows, an XDG autostart entry on Linux.
pub mod autostart {
    use super::exe;

    #[cfg(windows)]
    const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

    #[cfg(windows)]
    fn reg(args: &[&str]) -> bool {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().is_ok_and(|o| o.status.success())
    }

    #[cfg(windows)]
    pub fn enabled() -> bool {
        reg(&["query", KEY, "/v", "Setu"])
    }

    #[cfg(windows)]
    pub fn set(on: bool) -> Result<(), String> {
        let ok = if on {
            let cmd = format!("\"{}\" --background", exe()?.display());
            reg(&["add", KEY, "/v", "Setu", "/t", "REG_SZ", "/d", &cmd, "/f"])
        } else {
            !enabled() || reg(&["delete", KEY, "/v", "Setu", "/f"])
        };
        if ok { Ok(()) } else { Err("could not change the sign-in entry".into()) }
    }

    #[cfg(not(windows))]
    fn entry() -> std::path::PathBuf {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).unwrap_or(home.join(".config")).join("autostart").join("setu.desktop")
    }

    #[cfg(not(windows))]
    pub fn enabled() -> bool {
        entry().is_file()
    }

    #[cfg(not(windows))]
    pub fn set(on: bool) -> Result<(), String> {
        let p = entry();
        if !on {
            return match std::fs::remove_file(&p) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let exe = exe()?.display().to_string().replace('"', "\\\"");
        let text = format!(
            "[Desktop Entry]\nType=Application\nName=Setu\nComment=Keeps your folders in sync between your computers\nExec=\"{exe}\" --background\nIcon=setu\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"
        );
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, text).map_err(|e| e.to_string())
    }
}
