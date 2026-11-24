//! Everything the app and the command line do, in one place: open this computer's setup, edit a
//! draft of the rules, see the plan, and apply it.
//!
//! Edits go to a local draft first (never shared). Applying publishes the draft to the Brahma
//! account as the next revision and applies it here. Every other computer's helper then applies
//! the same revision by itself (`auto_apply`), unless it would replace or delete enough to need
//! the person there; then it holds and says so.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::apply::{self, Ctx, Plan, Report};
use crate::compile;
use crate::control::{now_secs, Control};
use crate::hub::{DeviceState, Held, Hub};
use crate::index::{self, Tree};
use crate::mesh;
use crate::local::{config_dir, join_rel, write_atomic, LocalConfig};
use crate::preview::{self, Preview};
use crate::rules::{Device, Os, RuleSet};
use crate::syncthing::Client;
use crate::accuracy::{self, Digest, HashCache};
use crate::tidy;
use crate::trust;
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
    pub hub: Hub,
}

/// What the helper did with the shared rules.
#[derive(Clone, Debug, PartialEq)]
pub enum Auto {
    /// Nothing new, or this computer is not in the rules yet.
    Nothing,
    Applied { revision: u64, report: Report },
    /// Waiting for the person: the change replaces or deletes a lot, or has problems.
    Held { revision: u64, reasons: Vec<String> },
    /// Waiting for another computer's file list, so the change can be checked first. Tried
    /// again next round.
    Waiting(String),
}

impl Session {
    pub fn open() -> Result<Session, Error> {
        let dir = config_dir();
        let local = LocalConfig::load(&dir)?;
        let client = Client::connect()?;
        let hub = Hub::open(&dir);
        Session::new(dir, local, client, hub)
    }

    /// A session from its parts (`open` finds them; tests pass their own).
    pub fn new(dir: PathBuf, local: LocalConfig, client: Client, hub: Hub) -> Result<Session, Error> {
        mesh::ensure_control(&client)?;
        let control = Control::find(&client)?;
        crate::control::check_outside(&control.dir, &local.root)?;
        let me = client.my_id()?;
        let ctx = Ctx { me, root: local.root.clone(), os: Os::current() };
        Ok(Session { dir, local, client, control, ctx, hub })
    }

    /// The shared rules, from the Brahma account. Rules from before the account (alpha.3 kept
    /// them in the control folder) move over once.
    pub fn shared(&self) -> Result<Option<RuleSet>, Error> {
        if let Some(r) = self.hub.rules()? {
            return Ok(Some(r));
        }
        match self.control.read_rules()? {
            Some(old) => {
                self.hub.put_rules(&old)?;
                Ok(Some(old))
            }
            None => Ok(None),
        }
    }

    /// Send and fetch the shared rows now. A failure is reported, never fatal: the helper tries
    /// again within a minute.
    pub fn sync_hub(&self) -> Result<usize, Error> {
        self.hub.sync()
    }

    /// This computer's row, changed by `f`, with fresh name, system, addresses and time.
    pub fn update_me(&self, rules: Option<&RuleSet>, f: impl FnOnce(&mut DeviceState)) -> Result<DeviceState, Error> {
        let mut d = self.hub.device(&self.ctx.me)?.unwrap_or_default();
        d.id = self.ctx.me.clone();
        d.os = Some(self.ctx.os);
        if let Some(n) = rules.and_then(|r| r.devices.get(&self.ctx.me)).map(|x| x.name.clone()).or_else(|| self.syncthing_name()) {
            d.name = n;
        }
        let addrs = mesh::listen_addresses(&self.client);
        if !addrs.is_empty() {
            d.addresses = addrs;
        }
        d.version = env!("CARGO_PKG_VERSION").into();
        f(&mut d);
        d.updated_at = now_secs();
        self.hub.put_device(&d)?;
        Ok(d)
    }

    fn syncthing_name(&self) -> Option<String> {
        let devs = self.client.devices().ok()?;
        devs.iter().find(|d| d["deviceID"] == self.ctx.me.as_str()).and_then(|d| d["name"].as_str()).filter(|n| !n.is_empty()).map(String::from)
    }

    fn record_applied(&self, rules: &RuleSet, plan: &Plan) -> Result<(), Error> {
        self.update_me(Some(rules), |d| {
            d.applied_revision = rules.revision;
            d.applied_at = now_secs();
            d.folders = plan.folders;
            d.problems.clear();
            d.held = None;
        })?;
        Ok(())
    }

    /// The helper's step: apply new shared rules here by itself when that is safe. Safe means
    /// no problems and nothing the preview would ask a second yes for (many files replaced or
    /// removed, or the other computer's file list missing while folders are added).
    pub fn auto_apply(&self) -> Result<Auto, Error> {
        let Some(rules) = self.shared()? else { return Ok(Auto::Nothing) };
        if !rules.devices.contains_key(&self.ctx.me) {
            return Ok(Auto::Nothing);
        }
        let mine = self.hub.device(&self.ctx.me)?.unwrap_or_default();
        if mine.applied_revision >= rules.revision && mine.applied_at > 0 {
            return Ok(Auto::Nothing);
        }
        if mine.held.as_ref().is_some_and(|h| h.revision == rules.revision) {
            return Ok(Auto::Nothing);
        }
        let plan = apply::plan(&self.ctx, &rules, &self.client)?;
        let hold = |reasons: Vec<String>| -> Result<Auto, Error> {
            let revision = rules.revision;
            self.update_me(Some(&rules), |d| d.held = Some(Held { revision, reasons: reasons.clone() }))?;
            Ok(Auto::Held { revision, reasons })
        };
        if !plan.problems.is_empty() {
            return hold(plan.problems.iter().map(|p| if p.path.is_empty() { p.message.clone() } else { format!("{}: {}", p.path, p.message) }).collect());
        }
        if !plan.is_empty() {
            let entries = index::scan(&self.local.root, &rules.always_ignore)?;
            index::write(&self.control.index_path(&self.ctx.me), &entries, &self.local.root.to_string_lossy())?;
            let here = Tree::new(entries);
            let other = self.other_list().map(|(l, _)| (Tree::new(l.entries), l.root));
            let preview = self.preview(&rules, &plan, &here, other.as_ref().map(|(t, r)| (t, r.as_deref())));
            if other.is_none() && rules.devices.len() > 1 && plan.actions.iter().any(|a| matches!(a, apply::Action::Add { .. })) {
                return Ok(Auto::Waiting("waiting for the other computer's file list".into()));
            }
            if !preview.guard.is_empty() {
                return hold(preview.guard);
            }
        }
        let report = apply::execute(&self.ctx, &rules, &plan, &self.client)?;
        self.record_applied(&rules, &plan)?;
        Ok(Auto::Applied { revision: rules.revision, report })
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

    /// Apply, but only if the plan is still exactly the one that was shown and confirmed, and any
    /// guard reasons in the shown preview were confirmed a second time.
    pub fn apply(&self, confirmed: &Plan, preview: &Preview, guard_confirmed: bool) -> Result<Report, Error> {
        if !preview.guard.is_empty() && !guard_confirmed {
            return Err(Error::Rules(format!("this change needs a second confirmation: {}", preview.guard.join("; "))));
        }
        let (rules, plan) = self.plan()?;
        if plan.actions != confirmed.actions || plan.revision != confirmed.revision {
            return Err(Error::Rules("something changed since the plan was shown; review it again".into()));
        }
        if !plan.problems.is_empty() {
            return Err(Error::Rules(format!("{} problem(s) must be fixed first", plan.problems.len())));
        }
        if self.shared()?.map(|s| s.revision) != Some(rules.revision) {
            self.hub.put_rules(&rules)?;
        }
        let report = apply::execute(&self.ctx, &rules, &plan, &self.client)?;
        self.record_applied(&rules, &plan)?;
        self.discard_draft()?;
        // The other computers pick the rules up from the account; the helper retries if this fails.
        if let Err(e) = self.sync_hub() {
            eprintln!("setu: rules saved here, not sent yet: {e}");
        }
        Ok(report)
    }

    /// The shared rules with this computer in them. With no rules yet, the first rules name
    /// only this computer, everything off. A computer joining rules made elsewhere is added with
    /// `receive_only` (a backup copy) as chosen; its folders follow the rules from then on.
    pub fn join(&self, receive_only: bool) -> Result<RuleSet, Error> {
        let name = self.syncthing_name().unwrap_or_else(|| crate::rules::short_id(&self.ctx.me).to_string());
        let me = Device { name, os: self.ctx.os, receive_only };
        let mut rs = match self.shared()? {
            Some(s) => s,
            None => RuleSet::new(BTreeMap::new()),
        };
        if rs.devices.get(&self.ctx.me) == Some(&me) {
            return Ok(rs);
        }
        if receive_only {
            // A backup never sends, so rules that name it as the sender go back to both ways.
            let id = self.ctx.me.clone();
            if rs.root == crate::rules::Mode::From(id.clone()) {
                rs.root = crate::rules::Mode::Both;
            }
            for r in rs.rules.values_mut() {
                if r.mode == crate::rules::Mode::From(id.clone()) {
                    r.mode = crate::rules::Mode::Both;
                }
            }
        }
        rs.devices.insert(self.ctx.me.clone(), me);
        rs.revision += 1;
        rs.validate()?;
        self.hub.put_rules(&rs)?;
        self.update_me(Some(&rs), |_| {})?;
        if let Err(e) = self.sync_hub() {
            eprintln!("setu: joined here, not sent yet: {e}");
        }
        Ok(rs)
    }

    /// Write this computer's file list again when it is older than `max_age` seconds (or
    /// missing), so the other computers can check changes before applying them.
    pub fn refresh_index(&self, max_age: u64) -> Result<bool, Error> {
        let p = self.control.index_path(&self.ctx.me);
        let age = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs());
        if age.is_some_and(|a| a < max_age) {
            return Ok(false);
        }
        let ignore = self.shared()?.map(|r| r.always_ignore).unwrap_or_default();
        let entries = index::scan(&self.local.root, &ignore)?;
        index::write(&p, &entries, &self.local.root.to_string_lossy())?;
        Ok(true)
    }

    /// Scan this computer's folder and publish the list to the control folder.
    pub fn scan_here(&self) -> Result<Vec<index::Entry>, Error> {
        let rules = self.working()?;
        let entries = index::scan(&self.local.root, &rules.always_ignore)?;
        index::write(&self.control.index_path(&self.ctx.me), &entries, &self.local.root.to_string_lossy())?;
        Ok(entries)
    }

    /// The other computer's latest file list and its age in seconds, if it has arrived.
    pub fn other_list(&self) -> Option<(index::List, Option<u64>)> {
        let other = self.other_device(&self.working().ok()?)?;
        let p = self.control.index_path(&other);
        let list = index::read(&p).ok()?;
        let age = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs());
        Some((list, age))
    }

    /// What applying `rules` does to files, given both lists. Adds a guard reason when folders
    /// would be added but the other computer's list is unknown.
    pub fn preview(&self, rules: &RuleSet, plan: &Plan, here: &index::Tree, there: Option<(&index::Tree, Option<&str>)>) -> Preview {
        let mut p = preview::preview(rules, &self.ctx.me, here, there.map(|t| t.0), there.and_then(|t| t.1).map(|r| r.chars().count()));
        if p.other_unknown && rules.devices.len() > 1 && plan.actions.iter().any(|a| matches!(a, apply::Action::Add { .. })) {
            p.guard.push("The other computer's file list has not arrived, so changes on that side cannot be shown. It arrives by itself while Setu runs there; or confirm to go ahead without it.".into());
        }
        p
    }

    /// The rules both computers share (what is applied), as opposed to the local draft.
    fn applied(&self) -> Result<RuleSet, Error> {
        self.shared()?.ok_or_else(|| Error::Setup("this computer is not in the sync choices yet; sign in and join first".into()))
    }

    fn folders(&self, rules: &RuleSet) -> Result<Vec<compile::FolderSpec>, Error> {
        compile::compile(rules).map_err(|p| Error::Rules(format!("{} problem(s) in the rules", p.len())))
    }

    /// Conflict copies in synced folders, from a fresh scan of this computer.
    pub fn conflicts(&self, here: &index::Tree) -> Result<Vec<trust::Conflict>, Error> {
        Ok(trust::conflicts(&self.applied()?, here))
    }

    /// Settle a conflict; the copy not kept goes to `.stversions`.
    pub fn resolve(&self, copy: &str, keep_copy: bool) -> Result<PathBuf, Error> {
        trust::resolve(&self.local.root, &self.applied()?, copy, keep_copy)
    }

    /// Old versions kept in every Setu folder on this computer, newest first.
    pub fn versions(&self) -> Result<Vec<trust::Kept>, Error> {
        let rules = self.applied()?;
        let mut out = Vec::new();
        for f in self.folders(&rules)? {
            let Ok(list) = trust::versions(&self.client, &f.id) else { continue };
            out.extend(list.into_iter().map(|v| trust::Kept {
                folder: f.id.clone(),
                rel: if f.rel.is_empty() { v.path.clone() } else { format!("{}/{}", f.rel, v.path) },
                version: v,
            }));
        }
        out.sort_by(|a, b| b.version.version_time.cmp(&a.version.version_time).then_with(|| a.rel.cmp(&b.rel)));
        Ok(out)
    }

    /// Put an old version back (in place, or beside the file in a receive-only folder).
    pub fn restore(&self, folder: &str, path: &str, version_time: &str) -> Result<trust::Restored, Error> {
        let rules = self.applied()?;
        let f = self.folders(&rules)?.into_iter().find(|f| f.id == folder).ok_or_else(|| Error::Rules(format!("{folder} is not a Setu folder")))?;
        let v = trust::versions(&self.client, folder)?
            .into_iter()
            .find(|v| v.path == path && v.version_time == version_time)
            .ok_or_else(|| Error::Rules(format!("that version of {path} is no longer kept")))?;
        let receive_only = compile::folder_type(&f.mode, &self.ctx.me, rules.receive_only(&self.ctx.me)) == "receiveonly";
        trust::restore(&self.client, folder, &join_rel(&self.local.root, &f.rel), receive_only, &v)
    }

    /// Check every Setu folder here against Syncthing's records.
    pub fn verify(&self, rescan: bool) -> Result<Vec<trust::FolderCheck>, Error> {
        trust::verify(&self.client, &self.applied()?, &self.ctx.me, rescan)
    }

    /// The folders on disk that the applied rules sync here.
    pub fn synced_dirs(&self, rules: &RuleSet) -> Result<Vec<PathBuf>, Error> {
        Ok(self.folders(rules)?.iter().map(|f| join_rel(&self.local.root, &f.rel)).collect())
    }

    /// Remove sync leftovers (conflict copies, old versions) that are identical to their file.
    pub fn tidy(&self) -> Result<Vec<tidy::Removed>, Error> {
        let rules = self.applied()?;
        let root = self.local.root.clone();
        let allow = |p: &std::path::Path| {
            p.strip_prefix(&root).ok().and_then(|r| r.to_str()).map(|r| r.replace('\\', "/")).is_some_and(|r| !rules.effective(&r).0.is_off())
        };
        tidy::tidy(&self.dir, &self.synced_dirs(&rules)?, &allow)
    }

    /// Hash the synced files here, publish one digest per top folder, and write the file hashes
    /// of any top folder that differs from another computer, so either side can name the files.
    pub fn check_accuracy(&self) -> Result<BTreeMap<String, Digest>, Error> {
        let rules = self.applied()?;
        let settled = self.verify(false).map(|c| c.iter().all(|f| f.ok)).unwrap_or(false);
        let cache = HashCache::open(&self.dir.join("hashes.db"))?;
        let (mut digests, hashes) = accuracy::digests(&self.local.root, &rules, &cache)?;
        for d in digests.values_mut() {
            d.settled = settled;
        }
        let mut differ: std::collections::BTreeSet<String> = Default::default();
        for other in self.hub.devices()?.iter().filter(|d| d.id != self.ctx.me) {
            differ.extend(accuracy::differing(&digests, &other.digests));
        }
        for (top, list) in &hashes {
            let p = accuracy::hashes_path(&self.control.dir, &self.ctx.me, top);
            if differ.contains(top) {
                accuracy::write_hashes(&p, list)?;
            } else if p.exists() {
                std::fs::remove_file(&p)?;
            }
        }
        self.update_me(Some(&rules), |d| d.digests = digests.clone())?;
        Ok(digests)
    }

    /// The files under top folder `top` that are not the same here and on `other`, once both
    /// computers have written their hashes for it (`None` until then).
    pub fn mismatches(&self, other: &str, top: &str) -> Option<Vec<accuracy::Mismatch>> {
        let mine = accuracy::read_hashes(&accuracy::hashes_path(&self.control.dir, &self.ctx.me, top)).ok()?;
        let theirs = accuracy::read_hashes(&accuracy::hashes_path(&self.control.dir, other, top)).ok()?;
        Some(accuracy::compare(&mine, &theirs))
    }

    pub fn other_device(&self, rules: &RuleSet) -> Option<String> {
        rules.devices.keys().find(|id| **id != self.ctx.me).cloned()
    }
}

fn same_choices(a: &RuleSet, b: &RuleSet) -> bool {
    a.root == b.root && a.rules == b.rules && a.always_ignore == b.always_ignore && a.devices == b.devices
}
