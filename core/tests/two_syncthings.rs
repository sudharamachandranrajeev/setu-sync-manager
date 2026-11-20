//! End to end: two real Syncthings on this computer, talking over loopback only, each with its
//! own temporary home, ports and folders. They never touch the Syncthing you use day to day.
//!
//! Slow (about a minute), so it is ignored by default: `cargo test -- --ignored`.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::json;
use setu_core::apply::{self, Action, Ctx};
use setu_core::control::Control;
use setu_core::rules::{Device, Kind, Mode, Os, RuleSet};
use setu_core::syncthing::Client;

struct St {
    child: Child,
    client: Client,
    id: String,
    listen: u16,
}

impl Drop for St {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start(dir: &Path) -> St {
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
fn regex_replace_gui(x: &str, port: u16) -> String {
    let start = x.find("<gui ").expect("gui section");
    let a = x[start..].find("<address>").unwrap() + start + "<address>".len();
    let b = x[a..].find("</address>").unwrap() + a;
    format!("{}127.0.0.1:{port}{}", &x[..a], &x[b..])
}

fn wait_for<T>(secs: u64, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if let Some(v) = f() {
            return Some(v);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    None
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn add_peer(me: &St, other: &St, name: &str) {
    let dev = json!({ "deviceID": other.id, "name": name, "addresses": [format!("tcp://127.0.0.1:{}", other.listen)] });
    me.client.put_device(&dev).unwrap();
}

fn add_control(st: &St, path: &Path, peer: &str) {
    std::fs::create_dir_all(path).unwrap();
    let mut f = st.client.default_folder().unwrap();
    f["id"] = json!("setu-control");
    f["path"] = json!(path.to_string_lossy());
    f["type"] = json!("sendreceive");
    f["devices"] = json!([{ "deviceID": st.id }, { "deviceID": peer }]);
    st.client.put_folder(&f).unwrap();
}

#[test]
#[ignore]
fn two_computers_sync_by_the_rules() {
    if Command::new("syncthing").arg("--version").output().is_err() {
        eprintln!("syncthing not installed; skipping");
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Setu keeps ignore-file backups in its config dir; keep the test's out of the real one.
    unsafe { std::env::set_var("SETU_CONFIG_DIR", tmp.path().join("setu-config")) };
    let (da, db) = (tmp.path().join("a"), tmp.path().join("b"));
    let (ra, rb): (PathBuf, PathBuf) = (da.join("root"), db.join("root"));
    std::fs::create_dir_all(&ra).unwrap();
    std::fs::create_dir_all(&rb).unwrap();
    let a = start(&da);
    let b = start(&db);
    add_peer(&a, &b, "b");
    add_peer(&b, &a, "a");
    add_control(&a, &da.join("control"), &b.id);
    add_control(&b, &db.join("control"), &a.id);

    put(&ra, "shared/a.txt", "from A");
    put(&ra, "shared/API keys/k.txt", "secret");
    put(&ra, "both/x.txt", "x");
    put(&ra, "aonly/sec.txt", "stays on A");
    put(&ra, "mixed/keep.txt", "keep");
    put(&ra, "mixed/skip.txt", "skip");
    put(&ra, "loose.txt", "loose");
    put(&ra, "we[ird]/w.txt", "weird name");
    put(&rb, "fromb/b.txt", "from B");
    put(&rb, "both/y.txt", "y");

    let mut devices = BTreeMap::new();
    devices.insert(a.id.clone(), Device { name: "a".into(), os: Os::Linux });
    devices.insert(b.id.clone(), Device { name: "b".into(), os: Os::Linux });
    let mut rs = RuleSet::new(devices);
    let from_a = Mode::From(a.id.clone());
    rs.set("shared", Kind::Dir, Some(from_a.clone())).unwrap();
    rs.set("both", Kind::Dir, Some(Mode::Both)).unwrap();
    rs.set("fromb", Kind::Dir, Some(Mode::From(b.id.clone()))).unwrap();
    rs.set("mixed", Kind::Dir, Some(from_a.clone())).unwrap();
    rs.set("mixed/skip.txt", Kind::File, Some(Mode::Off)).unwrap();
    rs.set("loose.txt", Kind::File, Some(from_a.clone())).unwrap();
    rs.set("we[ird]", Kind::Dir, Some(Mode::Both)).unwrap();
    rs.revision = 1;

    let ca = Ctx { me: a.id.clone(), root: ra.clone(), os: Os::Linux };
    let cb = Ctx { me: b.id.clone(), root: rb.clone(), os: Os::Linux };
    let (ctl_a, ctl_b) = (Control::at(da.join("control")), Control::at(db.join("control")));
    for (ctx, st, ctl) in [(&ca, &a, &ctl_a), (&cb, &b, &ctl_b)] {
        let plan = apply::plan(ctx, &rs, &st.client).unwrap();
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        assert_eq!(plan.folders, 6, "shared, both, fromb, mixed, we[ird], files-only root");
        apply::execute(ctx, &rs, &plan, &st.client, Some(ctl)).unwrap();
    }

    // What should arrive does.
    for (root, rel) in [(&rb, "shared/a.txt"), (&rb, "both/x.txt"), (&rb, "mixed/keep.txt"), (&rb, "loose.txt"), (&rb, "we[ird]/w.txt"), (&ra, "fromb/b.txt"), (&ra, "both/y.txt")] {
        assert!(wait_for(90, || read(root, rel)).is_some(), "{rel} never arrived in {}", root.display());
    }
    // What should not, did not.
    for rel in ["mixed/skip.txt", "aonly/sec.txt", "shared/API keys/k.txt"] {
        assert!(read(&rb, rel).is_none(), "{rel} must not reach B");
    }
    // Status files crossed through the control folder.
    assert!(wait_for(60, || ctl_a.read_status(&b.id)).is_some_and(|s| s.applied_revision == 1));

    // One way means one way: B's edit to a received file never comes back to A.
    put(&rb, "shared/a.txt", "edited on B");
    put(&rb, "both/z.txt", "z");
    for f in b.client.folders().unwrap() {
        let _ = b.client.scan(f["id"].as_str().unwrap());
    }
    assert!(wait_for(90, || read(&ra, "both/z.txt")).is_some(), "both-ways change from B never reached A");
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(read(&ra, "shared/a.txt").as_deref(), Some("from A"));

    // Nothing to do when nothing changed.
    let again = apply::plan(&ca, &rs, &a.client).unwrap();
    assert!(again.actions.is_empty(), "{:?}", again.actions);
    assert_eq!(again.unchanged, 6);

    // A Syncthing folder Setu does not manage, overlapping a Setu folder, blocks the plan.
    let mut stray = a.client.default_folder().unwrap();
    stray["id"] = json!("stray");
    stray["path"] = json!(ra.to_string_lossy());
    stray["paused"] = json!(true);
    a.client.put_folder(&stray).unwrap();
    let blocked = apply::plan(&ca, &rs, &a.client).unwrap();
    assert!(blocked.problems.iter().any(|p| p.message.contains("overlaps")), "{:?}", blocked.problems);
    a.client.delete_folder("stray").unwrap();

    // Revision 2: switch "both" off. The folder goes from Syncthing; its files stay on both sides.
    rs.set("both", Kind::Dir, None).unwrap();
    rs.revision = 2;
    for (ctx, st, ctl) in [(&ca, &a, &ctl_a), (&cb, &b, &ctl_b)] {
        let plan = apply::plan(ctx, &rs, &st.client).unwrap();
        assert!(matches!(plan.actions.as_slice(), [Action::Remove { .. }]), "{:?}", plan.actions);
        apply::execute(ctx, &rs, &plan, &st.client, Some(ctl)).unwrap();
    }
    for (root, rel) in [(&ra, "both/x.txt"), (&rb, "both/x.txt"), (&ra, "both/y.txt"), (&rb, "both/y.txt")] {
        assert!(read(root, rel).is_some(), "{rel} must stay in {}", root.display());
    }
}
