//! Setu's window: thin commands over `setu-core`. All rules, planning and applying live in the
//! core, so the app and the `setu` command always agree.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use setu_core::apply::{Plan, Report};
use setu_core::index::{self, Tree};
use setu_core::local::{config_dir, LocalConfig};
use setu_core::rules::{short_id, Kind, Mode, Os, RuleSet};
use setu_core::session::Session;
use setu_core::Error;

#[derive(Default)]
struct AppState {
    session: Option<Session>,
    local: Option<Tree>,
    remote: Option<Tree>,
    remote_age: Option<u64>,
    shown_plan: Option<Plan>,
}

type State<'a> = tauri::State<'a, Mutex<AppState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeviceView {
    id: String,
    short: String,
    name: String,
    os: Os,
    connected: bool,
    address: Option<String>,
    applied_revision: Option<u64>,
    applied_at: Option<u64>,
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
    /// "setup" (choose this computer's folder), "rules" (create the first rules) or "ready".
    stage: &'static str,
    message: Option<String>,
    root: Option<String>,
    me: Option<DeviceView>,
    other: Option<DeviceView>,
    shared_revision: Option<u64>,
    /// The choice for the whole folder, as the draft has it.
    root_mode: Option<String>,
    edits: Vec<EditView>,
    conflicts: Vec<String>,
    local_entries: usize,
    remote_entries: Option<usize>,
    remote_list_age: Option<u64>,
    always_ignore: Vec<String>,
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
    edits: Vec<EditView>,
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
    let rules = s.working().map_err(err)?;
    if st.local.is_none() {
        let entries = index::scan(&s.local.root, &rules.always_ignore).map_err(err)?;
        index::write(&s.control.index_path(&s.ctx.me), &entries).map_err(err)?;
        st.local = Some(Tree::new(entries));
    }
    if st.remote.is_none()
        && let Some(other) = s.other_device(&rules)
    {
        let p = s.control.index_path(&other);
        if let Ok(entries) = index::read(&p) {
            st.remote_age = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .map(|d| d.as_secs());
            st.remote = Some(Tree::new(entries));
        }
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

fn view(st: &mut AppState) -> View {
    let mut v = View {
        stage: "setup",
        message: None,
        root: None,
        me: None,
        other: None,
        shared_revision: None,
        root_mode: None,
        edits: Vec::new(),
        conflicts: Vec::new(),
        local_entries: 0,
        remote_entries: None,
        remote_list_age: None,
        always_ignore: Vec::new(),
    };
    if let Err(e) = open(st) {
        v.message = Some(e);
        return v;
    }
    let s = st.session.as_ref().unwrap();
    v.root = Some(s.local.root.to_string_lossy().into_owned());
    v.conflicts = s.control.conflicts();
    let shared = match s.shared() {
        Ok(Some(r)) => r,
        Ok(None) => {
            v.stage = "rules";
            return v;
        }
        Err(e) => {
            v.message = Some(err(e));
            return v;
        }
    };
    let working = s.working().unwrap_or_else(|_| shared.clone());
    let conns = s.client.connections().unwrap_or_default();
    let dev = |id: &String| {
        let d = &shared.devices[id];
        let c = &conns["connections"][id];
        let status = s.control.read_status(id);
        DeviceView {
            id: id.clone(),
            short: short_id(id).to_string(),
            name: d.name.clone(),
            os: d.os,
            connected: c["connected"].as_bool().unwrap_or(false),
            address: c["address"].as_str().filter(|a| !a.is_empty()).map(String::from),
            applied_revision: status.as_ref().map(|x| x.applied_revision),
            applied_at: status.as_ref().map(|x| x.applied_at),
        }
    };
    v.me = Some(dev(&s.ctx.me));
    v.other = s.other_device(&shared).map(|o| dev(&o));
    v.shared_revision = Some(shared.revision);
    v.root_mode = Some(working.root.to_string());
    v.edits = edits(Some(&shared), &working);
    v.always_ignore = working.always_ignore.clone();
    v.stage = "ready";
    if let Err(e) = ensure_trees(st) {
        v.message = Some(e);
    }
    v.local_entries = st.local.as_ref().map(|t| t.entries.len()).unwrap_or(0);
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
    let s = st.session.as_ref().unwrap();
    let (rules, plan) = s.plan().map_err(err)?;
    let shared = s.shared().map_err(err)?;
    let edits = edits(shared.as_ref(), &rules);
    st.shown_plan = Some(plan.clone());
    Ok(PlanView { plan, edits })
}

#[tauri::command]
async fn apply(state: State<'_>) -> Result<Report, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    let shown = st.shown_plan.take().ok_or("review the plan first")?;
    st.session.as_ref().unwrap().apply(&shown).map_err(err)
}

#[tauri::command]
async fn init(state: State<'_>, root: String) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    let root = PathBuf::from(root.trim());
    if !root.is_absolute() || !root.is_dir() {
        return Err(format!("{} is not an existing folder", root.display()));
    }
    LocalConfig { root, syncthing_home: None }.save(&config_dir()).map_err(err)?;
    *st = AppState::default();
    Ok(view(&mut st))
}

#[tauri::command]
async fn create_rules(state: State<'_>, other_os: Os) -> Result<View, String> {
    let mut st = state.lock().unwrap();
    open(&mut st)?;
    st.session.as_ref().unwrap().init_rules(other_os).map_err(err)?;
    Ok(view(&mut st))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(AppState::default()))
        .invoke_handler(tauri::generate_handler![state, refresh, children, set_rule, discard, plan, apply, init, create_rules])
        .run(tauri::generate_context!())
        .expect("error while running Setu");
}
