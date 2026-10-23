//! --run and --resume: the seven steps. Each step is journaled as doing, then done, and each is
//! written so that running it again does no harm: a step left "doing" by a run that stopped is
//! simply run again, and checks its own postcondition (a backup taken, a folder moved, an entry
//! gone) before acting.

use super::Out;
use super::detect::{self, AW_LABELS, AW_RUN_VALUE, AW_SERVICE, AW_TASK, AW_UNITS, Ctx, Found, RUN_KEY};
use super::edits::{self, fwd};
use super::journal::{self, Journal, Move, Tracked, crash};
use super::scan;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub const STEPS: [(&str, &str); 7] = [
    ("1", "Journal and backups"),
    ("2", "Stop agent-wiki"),
    ("3", "Xenolith's config"),
    ("4", "The curator's sign-in"),
    ("5", "xenolith install"),
    ("6", "agent-wiki's registrations"),
    ("7", "Check"),
];

/// An edit of a tracked file, recorded in the journal before it is written.
fn tracked_edit(ctx: &Ctx, j: &mut Journal, file: &Path, f: impl FnOnce(&str) -> Result<Option<String>, String>) -> Result<bool, String> {
    let Some(out) = edits::plan(file, f)? else { return Ok(false) };
    tracked_write(ctx, j, file, &out)?;
    Ok(true)
}

fn tracked_write(ctx: &Ctx, j: &mut Journal, file: &Path, out: &str) -> Result<(), String> {
    j.expect(file, Some(xenolith_core::text::sha256_hex(out.as_bytes())));
    j.save(&ctx.lay.work)?;
    edits::write_atomic(file, out.as_bytes())?;
    j.mark_post(file);
    Ok(())
}

/// A tracked file removed, recorded in the journal first.
fn tracked_remove(ctx: &Ctx, j: &mut Journal, file: &Path) -> Result<bool, String> {
    if !file.exists() {
        return Ok(false);
    }
    j.expect(file, None);
    j.save(&ctx.lay.work)?;
    std::fs::remove_file(file).map_err(|e| format!("cannot remove {}: {e}", fwd(file)))?;
    j.mark_post(file);
    Ok(true)
}

fn agent(ctx: &Ctx) -> PathBuf {
    ctx.lay.xen.runtime_dir().join(ctx.lay.exe("xenolith"))
}

/// A new journal from what the dry run found.
fn start(ctx: &Ctx, f: &Found) -> Journal {
    Journal {
        format: journal::FORMAT,
        started_at: journal::now(),
        xenolith_version: xenolith_core::VERSION.into(),
        os: ctx.lay.os.clone(),
        state: "running".into(),
        aw_era: f.era.clone(),
        aw_version: f.version.clone(),
        aw_folders: f.folders.iter().map(|(_, p)| p.clone()).collect(),
        aw_config: f.config_file.clone(),
        codex_plugin_id: f.codex_plugin_id.clone(),
        wiki_dir: f.wiki.path.clone().unwrap_or_default(),
        xenolith_installed_before: f.xenolith.installed,
        approve: f.approve,
        aw_created: f.install_state["created"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
        aw_service: f.aw_service.clone(),
        aw_task: f.aw_task,
        aw_run_value: f.aw_run_value.clone(),
        codex_home: Some(Move { from: f.codex_home.from.clone().unwrap_or_default(), to: f.codex_home.to.clone(), status: f.codex_home.action.clone() }),
        ..Default::default()
    }
}

/// Runs (or resumes) the migration. Ok(journal) when it got through step 7.
pub fn run(ctx: &mut Ctx, out: &Out, resume: bool) -> Result<Journal, String> {
    let work = ctx.lay.work.clone();
    let mut existing = Journal::load(&work)?;
    if let (Some(j), false) = (&existing, resume) {
        if j.state == "rolled-back" {
            // A new migration after a rollback: the old journal and backups are kept beside it.
            let stamp = xenolith_core::text::history_stamp(&xenolith_core::text::now());
            for name in ["journal.json", "backup", "migrate.log"] {
                let from = work.join(name);
                if from.exists() {
                    let to = work.join(format!("rolled-back-{stamp}-{name}"));
                    std::fs::rename(&from, &to).map_err(|e| format!("cannot set aside {}: {e}", fwd(&from)))?;
                }
            }
            out.say(format!("The last migration was rolled back; its journal and backups stay in {} (rolled-back-{stamp}-*).", fwd(&work)));
            existing = None;
        }
    }
    let mut j = match (existing, resume) {
        (Some(j), _) if j.state == "rolled-back" => return Err("the last migration was rolled back; --run starts a new one".into()),
        (Some(j), _) if j.state == "rolling-back" => return Err("a rollback did not finish: run --rollback again to finish it".into()),
        (Some(j), _) if j.state == "finished" => {
            out.say("agent-wiki was already moved to Xenolith and its folders removed (--finish). Nothing to do.");
            return Ok(j);
        }
        (Some(j), false) if j.state == "done" => {
            out.say(if j.pending.is_empty() {
                "agent-wiki was already moved to Xenolith. Nothing to do.".to_string()
            } else {
                format!("agent-wiki was already moved to Xenolith; {} thing(s) wait: run --resume once they can be done.", j.pending.len())
            });
            return Ok(j);
        }
        (Some(j), false) => {
            return Err(format!("a migration started on {} is not finished. Run --resume to finish it, or --rollback to undo it.", j.started_at));
        }
        (Some(mut j), true) => {
            if j.state == "done" {
                if j.pending.is_empty() {
                    out.say("agent-wiki was already moved to Xenolith. Nothing to do.");
                    return Ok(j);
                }
                // What waited (Claude desktop, the elevated step): the install, the removals and the check again.
                out.say(format!("Finishing what waited: {}", j.pending.join("; ")));
                j.pending.clear();
                j.items.retain(|_, s| s == "done");
                for n in ["5", "6", "7"] {
                    j.set_step(n, "again");
                }
                j.state = "running".into();
            }
            j
        }
        (None, true) => return Err("there is no migration to resume. Run --run to start one.".into()),
        (None, false) => {
            let f = detect::detect_with(ctx, false);
            if !f.found {
                return Err("no agent-wiki install was found here. Nothing to migrate.".into());
            }
            if !f.blockers.is_empty() {
                return Err(format!("nothing was changed, because:\n  - {}", f.blockers.join("\n  - ")));
            }
            let j = start(ctx, &f);
            j.save(&work)?;
            journal::log(&work, &format!("started: agent-wiki {} ({}), wiki {}", j.aw_version.as_deref().unwrap_or("?"), j.aw_era, j.wiki_dir));
            j
        }
    };
    // Later steps find the same wiki the first one did.
    ctx.opts.wiki_dir = Some(PathBuf::from(&j.wiki_dir));
    for (n, title) in STEPS {
        if j.step(n) == "done" {
            continue;
        }
        out.step(n, title);
        let again = !j.step(n).is_empty() && j.step(n) != "again";
        if again {
            out.info(&format!("(left \"{}\" by an earlier run: checking it and finishing it)", j.step(n)));
        }
        j.set_step(n, "doing");
        j.save(&work)?;
        // Fresh each step: what runs and what is registered changes as the steps go. Only step 1
        // counts the wiki (its snapshot is what step 7 compares with).
        let f = detect::detect_with(ctx, n == "1");
        let r = match n {
            "1" => step1(ctx, out, &mut j, &f),
            "2" => step2(ctx, out, &mut j, &f),
            "3" => step3(ctx, out, &mut j, &f),
            "4" => step4(ctx, out, &mut j),
            "5" => step5(ctx, out, &mut j),
            "6" => step6(ctx, out, &mut j, &f),
            _ => step7(ctx, out, &mut j),
        };
        match r {
            Ok(()) => {
                j.set_step(n, "done");
                j.save(&work)?;
                journal::log(&work, &format!("step {n} done: {title}"));
            }
            Err(e) if crash::is_kill(&e) => return Err(e),
            Err(e) => {
                j.set_step(n, "failed");
                j.note(n, e.clone());
                j.save(&work)?;
                journal::log(&work, &format!("step {n} failed: {title}"));
                return Err(format!("step {n} ({title}) failed: {e}\nFix that, then run --resume; or undo everything with --rollback."));
            }
        }
    }
    j.state = "done".into();
    j.save(&work)?;
    journal::log(&work, "done");
    Ok(j)
}

// ---------------------------------------------------------------- 1. journal and backups

fn step1(ctx: &Ctx, out: &Out, j: &mut Journal, f: &Found) -> Result<(), String> {
    let work = &ctx.lay.work;
    let wiki = PathBuf::from(&j.wiki_dir);
    j.files.clear();
    for (i, (p, role)) in detect::tracked_files(&ctx.lay, Some(&wiki)).into_iter().enumerate() {
        let bak_xenolith_pre = PathBuf::from(format!("{}.bak-xenolith", p.display())).exists();
        let b = journal::backup(work, i, &p)?;
        if b.is_some() {
            out.info(&format!("backed up {} ({role})", fwd(&p)));
        }
        j.files.push(Tracked { path: fwd(&p), role, pre: b.as_ref().map(|x| x.1.clone()), backup: b.map(|x| x.0), known: vec![], busy: false, bak_xenolith_pre });
    }
    j.snapshot = f.wiki.scan.clone().or_else(|| Some(scan::scan(&wiki)));
    j.protocol_before = f.wiki.protocol_hash.clone();
    if let Some(s) = &j.snapshot {
        out.info(&format!("the wiki holds {}", s.summary()));
    }
    out.info(&format!("journal: {}", fwd(&Journal::file(work))));
    Ok(())
}

// ---------------------------------------------------------------- 2. stop agent-wiki

fn step2(ctx: &Ctx, out: &Out, j: &mut Journal, f: &Found) -> Result<(), String> {
    let sys = ctx.sys;
    let wiki = PathBuf::from(&j.wiki_dir);
    // Paused first, so a curator in the middle of a batch finishes it and then stays idle.
    if !xenolith_core::inbox::is_paused(&wiki) {
        j.paused_by_us = true;
        j.save(&ctx.lay.work)?;
        xenolith_core::inbox::set_paused(&wiki, true).map_err(|e| format!("cannot pause the curator: {e}"))?;
        out.info("curator paused (.curator/paused) until the check");
    }
    for (tray, ini) in &f.programs.trays {
        let mut args = vec!["--quit"];
        if let Some(i) = ini.as_deref().filter(|_| !tray.ends_with("AgentWikiTray.exe")) {
            args.extend(["--config", i]);
        }
        let r = sys.run(Path::new(tray), &args, &[]);
        out.info(&format!("{}: {}", tray.rsplit('/').next().unwrap_or(tray), if r.ok { "quit" } else { "was not running" }));
    }
    match ctx.lay.os.as_str() {
        "macos" => {
            let domain = format!("gui/{}", sys.uid());
            for l in AW_LABELS {
                let plist = ctx.lay.launch_agents.join(format!("{l}.plist"));
                if plist.exists() {
                    let r = sys.run(Path::new("launchctl"), &["bootout", &domain, &plist.to_string_lossy()], &[]);
                    out.info(&format!("{l}: {}", if r.ok { "stopped" } else { "not loaded" }));
                }
            }
        }
        "windows" => {
            if win_running(ctx) {
                sys.run(Path::new("sc"), &["stop", AW_SERVICE], &[]);
                if !wait(ctx, || !win_running(ctx)) {
                    return Err(format!("the {AW_SERVICE} service did not stop"));
                }
                out.info(&format!("{AW_SERVICE} service stopped (it is removed in step 5)"));
            }
        }
        _ => {
            for u in AW_UNITS {
                if ctx.lay.systemd_user.join(u).exists() {
                    let r = sys.run(Path::new("systemctl"), &["--user", "stop", u], &[]);
                    out.info(&format!("{u}: {}", if r.ok { "stopped" } else { "not running" }));
                }
            }
        }
    }
    // The Node-era curator left a Codex app-server daemon running in its own Codex home.
    let codex_home = j.codex_home.as_ref().map(|m| PathBuf::from(&m.from)).filter(|p| p.is_dir());
    if let (true, Some(home)) = (j.aw_era == "node", codex_home) {
        let codex = f.config["curator"]["codexPath"].as_str().map(PathBuf::from).or_else(|| sys.which("codex"));
        if let Some(c) = codex {
            let h = home.to_string_lossy().into_owned();
            let r = sys.run(&c, &["app-server", "daemon", "stop"], &[("CODEX_HOME", &h)]);
            out.info(&format!("the curator's Codex app-server daemon: {}", if r.ok { "stopped" } else { "not running" }));
        }
    }
    if !wait(ctx, || detect::locks(&wiki).iter().all(|l| !l.live)) {
        let held: Vec<String> = detect::locks(&wiki).into_iter().filter(|l| l.live).map(|l| format!("{} lock ({})", l.name, l.owner)).collect();
        return Err(format!("the wiki is still in use: {}. Close what holds it, then run --resume.", held.join(", ")));
    }
    out.info("the wiki's locks are free");
    let busy = |ctx: &Ctx| detect::aw_processes(ctx, &ctx.sys.processes()).into_iter().filter(|p| p.kind != "session").collect::<Vec<_>>();
    if !wait(ctx, || busy(ctx).is_empty()) {
        let left: Vec<String> = busy(ctx).iter().map(|p| format!("pid {} ({})", p.pid, p.kind)).collect();
        return Err(format!("agent-wiki still runs: {}. End those processes, then run --resume.", left.join(", ")));
    }
    let sessions = detect::aw_processes(ctx, &sys.processes()).into_iter().filter(|p| p.kind == "session").count();
    if sessions > 0 {
        j.warn(format!("{sessions} agent-wiki server(s) started by open app sessions still run; restart Claude Code and Codex"));
        out.warn(&format!("{sessions} agent-wiki server(s) started by open app sessions still run. They keep working on the same folder until you restart those apps."));
    }
    Ok(())
}

fn win_running(ctx: &Ctx) -> bool {
    detect::win_service(ctx.sys).is_some_and(|s| s != "STOPPED")
}

/// Polls `done` until it holds or the wait runs out.
/// Polls `done` until it holds or the wait runs out (by count, and by the clock: listing processes
/// on Windows takes a second or two).
pub fn wait(ctx: &Ctx, done: impl Fn() -> bool) -> bool {
    let tries = (ctx.wait_ms / 250).max(1);
    let start = std::time::Instant::now();
    for _ in 0..tries {
        if done() {
            return true;
        }
        if start.elapsed().as_millis() as u64 > ctx.wait_ms {
            break;
        }
        ctx.sys.sleep(250);
    }
    done()
}

// ---------------------------------------------------------------- 3. Xenolith's config

fn step3(ctx: &Ctx, out: &Out, j: &mut Journal, f: &Found) -> Result<(), String> {
    let file = ctx.lay.xen.config_file();
    let prev = match edits::read(&file)? {
        Some(t) => edits::parse_json(&t, &file)?,
        None => json!({}),
    };
    let to = j.codex_home.as_ref().map(|m| m.to.clone()).unwrap_or_else(|| fwd(&ctx.lay.xen.curator_codex_home()));
    let (cfg, dropped) = detect::carry(&f.config, &prev, Path::new(&j.wiki_dir), &to);
    let text = edits::to_json(&cfg);
    if edits::read(&file)?.as_deref() != Some(text.as_str()) {
        tracked_write(ctx, j, &file, &text)?;
    }
    out.info(&format!("{}: wikiDir {} (adopted in place)", fwd(&file), j.wiki_dir));
    let keys = detect::carried_keys(&f.config).into_iter().filter(|k| ["writeMode", "curator", "ask", "logs", "hooks", "search"].contains(&k.as_str())).collect::<Vec<_>>();
    if !keys.is_empty() {
        out.info(&format!("carried over from agent-wiki: {}", keys.join(", ")));
    }
    for d in dropped {
        out.info(&format!("not carried: {d}"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 4. the curator's sign-in

fn empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut rd| rd.next().is_none())
}

/// Renames a folder; across volumes, copies it and then removes the original.
pub fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", fwd(parent)))?;
    }
    if empty_dir(to) {
        let _ = std::fs::remove_dir(to);
    }
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if cross_device(&e) => {
            copy_tree(from, to).map_err(|e| format!("cannot copy {} to {}: {e}", fwd(from), fwd(to)))?;
            std::fs::remove_dir_all(from).map_err(|e| format!("copied, but cannot remove {}: {e}", fwd(from)))
        }
        Err(e) => Err(format!("cannot move {} to {}: {e}", fwd(from), fwd(to))),
    }
}

/// EXDEV on Unix, ERROR_NOT_SAME_DEVICE on Windows: a rename cannot cross volumes.
pub fn cross_device(e: &std::io::Error) -> bool {
    e.raw_os_error() == Some(if cfg!(windows) { 17 } else { 18 })
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let (s, d) = (e.path(), to.join(e.file_name()));
        if e.file_type()?.is_dir() {
            copy_tree(&s, &d)?;
        } else {
            std::fs::copy(&s, &d)?;
        }
    }
    Ok(())
}

fn step4(ctx: &Ctx, out: &Out, j: &mut Journal) -> Result<(), String> {
    let Some(m) = j.codex_home.clone() else { return Ok(()) };
    let (from, to) = (PathBuf::from(&m.from), PathBuf::from(&m.to));
    match m.status.as_str() {
        "move" | "moving" => {
            if !from.exists() && to.exists() {
                out.info(&format!("already moved to {}", fwd(&to)));
            } else if from.is_dir() {
                if to.exists() && !empty_dir(&to) {
                    return Err(format!("{} already exists and is not empty; agent-wiki's sign-in is still in {}", fwd(&to), fwd(&from)));
                }
                j.codex_home.as_mut().unwrap().status = "moving".into();
                j.save(&ctx.lay.work)?;
                move_dir(&from, &to)?;
                out.info(&format!("moved {} -> {}", fwd(&from), fwd(&to)));
            }
            j.codex_home.as_mut().unwrap().status = "moved".into();
        }
        "kept" => out.info(&format!("kept at {} (outside agent-wiki's folders)", fwd(&to))),
        "kept-xenolith" => out.info(&format!("Xenolith's own sign-in in {} is kept; agent-wiki's stays in {} until --finish", fwd(&to), fwd(&from))),
        _ => out.info("agent-wiki's curator had no sign-in folder"),
    }
    Ok(())
}

// ---------------------------------------------------------------- 5. xenolith install

/// What step 5 passes to `xenolith install`.
pub fn install_args(ctx: &Ctx, j: &Journal) -> Vec<String> {
    let mut args: Vec<String> = vec!["install".into(), "--wiki-dir".into(), j.wiki_dir.clone()];
    if !j.approve {
        args.push("--no-approve".into());
    }
    if let Some(p) = ctx.opts.port {
        args.extend(["--port".into(), p.to_string()]);
    }
    if let Some(f) = &ctx.opts.from {
        args.extend(["--from".into(), f.to_string_lossy().into_owned()]);
    }
    if ctx.lay.os == "windows" && detect::win_service(ctx.sys).is_some() {
        // The AgentWiki service goes in the same elevated step that registers Xenolith's: one prompt.
        let s = &ctx.lay.standard;
        let grants = [PathBuf::from(&j.wiki_dir), s.config.clone(), s.data.clone(), s.log.clone()];
        args.extend(crate::migrate::retire::forward(&grants));
    }
    args
}

fn step5(ctx: &Ctx, out: &Out, j: &mut Journal) -> Result<(), String> {
    let args = install_args(ctx, j);
    out.info(&format!("xenolith {}", args.join(" ")));
    let all: Vec<PathBuf> = j.files.iter().map(|t| PathBuf::from(&t.path)).collect();
    j.set_busy(&all.iter().map(PathBuf::as_path).collect::<Vec<_>>());
    j.save(&ctx.lay.work)?;
    let r = ctx.sys.xenolith(&args);
    for line in format!("{}{}", r.stdout, r.stderr).lines() {
        out.info(&format!("  | {line}"));
    }
    j.mark_all();
    if !r.ok {
        return Err(format!("xenolith install failed: {}", r.last_line()));
    }
    if ctx.lay.os == "windows" && detect::win_service(ctx.sys).is_some() {
        j.pend(format!("the {AW_SERVICE} service is still registered (the UAC prompt was declined?): run --resume and approve it"));
    }
    Ok(())
}

// ---------------------------------------------------------------- 6. agent-wiki's registrations

/// One part of step 6, journaled on its own so a resumed run skips what is done.
fn item(ctx: &Ctx, j: &mut Journal, id: &str, f: impl FnOnce(&mut Journal) -> Result<String, String>) -> Result<(), String> {
    if j.items.get(id).is_some_and(|s| s == "done" || s == "none") {
        return Ok(());
    }
    let st = f(j)?;
    j.items.insert(id.into(), st);
    j.save(&ctx.lay.work)
}

fn created_by_aw(j: &Journal, p: &Path) -> bool {
    j.aw_created.iter().any(|c| c.eq_ignore_ascii_case(&fwd(p)))
}

fn step6(ctx: &Ctx, out: &Out, j: &mut Journal, f: &Found) -> Result<(), String> {
    let sys = ctx.sys;
    let lay = &ctx.lay;
    let has = |kind: &str| f.registrations.iter().any(|r| r.kind == kind);

    // Start at sign-in and the services (agent-wiki's uninstall step 1, and uninstall-service).
    item(ctx, j, "services", |j| {
        match lay.os.as_str() {
            "macos" => {
                let domain = format!("gui/{}", sys.uid());
                for l in AW_LABELS {
                    let plist = lay.launch_agents.join(format!("{l}.plist"));
                    if plist.exists() {
                        sys.run(Path::new("launchctl"), &["bootout", &domain, &plist.to_string_lossy()], &[]);
                        tracked_remove(ctx, j, &plist)?;
                        out.info(&format!("removed LaunchAgent {l}"));
                    }
                }
            }
            "windows" => {
                if detect::win_task(sys) {
                    sys.run(Path::new("schtasks"), &["/delete", "/tn", AW_TASK, "/f"], &[]);
                    out.info(&format!("removed the logon task \\{AW_TASK}"));
                }
                if detect::win_run_value(sys).is_some() {
                    sys.run(Path::new("reg"), &["delete", RUN_KEY, "/v", AW_RUN_VALUE, "/f"], &[]);
                    out.info(&format!("removed the Run value {AW_RUN_VALUE}"));
                }
            }
            _ => {
                let mut any = false;
                for u in AW_UNITS {
                    let file = lay.systemd_user.join(u);
                    if file.exists() {
                        sys.run(Path::new("systemctl"), &["--user", "disable", "--now", u], &[]);
                        tracked_remove(ctx, j, &file)?;
                        out.info(&format!("removed {u}"));
                        any = true;
                    }
                }
                if any {
                    sys.run(Path::new("systemctl"), &["--user", "daemon-reload"], &[]);
                }
                let desktop = lay.autostart.join(detect::AW_AUTOSTART);
                if tracked_remove(ctx, j, &desktop)? {
                    out.info("removed the tray's autostart entry");
                }
            }
        }
        Ok("done".into())
    })?;

    // The Claude Code plugin and its marketplace.
    let claude_registered = has("Claude Code plugin") || has("Claude Code marketplace");
    item(ctx, j, "claude-plugin", |j| {
        if !claude_registered {
            return Ok("none".into());
        }
        let Some(cl) = sys.which("claude") else {
            j.pend("remove the Claude Code plugin agent-wiki@agent-wiki-local in Claude Code (/plugin), since the claude CLI is not on PATH".into());
            return Ok("pending".into());
        };
        j.items.insert("claude-plugin".into(), "doing".into());
        j.set_busy(&[&lay.claude_settings]);
        j.save(&lay.work)?;
        let u = sys.run(&cl, &["plugin", "uninstall", edits::CLAUDE_PLUGIN_ID], &[]);
        let m = sys.run(&cl, &["plugin", "marketplace", "remove", edits::CLAUDE_MARKET], &[]);
        out.info(&format!(
            "claude plugin uninstall {}: {}; marketplace {}: {}",
            edits::CLAUDE_PLUGIN_ID,
            if u.ok { "done" } else { "not installed" },
            edits::CLAUDE_MARKET,
            if m.ok { "removed" } else { "not there" }
        ));
        j.mark_post(&lay.claude_settings);
        Ok("done".into())
    })?;

    // The ChatGPT / Codex plugin.
    let codex_id = j.codex_plugin_id.clone();
    let codex_registered = has("Codex plugin");
    item(ctx, j, "codex-plugin", |j| {
        if !codex_registered {
            return Ok("none".into());
        }
        let Some(cx) = sys.which("codex") else {
            j.pend(format!("remove the Codex plugin {codex_id} from the ChatGPT app's Plugins, since the codex CLI is not on PATH"));
            return Ok("pending".into());
        };
        j.items.insert("codex-plugin".into(), "doing".into());
        j.set_busy(&[&lay.codex_config]);
        j.save(&lay.work)?;
        let r = sys.run(&cx, &["plugin", "remove", &codex_id], &[]);
        out.info(&format!("codex plugin remove {codex_id}: {}", if r.ok { "done" } else { "not installed" }));
        j.mark_post(&lay.codex_config);
        Ok("done".into())
    })?;

    item(ctx, j, "personal-marketplace", |j| {
        let file = &lay.personal_market;
        let mut emptied = false;
        let changed = tracked_edit(ctx, j, file, |t| {
            Ok(edits::market_remove(t, file)?.map(|(next, e)| {
                emptied = e;
                next
            }))
        })?;
        if emptied && created_by_aw(j, file) {
            tracked_remove(ctx, j, file)?;
        }
        if changed {
            out.info(&format!("{}: entry agent-wiki removed", fwd(file)));
        }
        Ok("done".into())
    })?;

    item(ctx, j, "claude-desktop", |j| {
        let with: Vec<PathBuf> =
            lay.desktop_configs.iter().filter(|c| edits::read(c).ok().flatten().is_some_and(|t| edits::parse_json(&t, c).is_ok_and(|v| edits::desktop_has(&v)))).cloned().collect();
        if with.is_empty() {
            return Ok("none".into());
        }
        if sys.desktop_running() {
            j.pend("quit Claude desktop (tray icon > Quit), then run --resume to remove its agent-wiki entry".into());
            out.warn("Claude desktop is running and would write its config back: its agent-wiki entry waits");
            return Ok("pending".into());
        }
        for c in with {
            if tracked_edit(ctx, j, &c, |t| edits::desktop_remove(t, &c))? {
                out.info(&format!("{}: mcpServers.agent-wiki removed", fwd(&c)));
            }
        }
        Ok("done".into())
    })?;

    item(ctx, j, "instructions", |j| {
        for file in [&lay.claude_md, &lay.codex_agents] {
            let mut empty = false;
            let changed = tracked_edit(ctx, j, file, |t| {
                Ok(edits::remove_block(t).inspect(|next| {
                    empty = next.is_empty();
                }))
            })?;
            if empty && created_by_aw(j, file) {
                tracked_remove(ctx, j, file)?;
            }
            if changed {
                out.info(&format!("{}: agent-wiki's block removed", fwd(file)));
            }
        }
        Ok("done".into())
    })?;

    item(ctx, j, "codex-approval", |j| {
        let file = &lay.codex_config;
        let Some(text) = edits::read(file)? else { return Ok("none".into()) };
        let mut ids = edits::toml_plugin_ids(&edits::parse_toml(&text, file)?);
        if !ids.contains(&j.codex_plugin_id) {
            ids.push(j.codex_plugin_id.clone());
        }
        for id in ids {
            if tracked_edit(ctx, j, file, |t| edits::toml_remove_approval(t, &id, file))? {
                out.info(&format!("{}: [plugins.\"{id}\".mcp_servers.agent-wiki] removed", fwd(file)));
            }
        }
        Ok("done".into())
    })?;

    item(ctx, j, "claude-allow", |j| {
        let file = &lay.claude_settings;
        if tracked_edit(ctx, j, file, |t| edits::allow_remove(t, file))? {
            out.info(&format!("{}: {} removed from permissions.allow", fwd(file), edits::CLAUDE_ALLOW.join(", ")));
        }
        Ok("done".into())
    })?;

    // The files agent-wiki's installer created now hold Xenolith's entries: Xenolith's uninstall
    // deletes them when they end up empty, as agent-wiki's would have.
    item(ctx, j, "created", |j| {
        let state = lay.xen.install_state();
        let shared: Vec<String> =
            [&lay.claude_md, &lay.codex_agents, &lay.personal_market].into_iter().chain(lay.desktop_configs.iter()).filter(|p| created_by_aw(j, p) && p.exists()).map(|p| fwd(p)).collect();
        if shared.is_empty() {
            return Ok("none".into());
        }
        tracked_edit(ctx, j, &state, |t| {
            let mut v = edits::parse_json(t, &state)?;
            let mut list: Vec<Value> = v["created"].as_array().cloned().unwrap_or_default();
            let before = list.len();
            for s in &shared {
                if !list.iter().any(|x| x == s) {
                    list.push(json!(s));
                }
            }
            if list.len() == before {
                return Ok(None);
            }
            v["created"] = Value::Array(list);
            Ok(Some(edits::to_json(&v)))
        })?;
        Ok("done".into())
    })?;
    Ok(())
}

// ---------------------------------------------------------------- 7. check

fn step7(ctx: &Ctx, out: &Out, j: &mut Journal) -> Result<(), String> {
    let wiki = PathBuf::from(&j.wiki_dir);
    let mut failed: Vec<String> = vec![];
    let mut warned: Vec<String> = vec![];
    let mut v = serde_json::Map::new();

    // The service, on the adopted folder.
    if ctx.sandbox() {
        out.info("service: none in a sandbox (the tools are checked over stdio)");
    } else {
        let health = ctx.sys.get_json(ctx.port(), "/health");
        let status = ctx.sys.get_json(ctx.port(), "/status");
        let folder = status.as_ref().and_then(|s| s["wikiDir"].as_str().map(String::from));
        let ok = health.as_ref().is_some_and(|h| h["ok"] == true && h["name"] == "xenolith") && folder.as_deref() == Some(fwd(&wiki).as_str());
        v.insert("service".into(), json!(ok));
        if ok {
            out.info(&format!("service: /health OK, /status shows {}", fwd(&wiki)));
        } else {
            warned.push(format!(
                "the service on port {} does not answer on the adopted folder (it shows {}); the apps start the server themselves until it does",
                ctx.port(),
                folder.unwrap_or_else(|| "nothing".into())
            ));
        }
    }

    // The tools, by their names and by agent-wiki's.
    match ctx.sys.mcp_start(&agent(ctx), &[]) {
        Ok((a, b)) => {
            v.insert("contextStart".into(), json!(a));
            v.insert("wikiStart".into(), json!(b));
            if a && b {
                out.info("tools: context_start and wiki_start answer");
            } else {
                failed.push(format!("the tools did not answer (context_start {a}, wiki_start {b})"));
            }
        }
        Err(e) => failed.push(format!("the server did not start: {e}")),
    }

    // Everything the wiki held is still there.
    let now = scan::scan(&wiki);
    let diff = j.snapshot.as_ref().map(|s| scan::differences(s, &now)).unwrap_or_default();
    v.insert("wiki".into(), json!(diff.is_empty()));
    if diff.is_empty() {
        out.info(&format!("wiki: unchanged ({})", now.summary()));
    } else {
        let grew = j.snapshot.as_ref().is_some_and(|s| now.pages.count >= s.pages.count && now.vectors.count >= s.vectors.count && now.filed.count >= s.filed.count);
        let line = format!("the wiki changed during the migration: {}", diff.join("; "));
        if grew {
            warned.push(format!("{line} (an app still running agent-wiki wrote to it)"));
        } else {
            failed.push(line);
        }
    }

    // The curator signed in, with the sign-in that moved.
    let cfg = edits::read(&ctx.lay.xen.config_file()).ok().flatten().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or_default();
    if cfg["writeMode"] != "direct" {
        let home = cfg["curator"]["codexHome"].as_str().map(String::from).unwrap_or_else(|| fwd(&ctx.lay.xen.curator_codex_home()));
        let codex = cfg["curator"]["codexPath"].as_str().map(PathBuf::from).filter(|p| p.is_absolute() && p.exists()).or_else(|| ctx.sys.which("codex"));
        let signed = codex.map(|c| ctx.sys.run(&c, &["login", "status"], &[("CODEX_HOME", &home)]).ok);
        v.insert("curatorSignedIn".into(), json!(signed));
        match signed {
            Some(true) => out.info("curator: signed in"),
            Some(false) => warned.push("the curator is not signed in: tray icon > Curator > Sign in to ChatGPT for the curator".into()),
            None => warned.push("the codex CLI is not installed, so the curator cannot run; notes wait in the inbox".into()),
        }
    }

    // Nothing of agent-wiki's is registered any more.
    let (left, _) = detect::registrations(ctx, &j.codex_plugin_id);
    v.insert("leftovers".into(), json!(left.iter().map(|r| format!("{} {}", r.kind, r.key)).collect::<Vec<_>>()));
    if left.is_empty() {
        out.info("agent-wiki: nothing registered any more");
    }
    for r in &left {
        let kind = r.kind.to_lowercase();
        let known = j
            .pending
            .iter()
            .map(|p| p.to_lowercase())
            .any(|p| p.contains(&kind) || (kind == "windows service" && p.contains("agentwiki service")) || (kind.starts_with("claude code") && p.contains("claude code plugin")));
        if !known {
            let stand_in = ctx.sandbox() && (r.kind.starts_with("Claude Code") || r.kind == "Codex plugin");
            warned.push(format!(
                "still registered: {} {}{}{}",
                r.kind,
                r.key,
                r.file.as_ref().map(|f| format!(" in {f}")).unwrap_or_default(),
                if stand_in { " (in a sandbox the app's CLI is only a stand-in)" } else { "" }
            ));
        }
    }

    // The curator goes back to work, unless something is wrong.
    if j.paused_by_us && failed.is_empty() {
        xenolith_core::inbox::set_paused(&wiki, false).map_err(|e| format!("cannot resume the curator: {e}"))?;
        j.paused_by_us = false;
        out.info("curator resumed");
    } else if j.paused_by_us {
        out.warn("the curator stays paused while you look into this (or roll back)");
    }
    for w in &warned {
        out.warn(w);
        j.warn(w.clone());
    }
    for f in &failed {
        out.warn(f);
    }
    v.insert("ok".into(), json!(failed.is_empty()));
    v.insert("failed".into(), json!(failed));
    v.insert("warnings".into(), json!(warned));
    j.verify = Some(Value::Object(v));
    Ok(())
}
