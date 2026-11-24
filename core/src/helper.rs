//! The helper's Brahma round, once a minute: sync the shared rows, pair with the person's other
//! computers, apply new rules when safe, and now and then check the copies and tidy duplicates.
//! The person only hears about it when something needs them or changed.

use std::path::Path;
use std::time::{Duration, Instant};

use crate::control::now_secs;
use crate::mesh;
use crate::session::{Auto, Session};
use crate::watch::{notice, Notice};

pub const HUB_EVERY: Duration = Duration::from_secs(60);
/// How often the content check and tidy run by themselves.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// Ask the helper to check now: the app creates this file in Setu's settings folder.
pub const CHECK_REQUEST: &str = "check.request";

pub struct Ticker {
    next_hub: Instant,
    next_check: Instant,
    held_told: Option<u64>,
    failures: u32,
}

impl Default for Ticker {
    fn default() -> Self {
        // The first content check waits ten minutes, so starting up stays light.
        Ticker { next_hub: Instant::now(), next_check: Instant::now() + Duration::from_secs(600), held_told: None, failures: 0 }
    }
}

impl Ticker {
    pub fn due(&self, dir: &Path) -> bool {
        Instant::now() >= self.next_hub || dir.join(CHECK_REQUEST).exists()
    }

    /// One round. Errors are logged and become a notice only when they keep happening.
    pub fn run(&mut self, dir: &Path) -> Vec<Notice> {
        self.next_hub = Instant::now() + HUB_EVERY;
        let mut out = Vec::new();
        let s = match Session::open() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("setu watch: {e}");
                return out;
            }
        };
        if !s.hub.signed_in() {
            return out;
        }
        match s.sync_hub() {
            Ok(_) => self.failures = 0,
            Err(e) => {
                eprintln!("setu watch: Brahma sync: {e}");
                self.failures += 1;
                if self.failures == 5 {
                    out.push(notice("Setu: cannot reach your Brahma server", format!("{e}. Copying between computers goes on; new sync choices wait until the server is back.")));
                }
            }
        }
        let rules = s.shared().ok().flatten();
        if let Err(e) = s.update_me(rules.as_ref(), |_| {}) {
            eprintln!("setu watch: status: {e}");
        }
        match s.hub.devices().and_then(|d| mesh::pair(&s.client, &s.ctx.me, &d)) {
            Ok(added) => {
                for name in added {
                    out.push(notice("Setu: computer added", format!("{name} is in your Brahma account, so Setu connected to it over Tailscale.")));
                }
            }
            Err(e) => eprintln!("setu watch: pairing: {e}"),
        }
        if let Err(e) = s.refresh_index(3600) {
            eprintln!("setu watch: file list: {e}");
        }
        match s.auto_apply() {
            Ok(Auto::Applied { revision, report }) if report.added + report.updated + report.removed > 0 => out.push(notice(
                "Setu: new sync choices in use",
                format!("Revision {revision} is now set up here ({} folders added, {} changed, {} removed).", report.added, report.updated, report.removed),
            )),
            Ok(Auto::Held { revision, reasons }) if self.held_told != Some(revision) => {
                self.held_told = Some(revision);
                out.push(notice(
                    "Setu: new sync choices need you",
                    format!("{} Open Setu on this computer to check and start them.", reasons.first().cloned().unwrap_or_default()),
                ));
            }
            Ok(_) => {}
            Err(e) => eprintln!("setu watch: applying rules: {e}"),
        }
        let asked = dir.join(CHECK_REQUEST);
        if asked.exists() || Instant::now() >= self.next_check {
            let _ = std::fs::remove_file(&asked);
            self.next_check = Instant::now() + CHECK_EVERY;
            out.extend(self.check(&s));
        }
        if s.hub.pending().unwrap_or(0) > 0
            && let Err(e) = s.sync_hub()
        {
            eprintln!("setu watch: Brahma sync: {e}");
        }
        out
    }

    fn check(&self, s: &Session) -> Vec<Notice> {
        let mut out = Vec::new();
        match s.tidy() {
            Ok(removed) if !removed.is_empty() => {
                let bytes: u64 = removed.iter().map(|r| r.size).sum();
                out.push(notice(
                    "Setu: removed duplicate copies",
                    format!("{} copies identical to their file, byte for byte ({}). Undo is in Setu → Duplicates.", removed.len(), size(bytes)),
                ));
            }
            Ok(_) => {}
            Err(e) => eprintln!("setu watch: tidy: {e}"),
        }
        let started = now_secs();
        match s.check_accuracy() {
            Ok(_) => eprintln!("setu watch: content check took {} s", now_secs() - started),
            Err(e) => eprintln!("setu watch: content check: {e}"),
        }
        out
    }
}

fn size(n: u64) -> String {
    match n {
        n if n >= 1 << 30 => format!("{:.1} GB", n as f64 / (1u64 << 30) as f64),
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1u64 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}
