//! Setu's handle on its sync engine (Syncthing's code, built into Setu).
//!
//! Inside Setu the engine runs in the same process and `Client` calls it directly. The `setu`
//! command line reaches the running Setu through a local pipe that only this user can open
//! (see `pipe`). Either way the answers are JSON in the shapes of Syncthing's REST API, so the
//! rest of Setu reads them as it always did. Nothing listens on a local HTTP port.

use std::sync::{Arc, OnceLock};

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::Error;

/// Something that runs engine calls: the engine itself, or the pipe to the Setu that hosts it.
pub trait Transport: Send + Sync {
    fn call(&self, method: &str, args: &Value) -> Result<Value, Error>;
}

#[derive(Clone)]
pub struct Client {
    t: Arc<dyn Transport>,
}

static LOCAL: OnceLock<Arc<dyn Transport>> = OnceLock::new();

/// Called once by the process that hosts the engine (Setu), so `Client::connect` uses it.
pub fn set_local(t: Arc<dyn Transport>) {
    let _ = LOCAL.set(t);
}

impl Client {
    pub fn with(t: Arc<dyn Transport>) -> Client {
        Client { t }
    }

    /// The engine in this process when Setu hosts it here, else the running Setu over its
    /// local pipe.
    pub fn connect() -> Result<Client, Error> {
        if let Some(t) = LOCAL.get() {
            return Ok(Client { t: t.clone() });
        }
        crate::pipe::Pipe::connect().map(|p| Client { t: Arc::new(p) })
    }

    fn get<T: DeserializeOwned>(&self, method: &str, args: Value) -> Result<T, Error> {
        let v = self.t.call(method, &args)?;
        serde_json::from_value(v).map_err(|e| Error::Syncthing(format!("{method}: {e}")))
    }

    fn send(&self, method: &str, args: Value) -> Result<(), Error> {
        self.t.call(method, &args).map(drop)
    }

    pub fn my_id(&self) -> Result<String, Error> {
        self.get("my_id", Value::Null)
    }

    pub fn folders(&self) -> Result<Vec<Value>, Error> {
        self.get("folders", Value::Null)
    }

    pub fn devices(&self) -> Result<Vec<Value>, Error> {
        self.get("devices", Value::Null)
    }

    pub fn put_device(&self, device: &Value) -> Result<(), Error> {
        self.send("put_device", json!({ "value": device }))
    }

    /// Events after `since`, waiting up to `timeout` seconds for new ones.
    pub fn events(&self, since: u64, timeout: u64) -> Result<Vec<Value>, Error> {
        self.events_of(since, timeout, "ItemFinished,LocalIndexUpdated,FolderErrors")
    }

    /// Events of the given comma-separated types after `since`, waiting up to `timeout` seconds
    /// (0 = answer at once).
    pub fn events_of(&self, since: u64, timeout: u64, kinds: &str) -> Result<Vec<Value>, Error> {
        self.get("events", json!({ "since": since, "timeout": timeout, "kinds": kinds }))
    }

    pub fn default_folder(&self) -> Result<Value, Error> {
        self.get("default_folder", Value::Null)
    }

    pub fn put_folder(&self, folder: &Value) -> Result<(), Error> {
        self.send("put_folder", json!({ "value": folder }))
    }

    pub fn patch_folder(&self, id: &str, patch: &Value) -> Result<(), Error> {
        self.send("patch_folder", json!({ "id": id, "value": patch }))
    }

    pub fn delete_folder(&self, id: &str) -> Result<(), Error> {
        self.send("delete_folder", json!({ "id": id }))
    }

    pub fn ignores(&self, id: &str) -> Result<Vec<String>, Error> {
        let v: Value = self.get("ignores", json!({ "folder": id }))?;
        Ok(v["ignore"]
            .as_array()
            .map(|a| a.iter().filter_map(|l| l.as_str().map(String::from)).collect())
            .unwrap_or_default())
    }

    pub fn set_ignores(&self, id: &str, lines: &[String]) -> Result<(), Error> {
        self.send("set_ignores", json!({ "folder": id, "ignore": lines }))
    }

    pub fn connections(&self) -> Result<Value, Error> {
        self.get("connections", Value::Null)
    }

    pub fn folder_status(&self, id: &str) -> Result<Value, Error> {
        self.get("folder_status", json!({ "folder": id }))
    }

    pub fn completion(&self, folder: &str, device: &str) -> Result<Value, Error> {
        self.get("completion", json!({ "folder": folder, "device": device }))
    }

    /// Old versions kept by the folder's versioning: path → list of versions.
    pub fn versions(&self, folder: &str) -> Result<Value, Error> {
        self.get("versions", json!({ "folder": folder }))
    }

    /// Files this computer still needs from the other one.
    pub fn need(&self, folder: &str) -> Result<Value, Error> {
        self.get("need", json!({ "folder": folder, "perpage": 200 }))
    }

    /// Files changed here in a receive-only folder (not sent back).
    pub fn local_changed(&self, folder: &str) -> Result<Value, Error> {
        self.get("local_changed", json!({ "folder": folder, "perpage": 200 }))
    }

    /// Files that failed to sync, with the reason.
    pub fn folder_errors(&self, folder: &str) -> Result<Value, Error> {
        self.get("folder_errors", json!({ "folder": folder, "perpage": 200 }))
    }

    pub fn options(&self) -> Result<Value, Error> {
        self.get("options", Value::Null)
    }

    pub fn patch_options(&self, patch: &Value) -> Result<(), Error> {
        self.send("patch_options", json!({ "value": patch }))
    }

    pub fn scan(&self, id: &str) -> Result<(), Error> {
        self.send("scan", json!({ "folder": id }))
    }
}
