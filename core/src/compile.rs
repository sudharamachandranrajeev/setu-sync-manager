//! Turns the rules tree into Syncthing folders.
//!
//! Syncthing gives a whole folder one direction, so every directory whose direction differs from
//! the folder around it becomes a Syncthing folder of its own, and the folder around it ignores
//! that subtree. No file is ever inside two Syncthing folders.
//!
//! Files can be switched off anywhere. A file can only have its own direction when the directory
//! it is in is off: that directory then becomes a "files only" folder that syncs just the chosen
//! files and ignores everything else.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::rules::{parent, Kind, Mode, Os, RuleSet, NEVER_SYNC};

/// Prefix of every Syncthing folder ID that Setu manages.
pub const FOLDER_PREFIX: &str = "setu-f-";

pub const HEADER: &str = "// Managed by Setu. Change what syncs in Setu; edits here are replaced.";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSpec {
    pub id: String,
    /// Path relative to the root (`""` = the root itself).
    pub rel: String,
    pub mode: Mode,
    pub files_only: bool,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Line {
    /// Sync this path (relative to the folder). Only used in files-only folders.
    Include(String),
    /// Do not sync this path (relative to the folder).
    Exclude(String),
    /// Do not sync anything not included above.
    ExcludeAll,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

/// The Syncthing folder ID for a path: the same on both computers.
pub fn folder_id(rel: &str) -> String {
    let h = Sha256::digest(rel.as_bytes());
    let hex: String = h.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{FOLDER_PREFIX}{hex}")
}

pub fn compile(rs: &RuleSet) -> Result<Vec<FolderSpec>, Vec<Problem>> {
    let mut problems = Vec::new();
    // Folder roots: rel -> (mode, files_only)
    let mut roots: BTreeMap<String, (Mode, bool)> = BTreeMap::new();
    if !rs.root.is_off() {
        roots.insert(String::new(), (rs.root.clone(), false));
    }
    let eff_parent = |p: &str| rs.effective(parent(p).unwrap_or("")).0.clone();

    for (p, r) in &rs.rules {
        if r.kind == Kind::Dir && !r.mode.is_off() && eff_parent(p) != r.mode {
            roots.insert(p.clone(), (r.mode.clone(), false));
        }
    }

    // Files with their own direction.
    let mut files_only: BTreeMap<String, Vec<(String, Mode)>> = BTreeMap::new();
    for (p, r) in &rs.rules {
        if r.kind != Kind::File || r.mode.is_off() {
            continue;
        }
        let pe = eff_parent(p);
        if pe == r.mode {
            continue;
        }
        if pe.is_off() {
            let dir = parent(p).unwrap_or("").to_string();
            let name = p.rsplit('/').next().unwrap_or(p).to_string();
            files_only.entry(dir).or_default().push((name, r.mode.clone()));
        } else {
            problems.push(Problem {
                path: p.clone(),
                message: "This file is set to go a different way from the folder it is in. A file \
                          can only have its own direction when its folder is off."
                    .into(),
            });
        }
    }
    let mut files_only_names: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (dir, files) in files_only {
        let mode = files[0].1.clone();
        if files.iter().any(|(_, m)| *m != mode) {
            problems.push(Problem {
                path: dir.clone(),
                message: "Files in this folder are set to go different ways. One folder can only \
                          sync one way: put them in separate folders, or give them the same direction."
                    .into(),
            });
            continue;
        }
        roots.insert(dir.clone(), (mode, true));
        files_only_names.insert(dir, files.into_iter().map(|(n, _)| n).collect());
    }
    if !problems.is_empty() {
        return Err(problems);
    }

    let nearest_root = |p: &str| -> Option<&str> {
        let mut q = parent(p);
        while let Some(c) = q {
            if let Some((k, _)) = roots.get_key_value(c) {
                return Some(k.as_str());
            }
            q = parent(c);
        }
        None
    };

    let mut out = Vec::new();
    for (rel, (mode, is_files_only)) in &roots {
        let mut lines = BTreeSet::new();
        if *is_files_only {
            for n in &files_only_names[rel] {
                lines.insert(Line::Include(n.clone()));
            }
            lines.insert(Line::ExcludeAll);
        } else {
            // Everything this folder must leave out: nested folders, and paths switched off.
            let candidates = rs.rules.keys().map(String::as_str).chain(roots.keys().map(String::as_str));
            for p in candidates {
                if p == rel || nearest_root(p) != Some(rel.as_str()) {
                    continue;
                }
                let inner = if rel.is_empty() { p } else { &p[rel.len() + 1..] };
                let is_root = roots.contains_key(p);
                let off_here = rs.rules.get(p).is_some_and(|r| r.mode.is_off()) && !eff_parent(p).is_off();
                if is_root || off_here {
                    lines.insert(Line::Exclude(inner.to_string()));
                }
            }
            // A path under one already left out needs no line of its own.
            let excluded: Vec<String> = lines.iter().filter_map(|l| if let Line::Exclude(p) = l { Some(p.clone()) } else { None }).collect();
            lines.retain(|l| match l {
                Line::Exclude(p) => !excluded.iter().any(|q| p.starts_with(&format!("{q}/"))),
                _ => true,
            });
        }
        out.push(FolderSpec {
            id: folder_id(rel),
            rel: rel.clone(),
            mode: mode.clone(),
            files_only: *is_files_only,
            lines: lines.into_iter().collect(),
        });
    }
    Ok(out)
}

/// The `.stignore` text for a folder on a computer running `os`.
pub fn render(spec: &FolderSpec, always_ignore: &[String], os: Os) -> Vec<String> {
    let mut v = vec![HEADER.to_string()];
    v.extend(NEVER_SYNC.iter().map(|s| s.to_string()));
    v.extend(always_ignore.iter().cloned());
    for l in &spec.lines {
        v.push(match l {
            Line::Include(p) => format!("!/{}", escape(p, os)),
            Line::Exclude(p) => format!("/{}", escape(p, os)),
            Line::ExcludeAll => "*".to_string(),
        });
    }
    v
}

/// Escape glob characters so a path matches only itself. Syncthing's escape character is `\`,
/// except on Windows, where `\` separates paths and `|` escapes instead.
pub fn escape(path: &str, os: Os) -> String {
    let esc = if os == Os::Windows { '|' } else { '\\' };
    let mut s = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\' | '|') {
            s.push(esc);
        }
        s.push(c);
    }
    s
}

/// What this device's Syncthing folder type is for a mode.
/// The Syncthing folder type for `device`. A backup copy (`receive_only`) only ever receives.
pub fn folder_type(mode: &Mode, device: &str, receive_only: bool) -> &'static str {
    match mode {
        Mode::Both | Mode::From(_) if receive_only => "receiveonly",
        Mode::Both | Mode::Off => "sendreceive",
        Mode::From(id) if id == device => "sendonly",
        Mode::From(_) => "receiveonly",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Device, Kind};
    use std::collections::BTreeMap;

    const A: &str = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
    const B: &str = "BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB";

    fn rs() -> RuleSet {
        let mut d = BTreeMap::new();
        d.insert(A.into(), Device { name: "lin".into(), os: Os::Linux, receive_only: false });
        d.insert(B.into(), Device { name: "win".into(), os: Os::Windows, receive_only: false });
        RuleSet::new(d)
    }
    fn from_a() -> Mode {
        Mode::From(A.into())
    }
    fn from_b() -> Mode {
        Mode::From(B.into())
    }
    fn by_rel(v: &[FolderSpec]) -> BTreeMap<&str, &FolderSpec> {
        v.iter().map(|f| (f.rel.as_str(), f)).collect()
    }
    fn ex(p: &str) -> Line {
        Line::Exclude(p.into())
    }

    #[test]
    fn all_off_means_no_folders() {
        assert!(compile(&rs()).unwrap().is_empty());
    }

    #[test]
    fn root_one_way_is_one_folder() {
        let mut r = rs();
        r.set("", Kind::Dir, Some(from_a())).unwrap();
        let f = compile(&r).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].rel, "");
        assert_eq!(f[0].mode, from_a());
        assert!(f[0].lines.is_empty());
        assert_eq!(folder_type(&f[0].mode, A, false), "sendonly");
        assert_eq!(folder_type(&f[0].mode, B, false), "receiveonly");
        assert_eq!(folder_type(&Mode::Both, B, true), "receiveonly");
        assert_eq!(folder_type(&Mode::Both, A, false), "sendreceive");
    }

    #[test]
    fn users_example_everything_out_some_back() {
        // Root Linux -> Windows; two Windows folders come back; one is both ways; one off.
        let mut r = rs();
        r.set("", Kind::Dir, Some(from_a())).unwrap();
        r.set("notes", Kind::Dir, Some(from_b())).unwrap();
        r.set("shared", Kind::Dir, Some(Mode::Both)).unwrap();
        r.set("shared/tmp", Kind::Dir, Some(Mode::Off)).unwrap();
        r.set("big", Kind::Dir, Some(Mode::Off)).unwrap();
        let f = compile(&r).unwrap();
        let m = by_rel(&f);
        assert_eq!(m.len(), 3);
        assert_eq!(m[""].lines, vec![ex("big"), ex("notes"), ex("shared")]);
        assert_eq!(m["notes"].mode, from_b());
        assert!(m["notes"].lines.is_empty());
        assert_eq!(m["shared"].lines, vec![ex("tmp")]);
    }

    #[test]
    fn folder_under_off_folder_stands_alone() {
        let mut r = rs();
        r.set("", Kind::Dir, Some(Mode::Both)).unwrap();
        r.set("a", Kind::Dir, Some(Mode::Off)).unwrap();
        r.set("a/b", Kind::Dir, Some(Mode::Both)).unwrap();
        let m_ = compile(&r).unwrap();
        let m = by_rel(&m_);
        assert_eq!(m[""].lines, vec![ex("a")]);
        assert_eq!(m["a/b"].mode, Mode::Both);
        assert!(m["a/b"].lines.is_empty());
    }

    #[test]
    fn nested_folder_excluded_only_by_nearest() {
        let mut r = rs();
        r.set("", Kind::Dir, Some(from_a())).unwrap();
        r.set("x", Kind::Dir, Some(Mode::Both)).unwrap();
        r.set("x/y", Kind::Dir, Some(from_b())).unwrap();
        let f = compile(&r).unwrap();
        let m = by_rel(&f);
        assert_eq!(m[""].lines, vec![ex("x")]);
        assert_eq!(m["x"].lines, vec![ex("y")]);
        assert!(m["x/y"].lines.is_empty());
    }

    #[test]
    fn file_off_inside_a_folder() {
        let mut r = rs();
        r.set("d", Kind::Dir, Some(Mode::Both)).unwrap();
        r.set("d/secret.txt", Kind::File, Some(Mode::Off)).unwrap();
        let f = compile(&r).unwrap();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].lines, vec![ex("secret.txt")]);
    }

    #[test]
    fn files_with_own_direction_in_off_folder() {
        let mut r = rs();
        r.set("a.txt", Kind::File, Some(from_a())).unwrap();
        r.set("b.txt", Kind::File, Some(from_a())).unwrap();
        r.set("sub", Kind::Dir, Some(Mode::Both)).unwrap();
        let f = compile(&r).unwrap();
        let m = by_rel(&f);
        assert!(m[""].files_only);
        assert_eq!(m[""].lines, vec![Line::Include("a.txt".into()), Line::Include("b.txt".into()), Line::ExcludeAll]);
        assert_eq!(m["sub"].mode, Mode::Both);
        let text = render(m[""], &[], Os::Linux);
        let pos = |s: &str| text.iter().position(|l| l == s).unwrap();
        assert!(pos("*.jks") < pos("!/a.txt"), "secrets must win over includes");
        assert!(pos("!/b.txt") < pos("*"));
    }

    #[test]
    fn conflicting_directions_are_problems() {
        let mut r = rs();
        r.set("d", Kind::Dir, Some(Mode::Both)).unwrap();
        r.set("d/f.txt", Kind::File, Some(from_a())).unwrap();
        r.set("g.txt", Kind::File, Some(from_a())).unwrap();
        r.set("h.txt", Kind::File, Some(from_b())).unwrap();
        let p = compile(&r).unwrap_err();
        let paths: Vec<&str> = p.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, vec!["d/f.txt", ""]);
    }

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(folder_id("a"), folder_id("a"));
        assert_ne!(folder_id("a"), folder_id("b"));
        assert!(folder_id("").starts_with(FOLDER_PREFIX));
        assert_eq!(folder_id("x").len(), FOLDER_PREFIX.len() + 12);
    }

    #[test]
    fn escapes_globs_per_os() {
        assert_eq!(escape("a[1]*.txt", Os::Linux), "a\\[1\\]\\*.txt");
        assert_eq!(escape("a[1].txt", Os::Windows), "a|[1|].txt");
        let spec = FolderSpec { id: "x".into(), rel: String::new(), mode: Mode::Both, files_only: false, lines: vec![ex("we{ird}")] };
        assert_eq!(render(&spec, &[], Os::Linux).last().unwrap(), "/we\\{ird\\}");
    }

    #[test]
    fn every_file_lands_in_at_most_one_folder() {
        // Brute force over a small tree with every mode combination at three levels.
        let modes = [Mode::Off, Mode::Both, from_a(), from_b()];
        let paths = ["", "a", "a/b", "a/b/c.txt", "a/d.txt", "e.txt"];
        for m0 in &modes {
            for m1 in &modes {
                for m2 in &modes {
                    let mut r = rs();
                    r.set("", Kind::Dir, Some(m0.clone())).unwrap();
                    r.set("a", Kind::Dir, Some(m1.clone())).unwrap();
                    r.set("a/b", Kind::Dir, Some(m2.clone())).unwrap();
                    let Ok(f) = compile(&r) else { continue };
                    for p in paths.iter().filter(|p| p.ends_with(".txt")) {
                        let want = r.effective(p).0.clone();
                        let owners: Vec<&FolderSpec> = f.iter().filter(|s| covers(s, p)).collect();
                        if want.is_off() {
                            assert!(owners.is_empty(), "{p} off but synced by {owners:?}");
                        } else {
                            assert_eq!(owners.len(), 1, "{p} in {owners:?} ({m0:?},{m1:?},{m2:?})");
                            assert_eq!(owners[0].mode, want);
                        }
                    }
                }
            }
        }
    }

    /// Does this folder sync this file? A simple model of Syncthing's first-match-wins rules for
    /// the anchored lines Setu writes.
    fn covers(s: &FolderSpec, file: &str) -> bool {
        let inner = if s.rel.is_empty() {
            file
        } else if let Some(x) = file.strip_prefix(&format!("{}/", s.rel)) {
            x
        } else {
            return false;
        };
        for l in &s.lines {
            match l {
                Line::Include(p) if inner == p => return true,
                Line::Exclude(p) if inner == p || inner.starts_with(&format!("{p}/")) => return false,
                Line::ExcludeAll => return false,
                _ => {}
            }
        }
        true
    }
}
