//! Live copy progress: how much is left each way, how fast it goes, how long it has taken and
//! when it should finish.
//!
//! Syncthing knows what is still missing (`needBytes`, here and for each other computer), how
//! far half-copied files have got (`DownloadProgress` events) and how many bytes crossed each
//! connection. [`read`] collects one [`Reading`] of all that. A [`Meter`] keeps a few seconds of
//! readings and turns them into a [`Snapshot`]: for each direction, a percentage of the current
//! round of copying, a speed, the time since the round started and an estimate of the time left.
//!
//! A *round* starts when something needs copying and ends when nothing does. Its total is what
//! has been copied in it so far plus what is still left, so files that turn up halfway through
//! are counted too, and the percentage never claims more than has really arrived.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;
use serde_json::Value;

use crate::compile::FOLDER_PREFIX;
use crate::local::LocalConfig;
use crate::syncthing::Client;
use crate::watch::{Bar, Notice, WatchState};
use crate::Error;

/// How far back the speed looks.
const SPEED_WINDOW_MS: u64 = 15_000;
/// A `DownloadProgress` event older than this no longer describes what is being copied.
const PARTIAL_FRESH_SECS: i64 = 20;
/// A round smaller and shorter than this gets no progress notification.
const NOTICE_BYTES: u64 = 20 * 1024 * 1024;
const NOTICE_AFTER_MS: u64 = 20_000;
/// How many file names to show as "copying now".
const COPYING_SHOWN: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderProgress {
    pub id: String,
    /// The folder's name as Setu labels it in Syncthing ("Projects", or the root folder's name).
    pub name: String,
    /// Where it is on this computer.
    pub path: String,
    pub folder_type: String,
    /// Syncthing's state: "idle", "syncing", "scanning", "paused", ….
    pub state: String,
    pub paused: bool,
    /// Why it is paused: "you" (this folder), "safety" (many deletes), "hold" (whole computer
    /// paused or the network is not suitable). Filled in by [`mark_pauses`].
    pub paused_by: Option<String>,
    /// The whole folder, as all computers agree it should be.
    pub global_bytes: u64,
    pub global_files: u64,
    /// Still to copy to this computer.
    pub need_bytes: u64,
    pub need_files: u64,
    /// Bytes of half-copied files already here.
    pub partial_bytes: u64,
    /// Still to copy to the other computers that are connected, added up.
    pub other_need_bytes: u64,
    pub other_need_files: u64,
    /// The lowest completion among the connected other computers, in percent.
    pub other_completion: Option<f64>,
    /// Files being copied to this computer right now (the first few).
    pub copying: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub address: Option<String>,
}

/// One look at Syncthing.
#[derive(Clone, Debug, Default)]
pub struct Reading {
    pub at_ms: u64,
    pub folders: Vec<FolderProgress>,
    /// The other computers that share a Setu folder with this one.
    pub peers: Vec<Peer>,
    /// Bytes received from and sent to those computers over their current connections.
    pub received: u64,
    pub sent: u64,
}

/// The latest `DownloadProgress` event, if fresh: folder → (bytes done, file names).
fn partials(client: &Client) -> BTreeMap<String, (u64, Vec<String>)> {
    let mut out = BTreeMap::new();
    let Ok(evs) = client.events_of(0, 0, "DownloadProgress") else { return out };
    let Some(last) = evs.last() else { return out };
    let fresh = last["time"].as_str().and_then(parse_rfc3339).is_some_and(|t| crate::control::now_secs() as i64 - t <= PARTIAL_FRESH_SECS);
    if !fresh {
        return out;
    }
    for (folder, files) in last["data"].as_object().into_iter().flatten() {
        let mut done = 0;
        let mut names = Vec::new();
        for (name, f) in files.as_object().into_iter().flatten() {
            done += f["bytesDone"].as_u64().unwrap_or(0);
            names.push(name.clone());
        }
        out.insert(folder.clone(), (done, names));
    }
    out
}

/// Seconds since 1970 for an RFC 3339 time such as `2026-11-24T11:32:53.123+01:00`.
fn parse_rfc3339(s: &str) -> Option<i64> {
    let n = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 7)?, n(8, 10)?, n(11, 13)?, n(14, 16)?, n(17, 19)?);
    // Days from civil date (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (mo + if mo > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let tz = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match tz.as_bytes().first() {
        Some(b'+') | Some(b'-') => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            sign * (tz.get(1..3)?.parse::<i64>().ok()? * 3600 + tz.get(4..6)?.parse::<i64>().ok()? * 60)
        }
        _ => 0,
    };
    Some(days * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Read what is left to copy, both ways, in every Setu folder, and the connection counters.
pub fn read(client: &Client, me: &str, at_ms: u64) -> Result<Reading, Error> {
    let folders: Vec<Value> = client.folders()?.into_iter().filter(|f| f["id"].as_str().is_some_and(|i| i.starts_with(FOLDER_PREFIX))).collect();
    let mut sharing: BTreeSet<String> = BTreeSet::new();
    for f in &folders {
        for d in f["devices"].as_array().into_iter().flatten() {
            if let Some(id) = d["deviceID"].as_str().filter(|id| *id != me) {
                sharing.insert(id.to_string());
            }
        }
    }
    let names: BTreeMap<String, String> = client
        .devices()?
        .iter()
        .filter_map(|d| Some((d["deviceID"].as_str()?.to_string(), d["name"].as_str().unwrap_or_default().to_string())))
        .collect();
    let conns = client.connections()?;
    let mut r = Reading { at_ms, ..Default::default() };
    for id in &sharing {
        let c = &conns["connections"][id];
        let connected = c["connected"].as_bool().unwrap_or(false);
        if connected {
            r.received += c["inBytesTotal"].as_u64().unwrap_or(0);
            r.sent += c["outBytesTotal"].as_u64().unwrap_or(0);
        }
        r.peers.push(Peer {
            id: id.clone(),
            name: names.get(id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| crate::rules::short_id(id).to_string()),
            connected,
            address: c["address"].as_str().filter(|a| !a.is_empty()).map(String::from),
        });
    }
    let online: BTreeSet<&str> = r.peers.iter().filter(|p| p.connected).map(|p| p.id.as_str()).collect();
    let partial = partials(client);
    for f in &folders {
        let id = f["id"].as_str().unwrap_or_default().to_string();
        let label = f["label"].as_str().unwrap_or(&id);
        let mut p = FolderProgress {
            name: label.strip_prefix("Setu · ").unwrap_or(label).to_string(),
            path: f["path"].as_str().unwrap_or_default().to_string(),
            folder_type: f["type"].as_str().unwrap_or_default().to_string(),
            paused: f["paused"] == true,
            id: id.clone(),
            ..Default::default()
        };
        if p.paused {
            p.state = "paused".into();
        } else {
            match client.folder_status(&id) {
                Ok(st) => {
                    p.state = st["state"].as_str().unwrap_or("unknown").to_string();
                    p.global_bytes = st["globalBytes"].as_u64().unwrap_or(0);
                    p.global_files = st["globalFiles"].as_u64().unwrap_or(0);
                    p.need_bytes = st["needBytes"].as_u64().unwrap_or(0);
                    p.need_files = st["needFiles"].as_u64().unwrap_or(0) + st["needDeletes"].as_u64().unwrap_or(0);
                }
                Err(_) => p.state = "unknown".into(),
            }
        }
        for d in f["devices"].as_array().into_iter().flatten().filter_map(|d| d["deviceID"].as_str()) {
            if !online.contains(d) {
                continue;
            }
            if let Ok(c) = client.completion(&id, d) {
                p.other_need_bytes += c["needBytes"].as_u64().unwrap_or(0);
                p.other_need_files += c["needItems"].as_u64().unwrap_or(0) + c["needDeletes"].as_u64().unwrap_or(0);
                if let Some(pc) = c["completion"].as_f64() {
                    p.other_completion = Some(p.other_completion.map_or(pc, |x: f64| x.min(pc)));
                }
            }
        }
        if p.need_files > 0 {
            if let Some((done, names)) = partial.get(&id) {
                p.partial_bytes = (*done).min(p.need_bytes);
                p.copying = names.iter().take(COPYING_SHOWN).cloned().collect();
            }
            if p.copying.is_empty()
                && let Ok(n) = client.need(&id)
            {
                p.copying = ["progress", "queued", "rest"]
                    .iter()
                    .flat_map(|k| n[*k].as_array().cloned().unwrap_or_default())
                    .filter_map(|x| x["name"].as_str().map(String::from))
                    .take(COPYING_SHOWN)
                    .collect();
            }
        }
        r.folders.push(p);
    }
    Ok(r)
}

/// Say why each paused folder is paused, from the helper's state and this computer's settings.
pub fn mark_pauses(s: &mut Snapshot, w: &WatchState, cfg: &LocalConfig) {
    for f in s.folders.iter_mut().filter(|f| f.paused) {
        f.paused_by = Some(
            if w.holds.iter().any(|h| h.folder == f.id) {
                "safety"
            } else if cfg.paused_folders.contains(&f.id) {
                "you"
            } else {
                "hold"
            }
            .into(),
        );
    }
}

/// One direction of copying, now.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flow {
    /// A round of copying is going on.
    pub active: bool,
    pub left_bytes: u64,
    pub left_files: u64,
    /// Copied in this round so far, half-copied files included.
    pub done_bytes: u64,
    pub done_files: u64,
    pub total_bytes: u64,
    /// Of this round, 0 to 100.
    pub percent: f64,
    /// Bytes a second over the last few seconds.
    pub speed: u64,
    /// Bytes a second since the round started.
    pub average: u64,
    /// Seconds left, when it can be told.
    pub eta_secs: Option<u64>,
    /// When the round started (Unix seconds) and how long ago.
    pub started_at: Option<u64>,
    pub elapsed_secs: u64,
}

/// A round of copying that finished.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finished {
    /// Copied to this computer (otherwise from it to the others).
    pub to_here: bool,
    pub bytes: u64,
    pub files: u64,
    pub secs: u64,
    /// When it finished (Unix seconds).
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Unix seconds.
    pub at: u64,
    /// To this computer.
    pub down: Flow,
    /// From this computer to the others.
    pub up: Flow,
    pub folders: Vec<FolderProgress>,
    pub peers: Vec<Peer>,
    /// Copying is on hold on this computer (paused by the person or by the network gate).
    pub paused: bool,
    /// The last round that finished, either way.
    pub last_done: Option<Finished>,
}

impl Snapshot {
    pub fn active(&self) -> bool {
        self.down.active || self.up.active
    }
}

#[derive(Clone, Debug, Default)]
struct Round {
    started_ms: Option<u64>,
    done_bytes: u64,
    done_files: u64,
    last_left: u64,
    last_files: u64,
}

impl Round {
    /// Take in what is left now. Returns the finished round when nothing is left any more.
    fn step(&mut self, left: u64, files: u64, now_ms: u64, hold: bool) -> Option<(u64, u64, u64)> {
        let Some(start) = self.started_ms else {
            if left > 0 || files > 0 {
                *self = Round { started_ms: Some(now_ms), last_left: left, last_files: files, ..Default::default() };
            }
            return None;
        };
        self.done_bytes += self.last_left.saturating_sub(left);
        self.done_files += self.last_files.saturating_sub(files);
        self.last_left = left;
        self.last_files = files;
        if left == 0 && files == 0 && !hold {
            let out = (self.done_bytes, self.done_files, now_ms.saturating_sub(start));
            *self = Round::default();
            return Some(out);
        }
        None
    }

    fn flow(&self, partial: u64, speed: u64, now_ms: u64, paused: bool) -> Flow {
        let Some(start) = self.started_ms else { return Flow::default() };
        let partial = partial.min(self.last_left);
        let done = self.done_bytes + partial;
        let total = self.done_bytes + self.last_left;
        let elapsed_ms = now_ms.saturating_sub(start);
        let average = if elapsed_ms >= 1000 { done * 1000 / elapsed_ms } else { 0 };
        let left = self.last_left - partial;
        Flow {
            active: true,
            left_bytes: self.last_left,
            left_files: self.last_files,
            done_bytes: done,
            done_files: self.done_files,
            total_bytes: total,
            percent: if total == 0 { 0.0 } else { (done as f64 * 100.0 / total as f64).min(100.0) },
            speed,
            average,
            eta_secs: if paused { None } else { eta_secs(left, speed, average) },
            started_at: Some(start / 1000),
            elapsed_secs: elapsed_ms / 1000,
        }
    }
}

/// Seconds until `left` bytes are copied, from the speed over the last few seconds (`recent`)
/// and the average since the round started (`average`), both in bytes a second. `None` while it
/// cannot be told yet.
pub fn eta_secs(left: u64, recent: u64, average: u64) -> Option<u64> {
    if left == 0 {
        return Some(0);
    }
    // Lean on the recent speed so a real change shows within seconds, but blend in a quarter of
    // the average so one slow file does not swing the estimate. A stall (recent 0) falls back
    // to the average; with neither there is no basis yet.
    let speed = match (recent, average) {
        (0, 0) => return None,
        (r, 0) => r,
        (0, a) => a,
        (r, a) => (3 * r + a) / 4,
    };
    Some(left.div_ceil(speed.max(1)))
}

/// Turns readings into snapshots. Keep one per process and feed it every few seconds.
#[derive(Default)]
pub struct Meter {
    window: VecDeque<(u64, u64, u64)>,
    down: Round,
    up: Round,
    last: BTreeMap<String, FolderProgress>,
    last_done: Option<Finished>,
}

impl Meter {
    /// Add a reading. `hold`: copying is on hold on this computer, so a round does not end just
    /// because paused folders report nothing left. Returns the snapshot and any rounds that
    /// finished with this reading.
    pub fn observe(&mut self, mut r: Reading, hold: bool) -> (Snapshot, Vec<Finished>) {
        // A paused folder reports nothing; keep what it last said was left.
        for f in r.folders.iter_mut().filter(|f| f.paused) {
            if let Some(p) = self.last.get(&f.id) {
                f.global_bytes = p.global_bytes;
                f.global_files = p.global_files;
                f.need_bytes = p.need_bytes;
                f.need_files = p.need_files;
                f.other_need_bytes = p.other_need_bytes;
                f.other_need_files = p.other_need_files;
                f.other_completion = p.other_completion;
            }
        }
        let hold = hold || r.folders.iter().any(|f| f.paused);
        self.last = r.folders.iter().map(|f| (f.id.clone(), f.clone())).collect();

        // Connection counters start again at zero on a new connection.
        if self.window.back().is_some_and(|&(_, rx, tx)| r.received < rx || r.sent < tx) {
            self.window.clear();
        }
        self.window.push_back((r.at_ms, r.received, r.sent));
        while self.window.len() > 2 && self.window.front().is_some_and(|&(t, _, _)| t + SPEED_WINDOW_MS < r.at_ms) {
            self.window.pop_front();
        }
        let (down_speed, up_speed) = match (self.window.front(), self.window.back()) {
            (Some(&(t0, rx0, tx0)), Some(&(t1, rx1, tx1))) if t1 >= t0 + 1000 => ((rx1 - rx0) * 1000 / (t1 - t0), (tx1 - tx0) * 1000 / (t1 - t0)),
            _ => (0, 0),
        };

        let sum = |f: fn(&FolderProgress) -> u64| r.folders.iter().map(f).sum::<u64>();
        let (need, need_files, partial) = (sum(|f| f.need_bytes), sum(|f| f.need_files), sum(|f| f.partial_bytes));
        let (other_need, other_files) = (sum(|f| f.other_need_bytes), sum(|f| f.other_need_files));
        let mut finished = Vec::new();
        let now = r.at_ms / 1000;
        if let Some((bytes, files, ms)) = self.down.step(need, need_files, r.at_ms, hold) {
            finished.push(Finished { to_here: true, bytes, files, secs: ms / 1000, at: now });
        }
        if let Some((bytes, files, ms)) = self.up.step(other_need, other_files, r.at_ms, hold) {
            finished.push(Finished { to_here: false, bytes, files, secs: ms / 1000, at: now });
        }
        if let Some(f) = finished.last() {
            self.last_done = Some(f.clone());
        }
        let snap = Snapshot {
            at: now,
            down: self.down.flow(partial, down_speed, r.at_ms, hold),
            // Half-copied files on the other computers are not visible from here.
            up: self.up.flow(0, up_speed, r.at_ms, hold),
            folders: r.folders,
            peers: r.peers,
            paused: hold,
            last_done: self.last_done.clone(),
        };
        (snap, finished)
    }
}

pub fn bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n} B");
    }
    let mut v = n as f64 / 1024.0;
    let mut unit = "KB";
    for u in ["MB", "GB", "TB"] {
        if v < 1024.0 {
            break;
        }
        v /= 1024.0;
        unit = u;
    }
    if v < 10.0 { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") }
}

/// `45 s`, `3 min 12 s`, `2 h 05 min`.
pub fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {:02} s", secs / 60, secs % 60),
        _ => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
    }
}

/// The progress notifications the helper shows: one per direction, updated in place while a
/// round runs, and a last word when it finishes.
#[derive(Default)]
pub struct Announcer {
    /// Directions with a progress notification on screen: [to here, from here].
    shown: [bool; 2],
}

impl Announcer {
    pub fn notices(&mut self, s: &Snapshot, finished: &[Finished]) -> Vec<Notice> {
        let others = {
            let on: Vec<&str> = s.peers.iter().filter(|p| p.connected).map(|p| p.name.as_str()).collect();
            if on.is_empty() { "the other computer".to_string() } else { on.join(", ") }
        };
        let mut out = Vec::new();
        for f in finished {
            let i = usize::from(!f.to_here);
            if std::mem::take(&mut self.shown[i]) {
                let avg = f.bytes.checked_div(f.secs).map(|b| format!(", {}/s on average", bytes(b))).unwrap_or_default();
                let what = format!("{} ({} files) in {}{avg}", bytes(f.bytes), f.files, duration(f.secs));
                out.push(Notice {
                    title: if f.to_here { "Setu: all copied to this computer".into() } else { format!("Setu: all copied to {others}") },
                    body: format!("Done: {what}."),
                    bar: Some(Bar { tag: tag(f.to_here), permille: 1000, status: format!("Done in {}", duration(f.secs)), value: bytes(f.bytes), done: true }),
                });
            }
        }
        for (i, flow) in [&s.down, &s.up].into_iter().enumerate() {
            let elapsed_ms = flow.elapsed_secs * 1000;
            if !flow.active || (!self.shown[i] && flow.total_bytes < NOTICE_BYTES && elapsed_ms < NOTICE_AFTER_MS) {
                continue;
            }
            self.shown[i] = true;
            let to_here = i == 0;
            let speed = if s.paused { "paused".to_string() } else { format!("{}/s", bytes(flow.speed)) };
            let left = match (s.paused, flow.eta_secs) {
                (true, _) => "waits until you resume".to_string(),
                (false, Some(t)) => format!("about {} left", duration(t)),
                (false, None) => "working out time left".to_string(),
            };
            let of = format!("{} of {}", bytes(flow.done_bytes), bytes(flow.total_bytes));
            out.push(Notice {
                title: if to_here { "Setu: copying to this computer".into() } else { format!("Setu: copying to {others}") },
                body: format!("{:.0}% · {of} · {speed} · {left} · {} so far", flow.percent.floor(), duration(flow.elapsed_secs)),
                bar: Some(Bar {
                    tag: tag(to_here),
                    permille: (flow.percent * 10.0).clamp(0.0, 1000.0) as u16,
                    status: format!("{speed} · {left}"),
                    value: of,
                    done: false,
                }),
            });
        }
        out
    }
}

fn tag(to_here: bool) -> String {
    if to_here { "setu-down" } else { "setu-up" }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    fn folder(need: u64, files: u64, partial: u64) -> FolderProgress {
        FolderProgress { id: "setu-f-1".into(), need_bytes: need, need_files: files, partial_bytes: partial, ..Default::default() }
    }

    fn reading(t_ms: u64, need: u64, files: u64, partial: u64, received: u64) -> Reading {
        Reading { at_ms: t_ms, folders: vec![folder(need, files, partial)], received, ..Default::default() }
    }

    #[test]
    fn a_round_counts_what_arrived_and_ends_when_nothing_is_left() {
        let mut m = Meter::default();
        let (s, f) = m.observe(reading(0, 100 * MB, 10, 0, 0), false);
        assert!(s.down.active && f.is_empty());
        assert_eq!((s.down.done_bytes, s.down.total_bytes, s.down.percent), (0, 100 * MB, 0.0));
        // Two files done, one half way: 20 MB + 5 MB of 100 MB.
        let (s, _) = m.observe(reading(10_000, 80 * MB, 8, 5 * MB, 25 * MB), false);
        assert_eq!(s.down.done_bytes, 25 * MB);
        assert_eq!(s.down.percent, 25.0);
        assert_eq!(s.down.elapsed_secs, 10);
        assert_eq!(s.down.speed, 25 * MB * 1000 / 10_000);
        // New files turn up: the total grows, what was copied stays.
        let (s, _) = m.observe(reading(20_000, 130 * MB, 12, 0, 45 * MB), false);
        assert_eq!((s.down.done_bytes, s.down.total_bytes), (20 * MB, 150 * MB));
        let (s, f) = m.observe(reading(30_000, 0, 0, 0, 175 * MB), false);
        assert!(!s.down.active);
        // 2 files before the new ones turned up, then all 12 that were left.
        assert_eq!(f, vec![Finished { to_here: true, bytes: 150 * MB, files: 14, secs: 30, at: 30 }]);
        assert_eq!(s.last_done, Some(f[0].clone()));
    }

    #[test]
    fn a_pause_does_not_end_the_round() {
        let mut m = Meter::default();
        m.observe(reading(0, 50 * MB, 5, 0, 0), false);
        let mut r = reading(5_000, 0, 0, 0, 0);
        r.folders[0].paused = true;
        let (s, f) = m.observe(r, true);
        assert!(f.is_empty() && s.down.active && s.paused);
        assert_eq!(s.down.left_bytes, 50 * MB, "a paused folder keeps what it last needed");
        assert_eq!(s.down.eta_secs, None);
    }

    #[test]
    fn speed_survives_a_reconnect() {
        let mut m = Meter::default();
        m.observe(reading(0, 50 * MB, 5, 0, 900 * MB), false);
        let (s, _) = m.observe(reading(2_000, 50 * MB, 5, 0, MB), false);
        assert_eq!(s.down.speed, 0, "counters restarted: no made-up speed");
        let (s, _) = m.observe(reading(4_000, 50 * MB, 5, 0, 5 * MB), false);
        assert_eq!(s.down.speed, 2 * MB);
    }

    #[test]
    fn estimates_time_left() {
        assert_eq!(eta_secs(0, 0, 0), Some(0));
        assert_eq!(eta_secs(10 * MB, 0, 0), None);
        let t = eta_secs(100 * MB, MB, MB).unwrap();
        assert!((95..=105).contains(&t), "steady 1 MB/s with 100 MB left: {t}");
    }

    #[test]
    fn announces_big_rounds_only_and_says_when_done() {
        let mut m = Meter::default();
        let mut a = Announcer::default();
        let (s, f) = m.observe(reading(0, MB, 1, 0, 0), false);
        assert!(a.notices(&s, &f).is_empty(), "small and new: no notice");
        let (s, f) = m.observe(reading(1_000, 0, 0, 0, MB), false);
        assert!(a.notices(&s, &f).is_empty(), "never shown, so no 'done' either");

        let (s, f) = m.observe(reading(2_000, 500 * MB, 40, 0, MB), false);
        let n = a.notices(&s, &f);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].bar.as_ref().unwrap().permille, 0);
        let (s, f) = m.observe(reading(12_000, 0, 0, 0, 501 * MB), false);
        let n = a.notices(&s, &f);
        assert_eq!(n.len(), 1);
        let bar = n[0].bar.as_ref().unwrap();
        assert!(bar.done && bar.permille == 1000);
        assert!(n[0].body.contains("500 MB (40 files) in 10 s"), "{}", n[0].body);
    }

    #[test]
    fn reads_rfc3339_times() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-24T11:32:53.123456+01:00"), Some(1_795_516_373));
    }

    #[test]
    fn words_for_sizes_and_times() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(5 * MB / 2), "2.5 MB");
        assert_eq!(bytes(300 * MB), "300 MB");
        assert_eq!(duration(42), "42 s");
        assert_eq!(duration(192), "3 min 12 s");
        assert_eq!(duration(7500), "2 h 05 min");
    }
}
