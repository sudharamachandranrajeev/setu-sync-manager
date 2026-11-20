//! The rules: one choice per path, inherited by everything below it until another rule
//! overrides it. Both computers share the same rules file (in the `setu-control` folder), so it
//! names devices by their Syncthing device ID and paths relative to each computer's root.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

pub const FORMAT: &str = "setu-rules";
pub const VERSION: u32 = 1;

/// Which way a path syncs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Never synced; each computer keeps its own.
    Off,
    /// Changes on either computer reach the other.
    Both,
    /// Only this device (a Syncthing device ID) sends; the other only receives.
    From(String),
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, Error> {
        match s {
            "off" => Ok(Mode::Off),
            "both" => Ok(Mode::Both),
            _ => match s.strip_prefix("from:") {
                Some(id) if valid_device_id(id) => Ok(Mode::From(id.to_string())),
                _ => Err(Error::Rules(format!("unknown direction {s:?}"))),
            },
        }
    }

    pub fn is_off(&self) -> bool {
        matches!(self, Mode::Off)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Off => f.write_str("off"),
            Mode::Both => f.write_str("both"),
            Mode::From(id) => write!(f, "from:{id}"),
        }
    }
}

impl Serialize for Mode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Mode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Mode, D::Error> {
        let s = String::deserialize(d)?;
        Mode::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Dir,
    File,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub kind: Kind,
    pub mode: Mode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Linux,
    Windows,
    Macos,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Macos
        } else {
            Os::Linux
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub os: Os,
}

/// Never synced, whatever the rules say. These come first in every ignore file, and the first
/// matching line wins in Syncthing, so no rule can include them.
pub const NEVER_SYNC: &[&str] = &[
    "API keys",
    "*-android-keystore",
    "*.jks",
    "*.keystore",
    "key.properties",
    ".env",
    ".env.*",
];

/// The starting "always ignore" list: build output, caches, git data and OS clutter. Editable.
pub const DEFAULT_ALWAYS_IGNORE: &[&str] = &[
    ".git",
    "(?d)build",
    "(?d).dart_tool",
    "(?d)node_modules",
    "(?d).gradle",
    "(?d)target",
    "(?d)dist",
    "(?d).wrangler",
    "(?d).venv",
    "(?d)venv",
    "(?d)__pycache__",
    "(?d).pytest_cache",
    "(?d).mypy_cache",
    "(?d).cxx",
    "(?d)ephemeral",
    "(?d)local.properties",
    "(?d).flutter-plugins",
    "(?d).flutter-plugins-dependencies",
    "(?d).DS_Store",
    "(?d)Thumbs.db",
    "(?d)desktop.ini",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleSet {
    pub format: String,
    pub version: u32,
    /// Goes up by one on every apply, so each computer can tell which rules it has applied.
    pub revision: u64,
    /// The two computers, by Syncthing device ID.
    pub devices: BTreeMap<String, Device>,
    /// The choice for the root folder itself.
    pub root: Mode,
    /// Path (relative, `/`-separated) to its rule.
    #[serde(default)]
    pub rules: BTreeMap<String, Rule>,
    #[serde(default)]
    pub always_ignore: Vec<String>,
}

impl RuleSet {
    /// New rules for two devices: everything off.
    pub fn new(devices: BTreeMap<String, Device>) -> RuleSet {
        RuleSet {
            format: FORMAT.into(),
            version: VERSION,
            revision: 0,
            devices,
            root: Mode::Off,
            rules: BTreeMap::new(),
            always_ignore: DEFAULT_ALWAYS_IGNORE.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn from_json(s: &str) -> Result<RuleSet, Error> {
        let rs: RuleSet = serde_json::from_str(s).map_err(|e| Error::Rules(e.to_string()))?;
        rs.validate()?;
        Ok(rs)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("rules serialise") + "\n"
    }

    pub fn validate(&self) -> Result<(), Error> {
        let bad = |m: String| Err(Error::Rules(m));
        if self.format != FORMAT {
            return bad(format!("not a Setu rules file (format {:?})", self.format));
        }
        if self.version != VERSION {
            return bad(format!("rules version {} is not supported by this Setu", self.version));
        }
        if self.devices.len() != 2 {
            return bad(format!("rules must name exactly two devices, found {}", self.devices.len()));
        }
        for id in self.devices.keys() {
            if !valid_device_id(id) {
                return bad(format!("{id:?} is not a Syncthing device ID"));
            }
        }
        let check_mode = |m: &Mode, at: &str| match m {
            Mode::From(id) if !self.devices.contains_key(id) => {
                Err(Error::Rules(format!("{at}: sends from a device the rules do not name")))
            }
            _ => Ok(()),
        };
        check_mode(&self.root, "root")?;
        for (path, rule) in &self.rules {
            if normalize(path)? != *path {
                return bad(format!("{path:?} is not a clean relative path"));
            }
            check_mode(&rule.mode, path)?;
            let mut p = path.as_str();
            while let Some((parent, _)) = p.rsplit_once('/') {
                if self.rules.get(parent).is_some_and(|r| r.kind == Kind::File) {
                    return bad(format!("{path:?} sits under {parent:?}, which is a file"));
                }
                p = parent;
            }
        }
        Ok(())
    }

    /// The choice that applies at `path` and the path of the rule it comes from (`None` = root).
    pub fn effective(&self, path: &str) -> (&Mode, Option<&str>) {
        let mut p = path;
        loop {
            if p.is_empty() {
                return (&self.root, None);
            }
            if let Some((k, r)) = self.rules.get_key_value(p) {
                return (&r.mode, Some(k.as_str()));
            }
            p = parent(p).unwrap_or("");
        }
    }

    /// Set (`Some`) or clear (`None`, back to inherited) the rule at `path`. A rule equal to
    /// what it would inherit is dropped, so the file stays minimal.
    pub fn set(&mut self, path: &str, kind: Kind, mode: Option<Mode>) -> Result<(), Error> {
        let path = normalize(path)?;
        let before = self.clone();
        if path.is_empty() {
            self.root = mode.unwrap_or(Mode::Off);
        } else {
            self.rules.remove(&path);
            if let Some(mode) = mode {
                let inherited = self.effective(parent(&path).unwrap_or("")).0.clone();
                if mode != inherited {
                    self.rules.insert(path, Rule { kind, mode });
                }
            }
        }
        // Rules below that now repeat what they inherit add nothing.
        let redundant: Vec<String> = self
            .rules
            .iter()
            .filter(|(p, r)| *self.effective(parent(p).unwrap_or("")).0 == r.mode)
            .map(|(p, _)| p.clone())
            .collect();
        for p in redundant {
            self.rules.remove(&p);
        }
        self.validate().inspect_err(|_| *self = before)
    }

    pub fn device_name<'a>(&'a self, id: &'a str) -> &'a str {
        self.devices.get(id).map(|d| d.name.as_str()).unwrap_or(id)
    }
}

/// The parent of a relative path (`""` is the root, which has no parent).
pub fn parent(path: &str) -> Option<&str> {
    if path.is_empty() {
        None
    } else {
        Some(path.rsplit_once('/').map(|(p, _)| p).unwrap_or(""))
    }
}

/// A clean relative path: `/`-separated, no `.` or `..`, no empty parts, no backslashes.
pub fn normalize(path: &str) -> Result<String, Error> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', '\0', '\n', '\r']) {
            return Err(Error::Rules(format!("{path:?} is not a clean relative path")));
        }
    }
    Ok(trimmed.to_string())
}

/// A Syncthing device ID: eight groups of seven base32 characters.
pub fn valid_device_id(id: &str) -> bool {
    let groups: Vec<&str> = id.split('-').collect();
    groups.len() == 8
        && groups
            .iter()
            .all(|g| g.len() == 7 && g.bytes().all(|b| b.is_ascii_uppercase() || (b'2'..=b'7').contains(&b)))
}

/// The first group of a device ID, for file names and short labels.
pub fn short_id(id: &str) -> &str {
    id.split('-').next().unwrap_or(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const A: &str = "AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA-AAAAAAA";
    pub const B: &str = "BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB-BBBBBBB";

    fn two() -> RuleSet {
        let mut d = BTreeMap::new();
        d.insert(A.into(), Device { name: "lin".into(), os: Os::Linux });
        d.insert(B.into(), Device { name: "win".into(), os: Os::Windows });
        RuleSet::new(d)
    }

    #[test]
    fn inherits_from_nearest_rule() {
        let mut rs = two();
        rs.set("a", Kind::Dir, Some(Mode::Both)).unwrap();
        rs.set("a/b/c", Kind::Dir, Some(Mode::From(A.into()))).unwrap();
        assert_eq!(rs.effective("a/x"), (&Mode::Both, Some("a")));
        assert_eq!(rs.effective("a/b/c/d.txt"), (&Mode::From(A.into()), Some("a/b/c")));
        assert_eq!(rs.effective("z"), (&Mode::Off, None));
    }

    #[test]
    fn redundant_rule_is_dropped() {
        let mut rs = two();
        rs.set("a", Kind::Dir, Some(Mode::Both)).unwrap();
        rs.set("a/b", Kind::Dir, Some(Mode::Both)).unwrap();
        assert!(!rs.rules.contains_key("a/b"));
        rs.set("a", Kind::Dir, None).unwrap();
        assert!(rs.rules.is_empty());
    }

    #[test]
    fn round_trips_json() {
        let mut rs = two();
        rs.set("x/y.txt", Kind::File, Some(Mode::From(B.into()))).unwrap();
        assert_eq!(RuleSet::from_json(&rs.to_json()).unwrap(), rs);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(normalize("a/../b").is_err());
        assert!(normalize("a\\b").is_err());
        assert_eq!(normalize("/a/b/").unwrap(), "a/b");
        let mut rs = two();
        assert!(rs.set("a", Kind::Dir, Some(Mode::From("CCCCCCC-CCCCCCC-CCCCCCC-CCCCCCC-CCCCCCC-CCCCCCC-CCCCCCC-CCCCCCC".into()))).is_err());
        assert!(Mode::parse("sideways").is_err());
        assert!(!valid_device_id("abc"));
    }

    #[test]
    fn nothing_under_a_file() {
        let mut rs = two();
        rs.set("f", Kind::File, Some(Mode::Both)).unwrap();
        assert!(rs.set("f/g", Kind::File, Some(Mode::Off)).is_err());
    }
}
