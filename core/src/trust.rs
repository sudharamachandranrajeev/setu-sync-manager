//! Knowing the copies are right, and putting things back when they are not: conflict copies,
//! old versions, and a check of every Setu folder against Syncthing's own records.
//!
//! Nothing here deletes a file. A copy that is not kept goes into the folder's `.stversions`,
//! where Syncthing keeps old versions for 30 days.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

use crate::compile::{self, FolderSpec};
use crate::control::now_secs;
use crate::index::Tree;
use crate::local::join_rel;
use crate::rules::RuleSet;
use crate::syncthing::Client;
use crate::Error;

const MARK: &str = ".sync-conflict-";
/// `YYYYMMDD-HHMMSS-DEVICE7`
const STAMP_LEN: usize = 8 + 1 + 6 + 1 + 7;

/// The file a conflict copy belongs to, or `None` if `rel` is not a conflict copy.
pub fn original_of(rel: &str) -> Option<String> {
    let (dir, name) = rel.rsplit_once('/').map(|(d, n)| (Some(d), n)).unwrap_or((None, rel));
    let at = name.find(MARK)?;
    let rest = &name[at + MARK.len()..];
    let stamp = rest.get(..STAMP_LEN)?;
    let ok = stamp.bytes().enumerate().all(|(i, b)| match i {
        8 | 15 => b == b'-',
        0..=14 => b.is_ascii_digit(),
        _ => b.is_ascii_alphanumeric(),
    });
    if !ok {
        return None;
    }
    let name = format!("{}{}", &name[..at], &rest[STAMP_LEN..]);
    Some(match dir {
        Some(d) => format!("{d}/{name}"),
        None => name,
    })
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub copy: String,
    pub original: String,
    pub copy_size: u64,
    pub copy_mtime: i64,
    pub original_size: Option<u64>,
    pub original_mtime: Option<i64>,
}

/// Conflict copies on this computer, in folders the rules sync.
pub fn conflicts(rules: &RuleSet, here: &Tree) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = here
        .entries
        .iter()
        .filter(|e| !e.dir)
        .filter_map(|e| {
            let original = original_of(&e.path)?;
            if rules.effective(&e.path).0.is_off() {
                return None;
            }
            let o = here.get(&original);
            Some(Conflict {
                copy: e.path.clone(),
                copy_size: e.size,
                copy_mtime: e.mtime,
                original_size: o.map(|o| o.size),
                original_mtime: o.map(|o| o.mtime),
                original,
            })
        })
        .collect();
    out.sort_by(|a, b| a.copy.cmp(&b.copy));
    out
}

/// The Setu folder that holds `rel`, from the rules: the deepest folder whose path contains it.
pub fn folder_for<'a>(specs: &'a [FolderSpec], rel: &str) -> Option<&'a FolderSpec> {
    specs
        .iter()
        .filter(|s| s.rel.is_empty() || rel == s.rel || rel.starts_with(&format!("{}/", s.rel)))
        .max_by_key(|s| s.rel.len())
}

/// Move a file into its Setu folder's `.stversions`, keeping its path, with a time stamp.
fn shelve(root: &Path, folder_rel: &str, rel: &str) -> Result<PathBuf, Error> {
    let inner = if folder_rel.is_empty() { rel } else { &rel[folder_rel.len() + 1..] };
    shelve_in(&join_rel(root, folder_rel), inner)
}

/// Move `inner` (a path inside the Syncthing folder at `folder_dir`) into its `.stversions`.
/// The name `stem~setu-<seconds>.ext` is one Syncthing's version cleaner never removes.
fn shelve_in(folder_dir: &Path, inner: &str) -> Result<PathBuf, Error> {
    let src = join_rel(folder_dir, inner);
    let (dir, name) = inner.rsplit_once('/').unwrap_or(("", inner));
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let base = join_rel(&folder_dir.join(".stversions"), dir);
    std::fs::create_dir_all(&base)?;
    let dest = base.join(format!("{stem}~setu-{}{ext}", now_secs()));
    std::fs::rename(&src, &dest)?;
    Ok(dest)
}

/// Settle a conflict. `keep_copy` = the conflict copy becomes the file; otherwise the file stays.
/// The copy not kept is moved to `.stversions`. Returns where it went.
pub fn resolve(root: &Path, rules: &RuleSet, copy: &str, keep_copy: bool) -> Result<PathBuf, Error> {
    let original = original_of(copy).ok_or_else(|| Error::Rules(format!("{copy} is not a conflict copy")))?;
    let specs = compile::compile(rules).map_err(|p| Error::Rules(format!("{} problem(s) in the rules", p.len())))?;
    let folder = folder_for(&specs, copy).ok_or_else(|| Error::Rules(format!("{copy} is not in a folder Setu syncs")))?;
    if !join_rel(root, copy).is_file() {
        return Err(Error::Rules(format!("{copy} is no longer there")));
    }
    if keep_copy {
        let shelved = if join_rel(root, &original).exists() { Some(shelve(root, &folder.rel, &original)?) } else { None };
        std::fs::rename(join_rel(root, copy), join_rel(root, &original))?;
        Ok(shelved.unwrap_or_else(|| join_rel(root, &original)))
    } else {
        shelve(root, &folder.rel, copy)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    /// Path inside the Syncthing folder.
    pub path: String,
    pub version_time: String,
    pub mod_time: String,
    pub size: u64,
}

/// An old version with the Setu folder it belongs to.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Kept {
    pub folder: String,
    /// Path from this computer's root, as the tree shows it.
    pub rel: String,
    #[serde(flatten)]
    pub version: Version,
}

/// Old versions in one Setu folder, newest first.
pub fn versions(client: &Client, folder: &str) -> Result<Vec<Version>, Error> {
    let v = client.versions(folder)?;
    let mut out = Vec::new();
    for (path, list) in v.as_object().into_iter().flatten() {
        for x in list.as_array().into_iter().flatten() {
            out.push(Version {
                path: path.clone(),
                version_time: x["versionTime"].as_str().unwrap_or_default().to_string(),
                mod_time: x["modTime"].as_str().unwrap_or_default().to_string(),
                size: x["size"].as_u64().unwrap_or(0),
            });
        }
    }
    out.sort_by(|a, b| b.version_time.cmp(&a.version_time).then_with(|| a.path.cmp(&b.path)));
    Ok(out)
}

/// How a version came back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Restored {
    /// Put back in place; the file it replaced is kept in `.stversions`.
    InPlace,
    /// Saved beside the file, because a receive-only folder would pull the current file back.
    Beside(String),
}

/// The file in `.stversions` that holds `v`: `stem~YYYYMMDD-HHMMSS.ext` with the same size.
fn version_file(folder_dir: &Path, v: &Version) -> Result<PathBuf, Error> {
    let (dir, name) = v.path.rsplit_once('/').unwrap_or(("", &v.path));
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    let base = join_rel(&folder_dir.join(".stversions"), dir);
    let utc: String = v.version_time.chars().filter(|c| c.is_ascii_digit()).take(14).collect();
    let utc = if utc.len() == 14 { format!("{}-{}", &utc[..8], &utc[8..]) } else { String::new() };
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    for e in std::fs::read_dir(&base)?.filter_map(|e| e.ok()) {
        let n = e.file_name().to_string_lossy().into_owned();
        let Some(stamp) = n.strip_prefix(&format!("{stem}~")).and_then(|r| r.strip_suffix(ext)) else { continue };
        if stamp.len() == 15 && e.metadata().is_ok_and(|m| m.len() == v.size) {
            found.push((stamp.to_string(), e.path()));
        }
    }
    if let Some((_, p)) = found.iter().find(|(st, _)| *st == utc) {
        return Ok(p.clone());
    }
    match found.as_slice() {
        [(_, p)] => Ok(p.clone()),
        [] => Err(Error::Rules(format!("the old version of {} is no longer in .stversions", v.path))),
        _ => Err(Error::Rules(format!("several old versions of {} match; open .stversions to pick one", v.path))),
    }
}

/// Put an old version back. In a folder this computer sends from, it goes in place and syncs
/// on (the current file is kept in `.stversions`). In a receive-only folder it is saved beside the file instead.
pub fn restore(client: &Client, folder: &str, folder_dir: &Path, receive_only: bool, v: &Version) -> Result<Restored, Error> {
    if receive_only {
        let src = version_file(folder_dir, v)?;
        let (dir, name) = v.path.rsplit_once('/').map(|(d, n)| (Some(d), n)).unwrap_or((None, &v.path));
        let (stem, ext) = match name.rfind('.') {
            Some(i) if i > 0 => (&name[..i], &name[i..]),
            _ => (name, ""),
        };
        let day: String = v.version_time.chars().take(10).collect();
        let mut n = 1;
        let rel = loop {
            let suffix = if n == 1 { String::new() } else { format!(" {n}") };
            let name = format!("{stem} (restored {day}{suffix}){ext}");
            let rel = match dir {
                Some(d) => format!("{d}/{name}"),
                None => name,
            };
            if !join_rel(folder_dir, &rel).exists() {
                break rel;
            }
            n += 1;
        };
        std::fs::copy(&src, join_rel(folder_dir, &rel))?;
        return Ok(Restored::Beside(rel));
    }
    // Not Syncthing's own restore: with staggered versioning it can drop both copies when
    // versions are close together (seen on 2.1.7). Move the current file aside, then copy the
    // old version in; the old version also stays where it was.
    let src = version_file(folder_dir, v)?;
    let dest = join_rel(folder_dir, &v.path);
    if dest.exists() {
        shelve_in(folder_dir, &v.path)?;
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::copy(&src, &dest)?;
    client.scan(folder)?;
    Ok(Restored::InPlace)
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderCheck {
    pub id: String,
    pub rel: String,
    pub folder_type: String,
    /// "idle", "syncing", "scanning", "paused", "missing", ….
    pub state: String,
    /// Files this computer still needs.
    pub need_files: u64,
    pub need_bytes: u64,
    /// The other computer's completion of this folder, in percent, as this computer sees it.
    pub other_completion: Option<f64>,
    /// Files changed here that a receive-only folder does not send back.
    pub local_changes: u64,
    pub errors: Vec<String>,
    /// First paths still needed or failing, for the list.
    pub samples: Vec<String>,
    pub ok: bool,
}

/// Check each Setu folder against Syncthing's records (Syncthing compares block hashes, so a
/// folder at 100 % on both sides with nothing needed holds the same bytes on both computers).
pub fn verify(client: &Client, rules: &RuleSet, me: &str, rescan: bool) -> Result<Vec<FolderCheck>, Error> {
    let specs = compile::compile(rules).map_err(|p| Error::Rules(format!("{} problem(s) in the rules", p.len())))?;
    let other = rules.devices.keys().find(|d| *d != me).cloned().unwrap_or_default();
    let present: Vec<Value> = client.folders()?;
    let mut out = Vec::new();
    for s in &specs {
        let mut c = FolderCheck { id: s.id.clone(), rel: s.rel.clone(), folder_type: compile::folder_type(&s.mode, me).into(), ..Default::default() };
        let Some(f) = present.iter().find(|f| f["id"] == s.id.as_str()) else {
            c.state = "missing".into();
            c.errors.push("not set up here; apply the rules".into());
            out.push(c);
            continue;
        };
        if f["paused"] == true {
            c.state = "paused".into();
            out.push(c);
            continue;
        }
        if rescan {
            let _ = client.scan(&s.id);
        }
        let st = client.folder_status(&s.id)?;
        c.state = st["state"].as_str().unwrap_or("unknown").to_string();
        c.need_files = st["needFiles"].as_u64().unwrap_or(0) + st["needDeletes"].as_u64().unwrap_or(0);
        c.need_bytes = st["needBytes"].as_u64().unwrap_or(0);
        c.local_changes = st["receiveOnlyChangedFiles"].as_u64().unwrap_or(0);
        if let Some(e) = st["error"].as_str().filter(|e| !e.is_empty()) {
            c.errors.push(e.to_string());
        }
        c.other_completion = client.completion(&s.id, &other).ok().and_then(|v| v["completion"].as_f64());
        if let Ok(errs) = client.folder_errors(&s.id) {
            for e in errs["errors"].as_array().into_iter().flatten().take(50) {
                c.errors.push(format!("{}: {}", e["path"].as_str().unwrap_or("?"), e["error"].as_str().unwrap_or("?")));
            }
        }
        if c.need_files > 0
            && let Ok(n) = client.need(&s.id)
        {
            for k in ["progress", "queued", "rest"] {
                for x in n[k].as_array().into_iter().flatten() {
                    if c.samples.len() < 50 {
                        c.samples.push(x["name"].as_str().unwrap_or("?").to_string());
                    }
                }
            }
        }
        // This computer's side only. The other computer's percentage is shown beside it: it can
        // be below 100 for good reasons, such as edits kept locally in a receive-only folder.
        c.ok = c.state == "idle" && c.need_files == 0 && c.errors.is_empty();
        out.push(c);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_original_of_a_conflict_copy() {
        assert_eq!(original_of("a/pre.sync-conflict-20261120-235545-LIBZPNS.txt").as_deref(), Some("a/pre.txt"));
        assert_eq!(original_of("Makefile.sync-conflict-20261120-235545-4I2U2UH").as_deref(), Some("Makefile"));
        assert_eq!(original_of("x.tar.sync-conflict-20261120-235545-AAAAAAA.gz").as_deref(), Some("x.tar.gz"));
        assert_eq!(original_of("a/pre.txt"), None);
        assert_eq!(original_of("a/pre.sync-conflict-nope.txt"), None);
    }

    #[test]
    fn resolving_never_deletes() {
        use crate::rules::{Device, Kind, Mode, Os};
        use std::collections::BTreeMap;
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let mut devs = BTreeMap::new();
        let a = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
        devs.insert(a.to_string(), Device { name: "a".into(), os: Os::Linux });
        devs.insert("BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB".into(), Device { name: "b".into(), os: Os::Linux });
        let mut rs = RuleSet::new(devs);
        rs.set("p", Kind::Dir, Some(Mode::Both)).unwrap();
        std::fs::create_dir_all(root.join("p/s")).unwrap();
        let copy = "p/s/f.sync-conflict-20261120-235545-AAAAAAA.txt";
        std::fs::write(root.join("p/s/f.txt"), "mine").unwrap();
        std::fs::write(root.join(copy), "theirs").unwrap();
        // Keep the copy: it becomes the file; the old file goes to p/.stversions/s/.
        let shelved = resolve(root, &rs, copy, true).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("p/s/f.txt")).unwrap(), "theirs");
        assert!(shelved.starts_with(root.join("p/.stversions/s")));
        assert_eq!(std::fs::read_to_string(&shelved).unwrap(), "mine");
        assert!(!root.join(copy).exists());
        // Keep the file: the copy goes to .stversions.
        std::fs::write(root.join(copy), "again").unwrap();
        let shelved = resolve(root, &rs, copy, false).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("p/s/f.txt")).unwrap(), "theirs");
        assert_eq!(std::fs::read_to_string(&shelved).unwrap(), "again");
        // Outside any synced folder: refused.
        std::fs::create_dir_all(root.join("off")).unwrap();
        std::fs::write(root.join("off/g.sync-conflict-20261120-235545-AAAAAAA"), "x").unwrap();
        assert!(resolve(root, &rs, "off/g.sync-conflict-20261120-235545-AAAAAAA", false).is_err());
    }
}
