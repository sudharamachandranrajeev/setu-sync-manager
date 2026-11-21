//! A small client for this computer's own Syncthing, through its local REST API.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::Error;

pub struct Client {
    base: String,
    key: String,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(base: impl Into<String>, key: impl Into<String>) -> Client {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Client { base: base.into().trim_end_matches('/').to_string(), key: key.into(), agent }
    }

    /// Connect using Syncthing's own config file. `home` overrides where it is looked for.
    pub fn discover(home: Option<&Path>) -> Result<Client, Error> {
        let candidates: Vec<PathBuf> = match home {
            Some(h) => vec![h.join("config.xml")],
            None => default_homes().into_iter().map(|h| h.join("config.xml")).collect(),
        };
        let path = candidates
            .iter()
            .find(|p| p.is_file())
            .ok_or_else(|| Error::Syncthing("Syncthing's config.xml was not found; is Syncthing set up?".into()))?;
        let xml = std::fs::read_to_string(path)?;
        let (base, key) = parse_config(&xml)?;
        Ok(Client::new(base, key))
    }

    fn url(&self, path: &str) -> String {
        format!("{}/rest/{}", self.base, path)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        let mut resp = self
            .agent
            .get(&self.url(path))
            .header("X-API-Key", &self.key)
            .call()
            .map_err(|e| Error::Syncthing(format!("GET {path}: {e}")))?;
        resp.body_mut().read_json::<T>().map_err(|e| Error::Syncthing(format!("GET {path}: {e}")))
    }

    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(), Error> {
        let url = self.url(path);
        let r = match (method, body) {
            ("PUT", Some(b)) => self.agent.put(&url).header("X-API-Key", &self.key).send_json(b),
            ("PATCH", Some(b)) => self.agent.patch(&url).header("X-API-Key", &self.key).send_json(b),
            ("POST", Some(b)) => self.agent.post(&url).header("X-API-Key", &self.key).send_json(b),
            ("POST", None) => self.agent.post(&url).header("X-API-Key", &self.key).send_empty(),
            ("DELETE", None) => self.agent.delete(&url).header("X-API-Key", &self.key).call(),
            _ => unreachable!("unsupported request {method}"),
        };
        r.map(|_| ()).map_err(|e| Error::Syncthing(format!("{method} {path}: {e}")))
    }

    pub fn my_id(&self) -> Result<String, Error> {
        let v: Value = self.get("system/status")?;
        v["myID"].as_str().map(String::from).ok_or_else(|| Error::Syncthing("no myID in system/status".into()))
    }

    pub fn folders(&self) -> Result<Vec<Value>, Error> {
        self.get("config/folders")
    }

    pub fn devices(&self) -> Result<Vec<Value>, Error> {
        self.get("config/devices")
    }

    pub fn put_device(&self, device: &Value) -> Result<(), Error> {
        let id = device["deviceID"].as_str().unwrap_or_default();
        self.send("PUT", &format!("config/devices/{}", enc(id)), Some(device))
    }

    /// Events after `since`, waiting up to `timeout` seconds for new ones.
    pub fn events(&self, since: u64, timeout: u64) -> Result<Vec<Value>, Error> {
        self.get(&format!("events?since={since}&timeout={timeout}&events=ItemFinished"))
    }

    pub fn default_folder(&self) -> Result<Value, Error> {
        self.get("config/defaults/folder")
    }

    pub fn put_folder(&self, folder: &Value) -> Result<(), Error> {
        let id = folder["id"].as_str().unwrap_or_default();
        self.send("PUT", &format!("config/folders/{}", enc(id)), Some(folder))
    }

    pub fn patch_folder(&self, id: &str, patch: &Value) -> Result<(), Error> {
        self.send("PATCH", &format!("config/folders/{}", enc(id)), Some(patch))
    }

    pub fn delete_folder(&self, id: &str) -> Result<(), Error> {
        self.send("DELETE", &format!("config/folders/{}", enc(id)), None)
    }

    pub fn ignores(&self, id: &str) -> Result<Vec<String>, Error> {
        let v: Value = self.get(&format!("db/ignores?folder={}", enc(id)))?;
        Ok(v["ignore"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    pub fn set_ignores(&self, id: &str, lines: &[String]) -> Result<(), Error> {
        self.send("POST", &format!("db/ignores?folder={}", enc(id)), Some(&json!({ "ignore": lines })))
    }

    pub fn connections(&self) -> Result<Value, Error> {
        self.get("system/connections")
    }

    pub fn folder_status(&self, id: &str) -> Result<Value, Error> {
        self.get(&format!("db/status?folder={}", enc(id)))
    }

    pub fn completion(&self, folder: &str, device: &str) -> Result<Value, Error> {
        self.get(&format!("db/completion?folder={}&device={}", enc(folder), enc(device)))
    }

    pub fn scan(&self, id: &str) -> Result<(), Error> {
        self.send("POST", &format!("db/scan?folder={}", enc(id)), None)
    }
}

/// Where Syncthing keeps its config on this OS, in the order Syncthing itself looks.
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
        return Err(Error::Syncthing("Syncthing's GUI uses HTTPS; Setu needs it on plain HTTP at 127.0.0.1".into()));
    }
    let text = |tag: &str| gui.children().find(|n| n.has_tag_name(tag)).and_then(|n| n.text()).map(str::trim);
    let key = text("apikey").filter(|k| !k.is_empty()).ok_or_else(|| Error::Syncthing("config.xml has no API key".into()))?;
    let addr = text("address").unwrap_or("127.0.0.1:8384");
    let port = addr.rsplit(':').next().unwrap_or("8384");
    // Always talk over loopback, even if the GUI listens more widely.
    Ok((format!("http://127.0.0.1:{port}"), key.to_string()))
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_gui_and_key() {
        let xml = r#"<configuration><gui enabled="true" tls="false"><address>0.0.0.0:8385</address><apikey>abc</apikey></gui></configuration>"#;
        assert_eq!(parse_config(xml).unwrap(), ("http://127.0.0.1:8385".into(), "abc".into()));
        let tls = r#"<configuration><gui tls="true"><address>127.0.0.1:8384</address><apikey>k</apikey></gui></configuration>"#;
        assert!(parse_config(tls).is_err());
    }
}
