//! xenonet diagnose (cli/commands/diagnose.js): what XenoNet can tell
//! someone helping you, safe to share (core/diagnostics.js), and with
//! --ask, an agent on this computer to look at it with you. It works with
//! XenoNet running or not.

use super::doctor::checkup;
use crate::args::Flags;
use crate::cores;
use crate::js::{list, str};
use crate::out::{c, eprint, now, print, write, Done, Fail};
use crate::reach::find;
use crate::sys;
use serde_json::{json, Value};
use std::path::Path;
use xenonet_base::log::{log_dir, read_logs};

pub const HELP: &str = include_str!("help/diagnose.txt");

const VERSION: &str = env!("XENONET_VERSION");

/// What the bundle is made of, and the state folder it's about.
pub fn gather(flags: &Flags) -> Result<(String, Value), Fail> {
    let s = find(flags);
    let state_dir = sys::state_dir(flags.on("dev"));
    let doctor = checkup(flags, None).ok();
    let connect = s.as_ref().and_then(|s| s.get("/api/engine/connect").ok());
    let launcher = cores::machine_launcher().ok();
    let health = s.as_ref().map_or(Value::Null, |s| s.health.clone());
    let instance = crate::js::have(&health, "instance").cloned().unwrap_or_else(|| if flags.on("dev") { json!("dev") } else { Value::Null });
    let xenonet = json!({
        "version": crate::js::have(&health, "version").cloned().unwrap_or_else(|| json!(VERSION)),
        "instance": instance,
        "packaged": sys::var("XENONET_LAUNCHER_BIN").or_else(|| sys::var("XENONET_LAUNCHER_EXEC")).is_some(),
        "running": s.is_some(),
    });
    let errors = read_logs(Path::new(&log_dir(&state_dir)), now() - 24.0 * 3600e3, "warn", None, None, 500);
    let bundle = cores::make_bundle(&json!({ "xenonet": xenonet, "doctor": doctor, "errors": errors, "connect": connect, "launcher": launcher }))?;
    Ok((state_dir, bundle))
}

#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the name is lowercased first")]
pub fn run(_args: &[String], flags: &Flags) -> Done {
    // Codex, unless you ask for Claude Code; another app is refused before
    // anything is gathered.
    let asked = flags.text("app").or_else(|| sys::var("XENONET_DIAGNOSE_APP")).unwrap_or_else(|| "codex".into());
    let app = xenonet_base::apps::asked_or_text(Some(&asked), Some("codex"));
    let app = match app {
        Some(app) => app,
        None if flags.on("ask") => return Err(Fail::new(xenonet_base::apps::app_refusal_among("--app", &Value::String(asked), &xenonet_base::apps::ASKED), 2)),
        None => "codex",
    };
    let (state_dir, bundle) = gather(flags)?;
    if !flags.on("ask") {
        let text = if flags.on("json") { format!("{}\n", xenonet_base::json::stringify(&bundle, 2)) } else { cores::bundle_markdown(&bundle)? };
        if let Some(out) = flags.text("out") {
            std::fs::write(&out, &text).map_err(|e| Fail::error(xenonet_base::fs::FsError::of(e, "open", &out).message))?;
            print(format!("{} Wrote {out}. It's safe to share: no transcripts, titles, names or keys.", c::green("✓")));
            return Ok(0);
        }
        write(&text);
        return Ok(0);
    }
    let model = flags.text("model").or_else(|| sys::var("XENONET_DIAGNOSE_MODEL"));
    let named = if app == "claude" { "Claude Code" } else { "Codex" };
    let path_var = std::env::var("PATH").unwrap_or_default();
    let pathext = std::env::var("PATHEXT").ok();
    let Some(bin) = xenonet_base::paths::on_path(app, &path_var, pathext.as_deref(), cfg!(windows), &mut xenonet_base::paths::can_run) else {
        return Err(Fail::error(format!("{named} isn't on your PATH; install it, or use --app {}.", if app == "claude" { "codex" } else { "claude" })));
    };
    let root = sys::xenonet_root();
    let dir = cores::write_look(&state_dir, &bundle, std::slice::from_ref(&root))?;
    let cmd = cores::ask_command(app, model.as_deref(), &dir, &[root])?;
    print(format!(
        "{}\n{}\n",
        c::dim(format!("XenoNet wrote what it could tell to {dir}.")),
        c::dim(format!(
            "Starting {named}{}: it reads, explains and asks before changing anything, and never files anything.",
            model.as_ref().map_or(String::new(), |m| format!(" with {m}"))
        ))
    ));
    // A .cmd (an npm install) runs only through the shell. The session is
    // yours to talk to, in this terminal: it inherits the console, so it
    // doesn't take Background::no_window, which on Windows would give it a
    // console of its own with no window, cut off from this one.
    let args: Vec<String> = list(&cmd, "args").iter().map(|a| crate::js::text(Some(a))).collect();
    let lower = bin.to_ascii_lowercase();
    let mut child = if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        let line = std::iter::once(format!("\"{bin}\"")).chain(args.iter().map(|a| format!("\"{a}\""))).collect::<Vec<_>>().join(" ");
        let mut c = std::process::Command::new("cmd.exe");
        c.args(["/d", "/s", "/c", &format!("\"{line}\"")]);
        c
    } else {
        let mut c = std::process::Command::new(&bin);
        c.args(&args);
        c
    };
    if let Some(cwd) = str(&cmd, "cwd") {
        child.current_dir(cwd);
    }
    let code = match child.status() {
        Err(e) => {
            eprint(format!("xenonet: couldn't start {app}: {e}"));
            return Ok(1);
        }
        Ok(st) => st.code().unwrap_or(0),
    };
    let draft = sys::join(&dir, "issue-draft.md");
    if Path::new(&draft).exists() {
        print(format!("\n{} It wrote an issue draft: {draft}\n  Read it, and send it yourself if you choose: https://github.com/willctl/xenonet/issues/new", c::yellow("!")));
    }
    Ok(code)
}
