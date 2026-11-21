//! The `setu-control` folder: a small Syncthing folder the computers share both ways. It holds
//! each computer's file list, so any computer can show the others' files. (The rules and each
//! computer's status live in the Brahma account; see `hub`. Rules files left here by alpha.3
//! are read once, to move them over.)

use std::path::{Path, PathBuf};

use crate::local::expand_home;
use crate::rules::{short_id, RuleSet};
use crate::syncthing::Client;
use crate::Error;

pub const CONTROL_FOLDER: &str = "setu-control";
pub const RULES_FILE: &str = "setu.rules.json";

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

    pub fn index_path(&self, device: &str) -> PathBuf {
        self.dir.join(format!("index-{}.tsv.gz", short_id(device)))
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
