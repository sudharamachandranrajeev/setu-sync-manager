//! The engine linked into a Rust binary: start, calls from several threads, stop.

use serde_json::json;
use setu_engine::{StartArgs, call, start, stop};

#[test]
fn starts_answers_and_stops() {
    let home = std::env::temp_dir().join(format!("setu-engine-test-{}", std::process::id()));
    let data = home.with_extension("data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("a.txt"), "abc").unwrap();

    assert!(call("my_id", &json!(null)).unwrap_err().contains("not running"));
    let me = start(&StartArgs { home: home.clone(), listen: vec!["tcp://127.0.0.1:0".into()], name: "rust-test".into() }).unwrap();
    assert_eq!(call("my_id", &json!(null)).unwrap(), json!(me));

    call(
        "put_folder",
        &json!({"value": {"id": "setu-f-rs", "label": "Setu · rs", "path": data, "type": "sendreceive"}}),
    )
    .unwrap();
    // Calls arrive from Setu's threads (window, sampler, helper loop) at the same time.
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..20 {
                    let st = call("folder_status", &json!({"folder": "setu-f-rs"})).unwrap();
                    assert!(st["state"].is_string());
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let started = std::time::Instant::now();
    loop {
        let st = call("folder_status", &json!({"folder": "setu-f-rs"})).unwrap();
        if st["state"] == "idle" && st["localBytes"] == 3 {
            break;
        }
        assert!(started.elapsed().as_secs() < 20, "never scanned: {st}");
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(call("no_such_call", &json!({})).unwrap_err().contains("unknown"));
    stop().unwrap();
    assert!(call("my_id", &json!(null)).is_err());
    assert!(start(&StartArgs { home: home.clone(), listen: vec!["tcp://127.0.0.1:0".into()], name: String::new() }).is_err());
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&data);
}
