//! Setu's window: thin commands over `setu-core`. All rules, planning and applying live in the
//! core, so the app and the `setu` command always agree.

mod tray;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
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
#[tauri::command]
async fn set_pause(state: State<'_>, until: Option<u64>) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    let dir = config_dir();
    let mut cfg = LocalConfig::load(&dir).map_err(err)?;
    cfg.paused_until = until;
    cfg.save(&dir).map_err(err)?;
    st.session = None;
    open(&mut st)?;
    let s = st.session.as_ref().unwrap();
    if watch::read_state(&s.dir).alive_at + 120 < now_secs() {
        watch::gate_now(&s.dir, &cfg, &s.client).map_err(err)?;
    }
    Ok(view(&mut st))
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(AppState::default()))
        .setup(|app| {
            tray::setup(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, refresh, children, set_rule, set_leave_out, set_pause, discard, plan, apply, init, sign_in, recovery, sign_out, join, check_now, accuracy, differences, tidy_now, tidy_log, undo_tidy, set_allow, resume, conflicts, keep, versions, restore, verify])
        .run(tauri::generate_context!())
        .expect("error while running Setu");
}
