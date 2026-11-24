//! Moving from the old setup, where Syncthing ran as its own program, to the engine built into
//! Setu. Runs once, after the person agrees to the steps `OldSetup::steps` lists.
//!
//! Nothing is deleted before the built-in engine is proven to work: the old Syncthing's keys,
//! config and database are copied (not moved), so this computer keeps its device ID and the
//! other computers need no re-pairing, and the old home stays where it is.
//!
//! This is the only code left that talks to Syncthing's REST API (one status read and one
//! shutdown of the old Syncthing). It goes away in a later release.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::Error;
use crate::control::CONTROL_FOLDER;

/// What the old setup consists of on this computer.
#[derive(Debug, Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OldSetup {
    /// The old Syncthing's home (config.xml, cert.pem, key.pem, database).
    pub home: PathBuf,
    /// The old Syncthing answers on its GUI port right now.
    pub running: bool,
    /// Its device ID, when it was running.
    pub device_id: Option<String>,
    /// Folders the old Syncthing syncs that are not Setu's; the built-in engine does not take
    /// them over.
    pub other_folders: Vec<String>,
    /// Windows: the scheduled task that started it at sign-in.
    pub task: Option<String>,
    /// Windows: the start script and the plain-text GUI password file.
    pub files: Vec<PathBuf>,
    /// Windows: the old helper's sign-in entry ("Setu helper").
    pub run_value: bool,
    /// Linux: systemd user units that start the old helper or Syncthing at login.
    pub units: Vec<String>,
}

const TASK: &str = r"\Syncthing (Setu)";
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const OLD_RUN_VALUE: &str = "Setu helper";

/// Files of a Syncthing home that carry its identity, config and database.
const KEEP: &[&str] = &["cert.pem", "key.pem", "config.xml", "index-v2", "index-v0.14.0.db"];

/// Where Syncthing keeps its home on this OS, in the order Syncthing itself looks.
pub fn default_homes() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(h) = std::env::var_os("STHOMEDIR") {
        v.push(PathBuf::from(h));
    }
    if cfg!(windows) {
        if let Some(l) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(l).join("Syncthing"));
        }
    } else {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let state = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).unwrap_or(home.join(".local/state"));
        let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or(home.join(".config"));
        v.push(state.join("syncthing"));
        v.push(config.join("syncthing"));
    }
    v
}

/// The GUI address and API key from Syncthing's config.xml.
pub fn parse_config(xml: &str) -> Result<(String, String), Error> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| Error::Syncthing(format!("config.xml: {e}")))?;
    let gui = doc
        .descendants()
        .find(|n| n.has_tag_name("gui"))
        .ok_or_else(|| Error::Syncthing("config.xml has no <gui>".into()))?;
    if gui.attribute("tls") == Some("true") {
        return Err(Error::Syncthing("the old Syncthing's GUI uses HTTPS; stop it by hand, then try again".into()));
    }
    let text = |tag: &str| gui.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).map(str::trim);
    let key = text("apikey").filter(|k| !k.is_empty()).ok_or_else(|| Error::Syncthing("config.xml has no API key".into()))?;
    let addr = text("address").unwrap_or("127.0.0.1:8384");
    let port = addr.rsplit(':').next().unwrap_or("8384");
    // Always talk over loopback, even if the GUI listens more widely.
    Ok((format!("http://127.0.0.1:{port}"), key.to_string()))
}

/// Labels (or IDs) of folders in a config.xml that Setu does not manage.
fn foreign_folders(xml: &str) -> Vec<String> {
    let Ok(doc) = roxmltree::Document::parse(xml) else { return Vec::new() };
    doc.descendants()
        .filter(|n| n.has_tag_name("folder") && n.parent().is_some_and(|p| p.has_tag_name("configuration")))
        .filter_map(|n| {
            let id = n.attribute("id")?;
            (!is_setu_folder(id)).then(|| n.attribute("label").filter(|l| !l.is_empty()).unwrap_or(id).to_string())
        })
        .collect()
}

pub fn is_setu_folder(id: &str) -> bool {
    id.starts_with("setu-f-") || id == CONTROL_FOLDER
}

fn rest(base: &str, key: &str, method: &str, path: &str) -> Result<Value, Error> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(10))).build().into();
    let url = format!("{base}/rest/{path}");
    let r = match method {
        "POST" => agent.post(&url).header("X-API-Key", key).send_empty(),
        _ => agent.get(&url).header("X-API-Key", key).call(),
    };
    let mut r = r.map_err(|e| Error::Syncthing(format!("old Syncthing {path}: {e}")))?;
    Ok(r.body_mut().read_json::<Value>().unwrap_or(Value::Null))
}

fn quiet(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = c.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The old setup, when there is one to move from. `None` once Setu's engine has its own keys
/// (already moved, or a new install), or when no old Syncthing home exists.
pub fn find(engine_home: &Path, old_home: Option<&Path>) -> Option<OldSetup> {
    if engine_home.join("cert.pem").is_file() {
        return None;
    }
    let home = match old_home {
        Some(h) => h.to_path_buf(),
        None => default_homes().into_iter().find(|h| h.join("config.xml").is_file() && h.join("cert.pem").is_file())?,
    };
    let xml = std::fs::read_to_string(home.join("config.xml")).ok()?;
    let mut old = OldSetup { home, other_folders: foreign_folders(&xml), ..Default::default() };
    if let Ok((base, key)) = parse_config(&xml)
        && let Ok(st) = rest(&base, &key, "GET", "system/status") {
            old.running = true;
            old.device_id = st["myID"].as_str().map(String::from);
        }
    if cfg!(windows) {
        if quiet("schtasks", &["/Query", "/TN", TASK]).is_some() {
            old.task = Some(TASK.into());
        }
        if let Some(l) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            old.files = ["start-syncthing.ps1", "syncthing-gui-password.txt"].iter().map(|f| l.join("Setu").join(f)).filter(|p| p.is_file()).collect();
        }
        old.run_value = quiet("reg", &["query", RUN_KEY, "/v", OLD_RUN_VALUE]).is_some();
    } else {
        for unit in ["setu-watch.service", "syncthing.service"] {
            if quiet("systemctl", &["--user", "is-enabled", unit]).is_some_and(|s| s.trim() == "enabled") {
                old.units.push(unit.into());
            }
        }
    }
    Some(old)
}

impl OldSetup {
    /// Every step, in words, for the person to agree to.
    pub fn steps(&self) -> Vec<String> {
        let mut v = Vec::new();
        if self.running {
            v.push("Stop the Syncthing program that runs on its own now (it is replaced by the engine inside Setu).".into());
        }
        v.push(format!(
            "Copy this computer's Syncthing identity, settings and file index from {} into Setu. This computer keeps its device ID, so your other computers keep syncing with it without pairing again.",
            self.home.display()
        ));
        v.push("Start the engine inside Setu and check it has the same identity and every Setu folder.".into());
        let mut after = Vec::new();
        if let Some(t) = &self.task {
            after.push(format!("the scheduled task {t}"));
        }
        for f in &self.files {
            after.push(f.display().to_string());
        }
        if self.run_value {
            after.push(format!("the sign-in entry \"{OLD_RUN_VALUE}\""));
        }
        for u in &self.units {
            after.push(format!("the {u} login service (turned off)"));
        }
        if !after.is_empty() {
            v.push(format!("Only when that check passes, remove the old start-up pieces: {}.", after.join(", ")));
        }
        v.push(format!(
            "Nothing else is deleted: the old Syncthing folder ({}) and any installed Syncthing program stay, so you can remove them yourself later.",
            self.home.display()
        ));
        if !self.other_folders.is_empty() {
            v.push(format!(
                "These folders are not Setu's and will stop syncing on this computer, because the old Syncthing no longer runs: {}.",
                self.other_folders.join(", ")
            ));
        }
        v
    }

    /// Stop the old Syncthing and wait until it has exited. Returns its device ID if known.
    pub fn stop_old(&mut self) -> Result<(), Error> {
        if !self.running {
            return Ok(());
        }
        let xml = std::fs::read_to_string(self.home.join("config.xml"))?;
        let (base, key) = parse_config(&xml)?;
        if self.device_id.is_none() {
            self.device_id = rest(&base, &key, "GET", "system/status").ok().and_then(|s| s["myID"].as_str().map(String::from));
        }
        // Units first, so systemd does not start it again.
        for u in &self.units {
            quiet("systemctl", &["--user", "stop", u]);
        }
        let _ = rest(&base, &key, "POST", "system/shutdown");
        let deadline = Instant::now() + Duration::from_secs(60);
        while rest(&base, &key, "GET", "system/ping").is_ok() {
            if Instant::now() > deadline {
                return Err(Error::Setup("the old Syncthing did not stop within a minute; stop it by hand and try again".into()));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        // Give it a moment to close its database.
        std::thread::sleep(Duration::from_secs(2));
        Ok(())
    }

    /// Copy identity, config and database into the engine's home.
    pub fn copy_into(&self, engine_home: &Path) -> Result<(), Error> {
        std::fs::create_dir_all(engine_home)?;
        for name in KEEP {
            let from = self.home.join(name);
            if from.is_dir() {
                copy_dir(&from, &engine_home.join(name))?;
            } else if from.is_file() {
                std::fs::copy(&from, engine_home.join(name))?;
            }
        }
        if !engine_home.join("cert.pem").is_file() {
            return Err(Error::Setup(format!("no cert.pem in {}", self.home.display())));
        }
        Ok(())
    }

    /// After the engine started from the copy: same identity, and every Setu folder there.
    /// Folders that are not Setu's are taken out of the engine's config (their files stay).
    pub fn check(&self, client: &crate::syncthing::Client, engine_home: &Path) -> Result<(), Error> {
        let me = client.my_id()?;
        if let Some(old) = &self.device_id
            && *old != me {
                return Err(Error::Setup(format!("the engine has device ID {me}, the old Syncthing had {old}")));
            }
        if std::fs::read(engine_home.join("cert.pem"))? != std::fs::read(self.home.join("cert.pem"))? {
            return Err(Error::Setup("the engine's key differs from the old Syncthing's".into()));
        }
        let xml = std::fs::read_to_string(self.home.join("config.xml"))?;
        let want: Vec<String> = roxmltree::Document::parse(&xml)
            .map(|d| {
                d.descendants()
                    .filter(|n| n.has_tag_name("folder") && n.parent().is_some_and(|p| p.has_tag_name("configuration")))
                    .filter_map(|n| n.attribute("id").filter(|id| is_setu_folder(id)).map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let have: Vec<String> = client.folders()?.iter().filter_map(|f| f["id"].as_str().map(String::from)).collect();
        if let Some(missing) = want.iter().find(|id| !have.contains(id)) {
            return Err(Error::Setup(format!("folder {missing} is missing from the engine")));
        }
        for id in have.iter().filter(|id| !is_setu_folder(id)) {
            client.delete_folder(id)?;
        }
        Ok(())
    }

    /// Remove the old start-up pieces. Returns what could not be removed.
    pub fn clean_up(&self) -> Vec<String> {
        let mut failed = Vec::new();
        if let Some(t) = &self.task
            && quiet("schtasks", &["/Delete", "/TN", t, "/F"]).is_none() {
                failed.push(format!("scheduled task {t}"));
            }
        for f in &self.files {
            if std::fs::remove_file(f).is_err() {
                failed.push(f.display().to_string());
            }
        }
        if self.run_value && quiet("reg", &["delete", RUN_KEY, "/v", OLD_RUN_VALUE, "/f"]).is_none() {
            failed.push(format!("sign-in entry {OLD_RUN_VALUE}"));
        }
        for u in &self.units {
            if quiet("systemctl", &["--user", "disable", "--now", u]).is_none() {
                failed.push(u.clone());
            }
        }
        failed
    }

    /// Put the old setup back after a failed move: start the old Syncthing again.
    pub fn restore(&self) {
        if let Some(t) = &self.task {
            quiet("schtasks", &["/Run", "/TN", t]);
        }
        for u in &self.units {
            quiet("systemctl", &["--user", "start", u]);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Error> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let target = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<configuration version="51">
        <folder id="setu-f-abc" label="Setu · Work" path="/w"></folder>
        <folder id="setu-control" label="" path="/c"></folder>
        <folder id="photos-x1" label="Photos" path="/p"></folder>
        <folder id="music" label="" path="/m"></folder>
        <device id="AAA"></device>
        <gui enabled="true" tls="false"><address>0.0.0.0:8385</address><apikey>abc</apikey></gui>
    </configuration>"#;

    #[test]
    fn reads_gui_and_key() {
        assert_eq!(parse_config(XML).unwrap(), ("http://127.0.0.1:8385".into(), "abc".into()));
        let tls = r#"<configuration><gui tls="true"><address>127.0.0.1:8384</address><apikey>k</apikey></gui></configuration>"#;
        assert!(parse_config(tls).is_err());
    }

    #[test]
    fn tells_setu_folders_from_others() {
        assert_eq!(foreign_folders(XML), vec!["Photos".to_string(), "music".to_string()]);
    }

    #[test]
    fn finds_an_old_home_and_copies_it() {
        let t = tempfile::tempdir().unwrap();
        let old = t.path().join("old");
        let engine = t.path().join("engine");
        std::fs::create_dir_all(old.join("index-v2")).unwrap();
        std::fs::write(old.join("config.xml"), XML.replace("8385", "1")).unwrap();
        std::fs::write(old.join("cert.pem"), "CERT").unwrap();
        std::fs::write(old.join("key.pem"), "KEY").unwrap();
        std::fs::write(old.join("index-v2").join("db"), "DB").unwrap();
        std::fs::write(old.join("https-key.pem"), "not copied").unwrap();

        let found = find(&engine, Some(&old)).unwrap();
        assert!(!found.running);
        assert_eq!(found.other_folders.len(), 2);
        assert!(found.steps().iter().any(|s| s.contains("Photos, music")));
        found.copy_into(&engine).unwrap();
        assert_eq!(std::fs::read_to_string(engine.join("cert.pem")).unwrap(), "CERT");
        assert_eq!(std::fs::read_to_string(engine.join("index-v2").join("db")).unwrap(), "DB");
        assert!(!engine.join("https-key.pem").exists());
        // The old home is untouched.
        assert!(old.join("cert.pem").is_file());
        // Once the engine has its keys there is nothing left to move.
        assert!(find(&engine, Some(&old)).is_none());
    }
}
