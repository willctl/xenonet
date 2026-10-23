//! `xenolith migrate agent-wiki`: moves an agent-wiki install (any version up to 1.8.0, Node or Rust
//! era) to Xenolith. The wiki folder is adopted in place, never copied: the formats are the same.
//!
//!   xenolith migrate agent-wiki [--dry-run]    what it found and what it would do (the default; changes nothing)
//!   xenolith migrate agent-wiki --run          do it: journal and backups, stop agent-wiki, Xenolith's
//!                                              config, the curator's sign-in, xenolith install, agent-wiki's
//!                                              registrations out, check
//!   xenolith migrate agent-wiki --resume       finish a run that stopped (or what waited, such as Claude desktop)
//!   xenolith migrate agent-wiki --rollback     undo it, in reverse; agent-wiki runs again
//!   xenolith migrate agent-wiki --finish [--move-wiki]   delete agent-wiki's folders once nothing of it runs;
//!                                              --move-wiki also renames the wiki to ~/Xenolith
//!   xenolith migrate agent-wiki --update-protocol        rename wiki_* to context_* in an edited PROTOCOL.md
//!   options: --json, --wiki-dir <dir>, --port N, --from <dir> (passed to install), --sandbox <home>

mod detect;
mod edits;
mod finish;
mod journal;
mod locate;
pub mod retire;
mod rollback;
mod run;
mod scan;
mod sys;
#[cfg(test)]
mod tests;

use detect::{Ctx, Found, Opts};
use journal::Journal;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::PathBuf;

/// What the person reads: printed as it happens, or (--json) collected into the result.
pub struct Out {
    json: bool,
    lines: RefCell<Vec<String>>,
    /// Paths under it show as ~ (as everywhere in XenoNet).
    home: RefCell<Option<String>>,
}

impl Out {
    pub fn new(json: bool) -> Out {
        Out { json, lines: RefCell::new(vec![]), home: RefCell::new(None) }
    }
    pub fn set_home(&self, home: &std::path::Path) {
        *self.home.borrow_mut() = Some(edits::fwd(home).trim_end_matches('/').to_string());
    }
    pub fn say(&self, s: impl Into<String>) {
        let mut s = s.into();
        if let Some(h) = self.home.borrow().as_ref().filter(|h| h.len() > 1) {
            s = s.replace(&format!("{h}/"), "~/");
        }
        if !self.json {
            println!("{s}");
        }
        self.lines.borrow_mut().push(s);
    }
    pub fn step(&self, n: &str, title: &str) {
        self.say(format!("\n[{n}] {title}"));
    }
    pub fn info(&self, s: &str) {
        self.say(format!("    {s}"));
    }
    pub fn warn(&self, s: &str) {
        self.say(format!("    ! {s}"));
    }
    pub fn lines(&self) -> Vec<String> {
        self.lines.borrow().clone()
    }
}

const USAGE: &str = "usage: xenolith migrate agent-wiki [--dry-run | --run | --resume | --rollback | --finish [--move-wiki] | --update-protocol] [--json] [--wiki-dir <dir>] [--port N] [--from <dir>] [--sandbox <home>]";

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    DryRun,
    Run,
    Resume,
    Rollback,
    Finish,
    UpdateProtocol,
}

pub struct Args {
    pub mode: Mode,
    pub json: bool,
    pub move_wiki: bool,
    pub opts: Opts,
}

pub fn parse(args: &[String]) -> Result<Args, String> {
    if args.first().map(String::as_str) != Some("agent-wiki") {
        return Err("migrate: say what to move from: xenolith migrate agent-wiki".into());
    }
    let mut a = Args { mode: Mode::DryRun, json: false, move_wiki: false, opts: Opts { wiki_dir: None, port: None, sandbox: None, from: None } };
    let mut modes = vec![];
    let mut it = args[1..].iter();
    while let Some(x) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} takes a value"));
        match x.as_str() {
            "--dry-run" => modes.push(Mode::DryRun),
            "--run" => modes.push(Mode::Run),
            "--resume" => modes.push(Mode::Resume),
            "--rollback" => modes.push(Mode::Rollback),
            "--finish" => modes.push(Mode::Finish),
            "--update-protocol" => modes.push(Mode::UpdateProtocol),
            "--json" => a.json = true,
            "--move-wiki" => a.move_wiki = true,
            "--wiki-dir" => a.opts.wiki_dir = Some(xenolith_core::wiki::absolute(&PathBuf::from(value("--wiki-dir")?))),
            "--port" => a.opts.port = Some(value("--port")?.parse().map_err(|_| "--port takes a port number".to_string())?),
            "--from" => a.opts.from = Some(xenolith_core::wiki::absolute(&PathBuf::from(value("--from")?))),
            "--sandbox" => a.opts.sandbox = Some(xenolith_core::wiki::absolute(&PathBuf::from(value("--sandbox")?))),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    modes.dedup();
    if modes.len() > 1 {
        return Err("choose one of --dry-run, --run, --resume, --rollback, --finish, --update-protocol".into());
    }
    a.mode = modes.first().copied().unwrap_or(Mode::DryRun);
    if a.move_wiki && a.mode != Mode::Finish {
        return Err("--move-wiki goes with --finish".into());
    }
    Ok(a)
}

pub fn main(args: &[String]) -> ! {
    let a = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("xenolith migrate: {e}\n{USAGE}");
            std::process::exit(2)
        }
    };
    let (env, child_env) = crate::install::environment(a.opts.sandbox.as_deref());
    let real = sys::RealSys;
    let sandbox;
    let system: &dyn sys::Sys = match &a.opts.sandbox {
        Some(home) => {
            sandbox = sys::SandboxSys { home: home.clone(), child_env };
            &sandbox
        }
        None => &real,
    };
    let lay = locate::Layout::new(&env, system.os());
    let mut ctx = Ctx { sys: system, env, lay, opts: a.opts, wait_ms: 60_000 };
    let out = Out::new(a.json);
    out.set_home(&ctx.lay.home);
    let (code, result) = execute(&mut ctx, &out, a.mode, a.move_wiki);
    if a.json {
        let mut r = result;
        r["output"] = json!(out.lines());
        println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
    }
    std::process::exit(code)
}

/// Runs one mode: (exit code, the result as data).
pub fn execute(ctx: &mut Ctx, out: &Out, mode: Mode, move_wiki: bool) -> (i32, Value) {
    match mode {
        Mode::DryRun => dry_run(ctx, out),
        Mode::Run | Mode::Resume => match run::run(ctx, out, mode == Mode::Resume) {
            Ok(j) => {
                let ok = j.verify.as_ref().is_none_or(|v| v["ok"] != false);
                report(ctx, out, &j);
                (if ok { 0 } else { 1 }, json!({ "mode": if mode == Mode::Run { "run" } else { "resume" }, "ok": ok, "journal": j }))
            }
            Err(e) => fail(out, mode, e),
        },
        Mode::Rollback => match rollback::rollback(ctx, out) {
            Ok(j) => {
                let left: Vec<&String> = j.warnings.iter().filter(|w| w.starts_with("rollback: ")).collect();
                out.say(if left.is_empty() {
                    "\nRolled back: agent-wiki is as it was before the migration.".to_string()
                } else {
                    format!("\nRolled back, except {} file(s) changed since, left as they are (see above).", left.len())
                });
                for p in &j.pending {
                    out.say(format!("  - {p}"));
                }
                (if left.is_empty() { 0 } else { 1 }, json!({ "mode": "rollback", "ok": left.is_empty(), "journal": j }))
            }
            Err(e) => fail(out, mode, e),
        },
        Mode::Finish => match finish::finish(ctx, out, move_wiki) {
            Ok(j) => {
                out.say(format!("\nFinished: agent-wiki's folders are gone. Your wiki is at {}.", j.wiki_move.as_ref().map(|m| m.to.clone()).unwrap_or_else(|| j.wiki_dir.clone())));
                (0, json!({ "mode": "finish", "ok": true, "journal": j }))
            }
            Err(e) => fail(out, mode, e),
        },
        Mode::UpdateProtocol => {
            let wiki = Journal::load(&ctx.lay.work)
                .ok()
                .flatten()
                .map(|j| j.wiki_move.map(|m| m.to).unwrap_or(j.wiki_dir))
                .map(PathBuf::from)
                .or_else(|| ctx.opts.wiki_dir.clone())
                .or_else(|| detect::detect(ctx).wiki.path.map(PathBuf::from));
            match wiki.ok_or_else(|| "no wiki found: pass --wiki-dir".to_string()).and_then(|w| finish::update_protocol(&w, out)) {
                Ok(changed) => (0, json!({ "mode": "update-protocol", "ok": true, "changed": changed })),
                Err(e) => fail(out, mode, e),
            }
        }
    }
}

fn fail(out: &Out, mode: Mode, e: String) -> (i32, Value) {
    if out.json {
        out.lines.borrow_mut().push(e.clone());
    } else {
        eprintln!("\n{e}");
    }
    (1, json!({ "mode": format!("{mode:?}").to_lowercase(), "ok": false, "error": e }))
}

fn dry_run(ctx: &mut Ctx, out: &Out) -> (i32, Value) {
    let f = detect::detect(ctx);
    let existing = Journal::load(&ctx.lay.work).ok().flatten();
    let plan = detect::plan(ctx, &f);
    if !out.json {
        print_found(ctx, out, &f, existing.as_ref(), &plan);
    }
    let ready = f.found && f.blockers.is_empty();
    (
        if f.found && !f.blockers.is_empty() { 1 } else { 0 },
        json!({ "mode": "dry-run", "ready": ready, "found": f, "plan": plan, "migration": existing.map(|j| json!({ "state": j.state, "startedAt": j.started_at, "pending": j.pending })) }),
    )
}

fn print_found(ctx: &Ctx, out: &Out, f: &Found, existing: Option<&Journal>, plan: &[String]) {
    out.say("Moving from agent-wiki to Xenolith: dry run (nothing is changed)\n");
    if let Some(j) = existing {
        out.say(format!(
            "A migration started on {} is {}. {}\n",
            j.started_at,
            j.state,
            match j.state.as_str() {
                "done" if !j.pending.is_empty() => "Some things wait: run --resume once they can be done.",
                "done" => "Run --finish once no agent-wiki program runs, or --rollback to undo it.",
                "rolled-back" => "It was undone.",
                "finished" => "Nothing is left to do.",
                _ => "Run --resume to finish it, or --rollback to undo it.",
            }
        ));
    }
    if !f.found {
        out.say("No agent-wiki install was found here. Nothing to migrate.");
        return;
    }
    out.say(format!(
        "Found agent-wiki {} ({}){}.",
        f.version.as_deref().unwrap_or("(version unknown)"),
        match f.era.as_str() {
            "rust" => "the Rust programs",
            "node" => "the Node programs",
            _ => "its programs are gone",
        },
        if f.pre13 { ", in the layout from before 1.3" } else { "" }
    ));
    if let Some(c) = &f.config_file {
        out.info(&format!("settings: {c}"));
    }
    for (k, p) in &f.folders {
        out.info(&format!("{k}: {p}"));
    }
    out.say(format!("\nWiki: {} (from {})", f.wiki.path.as_deref().unwrap_or("?"), f.wiki.source));
    if let Some(s) = &f.wiki.scan {
        out.info(&s.summary());
    }
    out.info(&format!(
        "PROTOCOL.md: {}",
        match f.wiki.protocol.as_str() {
            "earlier-default" => "an earlier default, which the install replaces (a copy goes to .history/)",
            "current" => "Xenolith's already",
            "edited" => "your own edit, kept",
            _ => "missing; the install writes Xenolith's",
        }
    ));
    out.say("\nRegistrations to replace:");
    if f.registrations.is_empty() {
        out.info("none found");
    }
    for r in &f.registrations {
        out.info(&format!("{}: {}{}", r.kind, r.key, r.file.as_ref().map(|x| format!(" ({x})")).unwrap_or_default()));
    }
    if !f.processes.is_empty() {
        out.say("\nRunning:");
        for p in &f.processes {
            out.info(&format!("pid {} ({})", p.pid, p.kind));
        }
    }
    let carried = detect::carried_keys(&f.config).into_iter().filter(|k| ["writeMode", "curator", "ask", "logs", "hooks", "search"].contains(&k.as_str())).collect::<Vec<_>>();
    out.say(format!("\nSettings carried over: {}", if carried.is_empty() { "none (agent-wiki used its defaults)".to_string() } else { carried.join(", ") }));
    for d in &f.dropped {
        out.info(&format!("not carried: {d}"));
    }
    out.say("\nThe plan (xenolith migrate agent-wiki --run):");
    for p in plan {
        out.info(p);
    }
    for (title, list) in [("Notes", &f.notes), ("Warnings", &f.warnings)] {
        if !list.is_empty() {
            out.say(format!("\n{title}:"));
            for n in list {
                out.info(n);
            }
        }
    }
    if f.blockers.is_empty() {
        out.say(format!("\nReady. Run: xenolith migrate agent-wiki --run{}", if ctx.sandbox() { " --sandbox <home>" } else { "" }));
    } else {
        out.say("\nNot ready. Fix these first:");
        for b in &f.blockers {
            out.info(&format!("- {b}"));
        }
    }
}

/// The end of a run: what is now true, and exactly what is left for the person.
fn report(ctx: &Ctx, out: &Out, j: &Journal) {
    let ok = j.verify.as_ref().is_none_or(|v| v["ok"] != false);
    out.say(if ok {
        format!("\nDone. Your wiki stays at {}, and Xenolith now serves it as shared context.", j.wiki_dir)
    } else {
        "\nMoved, but the check found problems (above). Look into them, or undo everything with --rollback.".to_string()
    });
    let mut todo: Vec<String> = vec!["Restart Claude Code and Codex, and quit and reopen Claude desktop and the ChatGPT app, so they load Xenolith.".into()];
    let paste = ctx.lay.xen.data_dir.join("paste-into-app-settings.md");
    todo.push(format!(
        "In Claude's and ChatGPT's personal instructions, replace agent-wiki's text with this (also in {}):\n\n{}\n",
        edits::fwd(&paste),
        crate::install::files::POINTER.trim().lines().map(|l| format!("        {l}").trim_end().to_string()).collect::<Vec<_>>().join("\n")
    ));
    let wiki = PathBuf::from(&j.wiki_dir);
    let (protocol, _, old_names) = detect::protocol_state(&wiki);
    if protocol == "edited" && old_names {
        todo.push("Your PROTOCOL.md is your own edit, so it was kept. It still names agent-wiki's tools (wiki_start and the rest), which keep working for now. To rename them: xenolith migrate agent-wiki --update-protocol (a copy of the old text goes to .history/).".into());
    }
    todo.extend(j.pending.iter().cloned());
    out.say("\nWhat is left for you:");
    for (i, t) in todo.iter().enumerate() {
        out.say(format!("  {}. {t}", i + 1));
    }
    out.say("\nTo undo it: xenolith migrate agent-wiki --rollback");
    out.say("Once no agent-wiki program runs (after those restarts): xenolith migrate agent-wiki --finish (add --move-wiki to move the wiki to ~/Xenolith)");
}
