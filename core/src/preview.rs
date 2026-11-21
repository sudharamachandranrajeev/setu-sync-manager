//! What applying the rules will do to files, worked out from both computers' file lists before
//! anything changes.
//!
//! Measured on Syncthing 2.1.7 (see `tests/two_syncthings.rs`): when a file differs on the two
//! computers at the first sync, nothing is lost. One way, the sender's copy replaces the
//! receiver's and the receiver's old copy is kept beside it as a `.sync-conflict-…` file. Both
//! ways, the newer copy wins and the older is kept as a conflict copy on both computers.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::index::{Entry, Tree};
use crate::rules::{Mode, Os, RuleSet};

/// More than this many files replaced on one side needs a second confirmation…
pub const GUARD_FILES: u64 = 50;
/// …as does more than this share (in percent) of a group's files, once at least
/// `GUARD_MIN_FILES` are involved.
pub const GUARD_PERCENT: u64 = 5;
pub const GUARD_MIN_FILES: u64 = 5;
/// Modified times closer than this count as equal (FAT and some copies round to 2 s).
const MTIME_SLACK: i64 = 2;
/// Windows' classic path limit; longer paths fail in many Windows programs.
pub const WINDOWS_MAX_PATH: usize = 260;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Count {
    pub files: u64,
    pub bytes: u64,
}

impl Count {
    fn add(&mut self, bytes: u64) {
        self.files += 1;
        self.bytes += bytes;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    /// Copied from this computer to the other.
    Send,
    /// Copied from the other computer to this one.
    Receive,
    /// Exists on both but differs. `winner_here` says whose copy wins; the other is kept as a
    /// conflict copy.
    #[serde(rename_all = "camelCase")]
    Replace { winner_here: bool },
    /// Only on the receiving side of a one-way rule; it stays there and is not sent back.
    StaysHere,
    StaysThere,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub path: String,
    pub change: Change,
    pub size: u64,
}

/// One rule's area: every file whose direction comes from the same rule path.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    /// The rule path (`""` = the whole folder).
    pub rel: String,
    pub mode: String,
    pub send: Count,
    pub receive: Count,
    /// Files on this computer replaced by the other's copy (this copy kept as a conflict copy).
    pub replaced_here: Count,
    /// Files on the other computer replaced by this one's copy.
    pub replaced_there: Count,
    pub stays_here: Count,
    pub stays_there: Count,
    pub same: Count,
    pub files_here: u64,
    pub files_there: u64,
    /// The first items that replace a file, for the review list.
    pub samples: Vec<Item>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NameProblem {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    pub groups: Vec<Group>,
    /// Reasons a second confirmation is needed before applying.
    pub guard: Vec<String>,
    /// Names Windows cannot hold, for files that would be copied to a Windows computer.
    pub windows_names: Vec<NameProblem>,
    /// The other computer's file list has not arrived, so only this side is known.
    pub other_unknown: bool,
}

const SAMPLES: usize = 200;
const NAME_PROBLEMS: usize = 500;

fn differs(a: &Entry, b: &Entry) -> bool {
    a.size != b.size || (a.mtime - b.mtime).abs() > MTIME_SLACK
}

/// Work out the file changes for `rules`, from this computer's list (`here`) and the other's
/// (`there`, `None` when it has not arrived).
pub fn preview(rules: &RuleSet, me: &str, here: &Tree, there: Option<&Tree>, other_root_len: Option<usize>) -> Preview {
    let empty = Tree::default();
    let there_t = there.unwrap_or(&empty);
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut out = Preview { other_unknown: there.is_none(), ..Default::default() };
    let other = rules.devices.keys().find(|d| *d != me).cloned().unwrap_or_default();
    let other_windows = rules.devices.get(&other).is_some_and(|d| d.os == Os::Windows);

    let mut paths: Vec<&str> = here.entries.iter().chain(there_t.entries.iter()).filter(|e| !e.dir).map(|e| e.path.as_str()).collect();
    paths.sort_unstable();
    paths.dedup();
    let mut to_other: Vec<&str> = Vec::new();

    for p in paths {
        let (mode, from) = rules.effective(p);
        if mode.is_off() {
            continue;
        }
        let g = groups.entry(from.unwrap_or("").to_string()).or_insert_with(|| Group {
            rel: from.unwrap_or("").to_string(),
            mode: mode.to_string(),
            ..Default::default()
        });
        let (h, t) = (here.get(p).filter(|e| !e.dir), there_t.get(p).filter(|e| !e.dir));
        let i_send = matches!(mode, Mode::Both) || *mode == Mode::From(me.to_string());
        let they_send = matches!(mode, Mode::Both) || *mode == Mode::From(other.clone());
        if h.is_some() {
            g.files_here += 1;
        }
        if t.is_some() {
            g.files_there += 1;
        }
        let change = match (h, t) {
            (Some(h), None) if i_send => {
                g.send.add(h.size);
                to_other.push(p);
                Change::Send
            }
            (Some(h), None) => {
                g.stays_here.add(h.size);
                Change::StaysHere
            }
            (None, Some(t)) if they_send => {
                g.receive.add(t.size);
                Change::Receive
            }
            (None, Some(t)) => {
                g.stays_there.add(t.size);
                Change::StaysThere
            }
            (Some(h), Some(t)) if !differs(h, t) => {
                g.same.add(h.size);
                continue;
            }
            (Some(h), Some(t)) => {
                let winner_here = match mode {
                    Mode::Both => h.mtime >= t.mtime,
                    _ => i_send,
                };
                if winner_here {
                    g.replaced_there.add(t.size);
                    to_other.push(p);
                } else {
                    g.replaced_here.add(h.size);
                }
                Change::Replace { winner_here }
            }
            (None, None) => continue,
        };
        if matches!(change, Change::Replace { .. }) && g.samples.len() < SAMPLES {
            g.samples.push(Item { path: p.to_string(), size: h.or(t).map(|e| e.size).unwrap_or(0), change });
        }
    }

    for g in groups.values() {
        let name = if g.rel.is_empty() { "the whole folder".to_string() } else { g.rel.clone() };
        for (n, of, side) in [
            (g.replaced_here.files, g.files_here, "this computer"),
            (g.replaced_there.files, g.files_there, "the other computer"),
        ] {
            if n > GUARD_FILES || (n >= GUARD_MIN_FILES && n * 100 > of * GUARD_PERCENT) {
                out.guard.push(format!(
                    "{name}: {n} of {of} files on {side} will be replaced (each old copy is kept as a conflict copy)"
                ));
            }
        }
    }
    if other_windows {
        out.windows_names = windows_names(&to_other, other_root_len.unwrap_or(0), there_t);
    }
    out.groups = groups.into_values().collect();
    out
}

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Why Windows cannot hold this one name, if it cannot.
pub fn windows_name_problem(name: &str) -> Option<&'static str> {
    if name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*') || (c as u32) < 32) {
        return Some("has a character Windows does not allow (< > : \" \\ | ? * or a control character)");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some("ends with a dot or a space, which Windows drops");
    }
    let stem = name.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        return Some("is a name Windows reserves for devices");
    }
    None
}

/// Problems for files that would be copied to Windows: bad names, over-long paths, and names
/// that clash with another only by upper and lower case.
pub fn windows_names(paths: &[&str], root_len: usize, there: &Tree) -> Vec<NameProblem> {
    let mut out = Vec::new();
    let mut seen_dirs: HashMap<String, ()> = HashMap::new();
    // Lower-cased path → the spelling first seen, from what is already on Windows plus what arrives.
    let mut folded: HashMap<String, String> = there.entries.iter().map(|e| (e.path.to_lowercase(), e.path.clone())).collect();
    let mut clashed: Vec<String> = Vec::new();
    for p in paths {
        if clashed.iter().any(|c| p.starts_with(c.as_str())) {
            continue;
        }
        // Each directory on the way is checked once.
        let mut acc = String::new();
        for (i, part) in p.split('/').enumerate() {
            if i > 0 {
                acc.push('/');
            }
            acc.push_str(part);
            if seen_dirs.insert(acc.clone(), ()).is_some() {
                continue;
            }
            if let Some(why) = windows_name_problem(part) {
                out.push(NameProblem { path: acc.clone(), message: format!("“{part}” {why}") });
            }
            match folded.get(&acc.to_lowercase()) {
                Some(other) if *other != acc => {
                    out.push(NameProblem {
                        path: acc.clone(),
                        message: format!("clashes with “{other}”: Windows treats names that differ only in case as the same"),
                    });
                    // Everything inside clashes too; the top one says it all.
                    clashed.push(format!("{acc}/"));
                    break;
                }
                Some(_) => {}
                None => {
                    folded.insert(acc.to_lowercase(), acc.clone());
                }
            }
        }
        let full = root_len + 1 + p.chars().count();
        if full > WINDOWS_MAX_PATH {
            out.push(NameProblem { path: p.to_string(), message: format!("the full path would be {full} characters; many Windows programs stop at {WINDOWS_MAX_PATH}") });
        }
        if out.len() >= NAME_PROBLEMS {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Device, Kind};

    const A: &str = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
    const B: &str = "BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB";

    fn rules() -> RuleSet {
        let mut d = BTreeMap::new();
        d.insert(A.to_string(), Device { name: "a".into(), os: Os::Linux, receive_only: false });
        d.insert(B.to_string(), Device { name: "b".into(), os: Os::Windows, receive_only: false });
        RuleSet::new(d)
    }

    fn f(path: &str, size: u64, mtime: i64) -> Entry {
        Entry { path: path.into(), dir: false, size, mtime }
    }

    #[test]
    fn counts_each_kind_of_change() {
        let mut rs = rules();
        rs.set("out", Kind::Dir, Some(Mode::From(A.into()))).unwrap();
        rs.set("both", Kind::Dir, Some(Mode::Both)).unwrap();
        let here = Tree::new(vec![
            f("out/new.txt", 10, 1),
            f("out/same.txt", 5, 100),
            f("out/diff.txt", 7, 100),
            f("both/mine.txt", 3, 1),
            f("both/newer-here.txt", 4, 500),
            f("off/x.txt", 9, 1),
        ]);
        let there = Tree::new(vec![
            f("out/same.txt", 5, 101),
            f("out/diff.txt", 8, 900),
            f("out/only-there.txt", 2, 1),
            f("both/theirs.txt", 6, 1),
            f("both/newer-here.txt", 4, 100),
        ]);
        let p = preview(&rs, A, &here, Some(&there), Some(12));
        assert!(!p.other_unknown);
        let both = p.groups.iter().find(|g| g.rel == "both").unwrap();
        assert_eq!((both.send.files, both.receive.files, both.replaced_there.files), (1, 1, 1));
        let out = p.groups.iter().find(|g| g.rel == "out").unwrap();
        assert_eq!(out.send, Count { files: 1, bytes: 10 });
        assert_eq!(out.same.files, 1, "mtimes within 2 s count as the same");
        assert_eq!(out.replaced_there.files, 1, "one way, the sender wins even when older");
        assert_eq!(out.stays_there.files, 1);
        assert!(p.groups.iter().all(|g| g.rel != "off" && !g.rel.is_empty()));
        assert!(p.guard.is_empty());
    }

    #[test]
    fn many_replacements_need_a_second_yes() {
        let mut rs = rules();
        rs.set("d", Kind::Dir, Some(Mode::From(B.into()))).unwrap();
        let mine: Vec<Entry> = (0..60).map(|i| f(&format!("d/{i}"), 1, 1)).collect();
        let theirs: Vec<Entry> = (0..60).map(|i| f(&format!("d/{i}"), 2, 1)).collect();
        let p = preview(&rs, A, &Tree::new(mine), Some(&Tree::new(theirs)), None);
        assert_eq!(p.guard.len(), 1, "{:?}", p.guard);
        assert!(p.guard[0].contains("60 of 60 files on this computer"));
        // Five of a hundred is not more than 5 %.
        let mine: Vec<Entry> = (0..100).map(|i| f(&format!("d/{i}"), 1, 1)).collect();
        let theirs: Vec<Entry> = (0..100).map(|i| f(&format!("d/{i}"), if i < 5 { 2 } else { 1 }, 1)).collect();
        assert!(preview(&rs, A, &Tree::new(mine), Some(&Tree::new(theirs)), None).guard.is_empty());
    }

    #[test]
    fn windows_names_are_checked_for_files_going_there() {
        let mut rs = rules();
        rs.set("", Kind::Dir, Some(Mode::From(A.into()))).unwrap();
        let long = format!("deep/{}", "x".repeat(250));
        let here = Tree::new(vec![
            f("ok.txt", 1, 1),
            f("a:b.txt", 1, 1),
            f("aux.txt", 1, 1),
            f("trail. /x", 1, 1),
            f("Readme.md", 1, 1),
            f(&long, 1, 1),
        ]);
        let there = Tree::new(vec![f("README.md", 1, 1)]);
        let p = preview(&rs, A, &here, Some(&there), Some(12));
        let bad: Vec<&str> = p.windows_names.iter().map(|n| n.path.as_str()).collect();
        assert!(bad.contains(&"a:b.txt"));
        assert!(bad.contains(&"aux.txt"));
        assert!(bad.contains(&"trail. "));
        assert!(bad.contains(&"Readme.md"), "case clash with README.md");
        assert!(bad.contains(&long.as_str()));
        assert!(!bad.contains(&"ok.txt"));
        // A clashing folder is listed once, not once per file inside.
        let here = Tree::new(vec![f("proj/a", 1, 1), f("proj/b", 1, 1), f("proj/c/d", 1, 1)]);
        let there = Tree::new(vec![f("Proj/a", 1, 1)]);
        let p = preview(&rs, A, &here, Some(&there), Some(12));
        assert_eq!(p.windows_names.len(), 1, "{:?}", p.windows_names);
        // Nothing to check when the receiver is not Windows.
        let p = preview(&rs, B, &here, Some(&there), Some(12));
        assert!(p.windows_names.is_empty());
    }

    #[test]
    fn unknown_other_side() {
        let mut rs = rules();
        rs.set("", Kind::Dir, Some(Mode::Both)).unwrap();
        let p = preview(&rs, A, &Tree::new(vec![f("x", 1, 1)]), None, None);
        assert!(p.other_unknown);
        assert_eq!(p.groups[0].send.files, 1);
    }
}
