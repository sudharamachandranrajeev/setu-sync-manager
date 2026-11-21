//! Helpers shared by the end-to-end tests: real Syncthings on loopback with their own homes.
#![allow(dead_code)]

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use setu_core::syncthing::Client;

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

pub fn start(dir: &Path) -> St {
    let home = dir.join("home");
    let ok = Command::new("syncthing")
        .args(["generate", &format!("--home={}", home.display()), "--no-port-probing"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("syncthing runs");
    assert!(ok.success());
    let (gui, listen) = (free_port(), free_port());
    let cfg = home.join("config.xml");
    let mut x = std::fs::read_to_string(&cfg).unwrap();
    for (from, to) in [
        ("<listenAddress>default</listenAddress>".to_string(), format!("<listenAddress>tcp://127.0.0.1:{listen}</listenAddress>")),
        ("<globalAnnounceEnabled>true</globalAnnounceEnabled>".into(), "<globalAnnounceEnabled>false</globalAnnounceEnabled>".into()),
        ("<localAnnounceEnabled>true</localAnnounceEnabled>".into(), "<localAnnounceEnabled>false</localAnnounceEnabled>".into()),
        ("<relaysEnabled>true</relaysEnabled>".into(), "<relaysEnabled>false</relaysEnabled>".into()),
        ("<natEnabled>true</natEnabled>".into(), "<natEnabled>false</natEnabled>".into()),
        ("<startBrowser>true</startBrowser>".into(), "<startBrowser>false</startBrowser>".into()),
        ("<crashReportingEnabled>true</crashReportingEnabled>".into(), "<crashReportingEnabled>false</crashReportingEnabled>".into()),
        ("<urAccepted>0</urAccepted>".into(), "<urAccepted>-1</urAccepted>".into()),
    ] {
        x = x.replace(&from, &to);
    }
    x = regex_replace_gui(&x, gui);
    std::fs::write(&cfg, x).unwrap();
    let log = std::fs::File::create(dir.join("syncthing.log")).unwrap();
    let child = Command::new("syncthing")
        .args(["serve", &format!("--home={}", home.display()), "--no-browser", "--no-restart"])
        .env("STNODEFAULTFOLDER", "1")
        .env("STNOUPGRADE", "1")
        .env_remove("STHOMEDIR")
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let client = Client::discover(Some(&home)).unwrap();
    let id = wait_for(60, || client.my_id().ok()).expect("syncthing REST comes up");
    St { child, client, id, listen }
}

/// Point the GUI at our own port (the generated config uses 127.0.0.1:8384).
pub fn regex_replace_gui(x: &str, port: u16) -> String {
    let start = x.find("<gui ").expect("gui section");
    let a = x[start..].find("<address>").unwrap() + start + "<address>".len();
    let b = x[a..].find("</address>").unwrap() + a;
    format!("{}127.0.0.1:{port}{}", &x[..a], &x[b..])
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
