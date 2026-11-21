//! `setu`: Setu on the command line. The desktop app does the same through the same core.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use setu_core::apply::{Action, Plan};
use setu_core::index::Tree;
use setu_core::local::{config_dir, LocalConfig};
use setu_core::preview::Preview;
use setu_core::rules::{short_id, Kind, Mode, Os, RuleSet};
use setu_core::session::Session;
use setu_core::watch;
use setu_core::Error;

const USAGE: &str = "\
setu: choose what syncs between two computers, and which way.

  setu init --root <folder>          Set this computer's folder (once)
  setu create-rules [--other-os linux|windows|macos]
                                     First rules, everything off (once, on one computer)
  setu status                        This computer, the other one, and the rules
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
  setu watch                         Run the background helper (network gate, delete guard)
  setu allow metered|relay on|off    Let Setu's folders sync on a metered network or relay
  setu resume <folder-id>            Resume a folder the delete guard paused
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
        "create-rules" => {
            let s = Session::open()?;
            let os = match flag(args, "--other-os").as_deref() {
                Some("linux") => Os::Linux,
                Some("windows") => Os::Windows,
                Some("macos") => Os::Macos,
                Some(o) => return Err(Error::Setup(format!("unknown OS {o:?}"))),
                None if s.ctx.os == Os::Windows => Os::Linux,
                None => Os::Windows,
            };
            let rs = s.init_rules(os)?;
            println!("Rules revision {} for {}.", rs.revision, names(&rs));
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
            if plan.is_empty() && s.shared()?.is_some_and(|r| s.control.read_status(&s.ctx.me).map(|st| st.applied_revision) == Some(r.revision)) {
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
        "index" => {
            let n = Session::open()?.scan_here()?.len();
            println!("File list written: {n} entries.");
            Ok(())
        }
        "watch" => {
            let dir = config_dir();
            let cfg = LocalConfig::load(&dir)?;
            let client = setu_core::syncthing::Client::discover(cfg.syncthing_home.as_deref())?;
            println!("Setu is watching. Ctrl+C stops it.");
            watch::run(&dir, || LocalConfig::load(&config_dir()), &client)
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
            println!("Saved. `setu watch` uses it within a minute.");
            Ok(())
        }
        "resume" => {
            let s = Session::open()?;
            let id = args.get(1).ok_or_else(|| Error::Setup("usage: setu resume <folder-id>".into()))?;
            watch::release_hold(&s.dir, &s.client, id)?;
            println!("Resumed {id}.");
            Ok(())
        }
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    }
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

fn status(s: &Session) -> Result<(), Error> {
    println!("This computer: {} ({})", short_id(&s.ctx.me), s.local.root.display());
    println!("Control folder: {}", s.control.dir.display());
    let Some(rs) = s.shared()? else {
        println!("No rules yet: run `setu create-rules`.");
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
        let applied = s.control.read_status(id).map(|st| format!("applied revision {}", st.applied_revision)).unwrap_or_else(|| "has not applied any rules".into());
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
        println!("Background helper: not running (`setu watch`).");
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
