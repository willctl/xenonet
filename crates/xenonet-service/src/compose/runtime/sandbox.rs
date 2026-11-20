//! Sandboxed runs: a run held to its app's own sandbox (Claude Code's,
//! Codex's), which XenoNet drives and doesn't replace. What the app can do
//! is its adapter's (`SandboxWays`); here are the files a run is given, the
//! environment it runs with, and what is kept of a sandboxed session so a
//! turn resumed for it later, or a copy forked from it, is sandboxed too:
//! a record is written before the run starts (no record, no run), renewed
//! by every turn, written for a copy when its stream names it, and not
//! swept by age; a session started for anyone but the person that has none
//! is refused, never run bare. That session is also marked in the store
//! ("confined: vendor, for whom") before its run, for as long as it exists,
//! so deleting its record file, or a store that can't be read, refuses its
//! turns rather than running them bare.
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
    /// Who the run was started for when it isn't the person's own ("a
    /// session on contractor-vm", "an intent"): the store keeps it as a
    /// mark that outlives the record's file.
    pub for_others: Option<String>,
}

/// The store's key for a confined session's mark, in its settings.
fn mark_key(session: &str) -> String {
    format!("confined:{session}")
}

/// Marks `session` in the store as confined at the vendor level, for `who`.
/// A mark is never taken back: the session may be resumed as long as it
/// exists.
fn mark(w: &Runtime, session: &str, who: &str) -> Result<(), String> {
    let v = json!({ "level": "vendor", "for": who, "at": xenonet_net::node::now_ms() });
    w.store
        .0
        .db()
        .set_setting(&mark_key(session), Some(&v))
        .map_err(|e| format!("the sandbox of {session} couldn't be kept in the store ({}), and a session that can't be held to its sandbox later isn't started", e.message))
}

/// Who `session` was started for, if the store marks it as confined for
/// anyone but the person; Err when the store can't say.
pub fn confined_for(w: &Runtime, session: &str) -> Result<Option<String>, String> {
    let v = w.store.0.db().setting(&mark_key(session)).map_err(|e| e.message)?;
    Ok((!v.is_null()).then(|| v["for"].as_str().unwrap_or("someone else").to_string()))
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
    // The store's mark first: no mark, no record, no run.
    if let Some(who) = &r.for_others {
        mark(w, session, who)?;
    }
    let d = dir(w);
    let text = json!({ "folder": r.folder, "domains": r.domains, "for": r.for_others }).to_string();
    let file = d.join(name(session));
    let part = d.join(format!("{}.part", name(session)));
    write_private(&part, &text).and_then(|()| std::fs::rename(&part, &file)).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("the sandbox of {session} couldn't be kept ({e}), and a session that can't be held to its sandbox later isn't started")
    })?;
    trim(&d, MAX_KEPT);
    Ok(())
}

/// Marks, once, each record kept before the store marked who a session was
/// for: such a record has no `for` field at all (a person's own says `null`).
/// It is read as a start that wasn't the person's, the narrowest reading, so
/// a session that has one can never be resumed bare; the record is then
/// rewritten with the field, so this is done once. How many were marked.
pub fn backfill_marks(w: &Runtime) -> usize {
    let d = dir(w);
    let Ok(entries) = std::fs::read_dir(&d) else { return 0 };
    let mut done = 0;
    for e in entries.flatten() {
        let file = e.path();
        let Some(stem) = file.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".json")).map(str::to_string) else { continue };
        let Some(v) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
        if v.get("for").is_some() {
            continue;
        }
        // A file is named for its session with ':' made '_' (`name`).
        let Some(session) = stem.split_once('_').map(|(app, id)| format!("{app}:{id}")).filter(|s| name(s) == format!("{stem}.json")) else { continue };
        let who = "someone else (the sandbox was kept before XenoNet marked who a session was for)";
        let record = Record {
            folder: v["folder"].as_str().unwrap_or_default().to_string(),
            domains: v["domains"].as_array().map(|a| a.iter().filter_map(|d| d.as_str().map(str::to_string)).collect()).unwrap_or_default(),
            for_others: Some(who.to_string()),
        };
        match keep(w, &session, &record) {
            Ok(()) => done += 1,
            Err(why) => w.log("warn", "sandbox.backfill", &format!("{session}'s sandbox couldn't be marked yet: {why}")),
        }
    }
    done
}

fn read(w: &Runtime, session: &str) -> Option<Record> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(dir(w).join(name(session))).ok()?).ok()?;
    Some(Record {
        folder: v["folder"].as_str()?.to_string(),
        domains: v["domains"].as_array()?.iter().filter_map(|d| d.as_str().map(str::to_string)).collect(),
        for_others: v["for"].as_str().map(str::to_string),
    })
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
/// confined. A session XenoNet started for anyone but the person (the store
/// marks it, and keeps the mark as long as the session exists) with no
/// readable record is refused, never run bare, and so is one whose record
/// or mark can't be read.
pub fn for_turn(w: &Runtime, session: &str) -> Result<Option<Record>, String> {
    let root = crate::compose::runtime::starts::fork_root(w, session);
    let stored = |s: &str| dir(w).join(name(s)).exists();
    let rec = of(w, session);
    // The sessions whose marks count: this one, and the one it is a copy of.
    let marks: Vec<Result<Option<String>, String>> = std::iter::once(session).chain(root.as_deref()).map(|s| confined_for(w, s)).collect();
    let delegated = std::iter::once(session).chain(root.as_deref()).any(|s| crate::compose::runtime::starts::delegated(w, s).is_some());
    let record_file = stored(session) || root.as_deref().is_some_and(stored);
    let rec = verdict(rec, record_file, &marks, delegated)?;
    if let Some(rec) = &rec {
        keep(w, session, rec).map_err(|e| format!("that session's sandbox couldn't be renewed, so no turn is run for it: {e}"))?;
    }
    Ok(rec)
}

/// What a turn may do given what is known of its session: its record (if
/// one reads), whether a record file is there at all, the store's marks
/// (this session's and its original's) and whether it was started under a
/// delegation. A read that failed is a refusal, never "not confined".
fn verdict(rec: Option<Record>, record_file: bool, marks: &[Result<Option<String>, String>], delegated: bool) -> Result<Option<Record>, String> {
    if let Some(e) = marks.iter().find_map(|m| m.as_ref().err()) {
        return Err(format!("whether that session was started confined can't be read ({e}), so no turn is run for it"));
    }
    if let Some(rec) = rec {
        return Ok(Some(rec));
    }
    if record_file {
        return Err("that session was started sandboxed and its sandbox can't be read, so no turn is run for it".into());
    }
    if let Some(who) = marks.iter().flatten().flatten().next() {
        return Err(format!("that session was started for {who}, in a sandbox XenoNet has no record of now, so no turn is run for it"));
    }
    if delegated {
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
    // Claude Code loads the folder's own skills, agents and commands
    // whatever settings sources it is given, and their hooks run outside its
    // sandbox: looked for on every turn, since the folder may have changed
    // since the last.
    if app == "claude" {
        if let Some(why) = starts::project_hooks(Path::new(&rec.folder)) {
            return Err(why);
        }
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
    let server = xenonet_engine::delivery::setup::mcp_server_name(f64::from(w.setup.port));
    let args = starts::sandboxed(app, &SandboxFiles { plan, settings: &settings, mcp: &mcp, server: &server, not_own: rec.for_others.is_some() }, args)?;
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
pub fn env(w: &Runtime) -> Vec<(String, String)> {
    env_from(w.starts.environment())
}

/// A sandboxed run's environment from `vars`: without XenoNet's own
/// variables, session marks and anything named for a secret, and without
/// the ones that point at a Unix socket (`SSH_AUTH_SOCK`, `DOCKER_HOST`,
/// any name ending in `_SOCK` or `_SOCKET`), which a command in the sandbox
/// could use to reach the agent or the daemon outside it where the
/// sandbox's own filter against Unix sockets isn't there.
pub fn env_from(vars: Vec<(String, String)>) -> Vec<(String, String)> {
    let socket = |k: &str| {
        let up = k.to_ascii_uppercase();
        up == "DOCKER_HOST" || up == "CONTAINER_HOST" || up.ends_with("_SOCK") || up.ends_with("_SOCKET")
    };
    xenonet_acp::agent_env(xenonet_engine::delivery::resume::resume_env(vars), &[]).into_iter().filter(|(k, _)| !socket(k)).collect()
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
    fn a_session_marked_confined_is_refused_without_its_record_and_when_the_store_cant_say() {
        let rec = Record { folder: "/w".into(), domains: Vec::new(), for_others: Some("a session on contractor-vm".into()) };
        let marked = Ok(Some("a session on contractor-vm".to_string()));
        let unmarked: Result<Option<String>, String> = Ok(None);
        // Positive control: with its record, it runs confined.
        assert_eq!(verdict(Some(rec.clone()), true, std::slice::from_ref(&marked), false), Ok(Some(rec.clone())));
        // Without its record, the mark refuses it, for the session or the one it is a copy of.
        let why = verdict(None, false, std::slice::from_ref(&marked), false).unwrap_err();
        assert!(why.contains("started for a session on contractor-vm"), "{why}");
        assert!(verdict(None, false, &[unmarked.clone(), marked], false).is_err());
        // A store that can't be read refuses, with or without a record, never "not confined".
        let broken: Result<Option<String>, String> = Err("the database is locked".into());
        for have in [Some(rec), None] {
            let why = verdict(have, false, std::slice::from_ref(&broken), false).unwrap_err();
            assert!(why.contains("can't be read") && why.contains("the database is locked"), "{why}");
        }
        // A record file that doesn't read, and an old delegation row, still refuse.
        assert!(verdict(None, true, std::slice::from_ref(&unmarked), false).is_err());
        assert!(verdict(None, false, std::slice::from_ref(&unmarked), true).is_err());
        // Nothing known of it: not confined, as before.
        assert_eq!(verdict(None, false, &[unmarked], false), Ok(None));
    }

    #[test]
    fn the_environment_holds_no_secret_no_session_mark_and_no_socket() {
        xenonet_platforms::install();
        let vars = |names: &[&str]| names.iter().map(|n| ((*n).to_string(), "x".to_string())).collect::<Vec<_>>();
        let given = vars(&[
            "PATH",
            "HOME",
            // The sign-in the app's runs use, which the service's own environment may hold:
            "CLAUDE_CODE_OAUTH_TOKEN",
            "ANTHROPIC_API_KEY",
            "CLAUDECODE",
            "XENONET_PORT",
            "SSH_AUTH_SOCK",
            "DOCKER_HOST",
            "GPG_AGENT_SOCK",
            "CLAUDE_CODE_MESSAGING_SOCKET",
        ]);
        let names: Vec<String> = env_from(given).into_iter().map(|(k, _)| k).collect();
        assert_eq!(names, ["PATH", "HOME"], "{names:?}");
    }
}
