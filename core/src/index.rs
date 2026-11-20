//! A computer's file list (path, size, modified time), written to the control folder so the
//! other computer can show these files in its tree. Paths that are always ignored are skipped.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::Serialize;

use crate::rules::NEVER_SYNC;
use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub path: String,
    pub dir: bool,
    pub size: u64,
    pub mtime: i64,
}

/// Does an ignore pattern match this single name? Handles the simple, unanchored patterns Setu
/// uses for its ignore lists (`name`, `*.ext`, `prefix*`, `(?d)name`); others never match here.
pub fn name_matches(pattern: &str, name: &str) -> bool {
    let mut p = pattern;
    while let Some(rest) = p.strip_prefix("(?d)").or_else(|| p.strip_prefix("(?i)")) {
        p = rest;
    }
    if p.contains('/') || p.starts_with('!') || p.starts_with("//") {
        return false;
    }
    glob(p.as_bytes(), name.as_bytes())
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some(b'*'), _) => glob(&p[1..], s) || (!s.is_empty() && glob(p, &s[1..])),
        (Some(b'?'), Some(_)) => glob(&p[1..], &s[1..]),
        (Some(a), Some(b)) if a == b => glob(&p[1..], &s[1..]),
        _ => false,
    }
}

/// List everything under `root` except what the ignore lists leave out.
pub fn scan(root: &Path, always_ignore: &[String]) -> Result<Vec<Entry>, Error> {
    let skip = |name: &str| {
        name == ".stfolder"
            || name == ".stversions"
            || name == ".stignore"
            || name.starts_with(".syncthing.")
            || name.starts_with("~syncthing~")
            || NEVER_SYNC.iter().any(|p| name_matches(p, name))
            || always_ignore.iter().any(|p| name_matches(p, name))
    };
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(root).min_depth(1).follow_links(false).into_iter();
    for e in walker.filter_entry(|e| e.file_name().to_str().is_some_and(|n| !skip(n))) {
        let Ok(e) = e else { continue };
        let Ok(rel) = e.path().strip_prefix(root) else { continue };
        let Some(rel) = rel.to_str() else { continue };
        let rel = rel.replace('\\', "/");
        if rel.contains(['\t', '\n', '\r']) {
            continue;
        }
        let Ok(md) = e.metadata() else { continue };
        let mtime = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        out.push(Entry { path: rel, dir: md.is_dir(), size: if md.is_dir() { 0 } else { md.len() }, mtime });
    }
    Ok(out)
}

pub fn write(path: &Path, entries: &[Entry]) -> Result<(), Error> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    writeln!(gz, "setu-index\t1")?;
    for e in entries {
        writeln!(gz, "{}\t{}\t{}\t{}", if e.dir { 'd' } else { 'f' }, e.size, e.mtime, e.path)?;
    }
    let bytes = gz.finish()?;
    let tmp = path.with_file_name(format!(".syncthing.{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("index")));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub fn read(path: &Path) -> Result<Vec<Entry>, Error> {
    let f = std::fs::File::open(path)?;
    let mut lines = BufReader::new(GzDecoder::new(f)).lines();
    match lines.next() {
        Some(Ok(h)) if h == "setu-index\t1" => {}
        _ => return Err(Error::Setup(format!("{} is not a Setu file list", path.display()))),
    }
    let mut out = Vec::new();
    for line in lines {
        let line = line?;
        let mut it = line.splitn(4, '\t');
        let (Some(k), Some(size), Some(mtime), Some(p)) = (it.next(), it.next(), it.next(), it.next()) else { continue };
        out.push(Entry {
            path: p.to_string(),
            dir: k == "d",
            size: size.parse().unwrap_or(0),
            mtime: mtime.parse().unwrap_or(0),
        });
    }
    Ok(out)
}

/// Entries grouped by parent directory, with each directory's total size.
#[derive(Default)]
pub struct Tree {
    pub entries: Vec<Entry>,
    children: HashMap<String, Vec<usize>>,
    by_path: HashMap<String, usize>,
    totals: HashMap<String, (u64, u64)>,
}

impl Tree {
    pub fn new(entries: Vec<Entry>) -> Tree {
        let mut t = Tree { entries, ..Default::default() };
        for (i, e) in t.entries.iter().enumerate() {
            let parent = e.path.rsplit_once('/').map(|(p, _)| p).unwrap_or("").to_string();
            t.children.entry(parent).or_default().push(i);
            t.by_path.insert(e.path.clone(), i);
            if !e.dir {
                let mut p = e.path.as_str();
                loop {
                    let parent = p.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
                    let tot = t.totals.entry(parent.to_string()).or_default();
                    tot.0 += e.size;
                    tot.1 += 1;
                    if parent.is_empty() {
                        break;
                    }
                    p = parent;
                }
            }
        }
        t
    }

    pub fn children(&self, rel: &str) -> impl Iterator<Item = &Entry> {
        self.children.get(rel).into_iter().flatten().map(|&i| &self.entries[i])
    }

    pub fn get(&self, rel: &str) -> Option<&Entry> {
        self.by_path.get(rel).map(|&i| &self.entries[i])
    }

    /// Total bytes and file count under a directory.
    pub fn total(&self, rel: &str) -> (u64, u64) {
        self.totals.get(rel).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_simple_patterns() {
        assert!(name_matches("(?d)node_modules", "node_modules"));
        assert!(name_matches("*.jks", "release.jks"));
        assert!(name_matches("*-android-keystore", "artha-android-keystore"));
        assert!(name_matches(".env.*", ".env.local"));
        assert!(!name_matches(".env.*", ".env"));
        assert!(!name_matches("/anchored", "anchored"));
        assert!(!name_matches("build", "builder"));
    }

    #[test]
    fn scans_writes_and_reads_back() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().join("root");
        std::fs::create_dir_all(r.join("a/node_modules/x")).unwrap();
        std::fs::create_dir_all(r.join("API keys")).unwrap();
        std::fs::write(r.join("a/f.txt"), "hello").unwrap();
        std::fs::write(r.join("a/node_modules/x/y.js"), "y").unwrap();
        std::fs::write(r.join("API keys/k"), "secret").unwrap();
        let ignore = vec!["(?d)node_modules".to_string()];
        let mut e = scan(&r, &ignore).unwrap();
        e.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: Vec<&str> = e.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["a", "a/f.txt"]);
        let f = d.path().join("i.tsv.gz");
        write(&f, &e).unwrap();
        assert_eq!(read(&f).unwrap(), e);
        let t = Tree::new(e);
        assert_eq!(t.total(""), (5, 1));
        assert_eq!(t.children("a").count(), 1);
    }
}
