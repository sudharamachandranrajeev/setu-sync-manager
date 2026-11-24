//! In-app updates from Setu's GitHub releases.
//!
//! Setu asks GitHub for its releases (nothing about you is sent), picks the newest one above
//! the running version, downloads this platform's download (the Windows setup program or the
//! Linux .tar.gz) and checks its signature against the public key built into Setu
//! (`update-key.pub`, minisign, made with `tauri signer`). Only a download signed with the
//! matching private key, which only the release workflow holds, is ever run. Then the download
//! installs itself the usual way: it asks this Setu to quit (the engine saves its index),
//! replaces the programs and starts Setu again.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;

const REPO: &str = "sudharamachandranrajeev/setu-sync-manager";
const PUBLIC_KEY: &str = include_str!("../update-key.pub");
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    /// The release notes, as written on GitHub.
    pub notes: String,
    /// The release page.
    pub page: String,
    asset: String,
    url: String,
    sig_url: String,
    pub size: u64,
}

fn agent(timeout: u64) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .user_agent(concat!("Setu/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// This platform's download in a release.
fn wanted(name: &str) -> bool {
    if cfg!(windows) { name.ends_with("-windows-x64-setup.exe") } else { name.ends_with("-linux-x64.tar.gz") }
}

/// The newest signed release above the running version, if any. Pre-releases count: every
/// Setu release is one while Setu is in alpha.
pub fn check() -> Result<Option<Update>, String> {
    let current = semver::Version::parse(VERSION).map_err(|e| e.to_string())?;
    let mut r = agent(20)
        .get(&format!("https://api.github.com/repos/{REPO}/releases?per_page=20"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("could not reach GitHub: {e}"))?;
    let releases: Vec<Value> = r.body_mut().read_json().map_err(|e| format!("GitHub's answer: {e}"))?;
    Ok(newest(&releases, &current))
}

fn newest(releases: &[Value], current: &semver::Version) -> Option<Update> {
    releases
        .iter()
        .filter(|r| r["draft"] != true)
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?;
            let v = semver::Version::parse(tag.trim_start_matches('v')).ok().filter(|v| v > current)?;
            let assets = r["assets"].as_array()?;
            let asset = assets.iter().find(|a| a["name"].as_str().is_some_and(wanted))?;
            let name = asset["name"].as_str()?;
            // Releases from before in-app updates have no signature: they are skipped.
            let sig = assets.iter().find(|a| a["name"].as_str() == Some(&format!("{name}.sig")))?;
            Some((
                v,
                Update {
                    version: tag.trim_start_matches('v').to_string(),
                    notes: r["body"].as_str().unwrap_or_default().to_string(),
                    page: r["html_url"].as_str().unwrap_or_default().to_string(),
                    asset: name.to_string(),
                    url: asset["browser_download_url"].as_str()?.to_string(),
                    sig_url: sig["browser_download_url"].as_str()?.to_string(),
                    size: asset["size"].as_u64().unwrap_or(0),
                },
            ))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, u)| u)
}

fn download(url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut r = agent(600).get(url).call().map_err(|e| format!("download failed: {e}"))?;
    r.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("download failed: {e}"))
}

/// Does `sig` (a `tauri signer` signature: base64 of minisign's text) sign `data` with the
/// key built into Setu?
fn verify(data: &[u8], sig: &[u8], public_key: &str) -> Result<(), String> {
    use base64::Engine;
    let b64 = |s: &[u8]| -> Result<String, String> {
        let raw = base64::engine::general_purpose::STANDARD.decode(s.trim_ascii()).map_err(|e| e.to_string())?;
        String::from_utf8(raw).map_err(|e| e.to_string())
    };
    let pk = minisign_verify::PublicKey::decode(&b64(public_key.as_bytes())?).map_err(|e| format!("update key: {e}"))?;
    let sig = minisign_verify::Signature::decode(&b64(sig)?).map_err(|e| format!("update signature: {e}"))?;
    pk.verify(data, &sig, true).map_err(|_| "the download's signature does not match Setu's key; not installing it".to_string())
}

/// Download, check and start installing. On success Setu is asked to quit by the installer
/// shortly after, and started again when it is done.
pub fn install(u: &Update) -> Result<(), String> {
    let data = download(&u.url, 512 * 1024 * 1024)?;
    let sig = download(&u.sig_url, 64 * 1024)?;
    verify(&data, &sig, PUBLIC_KEY)?;
    // A new folder with a random name only this user can open (0700 on Linux), so nobody else
    // on the computer can swap the checked download, or what it unpacks to, before it runs.
    // Kept afterwards: the installer runs from it after Setu has quit.
    let dir = tempfile::Builder::new().prefix("setu-update-").tempdir().map_err(|e| e.to_string())?.keep();
    let file = dir.join(&u.asset);
    std::fs::write(&file, &data).map_err(|e| e.to_string())?;
    run_installer(&file, &dir, &u.version)
}

/// The setup program, silently, into the folder Setu runs from, for the same users.
#[cfg(windows)]
fn run_installer(file: &Path, _dir: &Path, _version: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let app_dir = exe.parent().ok_or("no program folder")?;
    let per_user = std::env::var_os("LOCALAPPDATA").is_some_and(|l| app_dir.starts_with(std::path::PathBuf::from(l)));
    std::process::Command::new(file)
        .args(["/SILENT", "/SUPPRESSMSGBOXES", "/NORESTART", if per_user { "/CURRENTUSER" } else { "/ALLUSERS" }])
        .arg(format!("/DIR={}", app_dir.display()))
        .spawn()
        .map_err(|e| format!("could not start the setup program: {e}"))?;
    Ok(())
}

/// Unpack and run `install.sh --restart` in its own process group, so it outlives this Setu.
#[cfg(not(windows))]
fn run_installer(file: &Path, dir: &Path, version: &str) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let ok = std::process::Command::new("tar").arg("-xzf").arg(file).arg("-C").arg(dir).status().map_err(|e| format!("tar: {e}"))?;
    if !ok.success() {
        return Err("could not unpack the download".into());
    }
    // The one folder packaging/linux/package.sh puts in the archive.
    let script = dir.join(format!("setu-{version}-linux-x64")).join("install.sh");
    if !script.is_file() {
        return Err("the download has no install.sh where Setu expects it".into());
    }
    // Keep the person's choice about starting at login.
    let mut args = vec!["--restart"];
    if !crate::host::autostart::enabled() {
        args.push("--no-autostart");
    }
    std::process::Command::new("bash")
        .arg(&script)
        .args(&args)
        .current_dir(script.parent().unwrap_or(dir))
        .process_group(0)
        .spawn()
        .map_err(|e| format!("could not start install.sh: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, signed: bool) -> Value {
        let name = if cfg!(windows) { format!("Setu-{tag}-windows-x64-setup.exe") } else { format!("setu-{tag}-linux-x64.tar.gz") };
        let mut assets = vec![json!({"name": name, "browser_download_url": format!("https://x/{name}"), "size": 5})];
        if signed {
            assets.push(json!({"name": format!("{name}.sig"), "browser_download_url": format!("https://x/{name}.sig")}));
        }
        json!({"tag_name": format!("v{tag}"), "draft": false, "assets": assets, "body": "notes", "html_url": "https://x"})
    }

    #[test]
    fn picks_the_newest_signed_release_above_this_one() {
        let cur = semver::Version::parse("0.1.0-alpha.5").unwrap();
        let rs = vec![release("0.1.0-alpha.4", true), release("0.1.0-alpha.7", false), release("0.1.0-alpha.6", true), release("0.1.0-alpha.5", true)];
        assert_eq!(newest(&rs, &cur).unwrap().version, "0.1.0-alpha.6");
        assert!(newest(&[release("0.1.0-alpha.5", true)], &cur).is_none());
        // 0.1.0 is above every 0.1.0 pre-release.
        assert_eq!(newest(&[release("0.1.0", true)], &cur).unwrap().version, "0.1.0");
    }

    #[test]
    fn checks_signatures() {
        // Signed with a throwaway key: `tauri signer generate --ci`, `tauri signer sign`.
        let pk = include_str!("../tests/test-key.pub");
        let sig = include_bytes!("../tests/data.txt.sig");
        let data = include_bytes!("../tests/data.txt");
        verify(data, sig, pk).unwrap();
        assert!(verify(b"something else", sig, pk).is_err());
        assert!(verify(data, sig, PUBLIC_KEY).is_err(), "another key must not verify it");
    }
}
