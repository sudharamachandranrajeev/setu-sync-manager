//! The `setu-control` folder: a small Syncthing folder both computers share both ways. It holds
//! the rules, each computer's status (which rules revision it has applied) and each computer's
//! file list, so either computer can show the other's files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::local::{expand_home, write_atomic};
use crate::rules::{short_id, Os, RuleSet};
use crate::syncthing::Client;
use crate::Error;

pub const CONTROL_FOLDER: &str = "setu-control";
pub const RULES_FILE: &str = "setu.rules.json";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub device: String,
    pub name: String,
    pub os: Os,
    pub applied_revision: u64,
    /// Seconds since 1970.
    pub applied_at: u64,
    pub folders: usize,
    #[serde(default)]
    pub problems: Vec<String>,
}

pub struct Control {
    pub dir: PathBuf,
}

impl Control {
    pub fn find(client: &Client) -> Result<Control, Error> {
        let f = client
            .folders()?
            .into_iter()
            .find(|f| f["id"] == CONTROL_FOLDER)
            .ok_or_else(|| Error::Setup(format!("Syncthing has no {CONTROL_FOLDER:?} folder; set it up first")))?;
        Ok(Control { dir: expand_home(f["path"].as_str().unwrap_or_default()) })
    }

    pub fn at(dir: impl Into<PathBuf>) -> Control {
        Control { dir: dir.into() }
    }

    pub fn rules_path(&self) -> PathBuf {
        self.dir.join(RULES_FILE)
    }

    pub fn read_rules(&self) -> Result<Option<RuleSet>, Error> {
        match std::fs::read_to_string(self.rules_path()) {
            Ok(s) => RuleSet::from_json(&s).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn write_rules(&self, rs: &RuleSet) -> Result<(), Error> {
        rs.validate()?;
        write_atomic(&self.rules_path(), &rs.to_json())
    }

    /// Syncthing conflict copies of control files: both computers changed the rules at once.
    pub fn conflicts(&self) -> Vec<String> {
        std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.contains(".sync-conflict-"))
            .collect()
    }

    pub fn status_path(&self, device: &str) -> PathBuf {
        self.dir.join(format!("status-{}.json", short_id(device)))
    }

    pub fn index_path(&self, device: &str) -> PathBuf {
        self.dir.join(format!("index-{}.tsv.gz", short_id(device)))
    }

    pub fn write_status(&self, s: &Status) -> Result<(), Error> {
        write_atomic(&self.status_path(&s.device), &(serde_json::to_string_pretty(s).expect("serialise") + "\n"))
    }

    pub fn read_status(&self, device: &str) -> Option<Status> {
        let s = std::fs::read_to_string(self.status_path(device)).ok()?;
        serde_json::from_str(&s).ok()
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The control folder's path must not be inside a synced root, or it would sync twice.
pub fn check_outside(control: &Path, root: &Path) -> Result<(), Error> {
    if control.starts_with(root) {
        return Err(Error::Setup(format!("the {CONTROL_FOLDER} folder must not be inside {}", root.display())));
    }
    Ok(())
}
