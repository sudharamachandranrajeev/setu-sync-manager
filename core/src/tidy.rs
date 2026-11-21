//! Keeping one copy of a file, not several identical ones. Syncing leaves extra copies behind:
//! conflict copies (`name.sync-conflict-….ext`) and old versions in `.stversions`. When such a
//! copy is the same file as the one it belongs to, it adds nothing, so it is removed.
//!
//! "The same" is checked three ways before anything is removed: same size, same SHA-256, and
//! the same bytes compared one by one. (Modified times are not compared: a conflict copy or an
//! old version carries its own time by design.) Only these sync leftovers are ever touched;
//! files of your own at other paths never are.
//!
//! Every removal is written to `tidy-log.jsonl` in Setu's settings folder with both paths and
//! the hash. Undo puts the copy back from the file that was kept, if that file still holds the
//! same bytes.

use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Error;
use crate::accuracy::sha256_file;
use crate::control::now_secs;
use crate::local::join_rel;
use crate::trust::original_of;

/// One removed copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub at: u64,
    /// "conflict copy" or "old version".
    pub kind: String,
    pub removed: PathBuf,
    pub kept: PathBuf,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub undone: bool,
}

/// Are two files the same in size, SHA-256 and every byte? Returns the hash when they are.
pub fn identical(a: &Path, b: &Path) -> std::io::Result<Option<String>> {
    let (ma, mb) = (std::fs::metadata(a)?, std::fs::metadata(b)?);
    if !ma.is_file() || !mb.is_file() || ma.len() != mb.len() {
        return Ok(None);
    }
    let (ha, hb) = (sha256_file(a)?, sha256_file(b)?);
    if ha != hb {
        return Ok(None);
    }
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    loop {
        let n = read_full(&mut fa, &mut ba)?;
        let m = read_full(&mut fb, &mut bb)?;
        if n != m || ba[..n] != bb[..m] {
            return Ok(None);
        }
        if n == 0 {
            return Ok(Some(ha));
        }
    }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

/// A leftover and the file it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub kind: &'static str,
    pub copy: PathBuf,
    pub original: PathBuf,
}

/// The file an old version in `.stversions` belongs to: `.stversions/a/b~20261120-101010.txt`
/// → `a/b.txt` under the folder.
pub fn version_original(folder: &Path, version: &Path) -> Option<PathBuf> {
    let rel = version.strip_prefix(folder.join(".stversions")).ok()?;
    let name = rel.file_name()?.to_str()?;
    let at = name.rfind('~')?;
    let rest = &name[at + 1..];
    let stamp = rest.get(..15)?;
    let ok = stamp.bytes().enumerate().all(|(i, b)| if i == 8 { b == b'-' } else { b.is_ascii_digit() });
    if !ok {
        return None;
    }
    let orig = format!("{}{}", &name[..at], &rest[15..]);
    Some(folder.join(rel.parent()?).join(orig))
}

/// Leftovers in one synced folder (its folder on disk): conflict copies anywhere in it, and
/// old versions in its `.stversions`.
pub fn candidates(folder: &Path) -> Vec<Candidate> {
    let mut out = Vec::new();
    let versions = folder.join(".stversions");
    for e in walkdir::WalkDir::new(folder).min_depth(1).follow_links(false).into_iter().flatten() {
        if !e.file_type().is_file() {
            continue;
        }
        let p = e.path();
        if p.starts_with(&versions) {
            if let Some(orig) = version_original(folder, p) {
                out.push(Candidate { kind: "old version", copy: p.to_path_buf(), original: orig });
            }
            continue;
        }
        let Ok(rel) = p.strip_prefix(folder) else { continue };
        let Some(rel) = rel.to_str().map(|r| r.replace('\\', "/")) else { continue };
        if let Some(orig) = original_of(&rel) {
            out.push(Candidate { kind: "conflict copy", copy: p.to_path_buf(), original: join_rel(folder, &orig) });
        }
    }
    out
}

pub fn log_path(dir: &Path) -> PathBuf {
    dir.join("tidy-log.jsonl")
}

pub fn read_log(dir: &Path) -> Vec<Removed> {
    let Ok(f) = std::fs::File::open(log_path(dir)) else { return Vec::new() };
    std::io::BufReader::new(f).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()).collect()
}

fn append_log(dir: &Path, r: &Removed) -> Result<(), Error> {
    std::fs::create_dir_all(dir)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(log_path(dir))?;
    writeln!(f, "{}", serde_json::to_string(r).expect("serialise"))?;
    Ok(())
}

/// Remove the leftovers in `folders` that are identical to their file. `allow` says whether a
/// copy's file is in a part the rules sync (copies in parts set to Don't sync are left alone).
/// Returns what was removed. `dir` is Setu's settings folder, for the log.
pub fn tidy(dir: &Path, folders: &[PathBuf], allow: &dyn Fn(&Path) -> bool) -> Result<Vec<Removed>, Error> {
    let mut out = Vec::new();
    for folder in folders {
        for c in candidates(folder) {
            if !allow(&c.original) {
                continue;
            }
            let Ok(Some(sha)) = identical(&c.copy, &c.original) else { continue };
            let size = std::fs::metadata(&c.copy).map(|m| m.len()).unwrap_or(0);
            let r = Removed { at: now_secs(), kind: c.kind.into(), removed: c.copy.clone(), kept: c.original.clone(), size, sha256: sha, undone: false };
            // Log first: a crash between the two leaves a log line, never an unlogged removal.
            append_log(dir, &r)?;
            std::fs::remove_file(&c.copy)?;
            out.push(r);
        }
    }
    Ok(out)
}

/// Put a removed copy back from the kept file, if that still holds the same bytes.
pub fn undo(dir: &Path, removed: &Path) -> Result<(), Error> {
    let mut log = read_log(dir);
    let Some(r) = log.iter_mut().rev().find(|r| r.removed == removed && !r.undone) else {
        return Err(Error::Setup(format!("{} is not in the tidy log", removed.display())));
    };
    if r.removed.exists() {
        return Err(Error::Setup(format!("{} is already there", r.removed.display())));
    }
    if sha256_file(&r.kept).ok().as_deref() != Some(r.sha256.as_str()) {
        return Err(Error::Setup(format!("{} has changed since, so it cannot give the copy back", r.kept.display())));
    }
    if let Some(p) = r.removed.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(&r.kept, &r.removed)?;
    r.undone = true;
    let text: String = log.iter().map(|r| serde_json::to_string(r).expect("serialise") + "\n").collect();
    crate::local::write_atomic(&log_path(dir), &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_only_identical_leftovers_and_can_undo() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("folder");
        let logdir = d.path().join("cfg");
        std::fs::create_dir_all(f.join(".stversions/sub")).unwrap();
        std::fs::create_dir_all(f.join("sub")).unwrap();
        std::fs::write(f.join("sub/a.txt"), "same").unwrap();
        std::fs::write(f.join("sub/a.sync-conflict-20261120-235545-AAAAAAA.txt"), "same").unwrap();
        std::fs::write(f.join("sub/b.txt"), "one").unwrap();
        std::fs::write(f.join("sub/b.sync-conflict-20261120-235545-AAAAAAA.txt"), "two").unwrap();
        std::fs::write(f.join(".stversions/sub/a~20261121-101010.txt"), "same").unwrap();
        std::fs::write(f.join(".stversions/sub/b~20261121-101010.txt"), "older").unwrap();
        // Same content at another path is your own file: never touched.
        std::fs::write(f.join("copy-of-a.txt"), "same").unwrap();

        let removed = tidy(&logdir, std::slice::from_ref(&f), &|_| true).unwrap();
        let mut names: Vec<String> = removed.iter().map(|r| r.removed.strip_prefix(&f).unwrap().to_string_lossy().replace('\\', "/")).collect();
        names.sort();
        assert_eq!(names, vec![".stversions/sub/a~20261121-101010.txt", "sub/a.sync-conflict-20261120-235545-AAAAAAA.txt"]);
        assert!(f.join("sub/b.sync-conflict-20261120-235545-AAAAAAA.txt").exists());
        assert!(f.join(".stversions/sub/b~20261121-101010.txt").exists());
        assert!(f.join("copy-of-a.txt").exists());
        assert_eq!(read_log(&logdir).len(), 2);

        let c = f.join("sub/a.sync-conflict-20261120-235545-AAAAAAA.txt");
        undo(&logdir, &c).unwrap();
        assert_eq!(std::fs::read_to_string(&c).unwrap(), "same");
        assert!(undo(&logdir, &c).is_err(), "already undone");
    }

    #[test]
    fn identical_needs_every_byte() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        std::fs::write(&a, vec![7u8; 3 << 20]).unwrap();
        std::fs::write(&b, vec![7u8; 3 << 20]).unwrap();
        assert!(identical(&a, &b).unwrap().is_some());
        let mut v = vec![7u8; 3 << 20];
        v[(3 << 20) - 1] = 8;
        std::fs::write(&b, v).unwrap();
        assert!(identical(&a, &b).unwrap().is_none());
    }

    #[test]
    fn version_names_map_back() {
        let f = Path::new("/x/f");
        assert_eq!(version_original(f, Path::new("/x/f/.stversions/a/b~20261120-101010.tar.gz")), Some(PathBuf::from("/x/f/a/b.tar.gz")));
        assert_eq!(version_original(f, Path::new("/x/f/.stversions/c~20261120-101010")), Some(PathBuf::from("/x/f/c")));
        assert_eq!(version_original(f, Path::new("/x/f/.stversions/c~notadate")), None);
    }
}
