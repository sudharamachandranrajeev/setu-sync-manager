//! `setu watch`: the small background helper that keeps two promises while Syncthing runs.
//!
//! 1. **Network gate.** On a metered network, or when the link to the other computer goes
//!    through a relay, Setu's folders are paused here (unless this computer allows it). They are
//!    resumed when the network is fine again. Only folders the gate paused are resumed.
//! 2. **Delete guard.** If more than [`DELETE_LIMIT`] files are deleted in one Setu folder on
//!    this computer within [`DELETE_WINDOW`] seconds because the other computer deleted them,
//!    that folder is paused here and Setu shows it. Deleted files are in the folder's
//!    `.stversions` for 30 days.
//!
//! Its state lives in `watch.json` next to Setu's local config, where the app reads it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::compile::FOLDER_PREFIX;
use crate::control::now_secs;
use crate::local::{write_atomic, LocalConfig};
use crate::syncthing::Client;
use crate::Error;

pub const DELETE_LIMIT: usize = 50;
pub const DELETE_WINDOW: u64 = 120;
const GATE_EVERY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    pub open: bool,
    pub metered: bool,
    pub relayed: bool,
    /// Closed because the person paused syncing in Setu.
    #[serde(default)]
    pub user_paused: bool,
    /// Plain words for the app, e.g. "metered network".
    pub reason: Option<String>,
    pub checked_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub folder: String,
    pub deletes: usize,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchState {
    pub gate: Gate,
    /// Folders the gate paused, to resume when it opens.
    #[serde(default)]
    pub gate_paused: BTreeSet<String>,
    /// Folders the delete guard paused, until someone resumes them in Setu.
    #[serde(default)]
    pub holds: Vec<Hold>,
    /// When the watcher last ran, so the app can tell it is alive.
    #[serde(default)]
    pub alive_at: u64,
}

/// Milliseconds since 1970.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn state_path(dir: &Path) -> PathBuf {
    dir.join("watch.json")
}

pub fn read_state(dir: &Path) -> WatchState {
    std::fs::read_to_string(state_path(dir)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_state(dir: &Path, s: &WatchState) -> Result<(), Error> {
    write_atomic(&state_path(dir), &(serde_json::to_string_pretty(s).expect("serialise") + "\n"))
}

/// Resume a folder the delete guard paused (the app's "Resume" button).
pub fn release_hold(dir: &Path, client: &Client, folder: &str) -> Result<(), Error> {
    let mut s = read_state(dir);
    s.holds.retain(|h| h.folder != folder);
    let by_hand = LocalConfig::load(dir).is_ok_and(|c| c.paused_folders.contains(folder));
    if !s.gate_paused.contains(folder) && s.gate.open && !by_hand {
        client.patch_folder(folder, &json!({ "paused": false }))?;
    }
    write_state(dir, &s)
}

/// Pause or resume one Setu folder on this computer by hand. The choice is kept in
/// `local.json`, so the network gate never resumes a folder the person paused. A folder resumed
/// while the whole computer is on hold starts again when the hold ends.
pub fn set_folder_paused(dir: &Path, client: &Client, folder: &str, pause: bool) -> Result<(), Error> {
    if !folder.starts_with(FOLDER_PREFIX) {
        return Err(Error::Rules(format!("{folder} is not a Setu folder")));
    }
    let mut cfg = LocalConfig::load(dir)?;
    if pause {
        cfg.paused_folders.insert(folder.to_string());
    } else {
        cfg.paused_folders.remove(folder);
    }
    cfg.save(dir)?;
    let s = read_state(dir);
    let held = s.holds.iter().any(|h| h.folder == folder);
    if pause {
        client.patch_folder(folder, &json!({ "paused": true }))?;
    } else if s.gate.open && !held {
        client.patch_folder(folder, &json!({ "paused": false }))?;
    }
    Ok(())
}

/// A command that opens no console window on Windows.
fn quiet(program: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// Is this computer's current network metered? `None` when it cannot tell.
pub fn metered() -> Option<bool> {
    if cfg!(windows) {
        let out = quiet("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[void][Windows.Networking.Connectivity.NetworkInformation,Windows.Networking.Connectivity,ContentType=WindowsRuntime];\
                 $p=[Windows.Networking.Connectivity.NetworkInformation]::GetInternetConnectionProfile();\
                 if($p){$p.GetConnectionCost().NetworkCostType}",
            ])
            .output()
            .ok()?;
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return match t.as_str() {
            "Unrestricted" => Some(false),
            "Fixed" | "Variable" => Some(true),
            _ => None,
        };
    }
    // NetworkManager's overall answer: 1 yes, 3 guessed yes, 2 no, 4 guessed no, 0 unknown.
    let o = quiet("busctl")
        .args(["get-property", "org.freedesktop.NetworkManager", "/org/freedesktop/NetworkManager", "org.freedesktop.NetworkManager", "Metered"])
        .output()
        .ok()?;
    match String::from_utf8_lossy(&o.stdout).trim() {
        "u 1" | "u 3" => Some(true),
        "u 2" | "u 4" => Some(false),
        _ => None,
    }
}

/// Does the link to the computer at `ip` go through a Tailscale relay? `None` if unknown.
pub fn relayed(ip: &str) -> Option<bool> {
    let tailscale = if cfg!(windows) { r"C:\Program Files\Tailscale\tailscale.exe" } else { "tailscale" };
    let out = quiet(tailscale).args(["status", "--json"]).output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let peer = v["Peer"].as_object()?.values().find(|p| p["TailscaleIPs"].as_array().is_some_and(|a| a.iter().any(|x| x == ip)))?;
    let direct = peer["CurAddr"].as_str().is_some_and(|a| !a.is_empty());
    let relay = peer["Relay"].as_str().is_some_and(|r| !r.is_empty());
    if direct {
        Some(false)
    } else if relay && peer["Online"].as_bool() == Some(true) {
        Some(true)
    } else {
        None
    }
}

/// The other computer's address as Syncthing sees it (`100.x.y.z`), if connected.
fn other_ip(client: &Client, me: &str) -> Option<String> {
    let c = client.connections().ok()?;
    c["connections"].as_object()?.iter().filter(|(id, _)| *id != me).find_map(|(_, v)| {
        let a = v["address"].as_str()?;
        let host = a.rsplit_once(':').map(|(h, _)| h).unwrap_or(a);
        Some(host.trim_start_matches('[').trim_end_matches(']').to_string())
    })
}

pub fn check_gate(cfg: &LocalConfig, client: &Client, me: &str) -> Gate {
    let m = metered().unwrap_or(false);
    let r = other_ip(client, me).and_then(|ip| relayed(&ip)).unwrap_or(false);
    let mut reasons = Vec::new();
    let user_paused = cfg.user_paused(now_secs());
    if user_paused {
        reasons.push("you paused syncing");
    }
    if m && !cfg.allow_metered {
        reasons.push("metered network");
    }
    if r && !cfg.allow_relay {
        reasons.push("link goes through a relay");
    }
    Gate {
        open: reasons.is_empty(),
        metered: m,
        relayed: r,
        user_paused,
        reason: (!reasons.is_empty()).then(|| reasons.join(", ")),
        checked_at: now_secs(),
    }
}

fn setu_folders(client: &Client) -> Result<Vec<(String, bool)>, Error> {
    Ok(client
        .folders()?
        .iter()
        .filter_map(|f| {
            let id = f["id"].as_str()?;
            id.starts_with(FOLDER_PREFIX).then(|| (id.to_string(), f["paused"].as_bool().unwrap_or(false)))
        })
        .collect())
}

/// Pause or resume folders to match the gate. `by_hand`: folders the person paused one by one,
/// which stay paused either way.
pub fn apply_gate(state: &mut WatchState, gate: Gate, client: &Client, by_hand: &BTreeSet<String>) -> Result<(), Error> {
    let folders = setu_folders(client)?;
    let held: BTreeSet<&str> = state.holds.iter().map(|h| h.folder.as_str()).collect();
    if gate.open {
        for id in std::mem::take(&mut state.gate_paused) {
            if !held.contains(id.as_str()) && !by_hand.contains(&id) && folders.iter().any(|(f, _)| *f == id) {
                client.patch_folder(&id, &json!({ "paused": false }))?;
            }
        }
    } else {
        for (id, paused) in folders {
            if held.contains(id.as_str()) || by_hand.contains(&id) {
                state.gate_paused.remove(&id);
                continue;
            }
            if !paused {
                client.patch_folder(&id, &json!({ "paused": true }))?;
            }
            // Also a folder resumed by hand while on hold: it starts again when the hold ends.
            state.gate_paused.insert(id);
        }
    }
    state.gate = gate;
    Ok(())
}

/// Check the network and the person's pause now and pause or resume Setu's folders to match.
/// For when the helper is not running; while it runs, the helper does this itself.
pub fn gate_now(dir: &Path, cfg: &LocalConfig, client: &Client) -> Result<Gate, Error> {
    let me = client.my_id()?;
    let mut state = read_state(dir);
    let gate = check_gate(cfg, client, &me);
    apply_gate(&mut state, gate, client, &cfg.paused_folders)?;
    write_state(dir, &state)?;
    Ok(state.gate)
}

/// Count deletions per folder in a sliding window.
#[derive(Default)]
pub struct DeleteCounter {
    seen: BTreeMap<String, VecDeque<u64>>,
}

impl DeleteCounter {
    /// Record one deletion at `t` (seconds). Returns how many in the window, including this one.
    pub fn record(&mut self, folder: &str, t: u64) -> usize {
        let q = self.seen.entry(folder.to_string()).or_default();
        q.push_back(t);
        while q.front().is_some_and(|&x| x + DELETE_WINDOW < t) {
            q.pop_front();
        }
        q.len()
    }

    pub fn reset(&mut self, folder: &str) {
        self.seen.remove(folder);
    }
}

/// Handle one Syncthing event. Returns a folder to hold, if this deletion crossed the limit.
pub fn on_event(ev: &Value, counter: &mut DeleteCounter, t: u64) -> Option<(String, usize)> {
    if ev["type"] != "ItemFinished" {
        return None;
    }
    let d = &ev["data"];
    let folder = d["folder"].as_str()?;
    if !folder.starts_with(FOLDER_PREFIX) || d["action"] != "delete" || !d["error"].is_null() {
        return None;
    }
    let n = counter.record(folder, t);
    (n > DELETE_LIMIT).then(|| (folder.to_string(), n))
}

/// Conflict copies an event reports in a Setu folder: received from the other computer
/// (`ItemFinished`) or made here by Syncthing and picked up by a scan (`LocalIndexUpdated`).
pub fn conflict_names(ev: &Value) -> Vec<String> {
    let d = &ev["data"];
    if !d["folder"].as_str().is_some_and(|f| f.starts_with(FOLDER_PREFIX)) {
        return Vec::new();
    }
    let names: Vec<&str> = match ev["type"].as_str() {
        Some("ItemFinished") if d["action"] == "update" && d["error"].is_null() => d["item"].as_str().into_iter().collect(),
        Some("LocalIndexUpdated") => d["filenames"].as_array().into_iter().flatten().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = names.into_iter().filter(|n| crate::trust::original_of(n).is_some()).map(String::from).collect();
    out.sort();
    out.dedup();
    out
}

/// Something worth a desktop notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    /// A progress bar. Notices with the same tag update one notification in place.
    pub bar: Option<Bar>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bar {
    pub tag: String,
    /// 0 to 1000.
    pub permille: u16,
    /// Under the bar, e.g. "12 MB/s · about 3 min left".
    pub status: String,
    /// Beside the bar, e.g. "1.2 GB of 3.4 GB".
    pub value: String,
    /// The last update: the round is over.
    pub done: bool,
}

pub fn notice(title: &str, body: String) -> Notice {
    Notice { title: title.into(), body, bar: None }
}

/// How often the helper reads copy progress, while copying and while not.
const PROGRESS_BUSY: Duration = Duration::from_secs(3);
const PROGRESS_IDLE: Duration = Duration::from_secs(15);

/// Folder errors are repeated by Syncthing on every pull; tell once an hour per folder.
const ERROR_NOTICE_EVERY: u64 = 3600;

/// Run forever: gate every minute, deletions as they happen. `notify` shows a notice to the
/// person at this computer (the helper uses desktop notifications).
pub fn run(dir: &Path, cfg_loader: impl Fn() -> Result<LocalConfig, Error>, client: &Client, notify: &dyn Fn(&Notice)) -> Result<(), Error> {
    let me = client.my_id()?;
    let mut state = read_state(dir);
    let mut counter = DeleteCounter::default();
    let mut since: u64 = client.events(0, 1)?.iter().filter_map(|e| e["id"].as_u64()).max().unwrap_or(0);
    let mut next_gate = Instant::now();
    let mut error_told: std::collections::BTreeMap<String, u64> = Default::default();
    // At start, treat the gate as open: a closed gate is told once, an open one not at all.
    let mut gate_known = false;
    let mut ticker = crate::helper::Ticker::default();
    let mut by_hand: BTreeSet<String> = BTreeSet::new();
    let mut meter = crate::progress::Meter::default();
    let mut announcer = crate::progress::Announcer::default();
    let mut next_progress = Instant::now();
    let mut copying = false;
    loop {
        // The Brahma round: shared rules, pairing, applying, content check and tidy.
        if ticker.due(dir) {
            for n in ticker.run(dir) {
                notify(&n);
            }
        }
        // A pause or resume in Setu, of everything or one folder, takes effect at once, not at
        // the next network check.
        if let Ok(cfg) = cfg_loader()
            && (cfg.user_paused(now_secs()) != state.gate.user_paused || cfg.paused_folders != by_hand)
        {
            next_gate = Instant::now();
        }
        if Instant::now() >= next_gate {
            if let Ok(cfg) = cfg_loader() {
                let was_open = if gate_known { state.gate.open } else { true };
                let was_user = state.gate.user_paused;
                let gate = check_gate(&cfg, client, &me);
                // The person paused or resumed it themselves: no notice for that.
                let quiet = was_user || gate.user_paused;
                by_hand = cfg.paused_folders.clone();
                match apply_gate(&mut state, gate, client, &by_hand) {
                    Ok(()) if quiet => {}
                    Ok(()) if was_open && !state.gate.open => notify(&notice(
                        "Setu: syncing on hold",
                        format!("{}. Copying starts again by itself when the connection is suitable.", state.gate.reason.as_deref().unwrap_or("network")),
                    )),
                    Ok(()) if !was_open && state.gate.open => notify(&notice("Setu: syncing again", "The connection is suitable again.".into())),
                    Ok(()) => {}
                    Err(e) => eprintln!("setu watch: gate: {e}"),
                }
                gate_known = true;
            }
            next_gate = Instant::now() + GATE_EVERY;
            state.alive_at = now_secs();
            write_state(dir, &state)?;
        }
        // Copy progress: a notification that updates itself while a big round of copying runs.
        if Instant::now() >= next_progress {
            match crate::progress::read(client, &me, now_ms()) {
                Ok(r) => {
                    let (snap, finished) = meter.observe(r, !state.gate.open);
                    copying = snap.active();
                    let quiet = cfg_loader().is_ok_and(|c| c.quiet_progress);
                    for n in announcer.notices(&snap, &finished) {
                        if !quiet {
                            notify(&n);
                        }
                    }
                }
                Err(e) => eprintln!("setu watch: progress: {e}"),
            }
            next_progress = Instant::now() + if copying { PROGRESS_BUSY } else { PROGRESS_IDLE };
        }
        // Holds released in the app since the last loop.
        let on_disk = read_state(dir);
        state.holds.retain(|h| on_disk.holds.contains(h));
        // While copying, come back soon for the next progress reading.
        match client.events(since, if copying { PROGRESS_BUSY.as_secs() } else { 20 }) {
            Ok(evs) => {
                let mut conflicts = Vec::new();
                for ev in &evs {
                    since = since.max(ev["id"].as_u64().unwrap_or(since));
                    if let Some((folder, n)) = on_event(ev, &mut counter, now_secs())
                        && !state.holds.iter().any(|h| h.folder == folder)
                    {
                        client.patch_folder(&folder, &json!({ "paused": true }))?;
                        eprintln!("setu watch: {n} deletions in {folder} within {DELETE_WINDOW} s; paused it");
                        state.holds.push(Hold { folder: folder.clone(), deletes: n, at: now_secs() });
                        counter.reset(&folder);
                        notify(&notice(
                            "Setu paused a folder",
                            format!("{n} files were deleted on the other computer within two minutes. That folder is paused here until you check it in Setu. Deleted files can be got back for 30 days."),
                        ));
                    }
                    conflicts.extend(conflict_names(ev));
                    if ev["type"] == "FolderErrors"
                        && let Some(folder) = ev["data"]["folder"].as_str().filter(|f| f.starts_with(FOLDER_PREFIX))
                    {
                        let n = ev["data"]["errors"].as_array().map_or(0, Vec::len);
                        let last = error_told.get(folder).copied().unwrap_or(0);
                        if n > 0 && now_secs() >= last + ERROR_NOTICE_EVERY {
                            error_told.insert(folder.to_string(), now_secs());
                            let first = ev["data"]["errors"][0]["path"].as_str().unwrap_or("?");
                            notify(&notice("Setu: files could not sync", format!("{n} file(s), for example {first}. Open Setu → Progress to see why.")));
                        }
                    }
                }
                conflicts.sort();
                conflicts.dedup();
                match conflicts.as_slice() {
                    [] => {}
                    [one] => notify(&notice(
                        "Setu: a file changed on both computers",
                        format!("Both copies are kept: {one}. Open Setu → Conflicts to choose."),
                    )),
                    many => notify(&notice("Setu: files changed on both computers", format!("{} conflict copies. Open Setu → Conflicts to choose.", many.len()))),
                }
            }
            Err(e) => {
                eprintln!("setu watch: events: {e}");
                std::thread::sleep(Duration::from_secs(10));
            }
        }
        state.alive_at = now_secs();
        write_state(dir, &state)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn del(folder: &str) -> Value {
        json!({ "type": "ItemFinished", "data": { "folder": folder, "item": "x", "action": "delete", "error": null } })
    }

    #[test]
    fn holds_after_too_many_deletions_in_the_window() {
        let mut c = DeleteCounter::default();
        let f = "setu-f-0123456789ab";
        for i in 0..DELETE_LIMIT {
            assert_eq!(on_event(&del(f), &mut c, 1000 + i as u64 % 10), None);
        }
        assert_eq!(on_event(&del(f), &mut c, 1010), Some((f.to_string(), DELETE_LIMIT + 1)));
        // Spread out over time, the same number is fine.
        let mut c = DeleteCounter::default();
        for i in 0..(DELETE_LIMIT * 3) {
            assert_eq!(on_event(&del(f), &mut c, i as u64 * 10), None);
        }
    }

    #[test]
    fn spots_conflict_copies_in_events() {
        let c = "a/f.sync-conflict-20261120-235545-AAAAAAA.txt";
        let got = conflict_names(&json!({ "type": "LocalIndexUpdated", "data": { "folder": "setu-f-1", "filenames": ["a/f.txt", c] } }));
        assert_eq!(got, vec![c.to_string()]);
        let got = conflict_names(&json!({ "type": "ItemFinished", "data": { "folder": "setu-f-1", "item": c, "action": "update", "error": null } }));
        assert_eq!(got, vec![c.to_string()]);
        assert!(conflict_names(&json!({ "type": "ItemFinished", "data": { "folder": "setu-f-1", "item": c, "action": "delete", "error": null } })).is_empty());
        assert!(conflict_names(&json!({ "type": "LocalIndexUpdated", "data": { "folder": "claude-memory", "filenames": [c] } })).is_empty());
    }

    #[test]
    fn ignores_other_events_and_folders() {
        let mut c = DeleteCounter::default();
        for _ in 0..100 {
            assert_eq!(on_event(&del("claude-memory"), &mut c, 1), None);
            assert_eq!(on_event(&json!({ "type": "ItemFinished", "data": { "folder": "setu-f-x", "action": "update", "error": null } }), &mut c, 1), None);
            assert_eq!(on_event(&json!({ "type": "ItemFinished", "data": { "folder": "setu-f-x", "action": "delete", "error": "denied" } }), &mut c, 1), None);
        }
    }
}
