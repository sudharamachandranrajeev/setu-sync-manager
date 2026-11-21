//! Brings this computer's Syncthing in line with the rules, in two steps: `plan` works out what
//! would change and why, `execute` does it. Nothing changes until `execute` runs, and `execute`
//! never deletes a file: removing a Syncthing folder leaves its files where they are.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::compile::{compile, folder_type, render, Problem, FOLDER_PREFIX, HEADER};
use crate::control::now_secs;
use crate::local::{expand_home, join_rel};
use crate::rules::{Os, RuleSet};
use crate::syncthing::Client;
use crate::Error;

/// Versions of replaced and deleted files are kept this long (seconds).
pub const KEEP_VERSIONS_SECS: u64 = 30 * 24 * 3600;

/// This computer, as the rules see it.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub me: String,
    pub root: PathBuf,
    pub os: Os,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Action {
    #[serde(rename_all = "camelCase")]
    Remove { id: String, rel: String, path: String },
    #[serde(rename_all = "camelCase")]
    Update { id: String, rel: String, path: String, changes: Vec<String>, folder_type: String, lines: Vec<String>, set_type: bool, set_devices: bool, set_ignores: bool },
    #[serde(rename_all = "camelCase")]
    Add { id: String, rel: String, path: String, folder_type: String, files_only: bool, lines: Vec<String> },
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub revision: u64,
    pub actions: Vec<Action>,
    /// Blocking: nothing can be applied until these are fixed.
    pub problems: Vec<Problem>,
    /// Not blocking, but worth reading before confirming.
    pub warnings: Vec<String>,
    /// Setu folders that already match.
    pub unchanged: usize,
    /// How many Syncthing folders the rules make.
    pub folders: usize,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

fn same_path(a: &Path, b: &Path, os: Os) -> bool {
    if os == Os::Windows {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

fn inside(child: &Path, parent: &Path, os: Os) -> bool {
    if os == Os::Windows {
        let (c, p) = (child.to_string_lossy().to_lowercase(), parent.to_string_lossy().to_lowercase());
        let p = p.trim_end_matches('\\').to_string() + "\\";
        c.starts_with(&p)
    } else {
        child.starts_with(parent) && child != parent
    }
}

fn label(rel: &str, root: &Path) -> String {
    let name = if rel.is_empty() {
        root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "root".into())
    } else {
        rel.to_string()
    };
    format!("Setu · {name}")
}

pub fn plan(ctx: &Ctx, rs: &RuleSet, client: &Client) -> Result<Plan, Error> {
    let mut plan = Plan { revision: rs.revision, ..Default::default() };
    if !rs.devices.contains_key(&ctx.me) {
        plan.problems.push(Problem { path: String::new(), message: "These rules do not name this computer.".into() });
        return Ok(plan);
    }
    let specs = match compile(rs) {
        Ok(s) => s,
        Err(p) => {
            plan.problems = p;
            return Ok(plan);
        }
    };
    plan.folders = specs.len();

    let known: BTreeSet<String> = client.devices()?.iter().filter_map(|d| d["deviceID"].as_str().map(String::from)).collect();
    for (id, d) in &rs.devices {
        if id != &ctx.me && !known.contains(id) {
            plan.problems.push(Problem {
                path: String::new(),
                message: format!("Syncthing on this computer does not know {} yet. Add it as a device first.", d.name),
            });
        }
    }
    let want_devices: BTreeSet<String> = rs.devices.keys().cloned().collect();

    let folders = client.folders()?;
    let mut current: BTreeMap<String, &Value> = BTreeMap::new();
    let mut others: Vec<(&str, PathBuf)> = Vec::new();
    for f in &folders {
        let id = f["id"].as_str().unwrap_or_default();
        if id.starts_with(FOLDER_PREFIX) {
            current.insert(id.to_string(), f);
        } else {
            others.push((id, expand_home(f["path"].as_str().unwrap_or_default())));
        }
    }

    let mut removes = Vec::new();
    let mut updates = Vec::new();
    let mut adds = Vec::new();
    for spec in &specs {
        let path = join_rel(&ctx.root, &spec.rel);
        let ftype = folder_type(&spec.mode, &ctx.me, rs.receive_only(&ctx.me)).to_string();
        let lines = render(spec, &rs.always_ignore, ctx.os);
        let shown = path.to_string_lossy().into_owned();

        for (oid, opath) in &others {
            if same_path(opath, &path, ctx.os) || inside(opath, &path, ctx.os) || inside(&path, opath, ctx.os) {
                plan.problems.push(Problem {
                    path: spec.rel.clone(),
                    message: format!(
                        "Another Syncthing folder ({oid}) already covers {}, which overlaps {shown}. A file must never be in two Syncthing folders: remove that folder from Syncthing first (its files stay).",
                        opath.display()
                    ),
                });
            }
        }
        if path.exists() && !path.is_dir() {
            plan.problems.push(Problem { path: spec.rel.clone(), message: format!("{shown} is a file here, but the rules treat it as a folder.") });
            continue;
        }

        match current.remove(&spec.id) {
            Some(cur) if same_path(&expand_home(cur["path"].as_str().unwrap_or_default()), &path, ctx.os) => {
                let cur_type = cur["type"].as_str().unwrap_or_default();
                let cur_devices: BTreeSet<String> = cur["devices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d["deviceID"].as_str().map(String::from))
                    .collect();
                let set_type = cur_type != ftype;
                let set_devices = cur_devices != want_devices;
                let set_ignores = client.ignores(&spec.id)? != lines;
                let mut changes = Vec::new();
                if set_type {
                    changes.push(format!("direction here: {} → {}", describe(cur_type), describe(&ftype)));
                    if ftype == "receiveonly" {
                        plan.warnings.push(format!(
                            "{shown}: this computer will now only receive. Files that differ here will be replaced by the other computer's copies; the old copies are kept for 30 days."
                        ));
                    }
                }
                if set_devices {
                    changes.push("devices".into());
                }
                if set_ignores {
                    changes.push("what is left out".into());
                }
                if changes.is_empty() {
                    plan.unchanged += 1;
                } else {
                    updates.push(Action::Update { id: spec.id.clone(), rel: spec.rel.clone(), path: shown, changes, folder_type: ftype, lines, set_type, set_devices, set_ignores });
                }
            }
            Some(cur) => {
                // Same folder ID at a different path: remove and add again, so the other computer
                // never sees "every file deleted".
                removes.push(Action::Remove { id: spec.id.clone(), rel: spec.rel.clone(), path: cur["path"].as_str().unwrap_or_default().to_string() });
                adds.push(Action::Add { id: spec.id.clone(), rel: spec.rel.clone(), path: shown, folder_type: ftype, files_only: spec.files_only, lines });
            }
            None => adds.push(Action::Add { id: spec.id.clone(), rel: spec.rel.clone(), path: shown, folder_type: ftype, files_only: spec.files_only, lines }),
        }
    }
    for (id, cur) in current {
        removes.push(Action::Remove { id, rel: String::new(), path: cur["path"].as_str().unwrap_or_default().to_string() });
    }
    // Removals first, so a folder that rejoins its parent is gone before the parent covers it;
    // then updates, so a parent leaves out a new nested folder before that folder is added.
    plan.actions = removes.into_iter().chain(updates).chain(adds).collect();
    Ok(plan)
}

fn describe(t: &str) -> &'static str {
    match t {
        "sendonly" => "send only",
        "receiveonly" => "receive only",
        "sendreceive" => "both ways",
        _ => "other",
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Report {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

pub fn execute(ctx: &Ctx, rs: &RuleSet, plan: &Plan, client: &Client) -> Result<Report, Error> {
    if !plan.problems.is_empty() {
        return Err(Error::Rules(format!("{} problem(s) must be fixed first", plan.problems.len())));
    }
    let devices: Vec<Value> = rs.devices.keys().map(|id| json!({ "deviceID": id })).collect();
    let mut report = Report::default();
    let mut defaults: Option<Value> = None;
    for a in &plan.actions {
        match a {
            Action::Remove { id, .. } => {
                client.delete_folder(id)?;
                report.removed += 1;
            }
            Action::Update { id, folder_type, lines, set_type, set_devices, set_ignores, .. } => {
                if *set_ignores {
                    client.set_ignores(id, lines)?;
                }
                if *set_type || *set_devices {
                    client.patch_folder(id, &json!({ "type": folder_type, "devices": devices }))?;
                }
                report.updated += 1;
            }
            Action::Add { id, rel, path, folder_type, lines, .. } => {
                let path = PathBuf::from(path);
                std::fs::create_dir_all(&path)?;
                write_stignore(&path, id, lines)?;
                if defaults.is_none() {
                    defaults = Some(client.default_folder()?);
                }
                let mut f = defaults.clone().unwrap();
                f["id"] = json!(id);
                f["label"] = json!(label(rel, &ctx.root));
                f["path"] = json!(path.to_string_lossy());
                f["type"] = json!(folder_type);
                f["devices"] = json!(devices);
                f["paused"] = json!(false);
                f["fsWatcherEnabled"] = json!(true);
                f["versioning"]["type"] = json!("staggered");
                f["versioning"]["params"] = json!({ "maxAge": KEEP_VERSIONS_SECS.to_string(), "cleanInterval": "3600" });
                client.put_folder(&f)?;
                report.added += 1;
            }
        }
    }
    Ok(report)
}

/// Write a folder's ignore file before Syncthing first sees the folder, so it never scans or
/// offers what should be left out. An ignore file Setu did not write is first copied to Setu's
/// own settings folder (not into the synced folder, where the copy would sync too).
fn write_stignore(dir: &Path, id: &str, lines: &[String]) -> Result<(), Error> {
    let p = dir.join(".stignore");
    if let Ok(old) = std::fs::read_to_string(&p)
        && !old.starts_with(HEADER)
    {
        let b = crate::local::config_dir().join("stignore-backups");
        std::fs::create_dir_all(&b)?;
        std::fs::write(b.join(format!("{id}-{}.stignore", now_secs())), old)?;
    }
    std::fs::write(&p, lines.join("\n") + "\n")?;
    Ok(())
}
