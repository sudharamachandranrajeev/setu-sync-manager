//! Helpers shared by the end-to-end tests: built-in engines on loopback, each in its own process
//! (the engine runs once per process) with its own home.
//!
//! A test starts an engine by running its own test binary again with only `common::host`
//! selected and `SETU_E2E_HOST` set. That copy starts the engine and answers Setu's call
//! protocol (`setu_core::pipe::answer`) on a loopback port, as the running Setu does on its pipe.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use setu_core::Error;
use setu_core::syncthing::{Client, Transport};

pub struct St {
    pub child: Child,
    pub client: Client,
    pub id: String,
    pub listen: u16,
}

impl Drop for St {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// Calls to an engine host on a loopback port, one connection per call (as `pipe::Pipe`).
struct Tcp(u16);

impl Transport for Tcp {
    fn call(&self, method: &str, args: &Value) -> Result<Value, Error> {
        let s = TcpStream::connect(("127.0.0.1", self.0)).map_err(|e| Error::Setup(e.to_string()))?;
        (&s).write_all(format!("{}\n", json!({ "method": method, "args": args })).as_bytes()).map_err(|e| Error::Setup(e.to_string()))?;
        let mut line = String::new();
        BufReader::new(&s).read_line(&mut line).map_err(|e| Error::Setup(e.to_string()))?;
        let mut v: Value = serde_json::from_str(&line).map_err(|e| Error::Setup(format!("{e}: {line:?}")))?;
        match v.get_mut("err").map(Value::take) {
            Some(Value::String(e)) => Err(Error::Syncthing(e)),
            _ => Ok(v.get_mut("ok").map(Value::take).unwrap_or(Value::Null)),
        }
    }
}

/// The engine host: does nothing in a normal test run.
#[test]
fn host() {
    let Some(dir) = std::env::var_os("SETU_E2E_HOST").map(PathBuf::from) else { return };
    let listen = std::env::var("SETU_E2E_LISTEN").unwrap();
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    setu_engine::start(&setu_engine::StartArgs { home: dir.join("home"), listen: vec![listen], name }).unwrap();
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::fs::write(dir.join("port.tmp"), port.to_string()).unwrap();
    std::fs::rename(dir.join("port.tmp"), dir.join("port")).unwrap();
    let handler: setu_core::pipe::Handler = Arc::new(setu_engine::call);
    for s in l.incoming().flatten() {
        let h = handler.clone();
        std::thread::spawn(move || setu_core::pipe::answer(&s, &s, &h));
    }
}

pub fn start(dir: &Path) -> St {
    std::fs::create_dir_all(dir).unwrap();
    let listen = free_port();
    let log = std::fs::File::create(dir.join("host.log")).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "common::host", "--nocapture", "--test-threads=1"])
        .env("SETU_E2E_HOST", dir)
        .env("SETU_E2E_LISTEN", format!("tcp://127.0.0.1:{listen}"))
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let port: u16 = wait_for(60, || std::fs::read_to_string(dir.join("port")).ok()?.parse().ok()).expect("engine host comes up");
    let client = Client::with(Arc::new(Tcp(port)));
    let id = client.my_id().unwrap();
    St { child, client, id, listen }
}

pub fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

pub fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

pub fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}


/// A throwaway Brahma sync server (brahma core's `server/src/home.ts`, Node 24+) on loopback.
pub struct Server {
    pub child: Child,
    pub url: String,
    pub setup: String,
    _dir: tempfile::TempDir,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The server script: `$BRAHMA_SERVER_TS`, else brahma checked out next to this repository.
pub fn server_script() -> Option<std::path::PathBuf> {
    let p = std::env::var_os("BRAHMA_SERVER_TS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../brahma/server/src/home.ts"));
    let v = Command::new("node").arg("--version").output().ok()?;
    let major: u32 = String::from_utf8_lossy(&v.stdout).trim().trim_start_matches('v').split('.').next()?.parse().ok()?;
    (major >= 24 && p.exists()).then_some(p)
}

pub fn start_server(script: &Path) -> Server {
    use std::io::BufRead;
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let ok = Command::new("node").args([script.to_str().unwrap(), "init", &port.to_string()]).env("BRAHMA_SERVER_HOME", dir.path()).stdout(Stdio::null()).status().unwrap();
    assert!(ok.success());
    let env_file = dir.path().join("server.env");
    let text: String = std::fs::read_to_string(&env_file)
        .unwrap()
        .lines()
        .map(|l| if l.starts_with("HOST=") { "HOST=127.0.0.1".to_string() } else if l.starts_with("PORT=") { format!("PORT={port}") } else { l.to_string() })
        .filter(|l| !l.starts_with("RATE_LIMIT_SCALE="))
        .collect::<Vec<_>>()
        .join("\n");
    // Lift the rate limits for this throwaway server only.
    let text = text + "\nRATE_LIMIT_SCALE=1000";
    std::fs::write(&env_file, text.clone() + "\n").unwrap();
    let setup = text.lines().find_map(|l| l.strip_prefix("SETUP_CODE=")).unwrap_or("").to_string();
    let mut child = Command::new("node").args([script.to_str().unwrap(), "serve"]).env("BRAHMA_SERVER_HOME", dir.path()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let started = Instant::now();
    let mut line = String::new();
    while !line.contains("listening") {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "the server stopped");
        assert!(started.elapsed() < Duration::from_secs(20), "the server did not start");
    }
    std::thread::spawn(move || for _ in out.lines() {});
    Server { child, url: format!("http://127.0.0.1:{port}"), setup, _dir: dir }
}

/// A `setu-control` folder shared with nobody yet (Setu shares it as computers pair).
pub fn add_lone_control(st: &St, path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    let mut f = st.client.default_folder().unwrap();
    f["id"] = serde_json::json!("setu-control");
    f["path"] = serde_json::json!(path.to_string_lossy());
    f["type"] = serde_json::json!("sendreceive");
    f["devices"] = serde_json::json!([{ "deviceID": st.id }]);
    st.client.put_folder(&f).unwrap();
}
