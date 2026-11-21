//! The background helper on its own, with no console window on Windows, for starting at logon.
//! Same as `setu watch`.
#![cfg_attr(windows, windows_subsystem = "windows")]

use setu_core::local::{config_dir, LocalConfig};
use setu_core::syncthing::Client;

fn main() {
    let dir = config_dir();
    // Syncthing may still be starting at logon: keep trying.
    loop {
        let r = LocalConfig::load(&dir).and_then(|cfg| {
            let client = Client::discover(cfg.syncthing_home.as_deref())?;
            client.my_id()?;
            setu_core::watch::run(&dir, || LocalConfig::load(&config_dir()), &client)
        });
        if let Err(e) = r {
            eprintln!("setu-watch: {e}; trying again in 30 s");
        }
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
