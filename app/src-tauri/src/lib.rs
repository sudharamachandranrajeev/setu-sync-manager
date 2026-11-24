//! Setu: the window, the tray, the sync engine and the background helper, in one process.
//! The window's commands are thin calls into `setu-core`. All rules, planning and applying
//! live in the core, so the app and the `setu` command always agree. The `setu` command reaches
//! this process through its local pipe (`setu_core::pipe`).

mod host;
mod live;
mod notify;
mod tray;
mod update;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::Manager;
use setu_core::apply::{Plan, Report};
use setu_core::index::Tree;
use setu_core::compile::folder_id;
use setu_core::control::now_secs;
use setu_core::preview::Preview;
use setu_core::watch;
use setu_core::local::{config_dir, LocalConfig};
use setu_core::accuracy::Mismatch;
use setu_core::hub::{DeviceState, Held};
use setu_core::rules::{short_id, Kind, Mode, Os, RuleSet};
use setu_core::tidy::Removed;
use setu_core::session::Session;
use setu_core::trust::{Conflict, FolderCheck, Kept, Restored};
use setu_core::Error;

#[derive(Default)]
struct AppState {
    session: Option<Session>,
    local: Option<Tree>,
    remote: Option<Tree>,
    remote_age: Option<u64>,
    remote_root: Option<String>,
    shown_plan: Option<(Plan, Preview)>,
}

type State<'a> = tauri::State<'a, Mutex<AppState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    short: String,
    name: String,
    os: Option<Os>,
    connected: bool,
    address: Option<String>,
    applied_revision: Option<u64>,
    applied_at: Option<u64>,
    /// In the rules (it syncs), and whether it only receives.
    in_rules: bool,
    receive_only: bool,
    /// New rules wait for the person on that computer, and why.
    held: Option<Held>,
    problems: Vec<String>,
    /// When it last reported to the Brahma account (Unix seconds).
    seen_at: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountView {
    server: String,
    email: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HoldView {
    folder: String,
    /// The rule path the folder belongs to, when it can be found.
    rel: Option<String>,
    deletes: usize,
    at: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WatchView {
    running: bool,
    gate_open: bool,
    reason: Option<String>,
    metered: bool,
    relayed: bool,
    allow_metered: bool,
    allow_relay: bool,
    holds: Vec<HoldView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EditView {
    path: String,
    before: Option<String>,
    after: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    /// "setup" (choose this computer's folder), "signin" (sign in to the Brahma account), "join"
    /// (add this computer to the rules) or "ready".
    stage: &'static str,
    account: Option<AccountView>,
    /// Every computer in the account and the rules, this one first.
    devices: Vec<DeviceView>,
    message: Option<String>,
    root: Option<String>,
    me: Option<DeviceView>,
    other: Option<DeviceView>,
    shared_revision: Option<u64>,
    /// The choice for the whole folder, as the draft has it.
    root_mode: Option<String>,
    edits: Vec<EditView>,
    conflicts: Vec<String>,
    /// Conflict copies of files in synced folders on this computer.
    file_conflicts: usize,
    local_entries: usize,
    remote_entries: Option<usize>,
    remote_list_age: Option<u64>,
    always_ignore: Vec<String>,
    /// Changes to the "always left out" list not saved yet.
    leave_out: LeaveOutEdits,
    /// Syncing paused here by the person: until this time, or 0 until resumed.
    paused_until: Option<u64>,
    /// No desktop notifications about copy progress.
    quiet_progress: bool,
    watch: Option<WatchView>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct LeaveOutEdits {
    added: Vec<String>,
    removed: Vec<String>,
}

fn leave_out_edits(shared: Option<&RuleSet>, working: &RuleSet) -> LeaveOutEdits {
    let Some(shared) = shared else { return LeaveOutEdits::default() };
    LeaveOutEdits {
        added: working.always_ignore.iter().filter(|l| !shared.always_ignore.contains(l)).cloned().collect(),
        removed: shared.always_ignore.iter().filter(|l| !working.always_ignore.contains(l)).cloned().collect(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Side {
    size: u64,
    files: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeView {
    name: String,
    path: String,
    dir: bool,
    here: Option<Side>,
    there: Option<Side>,
    explicit: Option<String>,
    effective: String,
    /// Where the effective choice comes from: a rule path, or `null` for the root.
    from: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanView {
    plan: Plan,
    preview: Preview,
    edits: Vec<EditView>,
    leave_out: LeaveOutEdits,
}

fn err(e: Error) -> String {
    e.to_string()
}

fn open(st: &mut AppState) -> Result<(), String> {
    if st.session.is_none() {
        st.session = Some(Session::open().map_err(err)?);
    }
    Ok(())
}

fn ensure_trees(st: &mut AppState) -> Result<(), String> {
    let s = st.session.as_ref().unwrap();
    if st.local.is_none() {
        st.local = Some(Tree::new(s.scan_here().map_err(err)?));
    }
    if st.remote.is_none()
        && let Some((list, age)) = s.other_list()
    {
        st.remote_age = age;
        st.remote_root = list.root;
        st.remote = Some(Tree::new(list.entries));
    }
    Ok(())
}

fn edits(shared: Option<&RuleSet>, working: &RuleSet) -> Vec<EditView> {
    let mut v = Vec::new();
    let Some(shared) = shared else { return v };
    if shared.root != working.root {
        v.push(EditView { path: String::new(), before: Some(shared.root.to_string()), after: Some(working.root.to_string()) });
    }
    let paths: BTreeSet<&String> = shared.rules.keys().chain(working.rules.keys()).collect();
    for p in paths {
        let (b, a) = (shared.rules.get(p).map(|r| r.mode.to_string()), working.rules.get(p).map(|r| r.mode.to_string()));
        if b != a {
            v.push(EditView { path: p.clone(), before: b, after: a });
        }
    }
    v
}

/// The account's computers and the ones the rules name, with how each is doing.
fn devices(s: &Session, rules: Option<&RuleSet>) -> Vec<DeviceView> {
    let conns = s.client.connections().unwrap_or_default();
    let states: Vec<DeviceState> = s.hub.devices().unwrap_or_default();
    let mut ids: Vec<String> = vec![s.ctx.me.clone()];
    for id in rules.into_iter().flat_map(|r| r.devices.keys()).chain(states.iter().map(|d| &d.id)) {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    ids.into_iter()
        .map(|id| {
            let st = states.iter().find(|d| d.id == id);
            let rd = rules.and_then(|r| r.devices.get(&id));
            let c = &conns["connections"][&id];
            DeviceView {
                short: short_id(&id).to_string(),
                name: rd.map(|d| d.name.clone()).or_else(|| st.map(|d| d.name.clone())).unwrap_or_else(|| short_id(&id).to_string()),
                os: rd.map(|d| d.os).or_else(|| st.and_then(|d| d.os)),
                connected: id == s.ctx.me || c["connected"].as_bool().unwrap_or(false),
                address: c["address"].as_str().filter(|a| !a.is_empty()).map(String::from),
                applied_revision: st.filter(|d| d.applied_at > 0).map(|d| d.applied_revision),
                applied_at: st.filter(|d| d.applied_at > 0).map(|d| d.applied_at),
                in_rules: rd.is_some(),
                receive_only: rd.is_some_and(|d| d.receive_only),
                held: st.and_then(|d| d.held.clone()),
                problems: st.map(|d| d.problems.clone()).unwrap_or_default(),
                seen_at: st.map(|d| d.updated_at).filter(|&t| t > 0),
                id,
            }
        })
        .collect()
}

fn view(st: &mut AppState) -> View {
    let mut v = View {
        stage: "setup",
        account: None,
        devices: Vec::new(),
        message: None,
        root: None,
        me: None,
        other: None,
        shared_revision: None,
        root_mode: None,
        edits: Vec::new(),
        conflicts: Vec::new(),
        file_conflicts: 0,
        local_entries: 0,
        remote_entries: None,
        remote_list_age: None,
        always_ignore: Vec::new(),
        leave_out: LeaveOutEdits::default(),
        paused_until: None,
        quiet_progress: false,
        watch: None,
    };
    if let Err(e) = open(st) {
        v.message = Some(e);
        return v;
    }
    let s = st.session.as_ref().unwrap();
    v.root = Some(s.local.root.to_string_lossy().into_owned());
    v.conflicts = s.control.conflicts();
    v.account = s.hub.account().map(|a| AccountView { server: a.server, email: a.email });
    if v.account.is_none() {
        v.stage = "signin";
        return v;
    }
    let shared = match s.shared() {
        Ok(Some(r)) if r.devices.contains_key(&s.ctx.me) => r,
        Ok(_) => {
            v.stage = "join";
            v.devices = devices(s, None);
            return v;
        }
        Err(e) => {
            v.message = Some(err(e));
            return v;
        }
    };
    let working = s.working().unwrap_or_else(|_| shared.clone());
    v.devices = devices(s, Some(&shared));
    v.me = v.devices.iter().position(|d| d.id == s.ctx.me).map(|i| v.devices.remove(i));
    v.other = v.devices.iter().position(|d| d.in_rules).map(|i| v.devices.remove(i));
    // `devices` keeps the rest; the window shows me, other, then these.
    v.shared_revision = Some(shared.revision);
    v.root_mode = Some(working.root.to_string());
    v.edits = edits(Some(&shared), &working);
    v.always_ignore = working.always_ignore.clone();
    v.leave_out = leave_out_edits(Some(&shared), &working);
    v.paused_until = s.local.paused_until.filter(|&t| t == 0 || t > now_secs());
    v.quiet_progress = s.local.quiet_progress;
    let w = watch::read_state(&s.dir);
    let rels: Vec<String> = std::iter::once(String::new()).chain(working.rules.keys().cloned()).collect();
    v.watch = Some(WatchView {
        running: w.alive_at + 120 >= now_secs(),
        gate_open: w.gate.open,
        reason: w.gate.reason.clone(),
        metered: w.gate.metered,
        relayed: w.gate.relayed,
        allow_metered: s.local.allow_metered,
        allow_relay: s.local.allow_relay,
        holds: w
            .holds
            .iter()
            .map(|h| HoldView { folder: h.folder.clone(), rel: rels.iter().find(|r| folder_id(r) == h.folder).cloned(), deletes: h.deletes, at: h.at })
            .collect(),
    });
    v.stage = "ready";
    if let Err(e) = ensure_trees(st) {
        v.message = Some(e);
    }
    v.local_entries = st.local.as_ref().map(|t| t.entries.len()).unwrap_or(0);
    if let (Some(s), Some(t)) = (st.session.as_ref(), st.local.as_ref()) {
        v.file_conflicts = s.conflicts(t).map(|c| c.len()).unwrap_or(0);
    }
    v.remote_entries = st.remote.as_ref().map(|t| t.entries.len());
    v.remote_list_age = st.remote_age;
    v
}

#[tauri::command]
async fn state(state: State<'_>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    Ok(view(&mut st))
}

#[tauri::command]
async fn refresh(state: State<'_>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    st.session = None;
    st.local = None;
    st.remote = None;
    st.shown_plan = None;
    Ok(view(&mut st))
}

#[tauri::command]
async fn children(state: State<'_>, path: String) -> Result<Vec<NodeView>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    ensure_trees(&mut st)?;
    let s = st.session.as_ref().unwrap();
    let rules = s.working().map_err(err)?;
    let empty = Tree::default();
    let (local, remote) = (st.local.as_ref().unwrap_or(&empty), st.remote.as_ref().unwrap_or(&empty));
    let side = |t: &Tree, p: &str| {
        t.get(p).map(|e| if e.dir { let (size, files) = t.total(p); Side { size, files } } else { Side { size: e.size, files: 1 } })
    };
    let mut names: Vec<(String, bool)> = Vec::new();
    let mut seen = BTreeSet::new();
    for e in local.children(&path).chain(remote.children(&path)) {
        let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
        if seen.insert(name.clone()) {
            names.push((name, e.dir));
        }
    }
    names.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));
    Ok(names
        .into_iter()
        .map(|(name, dir)| {
            let p = if path.is_empty() { name.clone() } else { format!("{path}/{name}") };
            let (eff, from) = rules.effective(&p);
            NodeView {
                here: side(local, &p),
                there: side(remote, &p),
                explicit: rules.rules.get(&p).map(|r| r.mode.to_string()),
                effective: eff.to_string(),
                from: from.map(String::from),
                name,
                path: p,
                dir,
            }
        })
        .collect())
}

#[tauri::command]
async fn set_rule(state: State<'_>, path: String, dir: bool, mode: Option<String>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    {
        let s = st.session.as_ref().unwrap();
        let mut rules = s.working().map_err(err)?;
        let mode = mode.as_deref().map(Mode::parse).transpose().map_err(err)?;
        rules.set(&path, if dir { Kind::Dir } else { Kind::File }, mode).map_err(err)?;
        s.save_draft(rules).map_err(err)?;
    }
    st.shown_plan = None;
    Ok(view(&mut st))
}

#[tauri::command]
async fn set_leave_out(state: State<'_>, list: Vec<String>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    {
        let s = st.session.as_ref().unwrap();
        let mut rules = s.working().map_err(err)?;
        rules.set_always_ignore(&list).map_err(err)?;
        s.save_draft(rules).map_err(err)?;
    }
    st.shown_plan = None;
    // The list changes what the folder view counts.
    st.local = None;
    Ok(view(&mut st))
}

/// Pause syncing on this computer until `until` (Unix seconds; 0 = until resumed), or resume
/// it (`None`). The helper acts on it within seconds; without the helper, Setu acts at once.
pub(crate) fn pause_until(until: Option<u64>) -> Result<(), String> {
    let dir = config_dir();
    let mut cfg = LocalConfig::load(&dir).map_err(err)?;
    cfg.paused_until = until;
    cfg.save(&dir).map_err(err)?;
    if watch::read_state(&dir).alive_at + 120 < now_secs() {
        let client = setu_core::syncthing::Client::connect().map_err(err)?;
        watch::gate_now(&dir, &cfg, &client).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
async fn set_pause(state: State<'_>, until: Option<u64>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    pause_until(until)?;
    st.session = None;
    Ok(view(&mut st))
}

/// Pause or resume one synced folder on this computer.
#[tauri::command]
async fn pause_folder(state: State<'_>, folder: String, pause: bool) -> Result<(), String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    watch::set_folder_paused(&s.dir, &s.client, &folder, pause).map_err(err)?;
    // The session holds a copy of local.json.
    st.session = None;
    Ok(())
}

/// The latest copy progress (`null` until the first reading, a few seconds after start).
#[tauri::command]
async fn progress(live: tauri::State<'_, live::Live>) -> Result<Option<setu_core::progress::Snapshot>, String> {
    Ok(live.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
}

#[tauri::command]
async fn set_quiet_progress(state: State<'_>, quiet: bool) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    let dir = config_dir();
    let mut cfg = LocalConfig::load(&dir).map_err(err)?;
    cfg.quiet_progress = quiet;
    cfg.save(&dir).map_err(err)?;
    st.session = None;
    Ok(view(&mut st))
}

/// Syncthing's speed limits on this computer, in KiB/s (0 = no limit): (download, upload).
#[tauri::command]
async fn limits(state: State<'_>) -> Result<(u64, u64), String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let o = st.session.as_ref().unwrap().client.options().map_err(err)?;
    Ok((o["maxRecvKbps"].as_u64().unwrap_or(0), o["maxSendKbps"].as_u64().unwrap_or(0)))
}

#[tauri::command]
async fn set_limits(state: State<'_>, down: u64, up: u64) -> Result<(u64, u64), String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let c = &st.session.as_ref().unwrap().client;
    c.patch_options(&serde_json::json!({ "maxRecvKbps": down, "maxSendKbps": up })).map_err(err)?;
    Ok((down, up))
}

/// Show a folder in the file manager: this computer's folder or a folder inside it.
#[tauri::command]
async fn open_path(state: State<'_>, path: Option<String>) -> Result<(), String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let root = st.session.as_ref().unwrap().local.root.clone();
    let p = path.map(PathBuf::from).unwrap_or_else(|| root.clone());
    // Only folders Setu looks after.
    if !p.starts_with(&root) || !p.is_dir() {
        return Err(format!("{} is not a folder inside {}", p.display(), root.display()));
    }
    let opener = if cfg!(windows) { "explorer" } else { "xdg-open" };
    std::process::Command::new(opener).arg(&p).spawn().map_err(|e| format!("{opener}: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn discard(state: State<'_>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().discard_draft().map_err(err)?;
    st.shown_plan = None;
    Ok(view(&mut st))
}

#[tauri::command]
async fn plan(state: State<'_>) -> Result<PlanView, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    // The preview must use the folder as it is now, and the newest list from the other side.
    st.local = None;
    st.remote = None;
    ensure_trees(&mut st)?;
    let s = st.session.as_ref().unwrap();
    let (rules, plan) = s.plan().map_err(err)?;
    let shared = s.shared().map_err(err)?;
    let edits = edits(shared.as_ref(), &rules);
    let leave_out = leave_out_edits(shared.as_ref(), &rules);
    let empty = Tree::default();
    let here = st.local.as_ref().unwrap_or(&empty);
    let there = st.remote.as_ref().map(|t| (t, st.remote_root.as_deref()));
    let preview = s.preview(&rules, &plan, here, there);
    st.shown_plan = Some((plan.clone(), preview.clone()));
    Ok(PlanView { plan, preview, edits, leave_out })
}

#[tauri::command]
async fn apply(state: State<'_>, guard_confirmed: bool) -> Result<Report, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let (plan, preview) = st.shown_plan.take().ok_or("review the plan first")?;
    st.session.as_ref().unwrap().apply(&plan, &preview, guard_confirmed).map_err(err)
}

#[tauri::command]
async fn init(state: State<'_>, root: String) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    let root = PathBuf::from(root.trim());
    if !root.is_absolute() || !root.is_dir() {
        return Err(format!("{} is not an existing folder", root.display()));
    }
    LocalConfig { root, ..Default::default() }.save(&config_dir()).map_err(err)?;
    *st = AppState::default();
    Ok(view(&mut st))
}

/// Sign in to the Brahma account. "recovery-key-needed" when the password was reset and the
/// recovery key must unlock the data.
#[tauri::command]
async fn sign_in(state: State<'_>, server: String, email: String, password: String) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    match s.hub.sign_in(server.trim(), email.trim(), &password) {
        Err(Error::RecoveryKeyNeeded) => return Err("recovery-key-needed".into()),
        r => r.map_err(err)?,
    }
    after_sign_in(s);
    Ok(view(&mut st))
}

#[tauri::command]
async fn recovery(state: State<'_>, key: String) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    s.hub.unlock_with_recovery_key(key.trim()).map_err(err)?;
    after_sign_in(s);
    Ok(view(&mut st))
}

/// Keep Syncthing on the tailnet and fetch the account's rows. Problems here are not fatal:
/// the view shows what is missing.
fn after_sign_in(s: &Session) {
    if let Err(e) = setu_core::mesh::harden(&s.client, &setu_core::mesh::tailnet_ips()) {
        eprintln!("setu: {e}");
    }
    if let Err(e) = s.sync_hub() {
        eprintln!("setu: Brahma sync: {e}");
    }
}

#[tauri::command]
async fn sign_out(state: State<'_>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().hub.sign_out().map_err(err)?;
    Ok(view(&mut st))
}

/// Add this computer to the rules. `receive_only`: a backup copy that never sends changes.
#[tauri::command]
async fn join(state: State<'_>, receive_only: bool) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().join(receive_only).map_err(err)?;
    Ok(view(&mut st))
}

/// Ask the helper to fetch new rules and check the copies now.
#[tauri::command]
async fn check_now() -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(setu_core::helper::CHECK_REQUEST), b"").map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AccuracyRow {
    top: String,
    files: u64,
    bytes: u64,
    checked_at: u64,
    /// One entry per other computer: "same", "differs", "syncing" or "unchecked".
    others: Vec<(String, String, &'static str)>,
}

/// The last content check: per top folder, does each other computer hold the same files?
#[tauri::command]
async fn accuracy(state: State<'_>) -> Result<Vec<AccuracyRow>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    let all = s.hub.devices().map_err(err)?;
    let Some(me) = all.iter().find(|d| d.id == s.ctx.me) else { return Ok(Vec::new()) };
    Ok(me
        .digests
        .iter()
        .map(|(top, mine)| AccuracyRow {
            top: top.clone(),
            files: mine.files,
            bytes: mine.bytes,
            checked_at: mine.at,
            others: all
                .iter()
                .filter(|d| d.id != me.id)
                .map(|d| {
                    let word = match d.digests.get(top) {
                        None => "unchecked",
                        Some(g) if g.hash == mine.hash => "same",
                        Some(g) if g.settled && mine.settled => "differs",
                        Some(_) => "syncing",
                    };
                    (d.id.clone(), d.name.clone(), word)
                })
                .collect(),
        })
        .collect())
}

/// Files that differ from another computer in one top folder, or `None` while its list is on
/// the way.
#[tauri::command]
async fn differences(state: State<'_>, device: String, top: String) -> Result<Option<Vec<Mismatch>>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    Ok(st.session.as_ref().unwrap().mismatches(&device, &top))
}

/// Remove sync leftovers identical to their file now.
#[tauri::command]
async fn tidy_now(state: State<'_>) -> Result<Vec<Removed>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let r = st.session.as_ref().unwrap().tidy().map_err(err)?;
    st.local = None;
    Ok(r)
}

/// What tidy removed, newest first.
#[tauri::command]
async fn tidy_log() -> Result<Vec<Removed>, String> {
    let mut log = setu_core::tidy::read_log(&config_dir());
    log.reverse();
    Ok(log)
}

#[tauri::command]
async fn undo_tidy(state: State<'_>, path: String) -> Result<Vec<Removed>, String> {
    let mut st = state.lock().unwrap();
    setu_core::tidy::undo(&config_dir(), std::path::Path::new(&path)).map_err(err)?;
    st.local = None;
    let mut log = setu_core::tidy::read_log(&config_dir());
    log.reverse();
    Ok(log)
}

#[tauri::command]
async fn set_allow(state: State<'_>, what: String, on: bool) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    let dir = config_dir();
    let mut cfg = LocalConfig::load(&dir).map_err(err)?;
    match what.as_str() {
        "metered" => cfg.allow_metered = on,
        "relay" => cfg.allow_relay = on,
        _ => return Err(format!("unknown setting {what}")),
    }
    cfg.save(&dir).map_err(err)?;
    st.session = None;
    Ok(view(&mut st))
}

#[tauri::command]
async fn resume(state: State<'_>, folder: String) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    watch::release_hold(&s.dir, &s.client, &folder).map_err(err)?;
    Ok(view(&mut st))
}

#[tauri::command]
async fn conflicts(state: State<'_>) -> Result<Vec<Conflict>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.local = None;
    ensure_trees(&mut st)?;
    st.session.as_ref().unwrap().conflicts(st.local.as_ref().unwrap()).map_err(err)
}

#[tauri::command]
async fn keep(state: State<'_>, copy: String, keep_copy: bool) -> Result<Vec<Conflict>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().resolve(&copy, keep_copy).map_err(err)?;
    st.local = None;
    ensure_trees(&mut st)?;
    st.session.as_ref().unwrap().conflicts(st.local.as_ref().unwrap()).map_err(err)
}

#[tauri::command]
async fn versions(state: State<'_>) -> Result<Vec<Kept>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().versions().map_err(err)
}

#[tauri::command]
async fn restore(state: State<'_>, folder: String, path: String, version_time: String) -> Result<Restored, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let r = st.session.as_ref().unwrap().restore(&folder, &path, &version_time).map_err(err)?;
    st.local = None;
    Ok(r)
}

#[tauri::command]
async fn verify(state: State<'_>, rescan: bool) -> Result<Vec<FolderCheck>, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().verify(rescan).map_err(err)
}

/// What the sync engine is doing (starting, waiting for Tailscale, an old setup to move, …).
#[tauri::command]
async fn engine(host: tauri::State<'_, Arc<host::Host>>) -> Result<host::Status, String> {
    Ok(host.status())
}

/// The person agreed to move the old, separately running Syncthing into Setu.
#[tauri::command]
async fn move_engine(host: tauri::State<'_, Arc<host::Host>>) -> Result<(), String> {
    host.agree()
}

/// This Setu's version.
#[tauri::command]
fn version() -> &'static str {
    update::VERSION
}

/// A newer signed release, or `null` when this one is the newest.
#[tauri::command]
async fn check_update() -> Result<Option<update::Update>, String> {
    tauri::async_runtime::spawn_blocking(update::check).await.map_err(|e| e.to_string())?
}

/// Download, check and install `version`. Setu quits shortly after and starts again updated.
#[tauri::command]
async fn install_update(version: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let u = update::check()?.filter(|u| u.version == version).ok_or("that update is no longer offered")?;
        update::install(&u)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Look for updates a minute after start, then twice a day. A new version is told once: a
/// notification, and an `update-available` event for the window.
fn watch_updates(app: tauri::AppHandle) {
    use tauri::Emitter;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let mut told = None;
        loop {
            match update::check() {
                Ok(Some(u)) if told.as_ref() != Some(&u.version) => {
                    notify::show(&watch::notice(
                        "Setu: an update is ready",
                        format!("Setu {} is out (you have {}). Open Setu to update.", u.version, update::VERSION),
                    ));
                    let _ = app.emit("update-available", &u);
                    told = Some(u.version);
                }
                Ok(_) => {}
                Err(e) => eprintln!("setu: update check: {e}"),
            }
            std::thread::sleep(std::time::Duration::from_secs(12 * 3600));
        }
    });
}

#[tauri::command]
async fn autostart() -> Result<bool, String> {
    Ok(host::autostart::enabled())
}

#[tauri::command]
async fn set_autostart(on: bool) -> Result<bool, String> {
    host::autostart::set(on)?;
    Ok(host::autostart::enabled())
}

/// The window opens at its configured size, which can be taller or wider than a small or
/// scaled screen (a 1366×768 laptop, or 1080p at 150 %). Then the bottom bar and the buttons
/// on the right are off the screen and cannot be reached. Shrink it to the screen's free area
/// (without the taskbar or panels) and centre it there.
fn fit_to_screen(w: &tauri::WebviewWindow) {
    let monitor = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten());
    let (Some(m), Ok(size)) = (monitor, w.outer_size()) else { return };
    let area = m.work_area();
    let width = size.width.min(area.size.width * 96 / 100);
    let height = size.height.min(area.size.height * 94 / 100);
    if (width, height) != (size.width, size.height) {
        let _ = w.set_size(tauri::PhysicalSize::new(width, height));
    }
    let x = area.position.x + (area.size.width.saturating_sub(width) / 2) as i32;
    let y = area.position.y + (area.size.height.saturating_sub(height) / 2) as i32;
    // Not every Linux desktop lets an app place its window (Wayland); the size still fits.
    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
}

/// `setu-app [--background | --quit]`. `--background` (used at sign-in) starts without the
/// window. `--quit` asks the running Setu to stop (installers use it before replacing files)
/// and waits up to 15 s for it to be gone.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    use setu_core::pipe::{self, Handler};
    use setu_core::syncthing::Transport;

    if std::env::args().any(|a| a == "--quit") {
        if let Err(e) = pipe::quit_running() {
            eprintln!("setu: {e}");
            std::process::exit(1);
        }
        return;
    }

    // One Setu per user: the pipe is the lock. A second start shows the first one's window.
    let listener = match pipe::listen() {
        Ok(Some(l)) => Some(l),
        Ok(None) => {
            if let Err(e) = pipe::Pipe.call("show", &serde_json::Value::Null) {
                eprintln!("setu: another Setu is running but did not answer: {e}");
            }
            return;
        }
        Err(e) => {
            eprintln!("setu: {e}; the setu command will not reach this Setu");
            None
        }
    };
    let background = std::env::args().any(|a| a == "--background");
    let host = Arc::new(host::Host::default());

    tauri::Builder::default()
        .manage(Mutex::new(AppState::default()))
        .manage(live::Live::default())
        .manage(host.clone())
        .setup(move |app| {
            if let Some(w) = app.get_webview_window("main") {
                fit_to_screen(&w);
                if !background {
                    let _ = w.show();
                }
            }
            tray::setup(app)?;
            let handle = app.handle().clone();
            if let Some(l) = listener {
                let h = handle.clone();
                let handler: Handler = Arc::new(move |method, args| match method {
                    "show" => {
                        tray::show_main(&h);
                        Ok(serde_json::Value::Null)
                    }
                    "quit" => {
                        h.exit(0);
                        Ok(serde_json::Value::Null)
                    }
                    _ => setu_engine::call(method, args),
                });
                l.serve(handler);
            }
            host::start(handle.clone(), host);
            watch_updates(handle.clone());
            live::start(handle, app.state::<live::Live>().inner().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            state, refresh, children, set_rule, set_leave_out, set_pause, pause_folder, progress, set_quiet_progress, limits, set_limits, open_path,
            discard, plan, apply, init, sign_in, recovery, sign_out, join, check_now, accuracy, differences, tidy_now, tidy_log, undo_tidy, set_allow,
            resume, conflicts, keep, versions, restore, verify, engine, move_engine, autostart, set_autostart, version, check_update, install_update
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Setu")
        .run(|_, event| {
            if let tauri::RunEvent::Exit = event {
                // Let the engine save its index and close its connections.
                let _ = setu_engine::stop();
            }
        });
}
