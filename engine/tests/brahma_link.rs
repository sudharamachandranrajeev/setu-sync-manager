//! End to end through Brahma: two built-in engines that have never met, and a throwaway Brahma
//! sync server, all on loopback. Both computers sign in to one account; the rules saved on one
//! reach the other through the server, the helpers pair the Syncthings and apply the rules by
//! themselves, the content check agrees, and an identical conflict copy is tidied away.
//!
//! Needs Node 24+ with brahma's server (see `common::server_script`). Ignored by default:
//! `cargo test -p setu-engine --test brahma_link -- --ignored`.

use std::path::{Path, PathBuf};

use brahma_sync::MemorySecretStore;
use serde_json::json;
use setu_core::hub::Hub;
use setu_core::local::LocalConfig;
use setu_core::mesh;
use setu_core::rules::{Kind, Mode};
use setu_core::session::{Auto, Session};
use setu_core::syncthing::Client;

mod common;
use common::*;

fn session(dir: &Path, client: &Client, root: &Path) -> Session {
    let cfg = dir.join("cfg");
    let local = LocalConfig { root: root.to_path_buf(), allow_metered: true, allow_relay: true, ..Default::default() };
    let hub = Hub::with_secrets(&cfg, MemorySecretStore::default());
    Session::new(cfg, local, client.clone(), hub).unwrap()
}

/// One helper round, as `helper::Ticker` does it.
fn round(s: &Session) -> Auto {
    s.sync_hub().unwrap();
    let rules = s.shared().unwrap();
    s.update_me(rules.as_ref(), |_| {}).unwrap();
    mesh::pair(&s.client, &s.ctx.me, &s.hub.devices().unwrap()).unwrap();
    s.refresh_index(5).unwrap();
    let a = s.auto_apply().unwrap();
    s.sync_hub().unwrap();
    a
}

fn applied(s: &Session, id: &str) -> u64 {
    s.hub.device(id).unwrap().map(|d| d.applied_revision).unwrap_or(0)
}

#[test]
#[ignore]
fn computers_join_and_follow_the_rules_through_brahma() {
    let Some(script) = server_script() else {
        eprintln!("Node 24+ or brahma's server not found (set BRAHMA_SERVER_TS); skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("SETU_CONFIG_DIR", tmp.path().join("setu-config"));
        std::env::set_var("SETU_TEST_LOOPBACK", "1");
    }
    let server = start_server(&script);
    let (da, db) = (tmp.path().join("a"), tmp.path().join("b"));
    let (ra, rb): (PathBuf, PathBuf) = (da.join("root"), db.join("root"));
    std::fs::create_dir_all(&ra).unwrap();
    std::fs::create_dir_all(&rb).unwrap();
    let a = start(&da);
    let b = start(&db);
    add_lone_control(&a, &da.join("control"));
    add_lone_control(&b, &db.join("control"));
    put(&ra, "proj/hello.txt", "from A");
    put(&ra, "pub/doc.txt", "published by A");
    put(&ra, "private/p.txt", "stays on A");

    let sa = session(&da, &a.client, &ra);
    let sb = session(&db, &b.client, &rb);
    sa.hub.sign_up(&server.url, "person@example.com", "correct horse battery staple", &server.setup).unwrap();
    sb.hub.sign_in(&server.url, "person@example.com", "correct horse battery staple").unwrap();
    assert!(sa.hub.signed_in() && sb.hub.signed_in());

    // A starts the rules and makes its choices.
    let mut rs = sa.join(false).unwrap();
    rs.set("proj", Kind::Dir, Some(Mode::Both)).unwrap();
    rs.set("pub", Kind::Dir, Some(Mode::From(a.id.clone()))).unwrap();
    rs.revision += 1;
    sa.hub.put_rules(&rs).unwrap();
    round(&sa);

    // B joins from the account: no Syncthing setup by hand, no Apply pressed on either side.
    sb.sync_hub().unwrap();
    let joined = sb.join(false).unwrap();
    assert_eq!(joined.devices.len(), 2);
    let rev = joined.revision;
    let settled = wait_for(120, || {
        let (x, y) = (round(&sa), round(&sb));
        eprintln!("A: {x:?}  B: {y:?}");
        (applied(&sa, &a.id) == rev && applied(&sa, &b.id) == rev && read(&rb, "proj/hello.txt").is_some() && read(&rb, "pub/doc.txt").is_some()).then_some(())
    });
    assert!(settled.is_some(), "B never caught up: A applied {}, B applied {}", applied(&sa, &a.id), applied(&sa, &b.id));
    assert!(read(&rb, "private/p.txt").is_none(), "an Off folder must not reach B");
    assert!(b.client.devices().unwrap().iter().any(|d| d["deviceID"] == a.id.as_str()), "B's Syncthing was paired with A");

    // A change that cannot be applied safely waits for the person, and says why.
    let mut stray = a.client.default_folder().unwrap();
    stray["id"] = json!("stray");
    stray["path"] = json!(ra.to_string_lossy());
    stray["paused"] = json!(true);
    a.client.put_folder(&stray).unwrap();
    let mut rs = sb.shared().unwrap().unwrap();
    rs.set("more", Kind::Dir, Some(Mode::Both)).unwrap();
    rs.revision += 1;
    sb.hub.put_rules(&rs).unwrap();
    sb.sync_hub().unwrap();
    let held = wait_for(30, || match round(&sa) {
        Auto::Held { revision, reasons } if revision == rs.revision => Some(reasons),
        _ => None,
    })
    .expect("A must hold the change");
    assert!(held.iter().any(|r| r.contains("overlaps")), "{held:?}");
    wait_for(20, || {
        round(&sb);
        sb.hub.device(&a.id).unwrap()?.held.filter(|h| h.revision == rs.revision)
    })
    .expect("B sees that A is holding");
    a.client.delete_folder("stray").unwrap();
    assert_eq!(round(&sa), Auto::Nothing, "a held revision waits for the person");
    // The person checks it in Setu on A and starts it.
    let (_, plan) = sa.plan().unwrap();
    let here = setu_core::index::Tree::new(sa.scan_here().unwrap());
    let preview = sa.preview(&rs, &plan, &here, None);
    sa.apply(&plan, &preview, true).unwrap();
    round(&sa);
    wait_for(20, || {
        round(&sb);
        let d = sb.hub.device(&a.id).unwrap()?;
        (d.applied_revision == rs.revision && d.held.is_none()).then_some(())
    })
    .expect("B sees A caught up");

    // The content check agrees once both sides are up to date; a local edit on the receiving
    // side of a one-way folder shows up as a difference, named file by file.
    wait_for(60, || (sa.verify(false).ok()?.iter().all(|c| c.ok) && sb.verify(false).ok()?.iter().all(|c| c.ok)).then_some(())).expect("folders settle");
    let (ga, gb) = (sa.check_accuracy().unwrap(), sb.check_accuracy().unwrap());
    assert_eq!(ga["proj"].hash, gb["proj"].hash);
    assert_eq!(ga["pub"].hash, gb["pub"].hash);
    put(&rb, "pub/doc.txt", "edited on B");
    sb.check_accuracy().unwrap();
    round(&sb);
    round(&sa);
    sa.check_accuracy().unwrap();
    let m = wait_for(60, || sb.mismatches(&a.id, "pub")).expect("both hash lists for pub arrive");
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!((m[0].path.as_str(), m[0].what), ("pub/doc.txt", "different content"));

    // Duplicates: a conflict copy identical to its file is removed after the checks, and logged.
    put(&ra, "proj/hello.sync-conflict-20261121-120000-AAAAAAA.txt", "from A");
    put(&ra, "proj/other.sync-conflict-20261121-120000-AAAAAAA.txt", "no original");
    let removed = sa.tidy().unwrap();
    assert_eq!(removed.len(), 1);
    assert!(removed[0].removed.ends_with("proj/hello.sync-conflict-20261121-120000-AAAAAAA.txt"));
    assert!(read(&ra, "proj/other.sync-conflict-20261121-120000-AAAAAAA.txt").is_some());
    drop(server);
}
