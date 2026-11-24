//! Joining computers together without hand-editing Syncthing. The Brahma account says which
//! computers belong to the person (each one's row in `setu_devices`, end-to-end encrypted), so
//! the helper on each computer adds the others to its Syncthing by their Tailscale addresses,
//! and shares the `setu-control` folder with them.
//!
//! Only computers in the person's own account are added, and only at Tailscale addresses
//! (100.64.0.0/10 and fd7a:115c:a1e0::/48): Syncthing never looks for them on the open internet.

use std::net::IpAddr;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Error;
use crate::control::CONTROL_FOLDER;
use crate::hub::DeviceState;
use crate::rules::valid_device_id;
use crate::syncthing::Client;

pub const PORT: u16 = 22000;

/// Is `ip` a Tailscale address? (Loopback too when `SETU_TEST_LOOPBACK=1`, for tests that run
/// two Syncthings on one computer.)
pub fn is_tailnet(ip: &IpAddr) -> bool {
    if ip.is_loopback() && std::env::var_os("SETU_TEST_LOOPBACK").is_some_and(|v| v == "1") {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 100 && (64..128).contains(&o[1])
        }
        IpAddr::V6(v6) => v6.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// This computer's Tailscale addresses, from `tailscale ip`, keeping only those it can listen
/// on. Tailscale reports an IPv6 address even where Windows has not put it on the adapter (IPv6
/// off there); listening on it would fail and retry for as long as Setu runs.
pub fn tailnet_ips() -> Vec<IpAddr> {
    let mut cmd = std::process::Command::new("tailscale");
    cmd.arg("ip");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: Setu has no console to share
    }
    let Ok(out) = cmd.output() else { return Vec::new() };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<IpAddr>().ok())
        .filter(is_tailnet)
        .filter(|ip| std::net::UdpSocket::bind((*ip, 0)).is_ok())
        .collect()
}

/// Syncthing addresses for these IPs, TCP and QUIC.
pub fn addresses(ips: &[IpAddr]) -> Vec<String> {
    let mut v = Vec::new();
    for ip in ips {
        let host = match ip {
            IpAddr::V4(a) => a.to_string(),
            IpAddr::V6(a) => format!("[{a}]"),
        };
        v.push(format!("tcp://{host}:{PORT}"));
        v.push(format!("quic://{host}:{PORT}"));
    }
    v
}

/// Where the other computers can reach this one's Syncthing: its listen addresses when they
/// are Tailscale addresses (as `harden` sets them), else this computer's Tailscale IPs on the
/// usual port.
pub fn listen_addresses(client: &Client) -> Vec<String> {
    let listen: Vec<String> = client
        .options()
        .ok()
        .and_then(|o| o["listenAddresses"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|a| a.as_str().map(String::from))
        .collect();
    let own = tailnet_only(&listen);
    if !own.is_empty() {
        return own;
    }
    addresses(&tailnet_ips())
}

/// Keep only addresses that point at a Tailscale IP.
pub fn tailnet_only(addrs: &[String]) -> Vec<String> {
    addrs
        .iter()
        .filter(|a| {
            let rest = a.strip_prefix("tcp://").or_else(|| a.strip_prefix("quic://"));
            let Some(rest) = rest else { return false };
            let host = match rest.rsplit_once(':') {
                Some((h, port)) if port.parse::<u16>().is_ok() => h,
                _ => return false,
            };
            host.trim_matches(['[', ']']).parse::<IpAddr>().is_ok_and(|ip| is_tailnet(&ip))
        })
        .cloned()
        .collect()
}

/// Settings that keep Syncthing on the tailnet: no global or local discovery, no relays, no
/// NAT traversal, no usage or crash reports, listening on this computer's Tailscale addresses.
/// Only what differs is changed; returns the names changed.
pub fn harden(client: &Client, ips: &[IpAddr]) -> Result<Vec<String>, Error> {
    if ips.is_empty() {
        return Err(Error::Setup("Tailscale is not running here (no Tailscale address); start Tailscale first".into()));
    }
    let now = client.options()?;
    let want = json!({
        "globalAnnounceEnabled": false,
        "localAnnounceEnabled": false,
        "relaysEnabled": false,
        "natEnabled": false,
        "urAccepted": -1,
        "crashReportingEnabled": false,
        "listenAddresses": addresses(ips),
    });
    let mut patch = serde_json::Map::new();
    for (k, v) in want.as_object().unwrap() {
        if now[k] != *v {
            patch.insert(k.clone(), v.clone());
        }
    }
    if !patch.is_empty() {
        client.patch_options(&Value::Object(patch.clone()))?;
    }
    Ok(patch.keys().cloned().collect())
}

/// Where a new computer keeps the `setu-control` folder: `%LOCALAPPDATA%\Setu\control` on
/// Windows, `$XDG_DATA_HOME/setu/control` elsewhere.
pub fn default_control_dir() -> PathBuf {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Setu").join("control");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home.join(".local/share")).join("setu").join("control")
}

/// Make the `setu-control` folder if this computer has none. Returns true when it was made.
pub fn ensure_control(client: &Client) -> Result<bool, Error> {
    if client.folders()?.iter().any(|f| f["id"] == CONTROL_FOLDER) {
        return Ok(false);
    }
    let dir = default_control_dir();
    std::fs::create_dir_all(&dir)?;
    let mut f = client.default_folder()?;
    f["id"] = json!(CONTROL_FOLDER);
    f["label"] = json!("Setu · control");
    f["path"] = json!(dir.to_string_lossy());
    f["type"] = json!("sendreceive");
    f["devices"] = json!([]);
    f["paused"] = json!(false);
    f["fsWatcherEnabled"] = json!(true);
    f["versioning"]["type"] = json!("trashcan");
    f["versioning"]["params"] = json!({ "cleanoutDays": "7" });
    client.put_folder(&f)?;
    Ok(true)
}

/// Add the person's other computers to Syncthing and share `setu-control` with them. Returns
/// the names of computers added.
pub fn pair(client: &Client, me: &str, devices: &[DeviceState]) -> Result<Vec<String>, Error> {
    let known: Vec<String> = client.devices()?.iter().filter_map(|d| d["deviceID"].as_str().map(String::from)).collect();
    let mut added = Vec::new();
    let mut share = Vec::new();
    for d in devices.iter().filter(|d| d.id != me && valid_device_id(&d.id)) {
        let addrs = tailnet_only(&d.addresses);
        if !known.contains(&d.id) {
            if addrs.is_empty() {
                continue;
            }
            client.put_device(&json!({
                "deviceID": d.id,
                "name": d.name,
                "addresses": addrs,
                "compression": "metadata",
                "introducer": false,
                "autoAcceptFolders": false,
                "paused": false,
            }))?;
            added.push(d.name.clone());
        }
        share.push(d.id.clone());
    }
    if let Some(control) = client.folders()?.into_iter().find(|f| f["id"] == CONTROL_FOLDER) {
        let mut ids: Vec<String> = control["devices"].as_array().into_iter().flatten().filter_map(|d| d["deviceID"].as_str().map(String::from)).collect();
        let before = ids.len();
        for id in share {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.len() != before {
            let list: Vec<Value> = ids.iter().map(|id| json!({ "deviceID": id })).collect();
            client.patch_folder(CONTROL_FOLDER, &json!({ "devices": list }))?;
        }
    }
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_tailscale_addresses_pass() {
        let a: Vec<String> = ["tcp://100.101.102.103:22000", "quic://[fd7a:115c:a1e0::1]:22000", "tcp://192.0.2.4:22000", "dynamic", "tcp://100.200.1.1:22000", "relay://100.64.0.1:22067"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(tailnet_only(&a), vec!["tcp://100.101.102.103:22000".to_string(), "quic://[fd7a:115c:a1e0::1]:22000".to_string()]);
        let ips: Vec<IpAddr> = vec!["100.64.0.9".parse().unwrap(), "fd7a:115c:a1e0::9".parse().unwrap()];
        assert_eq!(addresses(&ips), vec!["tcp://100.64.0.9:22000", "quic://100.64.0.9:22000", "tcp://[fd7a:115c:a1e0::9]:22000", "quic://[fd7a:115c:a1e0::9]:22000"]);
        assert!(!is_tailnet(&"100.128.0.1".parse().unwrap()));
    }
}
