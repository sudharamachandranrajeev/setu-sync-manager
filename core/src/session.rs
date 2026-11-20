//! Everything the app and the command line do, in one place: open this computer's setup, edit a
//! draft of the rules, see the plan, and apply it.
//!
//! Edits go to a local draft first (never shared). Applying publishes the draft to the control
//! folder as the next revision and applies it here; the other computer then applies the same
//! revision on its side.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::apply::{self, Ctx, Plan, Report};
use crate::control::Control;
use crate::index;
use crate::local::{config_dir, write_atomic, LocalConfig};
use crate::rules::{Device, Os, RuleSet};
use crate::syncthing::Client;
use crate::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    /// The shared revision this draft started from.
    pub base: u64,
    pub rules: RuleSet,
}

pub struct Session {
    pub dir: PathBuf,
    pub local: LocalConfig,
    pub client: Client,
    pub control: Control,
    pub ctx: Ctx,
}

impl Session {
    pub fn open() -> Result<Session, Error> {
        let dir = config_dir();
        let local = LocalConfig::load(&dir)?;
        let client = Client::discover(local.syncthing_home.as_deref())?;
        let control = Control::find(&client)?;
        crate::control::check_outside(&control.dir, &local.root)?;
        let me = client.my_id()?;
        let ctx = Ctx { me, root: local.root.clone(), os: Os::current() };
        Ok(Session { dir, local, client, control, ctx })
    }

    pub fn shared(&self) -> Result<Option<RuleSet>, Error> {
        self.control.read_rules()
    }

    fn draft_path(&self) -> PathBuf {
        self.dir.join("draft.json")
    }

    pub fn draft(&self) -> Result<Option<Draft>, Error> {
        match std::fs::read_to_string(self.draft_path()) {
            Ok(s) => {
                let d: Draft = serde_json::from_str(&s).map_err(|e| Error::Setup(format!("draft.json: {e}")))?;
                d.rules.validate()?;
                Ok(Some(d))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// The rules being worked on: the draft if there is one, else the shared rules.
    pub fn working(&self) -> Result<RuleSet, Error> {
        if let Some(d) = self.draft()? {
            return Ok(d.rules);
        }
        self.shared()?.ok_or_else(|| Error::Setup("no rules yet; create them first".into()))
    }

    pub fn save_draft(&self, rules: RuleSet) -> Result<(), Error> {
        rules.validate()?;
        let base = match self.draft()? {
            Some(d) => d.base,
            None => self.shared()?.map(|s| s.revision).unwrap_or(0),
        };
        std::fs::create_dir_all(&self.dir)?;
        let d = Draft { base, rules };
        write_atomic(&self.draft_path(), &(serde_json::to_string_pretty(&d).expect("serialise") + "\n"))
    }

    pub fn discard_draft(&self) -> Result<(), Error> {
        match std::fs::remove_file(self.draft_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// The rules that applying would publish (or re-apply), and the plan for this computer.
    pub fn plan(&self) -> Result<(RuleSet, Plan), Error> {
        let shared = self.shared()?;
        let rules = match (self.draft()?, &shared) {
            (Some(d), Some(s)) if d.base != s.revision => {
                return Err(Error::Rules(format!(
                    "the rules changed on the other computer (revision {} → {}) after this draft began. Discard the draft and edit again.",
                    d.base, s.revision
                )));
            }
            (Some(d), Some(s)) if same_choices(&d.rules, s) => s.clone(),
            (Some(d), _) => {
                let mut r = d.rules;
                r.revision = d.base + 1;
                r
            }
            (None, Some(s)) => s.clone(),
            (None, None) => return Err(Error::Setup("no rules yet; create them first".into())),
        };
        let plan = apply::plan(&self.ctx, &rules, &self.client)?;
        Ok((rules, plan))
    }

    /// Apply, but only if the plan is still exactly the one that was shown and confirmed.
    pub fn apply(&self, confirmed: &Plan) -> Result<Report, Error> {
        let (rules, plan) = self.plan()?;
        if plan.actions != confirmed.actions || plan.revision != confirmed.revision {
            return Err(Error::Rules("something changed since the plan was shown; review it again".into()));
        }
        if !plan.problems.is_empty() {
            return Err(Error::Rules(format!("{} problem(s) must be fixed first", plan.problems.len())));
        }
        if self.shared()?.map(|s| s.revision) != Some(rules.revision) {
            self.control.write_rules(&rules)?;
        }
        let report = apply::execute(&self.ctx, &rules, &plan, &self.client, Some(&self.control))?;
        self.discard_draft()?;
        Ok(report)
    }

    /// First rules for this computer and the one it shares `setu-control` with: everything off.
    pub fn init_rules(&self, other_os: Os) -> Result<RuleSet, Error> {
        if let Some(s) = self.shared()? {
            return Ok(s);
        }
        let folders = self.client.folders()?;
        let control = folders
            .iter()
            .find(|f| f["id"] == crate::control::CONTROL_FOLDER)
            .ok_or_else(|| Error::Setup("no setu-control folder".into()))?;
        let others: Vec<String> = control["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| d["deviceID"].as_str().map(String::from))
            .filter(|id| *id != self.ctx.me)
            .collect();
        let [other] = others.as_slice() else {
            return Err(Error::Setup(format!("setu-control must be shared with exactly one other computer (found {})", others.len())));
        };
        let names: BTreeMap<String, String> = self
            .client
            .devices()?
            .iter()
            .filter_map(|d| Some((d["deviceID"].as_str()?.to_string(), d["name"].as_str().unwrap_or("").to_string())))
            .collect();
        let name = |id: &str| names.get(id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| crate::rules::short_id(id).to_string());
        let mut devices = BTreeMap::new();
        devices.insert(self.ctx.me.clone(), Device { name: name(&self.ctx.me), os: self.ctx.os });
        devices.insert(other.clone(), Device { name: name(other), os: other_os });
        let rs = RuleSet::new(devices);
        self.control.write_rules(&rs)?;
        Ok(rs)
    }

    /// Write this computer's file list to the control folder. Returns how many entries.
    pub fn write_index(&self) -> Result<usize, Error> {
        let rules = self.working()?;
        let entries = index::scan(&self.local.root, &rules.always_ignore)?;
        index::write(&self.control.index_path(&self.ctx.me), &entries)?;
        Ok(entries.len())
    }

    pub fn other_device(&self, rules: &RuleSet) -> Option<String> {
        rules.devices.keys().find(|id| **id != self.ctx.me).cloned()
    }
}

fn same_choices(a: &RuleSet, b: &RuleSet) -> bool {
    a.root == b.root && a.rules == b.rules && a.always_ignore == b.always_ignore && a.devices == b.devices
}
