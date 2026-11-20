//! Sandboxed runs: a run held to its app's own sandbox (Claude Code's,
//! Codex's), which XenoNet drives and doesn't replace. What the app can do
//! is its adapter's (`SandboxWays`); here are the files a run is given, the
//! environment it runs with, and what is kept of a sandboxed session so a
//! turn resumed for it later, or a copy forked from it, is sandboxed too:
//! a record is written before the run starts (no record, no run), renewed
//! by every turn, written for a copy when its stream names it, and not
//! swept by age; a session started for anyone but the person that has none
//! is refused, never run bare.
//!
//! A sandboxed run's environment is the one an ACP agent gets
//! (`xenonet_acp::agent_env`): the service's without XenoNet's own
//! variables, session marks and anything named for a secret, plus the
//! sign-in the app's runs use. Its files are private, in the runs folder.

use crate::compose::runtime::Runtime;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use xenonet_adapter::delivery::{Plan, SandboxFiles};
use xenonet_engine::delivery::{apps, starts};

/// The most records kept. A record is kept as long as its session might be
/// resumed, so none goes by age: past this many, the ones longest unused go.
const MAX_KEPT: usize = 5000;

/// What a sandboxed session was started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub folder: String,
    pub domains: Vec<String>,
}

fn dir(w: &Runtime) -> PathBuf {
    Path::new(&w.setup.state_dir).join("sandboxed")
}

fn name(session: &str) -> String {
    let safe: String = session.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    format!("{safe}.json")
}

/// Keeps `session` as sandboxed, or says why it couldn't: a run whose
/// record can't be kept isn't made, since a turn for it later would run
/// without the sandbox. Keeping it again renews it, so a session in use
/// never ages out.
pub fn keep(w: &Runtime, session: &str, r: &Record) -> Result<(), String> {
    let d = dir(w);
    let text = json!({ "folder": r.folder, "domains": r.domains }).to_string();
    let file = d.join(name(session));
    let part = d.join(format!("{}.part", name(session)));
    write_private(&part, &text).and_then(|()| std::fs::rename(&part, &file)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("the sandbox of {session} couldn't be kept ({e}), and a session that can't be held to its sandbox later isn't started")
    })?;
    trim(&d, MAX_KEPT);
    Ok(())
}

fn read(w: &Runtime, session: &str) -> Option<Record> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir(w).join(name(session))).ok()?).ok()?;
    Some(Record { folder: v["folder"].as_str()?.to_string(), domains: v["domains"].as_array()?.iter().filter_map(|d| d.as_str().map(str::to_string)).collect() })
}

/// The sandbox `session` was started with, or the one of the session it is
/// a copy of.
pub fn of(w: &Runtime, session: &str) -> Option<Record> {
    read(w, session).or_else(|| crate::compose::runtime::starts::fork_root(w, session).and_then(|root| read(w, &root)))
}

/// A copy named by a turn forked from a sandboxed session is kept as
/// sandboxed in its own right, so it stays so after a restart forgets whose
/// copy it is.
pub fn fork_named(w: &Runtime, child: &str, parent: &Record) -> Result<(), String> {
    keep(w, child, parent)
}

/// The sandbox a turn for `session` runs in, with its record renewed; or
/// why no turn is run for it. None is for a session XenoNet never started
/// sandboxed. A session XenoNet started for anyone but the person (under a
/// delegation or for an intent, the store says) with no readable record is
/// refused, never run bare, and so is one whose record can't be read.
pub fn for_turn(w: &Runtime, session: &str) -> Result<Option<Record>, String> {
    let root = crate::compose::runtime::starts::fork_root(w, session);
    let stored = |s: &str| dir(w).join(name(s)).exists();
    if let Some(rec) = of(w, session) {
        keep(w, session, &rec).map_err(|e| format!("that session's sandbox couldn't be renewed, so no turn is run for it: {e}"))?;
        return Ok(Some(rec));
    }
    if stored(session) || root.as_deref().is_some_and(stored) {
        return Err("that session was started sandboxed and its sandbox can't be read, so no turn is run for it".into());
    }
    let for_others = |s: &str| crate::compose::runtime::starts::delegated(w, s).is_some();
    if for_others(session) || root.as_deref().is_some_and(for_others) {
        return Err("that session was started for someone else, in a sandbox XenoNet has no record of now, so no turn is run for it".into());
    }
    Ok(None)
}

/// Lets go of the records past `max`, the longest unused first.
fn trim(d: &Path, max: usize) {
    let Ok(entries) = std::fs::read_dir(d) else { return };
    let mut all: Vec<(std::time::SystemTime, PathBuf)> = entries.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if all.len() <= max {
        return;
    }
    all.sort();
    for (_, f) in &all[..all.len() - max] {
        let _ = std::fs::remove_file(f);
    }
}

fn write_private(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut o = std::fs::OpenOptions::new();
    o.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    std::io::Write::write_all(&mut o.open(file)?, text.as_bytes())
}

/// The arguments of a turn of `app` in a sandboxed run `run`, made to run
/// in its sandbox, with the files it is given written; or why it can't be.
/// The app's words of what the run may reach come with them.
pub fn turn(w: &Runtime, app: &str, rec: &Record, run: &str, args: Vec<String>) -> Result<(Vec<String>, String), String> {
    let ways = apps::of(app).and_then(|x| x.sandbox).ok_or_else(|| format!("{app} has no sandbox XenoNet can drive"))?;
    let port = w.setup.port.to_string();
    let bin = crate::compose::connect::bin(&w.setup);
    let plan = Plan { folder: &rec.folder, home: &w.setup.home, domains: &rec.domains, port: &port, bin: &bin };
    if let Some(why) = (ways.unavailable)(&plan, cfg!(windows)) {
        return Err(why);
    }
    let runs = Path::new(&w.setup.state_dir).join("runs");
    let (settings, mcp) = (runs.join(format!("{run}.settings.json")), runs.join(format!("{run}.mcp.json")));
    if let Some(files) = ways.files {
        let listen =
            w.router.resume.as_ref().and_then(|r| r.settings.as_deref()).and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok());
        let server = xenonet_engine::delivery::setup::mcp_server_name(f64::from(w.setup.port));
        let (s, m) = files(&plan, listen.as_ref(), &server);
        let pretty = |v: &Value| format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_default());
        write_private(&settings, &pretty(&s)).and_then(|()| write_private(&mcp, &pretty(&m))).map_err(|e| format!("the sandbox's settings couldn't be written: {e}"))?;
        sweep_runs(&runs, 2.0 * 24.0 * 3600e3);
    }
    let (settings, mcp) = (settings.to_string_lossy().into_owned(), mcp.to_string_lossy().into_owned());
    let args = starts::sandboxed(app, &SandboxFiles { plan, settings: &settings, mcp: &mcp }, args)?;
    Ok((args, (ways.words)(&plan)))
}

/// Lets go of the run files older than `older_than_ms`: a run's settings
/// and MCP configuration, which its turn has long read.
fn sweep_runs(d: &Path, older_than_ms: f64) {
    let Ok(entries) = std::fs::read_dir(d) else { return };
    for e in entries.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_millis() as f64 > older_than_ms);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// What a sandboxed run's environment holds: the service's, scrubbed as an
/// ACP agent's is. The caller adds the app's sign-in and tool timeout after
/// this (`starts::signed_env`), and nothing else.
pub fn env() -> Vec<(String, String)> {
    xenonet_acp::agent_env(xenonet_engine::delivery::resume::resume_env(std::env::vars().collect()), &[])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_sessions_name_is_a_safe_file_name() {
        assert_eq!(name("claude:0190d3e0-0000"), "claude_0190d3e0-0000.json");
        assert_eq!(name("../../x"), ".._.._x.json");
    }

    #[test]
    fn past_the_most_kept_the_longest_unused_records_go_and_none_goes_by_age() {
        let d = std::env::temp_dir().join(format!("xenonet-sandbox-trim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let t = std::time::SystemTime::now();
        for (n, ago) in [(1, 400u64), (2, 300), (3, 200), (4, 100)] {
            let f = d.join(format!("{n}.json"));
            std::fs::write(&f, "{}").unwrap();
            std::fs::File::options().write(true).open(&f).unwrap().set_modified(t - Duration::from_secs(ago * 24 * 3600)).unwrap();
        }
        trim(&d, 4);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 4, "a record a year old is kept while there is room");
        trim(&d, 2);
        let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["3.json", "4.json"]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_environment_holds_no_secret_and_no_session_mark() {
        std::env::set_var("SANDBOX_UNIT_API_KEY", "x");
        std::env::set_var("CLAUDECODE", "1");
        let names: Vec<String> = env().into_iter().map(|(k, _)| k).collect();
        assert!(!names.iter().any(|n| n == "SANDBOX_UNIT_API_KEY" || n == "CLAUDECODE" || n.starts_with("XENONET_")), "{names:?}");
    }
}
