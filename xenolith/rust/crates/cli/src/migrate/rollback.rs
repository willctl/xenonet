//! --rollback: undoes what the journal says was done, in reverse. A file goes back to its copy from
//! before only while it is exactly as the migration left it; a file changed since is left alone
//! and named, with where its copy is. agent-wiki's programs are still in place (--finish is what
//! removes them), so its services, tray and registrations come back as they were.

use super::Out;
use super::detect::{self, AW_LABELS, AW_RUN_VALUE, AW_SERVICE, AW_TASK, AW_UNITS, Ctx, RUN_KEY};
use super::edits::{self, fwd};
use super::journal::{self, Journal, Tracked, sha_file};
use super::run::move_dir;
use std::path::{Path, PathBuf};

/// Restores one tracked file: "restored", "as before", or why not.
fn restore(work: &Path, t: &Tracked) -> Result<String, String> {
    let p = PathBuf::from(&t.path);
    let cur = sha_file(&p);
    if cur == t.pre || (t.known.is_empty() && !t.busy) {
        return Ok("as before".into());
    }
    if !t.busy && !t.known.contains(&cur) {
        return Err(format!(
            "{} ({}) changed since the migration, so it was left as it is{}",
            t.path,
            t.role,
            t.backup.as_ref().map(|b| format!("; its copy from before is {}", fwd(&work.join(b)))).unwrap_or_default()
        ));
    }
    match &t.backup {
        Some(b) => {
            let bytes = std::fs::read(work.join(b)).map_err(|e| format!("cannot read the backup of {}: {e}", t.path))?;
            if Some(xenolith_core::text::sha256_hex(&bytes)) != t.pre {
                return Err(format!("the backup of {} does not match what was recorded; left as it is", t.path));
            }
            edits::write_atomic(&p, &bytes)?;
        }
        None => std::fs::remove_file(&p).map_err(|e| format!("cannot remove {}: {e}", t.path))?,
    }
    Ok("restored".into())
}

fn part(ctx: &Ctx, j: &mut Journal, id: &str, f: impl FnOnce(&mut Journal) -> Result<(), String>) -> Result<(), String> {
    if j.rollback.get(id).is_some_and(|s| s == "done") {
        return Ok(());
    }
    f(j)?;
    j.rollback.insert(id.into(), "done".into());
    j.save(&ctx.lay.work)
}

const XEN_ROLES: [&str; 2] = ["Xenolith config", "Xenolith install state"];

pub fn rollback(ctx: &mut Ctx, out: &Out) -> Result<Journal, String> {
    let work = ctx.lay.work.clone();
    let Some(mut j) = Journal::load(&work)? else { return Err("there is no migration to roll back".into()) };
    match j.state.as_str() {
        "finished" => return Err("--finish already removed agent-wiki's programs, so there is nothing to go back to. Reinstall agent-wiki to use it again.".into()),
        "rolled-back" => {
            out.say("This migration was already rolled back.");
            return Ok(j);
        }
        _ => {}
    }
    if j.state != "rolling-back" {
        // What waited for the run (Claude desktop, a CLI) no longer applies.
        j.pending.clear();
    }
    j.state = "rolling-back".into();
    j.save(&work)?;
    ctx.opts.wiki_dir = Some(PathBuf::from(&j.wiki_dir));
    let sys = ctx.sys;
    let started = |j: &Journal, n: &str| !j.step(n).is_empty();

    // 6: agent-wiki's registrations back: the files as they were, its plugins, its start at sign-in.
    out.step("6", "agent-wiki's registrations");
    let files: Vec<Tracked> = j.files.iter().filter(|t| !XEN_ROLES.contains(&t.role.as_str())).cloned().collect();
    part(ctx, &mut j, "files", |j| {
        for t in &files {
            match restore(&work, t) {
                Ok(s) if s == "restored" => out.info(&format!("{}: restored ({})", t.path, t.role)),
                Ok(_) => {}
                Err(e) => {
                    out.warn(&e);
                    j.warn(format!("rollback: {e}"));
                }
            }
        }
        Ok(())
    })?;
    let market = j.aw_folders.iter().map(|d| Path::new(d).join("marketplace")).find(|m| m.is_dir());
    part(ctx, &mut j, "plugins", |j| {
        let started = |id: &str| j.items.get(id).is_some_and(|s| s == "done" || s == "doing");
        if started("claude-plugin") {
            match (sys.which("claude"), &market) {
                (Some(cl), Some(m)) => {
                    sys.run(&cl, &["plugin", "marketplace", "add", &m.to_string_lossy()], &[]);
                    let r = sys.run(&cl, &["plugin", "install", edits::CLAUDE_PLUGIN_ID], &[]);
                    out.info(&format!("claude plugin install {}: {}", edits::CLAUDE_PLUGIN_ID, if r.ok { "done" } else { "failed" }));
                }
                _ => out.warn(&format!("put the Claude Code plugin back by hand: /plugin, marketplace {}", market.as_ref().map(|m| fwd(m)).unwrap_or_default())),
            }
        }
        if started("codex-plugin") {
            match sys.which("codex") {
                Some(cx) => {
                    let r = sys.run(&cx, &["plugin", "add", &j.codex_plugin_id], &[]);
                    out.info(&format!("codex plugin add {}: {}", j.codex_plugin_id, if r.ok { "done" } else { "failed" }));
                }
                None => out.warn(&format!("put the Codex plugin {} back from the ChatGPT app's Plugins", j.codex_plugin_id)),
            }
        }
        if ctx.lay.os == "windows" {
            let xml = j.aw_folders.iter().map(|d| Path::new(d).join("tray").join("AgentWikiTray.task.xml")).find(|x| x.exists());
            if let (true, false, Some(x)) = (j.aw_task, detect::win_task(sys), xml) {
                sys.run(Path::new("schtasks"), &["/create", "/tn", AW_TASK, "/xml", &x.to_string_lossy(), "/f"], &[]);
                out.info(&format!("logon task \\{AW_TASK} registered again"));
            }
            if let (Some(v), None) = (&j.aw_run_value, detect::win_run_value(sys)) {
                sys.run(Path::new("reg"), &["add", RUN_KEY, "/v", AW_RUN_VALUE, "/t", "REG_SZ", "/d", v, "/f"], &[]);
                out.info(&format!("Run value {AW_RUN_VALUE} written again"));
            }
        }
        Ok(())
    })?;

    // 5: Xenolith's install undone, unless it was there before.
    out.step("5", "xenolith install");
    if started(&j, "5") {
        let before = j.xenolith_installed_before;
        part(ctx, &mut j, "xenolith", |j| {
            if before {
                out.info("Xenolith was installed before the migration: it stays, with its settings as they were");
            } else {
                // The uninstall changes Xenolith's own files (and takes its entries out of the apps').
                let all: Vec<PathBuf> = j.files.iter().map(|t| PathBuf::from(&t.path)).collect();
                j.set_busy(&all.iter().map(PathBuf::as_path).collect::<Vec<_>>());
                j.save(&ctx.lay.work)?;
                let r = sys.xenolith(&["uninstall".to_string()]);
                j.mark_all();
                for line in format!("{}{}", r.stdout, r.stderr).lines() {
                    out.info(&format!("  | {line}"));
                }
                if !r.ok {
                    return Err(format!("xenolith uninstall failed: {}", r.last_line()));
                }
                if ctx.lay.os == "windows" {
                    out.warn("the Xenolith service stays until you run `xenolith uninstall-service` in a terminal opened with \"Run as administrator\"");
                }
            }
            if ctx.lay.os == "windows" && j.aw_service.is_some() && detect::win_service(sys).is_none() {
                let agent = j.aw_folders.iter().map(|d| Path::new(d).join("runtime").join("agent-wiki.exe")).find(|a| a.exists());
                let cmd = agent.map(|a| format!("\"{}\" install-service", a.display())).unwrap_or_else(|| "npm run install-service (in agent-wiki's folder)".into());
                j.pend(format!("put the {AW_SERVICE} service back: in a terminal opened with \"Run as administrator\", run {cmd}"));
                out.warn(&format!("the {AW_SERVICE} service needs administrator rights to come back: in a terminal opened with \"Run as administrator\", run {cmd}"));
            }
            // The installer's own first backups of files it had never edited before.
            for t in j.files.iter().filter(|t| !t.bak_xenolith_pre) {
                let _ = std::fs::remove_file(format!("{}.bak-xenolith", t.path));
            }
            Ok(())
        })?;
    }

    // 4: the curator's sign-in back where it was.
    out.step("4", "The curator's sign-in");
    part(ctx, &mut j, "codex-home", |j| {
        if let Some(m) = j.codex_home.as_mut().filter(|m| m.status == "moved" || m.status == "moving") {
            let (from, to) = (PathBuf::from(&m.from), PathBuf::from(&m.to));
            if to.is_dir() && !from.exists() {
                move_dir(&to, &from)?;
                out.info(&format!("moved {} back to {}", fwd(&to), fwd(&from)));
            }
            m.status = "rolled-back".into();
        }
        Ok(())
    })?;

    // 3: Xenolith's config and install state as they were.
    out.step("3", "Xenolith's config");
    let xen: Vec<Tracked> = j.files.iter().filter(|t| XEN_ROLES.contains(&t.role.as_str())).cloned().collect();
    part(ctx, &mut j, "config", |j| {
        for t in &xen {
            match restore(&work, t) {
                Ok(s) if s == "restored" => out.info(&format!("{}: {}", t.path, if t.pre.is_some() { "restored" } else { "removed (it did not exist before)" })),
                Ok(_) => {}
                Err(e) => {
                    out.warn(&e);
                    j.warn(format!("rollback: {e}"));
                }
            }
        }
        Ok(())
    })?;

    // 2: agent-wiki running again.
    out.step("2", "Start agent-wiki");
    let wiki = PathBuf::from(&j.wiki_dir);
    part(ctx, &mut j, "restart", |j| {
        if j.paused_by_us && xenolith_core::inbox::is_paused(&wiki) {
            let _ = xenolith_core::inbox::set_paused(&wiki, false);
            out.info("curator unpaused");
        }
        j.paused_by_us = false;
        match ctx.lay.os.as_str() {
            "macos" => {
                let domain = format!("gui/{}", sys.uid());
                for l in AW_LABELS {
                    let plist = ctx.lay.launch_agents.join(format!("{l}.plist"));
                    if plist.exists() {
                        let r = sys.run(Path::new("launchctl"), &["bootstrap", &domain, &plist.to_string_lossy()], &[]);
                        out.info(&format!("{l}: {}", if r.ok { "started" } else { "already loaded" }));
                    }
                }
            }
            "windows" => {
                if detect::win_service(sys).is_some_and(|s| s == "STOPPED") {
                    sys.run(Path::new("sc"), &["start", AW_SERVICE], &[]);
                    out.info(&format!("{AW_SERVICE} service started"));
                }
                if detect::win_task(sys) {
                    sys.run(Path::new("schtasks"), &["/run", "/tn", AW_TASK], &[]);
                    out.info("tray started");
                }
            }
            _ => {
                let units: Vec<&str> = AW_UNITS.into_iter().filter(|u| ctx.lay.systemd_user.join(u).exists()).collect();
                if !units.is_empty() {
                    sys.run(Path::new("systemctl"), &["--user", "daemon-reload"], &[]);
                    for u in units {
                        let r = sys.run(Path::new("systemctl"), &["--user", "enable", "--now", u], &[]);
                        out.info(&format!("{u}: {}", if r.ok { "started" } else { "did not start" }));
                    }
                }
                let tray = j.aw_folders.iter().map(|d| Path::new(d).join("tray").join("agent-wiki-tray")).find(|t| t.exists());
                if let (true, Some(t)) = (ctx.lay.autostart.join(detect::AW_AUTOSTART).exists(), tray) {
                    sys.spawn(&t, &["--from", "install"]);
                    out.info("tray started");
                }
            }
        }
        if j.xenolith_installed_before && started(j, "5") {
            // Xenolith's services pick up its restored settings.
            let mut args = vec!["install".to_string()];
            if let Some(f) = &ctx.opts.from {
                args.extend(["--from".into(), f.to_string_lossy().into_owned()]);
            }
            let r = sys.xenolith(&args);
            out.info(&format!("xenolith install (its own settings again): {}", if r.ok { "done" } else { "failed" }));
        }
        Ok(())
    })?;

    out.step("1", "Journal");
    j.state = "rolled-back".into();
    j.save(&work)?;
    let left = j.warnings.iter().filter(|w| w.starts_with("rollback: ")).count();
    journal::log(&work, &format!("rolled back ({left} file(s) left as they were)"));
    out.info(&format!("the journal and backups stay in {}", fwd(&work)));
    Ok(j)
}
