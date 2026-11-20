//! `setu`: Setu on the command line. The desktop app does the same through the same core.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use setu_core::apply::{Action, Plan};
use setu_core::local::{config_dir, LocalConfig};
use setu_core::rules::{short_id, Kind, Mode, Os, RuleSet};
use setu_core::session::Session;
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
  setu plan                          What applying would change here
  setu apply [--yes]                 Show the plan, confirm, apply
  setu discard                       Drop the draft
  setu index                         Write this computer's file list for the other one
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
            let cfg = LocalConfig { root: strip_verbatim(root), syncthing_home: flag(args, "--syncthing-home").map(PathBuf::from) };
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
            let (_, plan) = s.plan()?;
            print_plan(&plan);
            Ok(())
        }
        "apply" => {
            let s = Session::open()?;
            let (_, plan) = s.plan()?;
            print_plan(&plan);
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
            let r = s.apply(&plan)?;
            println!("Done: {} added, {} changed, {} removed. Revision {} applied here.", r.added, r.updated, r.removed, plan.revision);
            Ok(())
        }
        "discard" => {
            Session::open()?.discard_draft()?;
            println!("Draft dropped.");
            Ok(())
        }
        "index" => {
            let n = Session::open()?.write_index()?;
            println!("File list written: {n} entries.");
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
    Ok(())
}
