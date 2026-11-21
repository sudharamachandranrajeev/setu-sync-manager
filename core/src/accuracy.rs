//! Checking that the copies really hold the same bytes, beyond Syncthing's own word for it.
//!
//! Each computer hashes the files the rules sync (SHA-256, remembered by size and modified time
//! so only changed files are read again) and publishes one digest per top folder in its Brahma
//! row. Matching digests mean every file under that folder has the same path, size and content
//! on both computers. When a digest differs, the computer writes that folder's file hashes to
//! the `setu-control` folder, so the other side can name the files that differ.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::Error;
use crate::control::now_secs;
use crate::index::{self, Entry};
use crate::local::join_rel;
use crate::rules::{RuleSet, short_id};
use crate::trust::original_of;

/// One top folder's fingerprint on one computer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Digest {
    pub files: u64,
    pub bytes: u64,
    /// SHA-256 over the sorted `path \t size \t sha256` lines.
    pub hash: String,
    /// When it was worked out (seconds since 1970).
    pub at: u64,
    /// Every Setu folder here was up to date when it was worked out. Digests taken mid-copy
    /// are expected to differ.
    #[serde(default)]
    pub settled: bool,
}

/// Digests by top folder.
pub type Digests = BTreeMap<String, Digest>;

/// A file's hash, as remembered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hashed {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Remembered hashes, keyed by path and checked against size and modified time.
pub struct HashCache {
    conn: Connection,
}

impl HashCache {
    pub fn open(path: &Path) -> Result<HashCache, Error> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let conn = Connection::open(path).map_err(db)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS hashes (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, sha256 TEXT NOT NULL);",
        )
        .map_err(db)?;
        Ok(HashCache { conn })
    }

    pub fn memory() -> HashCache {
        let conn = Connection::open_in_memory().expect("memory db");
        conn.execute_batch("CREATE TABLE hashes (path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL, sha256 TEXT NOT NULL);").expect("schema");
        HashCache { conn }
    }

    /// The file's SHA-256, read again only when its size or modified time changed.
    pub fn hash(&self, root: &Path, e: &Entry) -> Result<String, Error> {
        let known: Option<(i64, i64, String)> = self
            .conn
            .query_row("SELECT size, mtime, sha256 FROM hashes WHERE path = ?", [&e.path], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()
            .map_err(db)?;
        if let Some((size, mtime, sha)) = known
            && size as u64 == e.size
            && mtime == e.mtime
        {
            return Ok(sha);
        }
        let sha = sha256_file(&join_rel(root, &e.path))?;
        self.conn
            .execute("INSERT OR REPLACE INTO hashes (path, size, mtime, sha256) VALUES (?, ?, ?, ?)", params![e.path, e.size as i64, e.mtime, sha])
            .map_err(db)?;
        Ok(sha)
    }
}

fn db(e: rusqlite::Error) -> Error {
    Error::Setup(format!("hash cache: {e}"))
}

/// The top folder a path belongs to ("" for files at the top).
pub fn top_of(path: &str) -> &str {
    match path.split_once('/') {
        Some((t, _)) => t,
        None => "",
    }
}

/// Should this file be in the comparison? It must be synced by the rules, and not one of
/// Syncthing's conflict copies (those are settled or tidied separately).
pub fn compared(rules: &RuleSet, e: &Entry) -> bool {
    !e.dir && !rules.effective(&e.path).0.is_off() && original_of(&e.path).is_none()
}

/// Hash every compared file under `root` and fold them into one digest per top folder.
/// Returns the digests and the hashes by top folder.
pub fn digests(root: &Path, rules: &RuleSet, cache: &HashCache) -> Result<(Digests, BTreeMap<String, Vec<Hashed>>), Error> {
    let mut entries: Vec<Entry> = index::scan(root, &rules.always_ignore)?.into_iter().filter(|e| compared(rules, e)).collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut by_top: BTreeMap<String, Vec<Hashed>> = BTreeMap::new();
    for e in &entries {
        // A file that vanished or cannot be read mid-scan is left out of this round.
        let Ok(sha) = cache.hash(root, e) else { continue };
        by_top.entry(top_of(&e.path).to_string()).or_default().push(Hashed { path: e.path.clone(), size: e.size, sha256: sha });
    }
    let at = now_secs();
    let digests = by_top.iter().map(|(t, list)| (t.clone(), fold(list, at))).collect();
    Ok((digests, by_top))
}

fn fold(list: &[Hashed], at: u64) -> Digest {
    let mut h = Sha256::new();
    let mut bytes = 0;
    for x in list {
        h.update(format!("{}\t{}\t{}\n", x.path, x.size, x.sha256).as_bytes());
        bytes += x.size;
    }
    Digest { files: list.len() as u64, bytes, hash: hex(&h.finalize()), at, settled: false }
}

/// Top folders whose digests differ between two computers (present on one side only counts).
pub fn differing(a: &BTreeMap<String, Digest>, b: &BTreeMap<String, Digest>) -> Vec<String> {
    let tops: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    tops.into_iter().filter(|t| a.get(*t).map(|d| &d.hash) != b.get(*t).map(|d| &d.hash)).cloned().collect()
}

/// The file in the control folder holding one computer's hashes for one top folder.
pub fn hashes_path(control: &Path, device: &str, top: &str) -> PathBuf {
    let key = hex(&Sha256::digest(top.as_bytes()))[..12].to_string();
    control.join(format!("hashes-{}-{key}.tsv.gz", short_id(device)))
}

pub fn write_hashes(path: &Path, list: &[Hashed]) -> Result<(), Error> {
    let tmp = path.with_extension("tmp");
    {
        let f = std::fs::File::create(&tmp)?;
        let mut z = GzEncoder::new(f, Compression::default());
        for x in list {
            writeln!(z, "{}\t{}\t{}", x.size, x.sha256, x.path)?;
        }
        z.finish()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read_hashes(path: &Path) -> Result<Vec<Hashed>, Error> {
    let r = BufReader::new(GzDecoder::new(std::fs::File::open(path)?));
    let mut out = Vec::new();
    for line in r.lines() {
        let line = line?;
        let mut it = line.splitn(3, '\t');
        let (Some(size), Some(sha), Some(path)) = (it.next(), it.next(), it.next()) else { continue };
        out.push(Hashed { path: path.into(), size: size.parse().unwrap_or(0), sha256: sha.into() });
    }
    Ok(out)
}

/// A file that is not the same on both computers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mismatch {
    pub path: String,
    /// "only here", "only there" or "different content".
    pub what: &'static str,
}

pub fn compare(here: &[Hashed], there: &[Hashed]) -> Vec<Mismatch> {
    let t: HashMap<&str, &Hashed> = there.iter().map(|h| (h.path.as_str(), h)).collect();
    let h: HashMap<&str, &Hashed> = here.iter().map(|h| (h.path.as_str(), h)).collect();
    let mut out: Vec<Mismatch> = Vec::new();
    for x in here {
        match t.get(x.path.as_str()) {
            None => out.push(Mismatch { path: x.path.clone(), what: "only here" }),
            Some(y) if y.sha256 != x.sha256 || y.size != x.size => out.push(Mismatch { path: x.path.clone(), what: "different content" }),
            Some(_) => {}
        }
    }
    for y in there {
        if !h.contains_key(y.path.as_str()) {
            out.push(Mismatch { path: y.path.clone(), what: "only there" });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Device, Kind, Mode, Os};

    #[test]
    fn same_bytes_same_digest_and_differences_are_named() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for d in [a.path(), b.path()] {
            std::fs::create_dir_all(d.join("p/sub")).unwrap();
            std::fs::create_dir_all(d.join("off")).unwrap();
            std::fs::write(d.join("p/sub/x.txt"), "hello").unwrap();
            std::fs::write(d.join("top.txt"), "t").unwrap();
            std::fs::write(d.join("off/whatever"), d.to_string_lossy().as_bytes()).unwrap();
            std::fs::write(d.join("p/x.sync-conflict-20261120-235545-AAAAAAA.txt"), d.to_string_lossy().as_bytes()).unwrap();
        }
        let id = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
        let mut devs = BTreeMap::new();
        devs.insert(id.to_string(), Device { name: "a".into(), os: Os::Linux, receive_only: false });
        let mut rules = RuleSet::new(devs);
        rules.set("", Kind::Dir, Some(Mode::Both)).unwrap();
        rules.set("off", Kind::Dir, Some(Mode::Off)).unwrap();
        let (da, ha) = digests(a.path(), &rules, &HashCache::memory()).unwrap();
        let (db_, _) = digests(b.path(), &rules, &HashCache::memory()).unwrap();
        assert_eq!(da.keys().collect::<Vec<_>>(), vec!["", "p"]);
        assert!(differing(&da, &db_).is_empty(), "the off folder and conflict copies are not compared");

        std::fs::write(b.path().join("p/sub/x.txt"), "hellO").unwrap();
        std::fs::write(b.path().join("p/new.txt"), "n").unwrap();
        let (db2, hb) = digests(b.path(), &rules, &HashCache::memory()).unwrap();
        assert_eq!(differing(&da, &db2), vec!["p".to_string()]);
        let m = compare(&ha["p"], &hb["p"]);
        assert_eq!(m, vec![Mismatch { path: "p/new.txt".into(), what: "only there" }, Mismatch { path: "p/sub/x.txt".into(), what: "different content" }]);

        let f = a.path().join("h.tsv.gz");
        write_hashes(&f, &hb["p"]).unwrap();
        assert_eq!(read_hashes(&f).unwrap(), hb["p"]);
    }

    #[test]
    fn cache_reads_a_file_again_only_when_it_changes() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("f"), "one").unwrap();
        let c = HashCache::memory();
        let e = Entry { path: "f".into(), dir: false, size: 3, mtime: 5 };
        let first = c.hash(d.path(), &e).unwrap();
        std::fs::write(d.path().join("f"), "two").unwrap();
        assert_eq!(c.hash(d.path(), &e).unwrap(), first, "same size and time: remembered");
        assert_ne!(c.hash(d.path(), &Entry { mtime: 6, ..e }).unwrap(), first);
    }
}
