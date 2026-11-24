//! This computer's own settings: where its root folder is. Never shared, never in the rules.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalConfig {
    /// The folder this computer syncs (for example `~/Work` or `C:\AI\Claude`).
    pub root: PathBuf,
    /// Where the old, separately running Syncthing kept its config, when not in its usual
    /// place. Only read when moving to the engine built into Setu (`migrate`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub syncthing_home: Option<PathBuf>,
    /// Let Setu's folders sync on a metered network (off by default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_metered: bool,
    /// Let Setu's folders sync when the link goes through a Tailscale relay (off by default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_relay: bool,
    /// Syncing paused here by the person: until this time (Unix seconds), or `0` until they
    /// resume it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_until: Option<u64>,
    /// Setu folders the person paused one by one. The network gate never resumes these.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub paused_folders: BTreeSet<String>,
    /// No desktop notifications about copy progress (they are on by default).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub quiet_progress: bool,
}

/// `SETU_CONFIG_DIR`, else `%APPDATA%\Setu` on Windows, else `$XDG_CONFIG_HOME/setu`.
pub fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SETU_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
        return base.join("Setu");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or(home.join(".config")).join("setu")
}

/// Where the built-in sync engine keeps this computer's identity (keys), its config and its file
/// index: `SETU_ENGINE_HOME`, else `%LOCALAPPDATA%\Setu\engine` on Windows (not roaming: the
/// keys belong to this computer), else `$XDG_STATE_HOME/setu/engine`.
pub fn engine_home() -> PathBuf {
    if let Some(d) = std::env::var_os("SETU_ENGINE_HOME") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
        return base.join("Setu").join("engine");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or(home.join(".local/state")).join("setu").join("engine")
}

impl LocalConfig {
    /// Has the person paused syncing on this computer, as of `now`?
    pub fn user_paused(&self, now: u64) -> bool {
        match self.paused_until {
            Some(0) => true,
            Some(t) => now < t,
            None => false,
        }
    }

    pub fn load(dir: &Path) -> Result<LocalConfig, Error> {
        let p = dir.join("local.json");
        let s = std::fs::read_to_string(&p)
            .map_err(|_| Error::Setup(format!("no {}; run `setu init --root <folder>` first", p.display())))?;
        let c: LocalConfig = serde_json::from_str(&s).map_err(|e| Error::Setup(format!("{}: {e}", p.display())))?;
        if !c.root.is_absolute() {
            return Err(Error::Setup(format!("root {} must be an absolute path", c.root.display())));
        }
        Ok(c)
    }

    pub fn save(&self, dir: &Path) -> Result<(), Error> {
        std::fs::create_dir_all(dir)?;
        write_atomic(&dir.join("local.json"), &(serde_json::to_string_pretty(self).expect("serialise") + "\n"))
    }
}

/// Write via a temporary file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), Error> {
    let tmp = path.with_file_name(format!(".syncthing.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("setu")));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Expand a leading `~` the way Syncthing does in folder paths.
pub fn expand_home(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix('~') {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default();
        return home.join(rest.trim_start_matches(['/', '\\']));
    }
    PathBuf::from(p)
}

/// A rules path (`a/b`) under a root, with this OS's separator.
pub fn join_rel(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}
