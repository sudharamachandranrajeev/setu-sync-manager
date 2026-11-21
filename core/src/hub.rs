//! The Brahma link: Setu's shared state lives in the person's Brahma account, as end-to-end
//! encrypted rows on their own Brahma sync server (the VPS, reached over Tailscale). The server
//! only ever sees ciphertext.
//!
//! Two tables:
//! - `setu_rules`: one row, the rules every computer follows.
//! - `setu_devices`: one row per computer (keyed by its Syncthing device ID): its name, system,
//!   Tailscale addresses, which rules revision it has applied, and anything holding it back.
//!
//! File contents never go through the server; Syncthing copies them computer to computer over
//! Tailscale. The file lists for the tree view stay in the `setu-control` Syncthing folder,
//! because they are too big for sync rows.
//!
//! The app and the background helper share one `brahma.db`. Every operation takes a file lock
//! and opens the store afresh, so the two never interleave a sync or stamp from a stale clock.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use brahma_sync::{Account, SecretStore, SyncError, SyncStore, Tables};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::Error;
use crate::rules::{Os, RuleSet};

pub const RULES_TABLE: &str = "setu_rules";
pub const DEVICES_TABLE: &str = "setu_devices";
const RULES_ROW: &str = "rules";
/// Prefix of Setu's secrets in the keyring (`setu.sync.session`, `setu.sync.dek`).
const APP: &str = "setu";

/// One computer, as the others see it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceState {
    /// Syncthing device ID.
    pub id: String,
    pub name: String,
    pub os: Option<Os>,
    /// Where its Syncthing listens: Tailscale addresses only.
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub applied_revision: u64,
    #[serde(default)]
    pub applied_at: u64,
    /// New rules wait for the person here, and why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held: Option<Held>,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub folders: usize,
    /// Content check: one digest per synced top folder (see `accuracy`).
    #[serde(default)]
    pub digests: BTreeMap<String, crate::accuracy::Digest>,
    /// Seconds since 1970 when this row was last written.
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Held {
    pub revision: u64,
    pub reasons: Vec<String>,
}

/// Who is signed in, without secrets.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub server: String,
    pub email: String,
}

/// The OS keyring: Secret Service on Linux, Credential Manager on Windows.
pub struct Keyring;

#[cfg(any(target_os = "linux", windows))]
fn keyring_ready() -> Result<(), SyncError> {
    static INIT: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        #[cfg(target_os = "linux")]
        let store = zbus_secret_service_keyring_store::Store::new().map_err(|e| e.to_string())?;
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new().map_err(|e| e.to_string())?;
        keyring_core::set_default_store(store);
        Ok(())
    })
    .clone()
    .map_err(|e| SyncError::Message(format!("the system keyring is not available: {e}")))
}

#[cfg(any(target_os = "linux", windows))]
impl SecretStore for Keyring {
    fn read(&self, key: &str) -> Result<Option<String>, SyncError> {
        keyring_ready()?;
        match entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(SyncError::Message(format!("keyring: {e}"))),
        }
    }
    fn write(&self, key: &str, value: &str) -> Result<(), SyncError> {
        keyring_ready()?;
        entry(key)?.set_password(value).map_err(|e| SyncError::Message(format!("keyring: {e}")))
    }
    fn delete(&self, key: &str) -> Result<(), SyncError> {
        keyring_ready()?;
        match entry(key)?.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(SyncError::Message(format!("keyring: {e}"))),
        }
    }
}

#[cfg(any(target_os = "linux", windows))]
fn entry(key: &str) -> Result<keyring_core::Entry, SyncError> {
    keyring_core::Entry::new("Setu", key).map_err(|e| SyncError::Message(format!("keyring: {e}")))
}

#[cfg(not(any(target_os = "linux", windows)))]
impl SecretStore for Keyring {
    fn read(&self, _: &str) -> Result<Option<String>, SyncError> {
        Err(SyncError::Message("no supported keyring on this system".into()))
    }
    fn write(&self, _: &str, _: &str) -> Result<(), SyncError> {
        Err(SyncError::Message("no supported keyring on this system".into()))
    }
    fn delete(&self, _: &str) -> Result<(), SyncError> {
        Ok(())
    }
}

fn err(e: SyncError) -> Error {
    match e {
        SyncError::RecoveryKeyNeeded => Error::RecoveryKeyNeeded,
        e => Error::Hub(e.to_string()),
    }
}

/// Any secret store: the keyring in the app, memory in tests.
pub struct Secrets(Box<dyn SecretStore + Send>);

impl SecretStore for Secrets {
    fn read(&self, key: &str) -> Result<Option<String>, SyncError> {
        self.0.read(key)
    }
    fn write(&self, key: &str, value: &str) -> Result<(), SyncError> {
        self.0.write(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), SyncError> {
        self.0.delete(key)
    }
}

pub struct Hub {
    dir: PathBuf,
    account: Mutex<Account<Secrets>>,
}

fn platform() -> &'static str {
    match Os::current() {
        Os::Windows => "windows",
        Os::Macos => "macos",
        Os::Linux => "linux",
    }
}

/// Run `f` on a freshly opened store in `dir`, holding the lock.
fn with_store<T>(dir: &Path, f: impl FnOnce(&mut SyncStore) -> Result<T, Error>) -> Result<T, Error> {
    std::fs::create_dir_all(dir)?;
    let lock = File::options().create(true).truncate(false).write(true).open(dir.join("brahma.lock"))?;
    lock.lock()?;
    let conn = rusqlite::Connection::open(dir.join("brahma.db")).map_err(|e| Error::Hub(e.to_string()))?;
    conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| Error::Hub(e.to_string()))?;
    let mut store = SyncStore::open(conn, Tables::new(&[], &[RULES_TABLE, DEVICES_TABLE])).map_err(err)?;
    let out = f(&mut store);
    drop(store);
    drop(lock);
    out
}

impl Hub {
    /// The link for this computer, its store in `dir` (Setu's settings folder), its secrets in
    /// the system keyring.
    pub fn open(dir: &Path) -> Hub {
        Hub::with_secrets(dir, Keyring)
    }

    fn with_store<T>(&self, f: impl FnOnce(&mut SyncStore) -> Result<T, Error>) -> Result<T, Error> {
        with_store(&self.dir, f)
    }

    pub fn with_secrets(dir: &Path, secrets: impl SecretStore + Send + 'static) -> Hub {
        Hub { dir: dir.to_path_buf(), account: Mutex::new(Account::new(Secrets(Box::new(secrets)), APP, platform())) }
    }

    fn acct(&self) -> std::sync::MutexGuard<'_, Account<Secrets>> {
        self.account.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn signed_in(&self) -> bool {
        self.acct().signed_in().unwrap_or(false)
    }

    pub fn account(&self) -> Option<AccountInfo> {
        let s = self.acct().session().ok().flatten()?;
        self.signed_in().then_some(AccountInfo { server: s.server, email: s.email })
    }

    /// Sign in to the Brahma account the other Brahma apps use. Fails with
    /// [`Error::RecoveryKeyNeeded`] after a password reset; then call
    /// [`Hub::unlock_with_recovery_key`].
    pub fn sign_in(&self, server: &str, email: &str, password: &str) -> Result<(), Error> {
        let device = self.with_store(|s| Ok(s.device_id().to_string()))?;
        self.acct().sign_in(server.trim(), email, password, &device).map_err(err)
    }

    pub fn unlock_with_recovery_key(&self, key: &str) -> Result<(), Error> {
        let device = self.with_store(|s| Ok(s.device_id().to_string()))?;
        self.acct().unlock_with_recovery_key(key, &device).map_err(err)
    }

    /// For tests: make the account (the person normally makes it in a Brahma app).
    pub fn sign_up(&self, server: &str, email: &str, password: &str, invite: &str) -> Result<String, Error> {
        let device = self.with_store(|s| Ok(s.device_id().to_string()))?;
        self.acct().sign_up(server, email, password, invite, &device).map_err(err)
    }

    /// Forget the account on this computer. The synced rows stay on the server and on the other
    /// computers; this computer's copy is cleared.
    pub fn sign_out(&self) -> Result<(), Error> {
        self.acct().sign_out().map_err(err)?;
        self.with_store(|s| s.reset().map_err(err))
    }

    /// Send what changed here and fetch what changed elsewhere. Returns how many rows came in.
    pub fn sync(&self) -> Result<usize, Error> {
        let mut account = self.acct();
        with_store(&self.dir, |store| {
            let Some(mut engine) = account.engine(store).map_err(err)? else {
                return Err(Error::Hub("not signed in to Brahma".into()));
            };
            Ok(engine.run().map_err(err)?.pulled)
        })
    }

    fn data(store: &SyncStore, table: &str, id: &str) -> Result<Option<String>, Error> {
        Ok(store.get(table, id).map_err(err)?.filter(|r| !r.deleted).map(|r| r.text("data").to_string()))
    }

    fn put(store: &mut SyncStore, table: &str, id: &str, data: String) -> Result<(), Error> {
        let mut values = Map::new();
        values.insert("data".into(), Value::String(data));
        store.put(table, id, values).map_err(err)
    }

    /// The shared rules, as last synced.
    pub fn rules(&self) -> Result<Option<RuleSet>, Error> {
        self.with_store(|s| Hub::data(s, RULES_TABLE, RULES_ROW)?.map(|d| RuleSet::from_json(&d)).transpose())
    }

    /// Store new shared rules here; the next sync sends them.
    pub fn put_rules(&self, rs: &RuleSet) -> Result<(), Error> {
        rs.validate()?;
        self.with_store(|s| Hub::put(s, RULES_TABLE, RULES_ROW, rs.to_json()))
    }

    pub fn devices(&self) -> Result<Vec<DeviceState>, Error> {
        self.with_store(|s| {
            Ok(s.rows(DEVICES_TABLE)
                .map_err(err)?
                .into_iter()
                .filter(|r| !r.deleted)
                .filter_map(|r| serde_json::from_str::<DeviceState>(r.text("data")).ok())
                .collect())
        })
    }

    pub fn device(&self, id: &str) -> Result<Option<DeviceState>, Error> {
        self.with_store(|s| Ok(Hub::data(s, DEVICES_TABLE, id)?.and_then(|d| serde_json::from_str(&d).ok())))
    }

    /// Write this computer's row, unless nothing but the time changed.
    pub fn put_device(&self, d: &DeviceState) -> Result<bool, Error> {
        self.with_store(|s| {
            let old: Option<DeviceState> = Hub::data(s, DEVICES_TABLE, &d.id)?.and_then(|t| serde_json::from_str(&t).ok());
            if old.as_ref().is_some_and(|o| DeviceState { updated_at: d.updated_at, ..o.clone() } == *d) {
                return Ok(false);
            }
            Hub::put(s, DEVICES_TABLE, &d.id, json!(d).to_string())?;
            Ok(true)
        })
    }

    /// Changes written here and not yet sent.
    pub fn pending(&self) -> Result<i64, Error> {
        self.with_store(|s| s.pending().map_err(err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brahma_sync::MemorySecretStore;

    #[test]
    fn rows_round_trip_and_unchanged_device_rows_are_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Hub::with_secrets(dir.path(), MemorySecretStore::default());
        assert!(!hub.signed_in());
        assert_eq!(hub.rules().unwrap(), None);
        let a = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
        let mut devices = BTreeMap::new();
        devices.insert(a.to_string(), crate::rules::Device { name: "one".into(), os: Os::Linux, receive_only: false });
        let mut rs = RuleSet::new(devices);
        rs.revision = 3;
        hub.put_rules(&rs).unwrap();
        assert_eq!(hub.rules().unwrap(), Some(rs));
        let d = DeviceState { id: a.into(), name: "one".into(), applied_revision: 3, updated_at: 10, ..Default::default() };
        assert!(hub.put_device(&d).unwrap());
        assert!(!hub.put_device(&DeviceState { updated_at: 99, ..d.clone() }).unwrap());
        assert!(hub.put_device(&DeviceState { applied_revision: 4, ..d.clone() }).unwrap());
        assert_eq!(hub.devices().unwrap().len(), 1);
        // The rules row and this device's row; rewrites of a row send once.
        assert_eq!(hub.pending().unwrap(), 2);
    }
}
