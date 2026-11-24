//! `setu`: Setu on the command line. The desktop app does the same through the same core.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use setu_core::apply::{Action, Plan};
use setu_core::index::Tree;
use setu_core::local::{config_dir, LocalConfig};
use setu_core::preview::Preview;
use setu_core::rules::{short_id, Kind, Mode, RuleSet};
use setu_core::session::Session;
use setu_core::watch;
use setu_core::Error;

const USAGE: &str = "\
setu: choose what syncs between your computers, and which way.
Most commands talk to the running Setu app, which holds the sync engine.

  setu init --root <folder>          Set this computer's folder (once)
  setu sign-in <server> <email>      Sign in to your Brahma account (asks for the password).
                                     Your computers find each other through it
  setu recovery-key                  Unlock with your recovery key after a password reset
  setu sign-out                      Sign out here (syncing stops being coordinated)
  setu join [--backup]               Add this computer to the rules (--backup: it only
                                     receives, never sends changes back)
  setu status                        This computer, the others, and the rules
  setu devices                       Your computers: rules in use, waiting, last seen
  setu check                         Compare file contents with the other computers now
  setu differences <computer> <top-folder>
                                     Files that differ from that computer, one by one
  setu tidy                          Remove sync leftovers identical to their file
  setu tidy-log                      What tidy removed
  setu undo <removed-path>           Put a removed copy back
  setu rule <path> <choice> [--file] Change a choice in the draft. <choice> is
                                     off, both, inherit, from-here, from-other
  setu rules                         Show the rules being worked on
  setu plan                          What applying would change: Syncthing here, and files
                                     on both computers (scans this folder)
  setu apply [--yes] [--accept-replacements]
                                     Show the plan, confirm, apply. Many replaced files
                                     need a second yes (or --accept-replacements)
  setu discard                       Drop the draft
  setu index                         Write this computer's file list for the other one
  setu allow metered|relay on|off    Let Setu's folders sync on a metered network or relay
  setu resume <folder-id>            Resume a folder the delete guard paused
  setu pause [<minutes>]             Pause syncing on this computer (for a while, or until
                                     `setu unpause`)
  setu unpause                       Resume syncing after `setu pause`
  setu pause-folder <folder-id>      Pause one synced folder here (see `setu progress`)
  setu resume-folder <folder-id>     Resume a folder paused with `setu pause-folder`
  setu progress [--watch]            What is copying: %, speed, time so far, time left,
                                     per folder (--watch: keep showing it)
  setu progress-notices on|off       Desktop notifications about copy progress
  setu leave-out [add|remove <name>] Show or change the names always left out, such as
                                     *.iso or videos (goes into the draft)
  setu conflicts                     List conflict copies in synced folders
  setu keep <conflict-copy> copy|file
                                     Settle a conflict: keep the copy or the current file.
                                     The other one goes to .stversions, never deleted
  setu versions [<path>]             Old versions kept (30 days), newest first
  setu restore <folder-id> <path> <version-time>
                                     Put an old version back
  setu verify [--rescan]             Check every synced folder is complete and error-free
  setu quit                          Stop the running Setu (the sync engine stops too)
  setu version                       Show Setu's version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("setu: {e}");
            ExitCode::FAILURE
        }
    }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn run(args: &[String]) -> Result<(), Error> {
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    match cmd {
        "init" => {
            let root = flag(args, "--root").ok_or_else(|| Error::Setup("--root <folder> is required".into()))?;
            let root = std::fs::canonicalize(&root).map_err(|e| Error::Setup(format!("{root}: {e}")))?;
            let cfg = LocalConfig { root: strip_verbatim(root), syncthing_home: flag(args, "--syncthing-home").map(PathBuf::from), ..Default::default() };
            cfg.save(&config_dir())?;
            println!("This computer's folder: {}", cfg.root.display());
            Ok(())
        }
        "sign-in" => {
            let (Some(server), Some(email)) = (args.get(1), args.get(2)) else {
                return Err(Error::Setup("usage: setu sign-in <server> <email>".into()));
            };
            let s = Session::open()?;
            let pw = rpassword::prompt_password("Brahma password: ")?;
            match s.hub.sign_in(server, email, &pw) {
                Err(Error::RecoveryKeyNeeded) => {
                    println!("Your password was reset. Run `setu recovery-key` to unlock your data.");
                    return Ok(());
                }
                r => r?,
            }
            println!("Signed in as {email}.");
            after_sign_in(&s)
        }
        "recovery-key" => {
            let s = Session::open()?;
            let key = rpassword::prompt_password("Recovery key: ")?;
            s.hub.unlock_with_recovery_key(key.trim())?;
            println!("Unlocked.");
            after_sign_in(&s)
        }
        "sign-out" => {
            Session::open()?.hub.sign_out()?;
            println!("Signed out. Folders already set up keep syncing; new choices no longer arrive.");
            Ok(())
        }
        "join" => {
            let s = Session::open()?;
            if !s.hub.signed_in() {
                return Err(Error::Setup("sign in first: setu sign-in <server> <email>".into()));
            }
            let rs = s.join(args.iter().any(|a| a == "--backup"))?;
            println!("Rules revision {} for {}. Choose folders with `setu rule`, then `setu apply`.", rs.revision, names(&rs));
            Ok(())
        }
        "devices" => devices(&Session::open()?),
        "check" => {
            let s = Session::open()?;
            let digests = s.check_accuracy()?;
            let mut differ = 0;
            for d in s.hub.devices()?.iter().filter(|d| d.id != s.ctx.me) {
                for (top, mine) in &digests {
                    match d.digests.get(top) {
                        Some(theirs) if theirs.hash == mine.hash => {}
                        Some(_) if !mine.settled || !theirs_settled(d, top) => println!("  {top}: still syncing with {}", d.name),
                        Some(_) => {
                            differ += 1;
                            println!("  {top}: differs from {} (`setu differences {} {top}`)", d.name, short_id(&d.id));
                        }
                        None => println!("  {top}: {} has not checked it yet", d.name),
                    }
                }
            }
            println!("Checked {} top folders here.", digests.len());
            if differ > 0 {
                return Err(Error::Setup(format!("{differ} folder(s) differ")));
            }
            Ok(())
        }
        "differences" => {
            let (Some(who), Some(top)) = (args.get(1), args.get(2)) else {
                return Err(Error::Setup("usage: setu differences <computer> <top-folder>".into()));
            };
            let s = Session::open()?;
            let id = s.hub.devices()?.into_iter().find(|d| d.id.starts_with(who.as_str()) || d.name == *who).map(|d| d.id).ok_or_else(|| Error::Setup(format!("no computer {who:?} in your account")))?;
            let Some(m) = s.mismatches(&id, top) else {
                println!("The file lists for {top} have not arrived yet. Run `setu check` on both computers.");
                return Ok(());
            };
            for x in &m {
                println!("  {}: {}", x.path, x.what);
            }
            println!("{} difference(s).", m.len());
            Ok(())
        }
        "tidy" => {
            let removed = Session::open()?.tidy()?;
            for r in &removed {
                println!("  removed {} ({}, same as {})", r.removed.display(), r.kind, r.kept.display());
            }
            println!("{} copies removed. `setu undo <path>` puts one back.", removed.len());
            Ok(())
        }
        "tidy-log" => {
            for r in setu_core::tidy::read_log(&config_dir()) {
                let undone = if r.undone { " (put back)" } else { "" };
                println!("  {} {} {} ({}){undone}", r.at, r.kind, r.removed.display(), size(r.size));
            }
            Ok(())
        }
        "undo" => {
            let path = args.get(1).ok_or_else(|| Error::Setup("usage: setu undo <removed-path>".into()))?;
            setu_core::tidy::undo(&config_dir(), std::path::Path::new(path))?;
            println!("Put back: {path}");
            Ok(())
        }
        "status" => status(&Session::open()?),
        "rules" => {
            let s = Session::open()?;
            print_rules(&s.working()?, &s);
            Ok(())
        }
        "rule" => {
            let s = Session::open()?;
            let (Some(path), Some(choice)) = (args.get(1), args.get(2)) else {
                return Err(Error::Setup("usage: setu rule <path> <choice> [--file]".into()));
            };
            let mut rs = s.working()?;
            let other = s.other_device(&rs).ok_or_else(|| Error::Rules("no other computer in the rules".into()))?;
            let mode = match choice.as_str() {
                "inherit" => None,
                "from-here" => Some(Mode::From(s.ctx.me.clone())),
                "from-other" => Some(Mode::From(other)),
                c => Some(Mode::parse(c)?),
            };
            let kind = if args.iter().any(|a| a == "--file") { Kind::File } else { Kind::Dir };
            rs.set(path, kind, mode)?;
            s.save_draft(rs)?;
            println!("Draft updated. `setu plan` shows what applying would change.");
            Ok(())
        }
        "plan" => {
            let s = Session::open()?;
            let (rules, plan) = s.plan()?;
            print_plan(&plan);
            print_preview(&preview_for(&s, &rules, &plan)?, &rules);
            Ok(())
        }
        "apply" => {
            let s = Session::open()?;
            let (rules, plan) = s.plan()?;
            print_plan(&plan);
            let pv = preview_for(&s, &rules, &plan)?;
            print_preview(&pv, &rules);
            if !plan.problems.is_empty() {
                return Err(Error::Rules("fix the problems above first".into()));
            }
            if plan.is_empty() && s.shared()?.is_some_and(|r| s.hub.device(&s.ctx.me).ok().flatten().is_some_and(|d| d.applied_revision == r.revision)) {
                println!("Nothing to do.");
                return Ok(());
            }
            if !args.iter().any(|a| a == "--yes") {
                print!("Apply this? Type yes: ");
                std::io::stdout().flush()?;
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                if line.trim() != "yes" {
                    println!("Nothing changed.");
                    return Ok(());
                }
            }
            let yes = args.iter().any(|a| a == "--yes");
            let mut guard_ok = args.iter().any(|a| a == "--accept-replacements");
            if !pv.guard.is_empty() && !guard_ok && !yes {
                print!("This needs a second yes. Type replace to go ahead: ");
                std::io::stdout().flush()?;
                let mut line = String::new();
                std::io::stdin().lock().read_line(&mut line)?;
                guard_ok = line.trim() == "replace";
            }
            let r = s.apply(&plan, &pv, guard_ok)?;
            println!("Done: {} added, {} changed, {} removed. Revision {} applied here.", r.added, r.updated, r.removed, plan.revision);
            Ok(())
        }
        "discard" => {
            Session::open()?.discard_draft()?;
            println!("Draft dropped.");
            Ok(())
        }
        "quit" => {
            if setu_core::pipe::quit_running()? {
                println!("Setu stopped.");
            } else {
                println!("Setu was not running.");
            }
            Ok(())
        }
        "version" | "--version" => {
            println!("setu {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "index" => {
            let n = Session::open()?.scan_here()?.len();
            println!("File list written: {n} entries.");
            Ok(())
        }
        "allow" => {
            let dir = config_dir();
            let mut cfg = LocalConfig::load(&dir)?;
            let on = match args.get(2).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => return Err(Error::Setup("usage: setu allow metered|relay on|off".into())),
            };
            match args.get(1).map(String::as_str) {
                Some("metered") => cfg.allow_metered = on,
                Some("relay") => cfg.allow_relay = on,
                _ => return Err(Error::Setup("usage: setu allow metered|relay on|off".into())),
            }
            cfg.save(&dir)?;
            println!("Saved. Setu uses it within a minute.");
            Ok(())
        }
        "pause" | "unpause" => {
            let dir = config_dir();
            let mut cfg = LocalConfig::load(&dir)?;
            cfg.paused_until = match (args[0].as_str(), args.get(1)) {
                ("unpause", _) => None,
                (_, None) => Some(0),
                (_, Some(m)) => {
                    let m: u64 = m.parse().ok().filter(|&m| m > 0).ok_or_else(|| Error::Setup("usage: setu pause [<minutes>]".into()))?;
                    Some(setu_core::control::now_secs() + m * 60)
                }
            };
            cfg.save(&dir)?;
            let s = Session::open()?;
            if watch::read_state(&s.dir).alive_at + 120 < setu_core::control::now_secs() {
                watch::gate_now(&s.dir, &cfg, &s.client)?;
            }
            println!("{}", match cfg.paused_until {
                None => "Syncing resumes (unless the network is on hold).".to_string(),
                Some(0) => "Syncing paused here until `setu unpause`.".to_string(),
                Some(_) => format!("Syncing paused here for {} minutes.", args[1]),
            });
            Ok(())
        }
        "leave-out" => {
            let s = Session::open()?;
            let mut rules = s.working()?;
            match (args.get(1).map(String::as_str), args.get(2)) {
                (None, _) => {
                    for l in &rules.always_ignore {
                        println!("{l}");
                    }
                    return Ok(());
                }
                (Some("add"), Some(n)) => {
                    let mut list = rules.always_ignore.clone();
                    list.push(n.clone());
                    rules.set_always_ignore(&list)?;
                }
                (Some("remove"), Some(n)) => {
                    let list: Vec<String> = rules.always_ignore.iter().filter(|l| *l != n).cloned().collect();
                    if list.len() == rules.always_ignore.len() {
                        return Err(Error::Setup(format!("{n:?} is not in the list")));
                    }
                    rules.set_always_ignore(&list)?;
                }
                _ => return Err(Error::Setup("usage: setu leave-out [add|remove <name>]".into())),
            }
            s.save_draft(rules)?;
            println!("Saved in the draft. `setu apply` makes it count, on both computers.");
            Ok(())
        }
        "resume" => {
            let s = Session::open()?;
            let id = args.get(1).ok_or_else(|| Error::Setup("usage: setu resume <folder-id>".into()))?;
            watch::release_hold(&s.dir, &s.client, id)?;
            println!("Resumed {id}.");
            Ok(())
        }
        "pause-folder" | "resume-folder" => {
            let pause = args[0] == "pause-folder";
            let id = args.get(1).ok_or_else(|| Error::Setup(format!("usage: setu {} <folder-id>", args[0])))?;
            let s = Session::open()?;
            watch::set_folder_paused(&s.dir, &s.client, id, pause)?;
            println!("{} {id}.", if pause { "Paused" } else { "Resumed" });
            Ok(())
        }
        "progress-notices" => {
            let dir = config_dir();
            let mut cfg = LocalConfig::load(&dir)?;
            cfg.quiet_progress = match args.get(1).map(String::as_str) {
                Some("on") => false,
                Some("off") => true,
                _ => return Err(Error::Setup("usage: setu progress-notices on|off".into())),
            };
            cfg.save(&dir)?;
            println!("Saved. Setu uses it at its next reading.");
            Ok(())
        }
        "progress" => progress(&Session::open()?, args.iter().any(|a| a == "--watch")),
        "conflicts" => {
            let s = Session::open()?;
            let here = Tree::new(setu_core::index::scan(&s.local.root, &s.working()?.always_ignore)?);
            let cs = s.conflicts(&here)?;
            if cs.is_empty() {
                println!("No conflict copies.");
            }
            for c in &cs {
                let file = match c.original_size {
                    Some(n) => format!("file {}", size(n)),
                    None => "file is gone".into(),
                };
                println!("{}\n    copy {}, {file}", c.copy, size(c.copy_size));
            }
            Ok(())
        }
        "keep" => {
            let s = Session::open()?;
            let usage = || Error::Setup("usage: setu keep <conflict-copy> copy|file".into());
            let copy = args.get(1).ok_or_else(usage)?;
            let keep_copy = match args.get(2).map(String::as_str) {
                Some("copy") => true,
                Some("file") => false,
                _ => return Err(usage()),
            };
            let moved = s.resolve(copy, keep_copy)?;
            println!("Done. Kept the {}; the other one is in {}.", if keep_copy { "copy" } else { "file" }, moved.display());
            Ok(())
        }
        "versions" => {
            let s = Session::open()?;
            let only = args.get(1);
            let list = s.versions()?;
            let list: Vec<_> = list.iter().filter(|k| only.is_none_or(|p| k.rel == *p || k.rel.starts_with(&format!("{p}/")))).collect();
            if list.is_empty() {
                println!("No old versions kept.");
            }
            for k in list {
                println!("{}  {}  {}  {} {}", k.version.version_time, size(k.version.size), k.rel, k.folder, k.version.path);
            }
            Ok(())
        }
        "restore" => {
            let s = Session::open()?;
            let [_, folder, path, time] = args else {
                return Err(Error::Setup("usage: setu restore <folder-id> <path> <version-time> (copy them from `setu versions`)".into()));
            };
            match s.restore(folder, path, time)? {
                setu_core::trust::Restored::InPlace => println!("Restored {path}. The file it replaced is kept in .stversions."),
                setu_core::trust::Restored::Beside(rel) => println!("This computer only receives this folder, so the old version is saved beside it: {rel}"),
            }
            Ok(())
        }
        "verify" => {
            let checks = Session::open()?.verify(args.iter().any(|a| a == "--rescan"))?;
            if checks.is_empty() {
                println!("No folders sync yet.");
                return Ok(());
            }
            let mut bad = 0;
            for c in &checks {
                let other = c.other_completion.map(|p| format!(", other computer {p:.0} %")).unwrap_or_default();
                let mark = if c.ok { "ok  " } else { bad += 1; "NOT " };
                let name = if c.rel.is_empty() { "(whole folder)" } else { &c.rel };
                println!("{mark} {name}  [{} {}]  needs {} files ({}){other}", c.folder_type, c.state, c.need_files, size(c.need_bytes));
                if c.local_changes > 0 {
                    println!("       {} files changed here are not sent back (this side only receives)", c.local_changes);
                }
                for e in c.errors.iter().take(10) {
                    println!("       error: {e}");
                }
                for p in c.samples.iter().take(10) {
                    println!("       waiting: {p}");
                }
            }
            if bad > 0 {
                return Err(Error::Syncthing(format!("{bad} folder(s) not complete")));
            }
            println!("All {} folder(s) complete here.", checks.len());
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
}

/// After signing in: keep Syncthing on the tailnet, and say what changed.
fn after_sign_in(s: &Session) -> Result<(), Error> {
    match setu_core::mesh::harden(&s.client, &setu_core::mesh::tailnet_ips()) {
        Ok(changed) if !changed.is_empty() => println!("Syncthing now stays on Tailscale (changed: {}).", changed.join(", ")),
        Ok(_) => {}
        Err(e) => println!("Note: {e}"),
    }
    s.sync_hub()?;
    if s.shared()?.is_some_and(|r| r.devices.contains_key(&s.ctx.me)) {
        println!("This computer is in your rules already; the helper (`setu watch`) keeps it up to date.");
    } else {
        println!("Next: `setu join` (or `setu join --backup` for a copy that only receives).");
    }
    Ok(())
}

fn theirs_settled(d: &setu_core::hub::DeviceState, top: &str) -> bool {
    d.digests.get(top).is_some_and(|g| g.settled)
}

fn devices(s: &Session) -> Result<(), Error> {
    let rules = s.shared()?;
    let conns = s.client.connections()?;
    let now = setu_core::control::now_secs();
    for d in s.hub.devices()? {
        let me = d.id == s.ctx.me;
        let link = if me {
            "this computer".to_string()
        } else if conns["connections"][&d.id]["connected"].as_bool() == Some(true) {
            "connected".to_string()
        } else {
            "not connected".to_string()
        };
        let state = match (&d.held, &rules) {
            (Some(h), _) => format!("needs you: revision {} waits ({})", h.revision, h.reasons.first().cloned().unwrap_or_default()),
            (None, Some(r)) if d.applied_at > 0 && d.applied_revision >= r.revision => "up to date".to_string(),
            (None, Some(r)) if r.devices.contains_key(&d.id) => format!("applying revision {}", r.revision),
            _ => "not in the rules".to_string(),
        };
        let seen = if me { String::new() } else { format!("; last seen {} min ago", now.saturating_sub(d.updated_at) / 60) };
        println!("  {} [{}] {}: {link}; {state}{seen}", d.name, short_id(&d.id), d.os.map(|o| format!("{o:?}")).unwrap_or_default());
        for p in &d.problems {
            println!("      problem: {p}");
        }
    }
    Ok(())
}

/// Windows `canonicalize` returns `\\?\C:\...`; Syncthing and people expect `C:\...`.
fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

fn names(rs: &RuleSet) -> String {
    rs.devices.values().map(|d| d.name.as_str()).collect::<Vec<_>>().join(" and ")
}

fn describe(rs: &RuleSet, m: &Mode) -> String {
    match m {
        Mode::Off => "off".into(),
        Mode::Both => "both ways".into(),
        Mode::From(id) => {
            let to = rs.devices.iter().find(|(k, _)| *k != id).map(|(_, d)| d.name.as_str()).unwrap_or("?");
            format!("{} → {}", rs.device_name(id), to)
        }
    }
}

fn print_rules(rs: &RuleSet, s: &Session) {
    let draft = s.draft().ok().flatten().is_some();
    println!("Rules revision {}{}", rs.revision, if draft { " (with unapplied draft changes)" } else { "" });
    println!("  (root)  {}", describe(rs, &rs.root));
    for (p, r) in &rs.rules {
        println!("  {p}{}  {}", if r.kind == Kind::Dir { "/" } else { "" }, describe(rs, &r.mode));
    }
}

fn preview_for(s: &Session, rules: &RuleSet, plan: &Plan) -> Result<Preview, Error> {
    let here = Tree::new(s.scan_here()?);
    let other = s.other_list();
    let there = other.as_ref().map(|(l, _)| Tree::new(l.entries.clone()));
    let root = other.as_ref().and_then(|(l, _)| l.root.clone());
    Ok(s.preview(rules, plan, &here, there.as_ref().map(|t| (t, root.as_deref()))))
}

fn size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (b as f64, 0);
    while v >= 1024.0 && i < 4 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

fn print_preview(p: &Preview, rules: &RuleSet) {
    println!("Files:");
    if p.other_unknown {
        println!("  (the other computer's file list has not arrived; only this side is counted)");
    }
    if p.groups.is_empty() {
        println!("  nothing syncs under these rules");
    }
    for g in &p.groups {
        let name = if g.rel.is_empty() { "(whole folder)" } else { &g.rel };
        let mode = Mode::parse(&g.mode).map(|m| describe(rules, &m)).unwrap_or_else(|_| g.mode.clone());
        println!("  {name}  [{mode}]");
        for (label, c) in [
            ("send", g.send),
            ("receive", g.receive),
            ("replaced here (old copy kept as conflict copy)", g.replaced_here),
            ("replaced there (old copy kept as conflict copy)", g.replaced_there),
            ("only here, stays here", g.stays_here),
            ("only there, stays there", g.stays_there),
            ("already the same", g.same),
        ] {
            if c.files > 0 {
                println!("    {label}: {} file(s), {}", c.files, size(c.bytes));
            }
        }
    }
    for n in &p.windows_names {
        println!("  WINDOWS NAME {}: {}", n.path, n.message);
    }
    for g in &p.guard {
        println!("  CAREFUL {g}");
    }
}

fn print_plan(plan: &Plan) {
    println!("Plan for revision {} ({} Setu folder(s); {} already right):", plan.revision, plan.folders, plan.unchanged);
    for p in &plan.problems {
        println!("  PROBLEM {}: {}", if p.path.is_empty() { "(rules)" } else { &p.path }, p.message);
    }
    for a in &plan.actions {
        match a {
            Action::Remove { id, path, .. } => println!("  remove  {id}  {path}  (files stay)"),
            Action::Update { id, path, changes, .. } => println!("  change  {id}  {path}: {}", changes.join(", ")),
            Action::Add { id, path, folder_type, files_only, .. } => {
                println!("  add     {id}  {path}  [{folder_type}{}]", if *files_only { ", chosen files only" } else { "" })
            }
        }
    }
    for w in &plan.warnings {
        println!("  NOTE {w}");
    }
    if plan.actions.is_empty() && plan.problems.is_empty() {
        println!("  no changes to Syncthing here");
    }
}

/// Copy progress. The speed needs two readings, so the first line comes after two seconds.
fn progress(s: &Session, keep: bool) -> Result<(), Error> {
    use setu_core::progress::{self, bytes, duration, Flow, Meter};
    let mut meter = Meter::default();
    let hold = || !watch::read_state(&s.dir).gate.open || s.local.user_paused(setu_core::control::now_secs());
    meter.observe(progress::read(&s.client, &s.ctx.me, watch::now_ms())?, hold());
    let line = |what: &str, f: &Flow| {
        if !f.active {
            return format!("{what}: nothing to copy");
        }
        let eta = f.eta_secs.map_or("working out".to_string(), duration);
        format!(
            "{what}: {:.1}% · {} of {} · {} files left · {}/s (average {}/s) · {} so far · {eta} left",
            f.percent,
            bytes(f.done_bytes),
            bytes(f.total_bytes),
            f.left_files,
            bytes(f.speed),
            bytes(f.average),
            duration(f.elapsed_secs)
        )
    };
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let (mut snap, _) = meter.observe(progress::read(&s.client, &s.ctx.me, watch::now_ms())?, hold());
        progress::mark_pauses(&mut snap, &watch::read_state(&s.dir), &s.local);
        if keep {
            print!("\x1b[2J\x1b[H");
        }
        if snap.paused {
            println!("On hold on this computer: copying waits.");
        }
        println!("{}", line("To this computer", &snap.down));
        println!("{}", line("To the others", &snap.up));
        for p in &snap.peers {
            println!("  {} {}", p.name, if p.connected { "connected" } else { "offline" });
        }
        for f in &snap.folders {
            let here = if f.global_bytes == 0 { 100.0 } else { (f.global_bytes - f.need_bytes.min(f.global_bytes)) as f64 * 100.0 / f.global_bytes as f64 };
            let paused = f.paused_by.as_deref().map(|b| format!(" (paused: {b})")).unwrap_or_default();
            println!("  {}  {}  {}  here {here:.1}%{}{paused}", f.id, f.name, f.state, f.other_completion.map(|c| format!(", others {c:.1}%")).unwrap_or_default());
            for c in &f.copying {
                println!("      copying {c}");
            }
        }
        if !keep {
            return Ok(());
        }
    }
}

fn status(s: &Session) -> Result<(), Error> {
    println!("This computer: {} ({})", short_id(&s.ctx.me), s.local.root.display());
    println!("Control folder: {}", s.control.dir.display());
    match s.hub.account() {
        Some(a) => println!("Brahma account: {} on {}", a.email, a.server),
        None => println!("Brahma account: not signed in"),
    }
    let Some(rs) = s.shared()? else {
        let how = if s.hub.signed_in() { "`setu join`" } else { "`setu sign-in <server> <email>`, then `setu join`" };
        println!("No rules yet: run {how}.");
        return Ok(());
    };
    let conns = s.client.connections()?;
    for (id, d) in &rs.devices {
        let me = *id == s.ctx.me;
        let link = if me {
            "this computer".to_string()
        } else if conns["connections"][id]["connected"].as_bool() == Some(true) {
            format!("connected via {}", conns["connections"][id]["address"].as_str().unwrap_or("?"))
        } else {
            "not connected".to_string()
        };
        let applied = match s.hub.device(id)? {
            Some(st) if st.held.is_some() => format!("waiting for the person to check revision {}", st.held.map(|h| h.revision).unwrap_or(0)),
            Some(st) if st.applied_at > 0 => format!("applied revision {}", st.applied_revision),
            _ => "has not applied any rules".into(),
        };
        println!("  {} [{}] {:?}: {link}; {applied}", d.name, short_id(id), d.os);
    }
    println!("Shared rules: revision {}", rs.revision);
    for c in s.control.conflicts() {
        println!("  WARNING conflicting copy in control folder: {c}");
    }
    if s.draft()?.is_some() {
        println!("A draft with unapplied changes exists (`setu plan`).");
    }
    let w = watch::read_state(&s.dir);
    if w.alive_at + 120 < setu_core::control::now_secs() {
        println!("Background helper: not running (it runs inside Setu; is Setu still starting?).");
    } else if w.gate.open {
        println!("Background helper: running; network fine.");
    } else {
        println!("Background helper: running; Setu folders held here: {}.", w.gate.reason.as_deref().unwrap_or("network"));
    }
    for h in &w.holds {
        println!("  HELD {}: {} deletions arrived at once; paused. `setu resume {}` after checking.", h.folder, h.deletes, h.folder);
    }
    Ok(())
}
