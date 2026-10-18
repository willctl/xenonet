//! Hermes Agent's observer (core/actors/hermes.js;
//! docs/design/actors/hermes.md): observed and attributed only, with no
//! installs and no change to Hermes's settings. One actor per profile:
//! actor:hermes for the default one, `actor:hermes-<profile>` for each named
//! one.
//!
//! What it reads (hermes_files, metadata only): Hermes's processes in the
//! process table, its spawn ledger, each profile's gateway state,
//! scheduler ticker, background processes, scheduled jobs and session
//! database.
//!
//! Presence, by section 1's table for a local agent:
//!
//!   working   a process that hosts the agent runs, and a session was
//!             active in the last two minutes, or its gateway, with a fresh
//!             heartbeat, has agents at work
//!   idle      one runs, with nothing recent; or only Hermes.app runs
//!   away      its gateway's heartbeat is over 120 s old while its process
//!             runs, and no session of it is active
//!   offline   no process of it runs
//!   unknown   its session database can't be read and nothing of it runs
//!
//! Its status is XenoNet's own words: what its busiest session's activity
//! says it's doing, matched to Hermes's templates and never shown raw, with
//! the project; or its gateway's state.
//!
//! Events, each from facts: ran and edited, messaged, finished and failed,
//! and spawned: the Claude Code and Codex sessions it started, by process
//! ancestry, a Codex thread whose originator is "hermes", or one in a
//! scratch worktree of its codex skill.
//!
//! The look calls the observer's own fields and methods, as the JS did:
//! "actors", "files", "procs" (through __census), keep, say, touch, gather,
//! happened, and the readers' "vendor", "dirs", "dateParse" and "db".

use crate::attribution::{self, Census, Paths, SpawnsState};
use crate::grok_bot::spawned;
use crate::hermes_files::{self as hf, ms_of, parse_date, same_start};
use crate::muse::census;
use crate::names::hermes_slug;
use crate::registry::value_arg;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_base::paths::{is_under, path_key, tilde};
use xenonet_base::sync::Locked;
use xenonet_engine::js::{template, truthy};
use xenonet_engine::jsre::js;
use xenonet_engine::peer::{list, Calls, Result};
use xenonet_engine::sql::Arg;

/// Every Hermes heartbeat (the gateway's, the ticker's, a backend's)
/// comes every 60 s.
pub const HERMES_HEARTBEAT_MS: f64 = 60e3;
const ACTIVE_MS: f64 = 2.0 * 60e3;
const STALE_MS: f64 = 120e3;
const TICKER_STALE_MS: f64 = 150e3;
// A first read of a session database looks this far back; later reads take
// rows newer than this as new work, and older ones as history.
const REPLAY_MS: f64 = 10.0 * 60e3;
const HISTORY_MS: f64 = 3600e3;
const DAY_MS: f64 = 24.0 * 3600e3;
const PROFILES_MS: f64 = 60e3;

fn running_gateway(s: &Value) -> bool {
    matches!(s.as_str(), Some("starting" | "running" | "degraded" | "draining"))
}

// The purposes in the ledger whose process hosts the agent.
fn host_purpose(p: &Value) -> bool {
    matches!(p.as_str(), Some("cli" | "serve" | "dashboard" | "gateway"))
}

// What Hermes's tools are, as XenoNet counts them.
fn tool_kind(t: &str) -> Option<&'static str> {
    matches!(t, "terminal" | "process_manage" | "execute_code").then_some("ran")
}

// Messaging sources of Hermes's sessions, as the platforms an event names.
fn platform(source: &str) -> Option<&'static str> {
    Some(match source {
        "telegram" => "telegram",
        "discord" => "discord",
        "slack" => "slack",
        "whatsapp" => "whatsapp",
        "signal" => "signal",
        "matrix" => "matrix",
        "mattermost" => "mattermost",
        "email" => "email",
        "sms" => "sms",
        "imessage" | "bluebubbles" => "imessage",
        "teams" => "teams",
        _ => return None,
    })
}

/// What a tool does, in XenoNet's words, by its name.
pub fn tool_words(tool: &str) -> Option<&'static str> {
    if tool_kind(tool).is_some() {
        return Some("Running a command");
    }
    Some(match tool {
        "write_file" | "patch" => "Editing files",
        "read_file" | "search_files" => "Reading files",
        t if t.starts_with("browser_") || t.starts_with("web_") => "Browsing the web",
        "delegate_task" => "Working with a subagent",
        "cronjob_manage" => "Scheduling a job",
        "computer_use" => "Using the computer",
        _ => return None,
    })
}

/// What a session's activity stamp says it's doing, in XenoNet's words, or
/// None. The stamp itself is never shown: some templates carry free text.
pub fn doing_of(d: &str) -> Option<&'static str> {
    let tool = js(r"^(?:executing tool|tool completed|sequential tool running \([^)]*\)):\s*([a-z_][a-z0-9_]*)", true)
        .captures(d)
        .or_else(|| js(r"^executing \d+ tools concurrently:\s*([a-z_][a-z0-9_]*)", true).captures(d))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    if let Some(t) = tool {
        return tool_words(&t);
    }
    if js(r"^(waiting for (stream|provider|non-streaming)|receiving stream|local model loading|stream retry|stale stream)", true).is_match(d) {
        return Some("Thinking");
    }
    if js(r"^(context )?compress|compaction", true).is_match(d) {
        return Some("Tidying its context");
    }
    None
}

/// How a session ended, as an event's kind: failed when its reason says
/// so, nothing for one that went on in another (compaction), else finished.
pub fn ending_of(r: &str) -> Option<&'static str> {
    if js(r"compress|compact", true).is_match(r) {
        return None;
    }
    if js(r"error|fail|crash|unreachable|overflow|timed?_?out|out_of_credits|filter", true).is_match(r) {
        return Some("failed");
    }
    Some("finished")
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => template(Some(v)),
    }
}

// A file read again only when it changes: its mtime and size, or why it
// can't be had.
fn stamp_of(c: &mut dyn Calls, paths: &[String]) -> String {
    let mut out = Vec::new();
    for p in paths {
        out.push(match c.call("files", "statSync", vec![Arg::text(p)]) {
            Ok(st) => format!("{}:{}", xenonet_engine::store::js_text(&st.field("mtimeMs")), xenonet_engine::store::js_text(&st.field("size"))),
            Err(e) => e.code.as_ref().map_or_else(|| "?".into(), |c| template(Some(c))),
        });
    }
    out.join("|")
}

fn join(base: &str, parts: &[&str]) -> String {
    parts.iter().fold(base.to_string(), |a, p| xenonet_base::paths::join(&a, p))
}

fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        let b = p.as_bytes();
        p.starts_with('\\') || p.starts_with('/') || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
    } else {
        p.starts_with('/')
    }
}

#[derive(Clone, Debug, Default)]
struct Db {
    stamp: Option<String>,
    last_id: Value,
    read: bool,
    heartbeats: Option<Value>,
    state: Option<String>,
}

#[derive(Clone, Debug)]
struct Session {
    id: String,
    source: Value,
    cwd: Value,
    repo: Value,
    parent: Value,
    started_at: Option<f64>,
    ended_at: Option<f64>,
    end_reason: Value,
    activity_at: Option<f64>,
    activity: Value,
    archived: bool,
    hidden: bool,
    last_message_at: Option<f64>,
}

#[derive(Clone, Debug)]
struct Profile {
    name: String,
    id: String,
    home: String,
    title: String,
    db: Db,
    sessions: Vec<(String, Session)>,
    last_tool: Vec<(String, String)>,
}

#[derive(Default)]
struct State {
    cache: Vec<(String, String, Value)>,
    profiles: Vec<Profile>,
    profiles_at: Option<f64>,
    profile_list: Vec<String>,
    source_states: Vec<(String, Value)>,
    recognized: Value,
    table: Option<Census>,
}

pub struct Hermes {
    state: Mutex<State>,
    spawns: Arc<SpawnsState>,
    root: String,
    home: String,
}

fn paths() -> Paths {
    Paths { store: "spawns.store".into(), map: "spawns.map".into(), procs: "spawns.procs".into() }
}

fn n(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

/// What a profile's files say, as read for a look.
struct Read {
    gateway: Value,
    gateway_alive: bool,
    ticker: Value,
    ticker_alive: bool,
    processes: Vec<Value>,
    jobs: Vec<Value>,
    fresh: Option<Value>,
    heartbeats: Vec<Value>,
    db_state: String,
}

impl Hermes {
    pub fn new(spawns: Arc<SpawnsState>, root: &str, home: &str) -> Self {
        Hermes { state: Mutex::new(State { recognized: Value::Null, ..Default::default() }), spawns, root: root.into(), home: home.into() }
    }

    fn s(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.locked()
    }

    // A source read again only when its files change.
    fn stamped(&self, c: &mut dyn Calls, key: &str, paths: &[String], read: &mut dyn FnMut(&mut dyn Calls) -> Result<Value>) -> Result<Value> {
        let stamp = stamp_of(c, paths);
        if let Some((_, s, v)) = self.s().cache.iter().find(|(k, _, _)| k == key) {
            if *s == stamp {
                return Ok(v.clone());
            }
        }
        let value = read(c)?;
        let mut st = self.s();
        match st.cache.iter_mut().find(|(k, _, _)| k == key) {
            Some(slot) => *slot = (key.into(), stamp, value.clone()),
            None => st.cache.push((key.into(), stamp, value.clone())),
        }
        Ok(value)
    }

    fn source(&self, name: &str, file: &str, state: &Value) {
        let v = json!({ "name": name, "file": tilde(file, &self.home), "state": state });
        let mut s = self.s();
        match s.source_states.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = v,
            None => s.source_states.push((name.into(), v)),
        }
    }

    // A profile's actor: its id and name, and what XenoNet keeps of it
    // between looks.
    fn profile(&self, name: &str) -> Profile {
        let mut s = self.s();
        if let Some(p) = s.profiles.iter().find(|p| p.name == name) {
            return p.clone();
        }
        let slug = hermes_slug(Some(&Value::String(name.into())));
        let title = if name == "default" {
            "Hermes".to_string()
        } else if js(r"^[A-Za-z0-9][\w .-]{0,31}$", false).is_match(name) {
            format!("Hermes ({name})")
        } else {
            "Hermes (a profile)".into()
        };
        let p = Profile {
            name: name.into(),
            id: format!("actor:{slug}"),
            home: if name == "default" { self.root.clone() } else { join(&self.root, &["profiles", name]) },
            title,
            db: Db { last_id: Value::Null, ..Default::default() },
            sessions: Vec::new(),
            last_tool: Vec::new(),
        };
        s.profiles.push(p.clone());
        p
    }

    fn put(&self, p: &Profile) {
        let mut s = self.s();
        if let Some(slot) = s.profiles.iter_mut().find(|x| x.name == p.name) {
            *slot = p.clone();
        }
    }

    // The profiles there are: the folders Hermes made, and any a running
    // process of the ledger names.
    fn profiles_now(&self, c: &mut dyn Calls, at: f64, ledger: &[Value]) -> Result<Vec<Profile>> {
        if at - self.s().profiles_at.unwrap_or(f64::NEG_INFINITY) >= PROFILES_MS {
            let list = hf::list_profiles(c, &self.root)?;
            let mut s = self.s();
            s.profiles_at = Some(at);
            s.profile_list = list.as_array().into_iter().flatten().map(|p| text_of(p.get("name"))).collect();
        }
        let mut names = self.s().profile_list.clone();
        // Only a plain name, so a profile's folder stays inside Hermes's.
        for e in ledger {
            let profile = e.get("profile").filter(|p| truthy(Some(p)));
            if profile.is_none_or(|p| js(r"^[A-Za-z0-9][\w.-]{0,63}$", false).is_match(&template(Some(p)))) {
                let name = profile.map_or_else(|| "default".to_string(), |p| template(Some(p)));
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        Ok(names.iter().map(|n| self.profile(n)).collect())
    }

    // Whether a process Hermes recorded still runs, from the last census.
    fn alive(&self, pid: Option<&Value>, start: Option<&Value>) -> bool {
        let s = self.s();
        let Some(pid) = pid.and_then(Value::as_f64).filter(|p| *p != 0.0) else { return false };
        let Some(row) = s.table.as_ref().and_then(|t| t.get(pid)) else { return false };
        start.filter(|s| !s.is_null()).is_none_or(|s| same_start(s, &xenonet_base::json::number(row.start), 2000.0))
    }

    // Where a session works, as a place on the map: its repo's root, else
    // its folder.
    fn place_of_session(c: &mut dyn Calls, s: Option<&Session>) -> Result<Value> {
        let Some(s) = s else { return Ok(Value::Null) };
        for dir in [&s.repo, &s.cwd] {
            if let Some(d) = dir.as_str().filter(|d| !d.is_empty() && is_absolute(d)) {
                let place = c.call("actors", "placeOf", vec![Arg::text(d)])?.to_json();
                if truthy(Some(&place)) {
                    return Ok(place);
                }
            }
        }
        Ok(Value::Null)
    }

    // A path from a tool call, made absolute: Hermes takes them absolute,
    // relative to the session's folder, or under ~.
    fn absolute(&self, p: &Value, s: Option<&Session>) -> Option<String> {
        let p = p.as_str().filter(|p| !p.is_empty() && !p.contains('\0'))?;
        if p == "~" || p.starts_with("~/") {
            return Some(xenonet_base::paths::join(&self.home, &p[1..]));
        }
        if is_absolute(p) {
            return Some(xenonet_engine::js::join_style(p, &[]));
        }
        let cwd = s.and_then(|s| s.cwd.as_str()).filter(|c| is_absolute(c))?;
        let joined = xenonet_engine::js::join_style(cwd, &[p]);
        Some(if joined.len() > 1 { joined.trim_end_matches(['/', '\\']).to_string() } else { joined })
    }

    fn exists(&self, c: &mut dyn Calls, p: &str) -> Result<bool> {
        let dir = std::path::Path::new(p).parent().map_or_else(|| ".".into(), |d| d.to_string_lossy().into_owned());
        Ok(self.stamped(c, &format!("exists:{p}"), &[dir], &mut |c| Ok(json!(c.call("files", "existsSync", vec![Arg::text(p)])?.truthy())))?.as_bool().unwrap_or(false))
    }

    // Whether Hermes's computer-use driver is installed among its tools.
    fn cua_driver(&self, c: &mut dyn Calls) -> Result<bool> {
        let dir = join(&self.root, &["tools"]);
        Ok(self
            .stamped(c, "cua-driver", std::slice::from_ref(&dir), &mut |c| {
                Ok(json!(match c.call("files", "readdirSync", vec![Arg::text(&dir)]) {
                    Ok(names) => list(names).iter().any(|n| n.as_str().is_some_and(|n| n.starts_with("cua-driver"))),
                    Err(_) => false,
                }))
            })?
            .as_bool()
            .unwrap_or(false))
    }

    // A profile's files, each read again only when it changes, and what's
    // new in its session database.
    fn read_profile(&self, c: &mut dyn Calls, p: &mut Profile, version: Option<&str>, at: f64) -> Result<Read> {
        let f = |parts: &[&str]| join(&p.home, parts);
        let home = p.home.clone();
        let v = version.map(str::to_string);
        let gateway = self.stamped(c, &format!("{}:gateway", p.name), &[f(&["gateway_state.json"])], &mut |c| hf::read_gateway(c, &home, v.clone()))?;
        let ticker = self
            .stamped(c, &format!("{}:ticker", p.name), &[f(&["cron", "ticker_heartbeat"]), f(&["cron", "ticker_last_success"])], &mut |c| hf::read_ticker(c, &home, v.clone()))?;
        let processes = self.stamped(c, &format!("{}:processes", p.name), &[f(&["processes.json"])], &mut |c| hf::read_processes(c, &home, v.clone()))?;
        let jobs = self.stamped(c, &format!("{}:jobs", p.name), &[f(&["cron", "jobs.json"])], &mut |c| hf::read_jobs(c, &home, v.clone()))?;
        let label = if p.name == "default" { String::new() } else { format!(" ({})", p.name) };
        self.source(&format!("gateway's state{label}"), &f(&["gateway_state.json"]), &gateway["state"]);
        self.source(&format!("scheduler's ticker{label}"), &f(&["cron", "ticker_heartbeat"]), &ticker["state"]);
        self.source(&format!("background processes{label}"), &f(&["processes.json"]), &processes["state"]);
        self.source(&format!("scheduled jobs{label}"), &f(&["cron", "jobs.json"]), &jobs["state"]);
        // The session database, when it or its log changed.
        let db_file = f(&["state.db"]);
        let stamp = stamp_of(c, &[db_file.clone(), format!("{db_file}-wal")]);
        let mut fresh = None;
        if Some(&stamp) != p.db.stamp.as_ref() {
            let since = at - if p.db.read { HISTORY_MS } else { REPLAY_MS };
            let got = hf::read_state_db(c, &db_file, &p.db.last_id, &xenonet_base::json::number(since), &[])?;
            let state = got["state"].as_str().unwrap_or("").to_string();
            // A database that's busy just now is read again next time.
            if state != "busy" {
                p.db.stamp = Some(stamp);
            }
            if state == "ok" {
                p.db.last_id = got["lastId"].clone();
                p.db.read = true;
                p.db.heartbeats = Some(got["heartbeats"].clone());
            }
            p.db.state = Some(if state == "busy" { p.db.state.clone().unwrap_or_else(|| "ok".into()) } else { state });
            fresh = Some(got);
        }
        self.source(&format!("session database{label}"), &db_file, &json!(p.db.state.clone().unwrap_or_else(|| "missing".into())));
        let g = gateway["gateway"].clone();
        let t = ticker["ticker"].clone();
        let gateway_alive = g.is_object() && running_gateway(&g["state"]) && self.alive(g.get("pid"), g.get("startTime"));
        let ticker_alive = t.is_object() && self.alive(t.get("pid"), None) && at - n(t.get("at")).unwrap_or(0.0) < TICKER_STALE_MS;
        Ok(Read {
            gateway: g,
            gateway_alive,
            ticker: t,
            ticker_alive,
            processes: processes["entries"].as_array().cloned().unwrap_or_default(),
            jobs: jobs["jobs"].as_array().cloned().unwrap_or_default(),
            fresh,
            heartbeats: p.db.heartbeats.as_ref().and_then(Value::as_array).cloned().unwrap_or_default(),
            db_state: p.db.state.clone().unwrap_or_else(|| "missing".into()),
        })
    }

    pub fn look(&self, c: &mut dyn Calls, at: f64) -> Result<()> {
        let mut known = 0;
        for id in list(c.call("actors", "ofProduct", vec![Arg::text("hermes-agent")])?) {
            let info = c.call("actors", "info", vec![id])?;
            if !list(info.field("via")).iter().any(|v| v.as_str() == Some("demo")) {
                known += 1;
            }
        }
        let is_installed = hf::installed(c, &self.root)?;
        if !is_installed && known == 0 {
            return Ok(());
        }
        let mut table_state = None;
        let census = census(c, &mut table_state)?;
        let role = |i: usize| {
            census.products[i]
                .get("product")
                .and_then(Value::as_str)
                .filter(|p| *p == "hermes-agent")
                .and_then(|_| census.products[i].get("role").and_then(Value::as_str))
                .map(str::to_string)
        };
        let (mut hosts, mut tuis, mut apps) = (Vec::new(), Vec::new(), Vec::new());
        for i in 0..census.table.len() {
            match role(i).as_deref() {
                Some("host") => hosts.push(i),
                Some("tui") => tuis.push(i),
                Some("app") => apps.push(i),
                _ => {}
            }
        }
        let role_of_pid = |pid: f64| census.table.iter().rposition(|p| p.pid == pid).and_then(role);
        let stamp_file = join(&self.root, &["hermes-agent", "install-stamp.json"]);
        let root = self.root.clone();
        let stamp = self.stamped(c, "stamp", std::slice::from_ref(&stamp_file), &mut |c| hf::read_stamp(c, &root))?;
        self.source("build stamp", &stamp_file, &stamp["state"]);
        let version = stamp["version"].as_str().map(str::to_string);
        let ledger_file = join(&self.root, &["spawn-ledger.json"]);
        let v = version.clone();
        let ledger = self.stamped(c, "ledger", std::slice::from_ref(&ledger_file), &mut |c| hf::read_ledger(c, &root, v.clone()))?;
        self.source("spawn ledger", &ledger_file, &ledger["state"]);
        let live: Vec<Value> = ledger["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| {
                let Some(row) = n(e.get("pid")).and_then(|p| census.get(p)) else { return false };
                e.get("createTime").is_none_or(Value::is_null) || same_start(&e["createTime"], &xenonet_base::json::number(row.start), 2000.0)
            })
            .collect();
        // The process check's self-test: every live host the ledger lists
        // should be one the process table knows as Hermes's.
        let expected: Vec<&Value> = live.iter().filter(|e| host_purpose(&e["purpose"])).collect();
        let found = expected.iter().filter(|e| n(e.get("pid")).and_then(role_of_pid).as_deref() == Some("host")).count();
        let purposes: Vec<Value> = expected.iter().filter(|e| n(e.get("pid")).and_then(role_of_pid).as_deref() != Some("host")).map(|e| e["purpose"].clone()).collect();
        self.s().recognized = json!({ "expected": expected.len(), "found": found, "purposes": purposes });
        self.s().table = Some(Census { table: census.table.clone(), agents: census.agents.clone(), products: census.products.clone() });
        let mut profiles = self.profiles_now(c, at, &live)?;
        let mut profile_of: Vec<(f64, String)> = Vec::new();
        let set = |pid: f64, name: &str, to: &mut Vec<(f64, String)>| match to.iter_mut().find(|(p, _)| *p == pid) {
            Some(slot) => slot.1 = name.into(),
            None => to.push((pid, name.into())),
        };
        for e in &live {
            let name = e.get("profile").filter(|p| truthy(Some(p))).map_or_else(|| "default".to_string(), |p| template(Some(p)));
            if self.s().profiles.iter().any(|p| p.name == name) {
                set(n(e.get("pid")).unwrap_or(f64::NAN), &name, &mut profile_of);
            }
        }
        let mut read: Vec<(String, Read)> = Vec::new();
        for p in &mut profiles {
            let r = self.read_profile(c, p, version.as_deref(), at)?;
            self.put(p);
            if r.gateway_alive {
                set(n(r.gateway.get("pid")).unwrap_or(f64::NAN), &p.name, &mut profile_of);
            }
            if r.ticker_alive {
                set(n(r.ticker.get("pid")).unwrap_or(f64::NAN), &p.name, &mut profile_of);
            }
            for hb in &r.heartbeats {
                let mut parse = parse_date(c);
                if at - hb.get("last_heartbeat").and_then(|v| ms_of(v, &mut parse)).unwrap_or(0.0) < TICKER_STALE_MS {
                    set(n(hb.get("pid")).unwrap_or(f64::NAN), &p.name, &mut profile_of);
                }
            }
            read.push((p.name.clone(), r));
        }
        // Each process of Hermes's to its profile: by the ledger, the
        // gateway's and the ticker's records and the backends' heartbeats; a
        // terminal UI by the backend it runs; the app by the backend it
        // started.
        let profile_of_host = |pid: f64| profile_of.iter().find(|(p, _)| *p == pid).map_or_else(|| "default".to_string(), |(_, n)| n.clone());
        let mut running: Vec<(String, Vec<usize>)> = profiles.iter().map(|p| (p.name.clone(), Vec::new())).collect();
        let mut apps_of: Vec<(String, Vec<usize>)> = profiles.iter().map(|p| (p.name.clone(), Vec::new())).collect();
        let add = |map: &mut Vec<(String, Vec<usize>)>, name: String, i: usize| match map.iter_mut().find(|(n, _)| *n == name) {
            Some((_, l)) => l.push(i),
            None => map.push((name, vec![i])),
        };
        for &i in &hosts {
            add(&mut running, profile_of_host(census.table[i].pid), i);
        }
        for &t in &tuis {
            let backend = hosts.iter().copied().find(|&h| census.table[h].ppid == census.table[t].pid).unwrap_or(t);
            add(&mut running, profile_of_host(census.table[backend].pid), t);
        }
        let main_apps: Vec<usize> = apps.iter().copied().filter(|&a| !apps.iter().any(|&b| census.table[b].pid == census.table[a].ppid)).collect();
        for a in main_apps {
            let profile = live
                .iter()
                .find(|e| e["purpose"].as_str() == Some("serve") && n(e.get("spawnerPid")) == Some(census.table[a].pid))
                .and_then(|e| e.get("profile").filter(|p| truthy(Some(p))))
                .map_or_else(|| "default".to_string(), |p| template(Some(p)));
            add(&mut apps_of, profile, a);
        }
        for p in &mut profiles {
            let r = read.iter().find(|(n, _)| *n == p.name).map(|(_, r)| r).expect("each profile was read above");
            let procs = running.iter().find(|(n, _)| *n == p.name).map_or(0, |(_, l)| l.len());
            let app_count = apps_of.iter().find(|(n, _)| *n == p.name).map_or(0, |(_, l)| l.len());
            self.observe(c, p, r, procs, app_count, version.as_deref(), at, is_installed)?;
            self.put(p);
        }
        // The sessions Hermes started.
        self.attribute(c, &census, &hosts, &profile_of_host, &read, at)
    }

    // What a profile's database says, its presence and status, and the
    // events and targets they make.
    #[expect(clippy::too_many_arguments, reason = "the JS's observe and what one look found, one argument each")]
    fn observe(&self, c: &mut dyn Calls, p: &mut Profile, r: &Read, processes: usize, apps: usize, version: Option<&str>, at: f64, is_installed: bool) -> Result<()> {
        let there = processes > 0 || r.gateway_alive || r.ticker_alive;
        let fresh_ok = r.fresh.as_ref().filter(|f| f["state"].as_str() == Some("ok"));
        // A named profile is an actor once something of it has run; the
        // default one is while Hermes is installed.
        let installed_default = p.name == "default" && is_installed;
        if !there && apps == 0 && !c.call("actors", "has", vec![Arg::text(&p.id)])?.truthy() && !installed_default {
            if let Some(f) = fresh_ok {
                self.take(c, p, f, at, false)?;
            }
            return Ok(());
        }
        let mut capabilities = vec!["run-commands", "edit-repos", "browse", "schedule"];
        if self.exists(c, &join(&p.home, &["skills", "autonomous-ai-agents"]))? {
            capabilities.push("spawn-sessions");
        }
        if r.gateway.get("platforms").and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x["state"].as_str() == Some("connected"))) {
            capabilities.push("message-people");
        }
        if self.cua_driver(c)? {
            capabilities.push("use-computer");
        }
        // Installed: Hermes is, and so is the profile's folder.
        let installed = is_installed && self.s().profile_list.contains(&p.name);
        let keep = Arg::Obj(vec![
            ("name".into(), Arg::text(&p.title)),
            ("version".into(), version.map_or(Arg::Null, Arg::text)),
            ("via".into(), Arg::List(vec![Arg::text("observed"), Arg::text("attributed")])),
            ("heartbeatMs".into(), Arg::Num(HERMES_HEARTBEAT_MS)),
            ("capabilities".into(), Arg::List(capabilities.iter().map(|c| Arg::text(*c)).collect())),
            ("installed".into(), Arg::Bool(installed)),
            ("at".into(), Arg::Num(at)),
        ]);
        c.call("", "keep", vec![Arg::text(&p.id), keep])?;
        if !c.call("actors", "enabled", vec![Arg::text(&p.id)])?.truthy() {
            return Ok(());
        }
        // What the database says that's new.
        if let Some(f) = fresh_ok {
            self.take(c, p, f, at, true)?;
        }
        // The sessions at work: open, not hidden, active in the last two
        // minutes.
        let latest = |s: &Session| s.activity_at.unwrap_or(0.0).max(s.last_message_at.unwrap_or(0.0));
        let mut active: Vec<Session> =
            p.sessions.iter().map(|(_, s)| s.clone()).filter(|s| s.ended_at.is_none() && !s.archived && !s.hidden && latest(s) >= at - ACTIVE_MS).collect();
        active.sort_by(|a, b| latest(b).partial_cmp(&latest(a)).unwrap_or(std::cmp::Ordering::Equal));
        // A gateway whose heartbeat is stale while its process runs has gone
        // quiet: only its sessions' own activity says what it's doing.
        let quiet = r.gateway_alive && n(r.gateway.get("updatedAt")).is_some_and(|u| at - u > STALE_MS);
        let busy = r.gateway_alive && !quiet && n(r.gateway.get("activeAgents")).unwrap_or(0.0) > 0.0;
        let (presence, status): (&str, Value) = if there && (!active.is_empty() || busy) {
            let status = match active.first() {
                Some(top) => {
                    let doing = if top.activity_at.is_some_and(|a| a >= at - ACTIVE_MS) { doing_of(&text_of(Some(&top.activity))) } else { None };
                    let tool = p.last_tool.iter().find(|(k, _)| *k == top.id).map(|(_, t)| t.clone()).unwrap_or_default();
                    let words = doing.or_else(|| tool_words(&tool)).unwrap_or("Working");
                    let place = Self::place_of_session(c, Some(top))?;
                    let name = c.call("actors", "where", vec![value_arg(&place)])?.field("name");
                    if name.truthy() {
                        json!(format!("{words} in {}", xenonet_engine::store::js_text(&name)))
                    } else {
                        json!(words)
                    }
                }
                None => json!("Working through its gateway"),
            };
            ("working", status)
        } else if quiet {
            ("away", json!("Its gateway has gone quiet"))
        } else if there {
            let s = if r.gateway_alive {
                match r.gateway["state"].as_str() {
                    Some("starting") => json!("Its gateway is starting"),
                    Some("degraded") => json!("Its gateway is degraded"),
                    Some("draining") => json!("Its gateway is finishing its work"),
                    _ => Value::Null,
                }
            } else {
                Value::Null
            };
            ("idle", s)
        } else if apps > 0 {
            ("idle", json!("Hermes.app runs with no backend on this Mac"))
        } else if r.db_state == "denied" || r.db_state == "unreadable" {
            ("unknown", json!("XenoNet can't read its session database"))
        } else {
            ("offline", if r.gateway["state"].as_str() == Some("startup_failed") { json!("Its gateway failed to start") } else { Value::Null })
        };
        c.call("", "say", vec![Arg::text(&p.id), Arg::Obj(vec![("presence".into(), Arg::text(presence)), ("status".into(), value_arg(&status)), ("at".into(), Arg::Num(at))])])?;
        // Where it works: its sessions' places, and its jobs' folders.
        for s in &active {
            let place = Self::place_of_session(c, Some(s))?;
            c.call("", "touch", vec![Arg::text(&p.id), value_arg(&place), Arg::text("works-on"), Arg::Num(at)])?;
        }
        for j in &r.jobs {
            let workdir = j["workdir"].as_str().filter(|w| !w.is_empty() && is_absolute(w));
            if j["enabled"].as_bool() == Some(true) {
                if let Some(w) = workdir {
                    let place = c.call("actors", "placeOf", vec![Arg::text(w)])?;
                    c.call("", "touch", vec![Arg::text(&p.id), place, Arg::text("watches"), Arg::Num(at)])?;
                }
            }
            // A job that ran: finished or failed, once per run.
            let Some(last) = n(j.get("lastRunAt")) else { continue };
            if at - last > DAY_MS {
                continue;
            }
            let kind = match j["status"].as_str() {
                Some("ok") => Some("finished"),
                Some("error" | "delivery_failed" | "blocked_config") => Some("failed"),
                _ => None,
            };
            if let Some(kind) = kind {
                let target = match workdir {
                    Some(w) => c.call("actors", "placeOf", vec![Arg::text(w)])?,
                    None => Arg::Null,
                };
                let o = Arg::Obj(vec![
                    ("ref".into(), Arg::text(format!("job:{}:{}", text_of(j.get("id")), xenonet_base::json::number_string(last)))),
                    ("kind".into(), Arg::text(kind)),
                    ("target".into(), target),
                    ("subject".into(), Arg::text("job")),
                    ("at".into(), Arg::Num(last.min(at))),
                ]);
                c.call("", "happened", vec![Arg::text(&p.id), o])?;
            }
        }
        Ok(())
    }

    // New rows of a profile's database: sessions kept current, its tools
    // gathered per minute, its replies on messaging platforms, sessions that
    // ended.
    fn take(&self, c: &mut dyn Calls, p: &mut Profile, fresh: &Value, at: f64, record: bool) -> Result<()> {
        let parse = |c: &mut dyn Calls, v: Option<&Value>| -> Option<f64> {
            let mut pd = parse_date(c);
            v.and_then(|v| ms_of(v, &mut pd))
        };
        let sessions = fresh["sessions"].as_array().cloned().unwrap_or_default();
        for s in &sessions {
            let id = text_of(s.get("id"));
            let was = p.sessions.iter().find(|(k, _)| *k == id).and_then(|(_, x)| x.last_message_at);
            let opt = |k: &str| s.get(k).cloned().unwrap_or(Value::Null);
            let session = Session {
                id: id.clone(),
                source: opt("source"),
                cwd: opt("cwd"),
                repo: opt("git_repo_root"),
                parent: opt("parent_session_id"),
                started_at: parse(c, s.get("started_at")),
                ended_at: parse(c, s.get("ended_at")),
                end_reason: opt("end_reason"),
                activity_at: parse(c, s.get("last_activity_at")),
                activity: opt("last_activity_description"),
                archived: truthy(s.get("archived")),
                hidden: truthy(s.get("hidden")),
                last_message_at: was,
            };
            match p.sessions.iter_mut().find(|(k, _)| *k == id) {
                Some(slot) => slot.1 = session,
                None => p.sessions.push((id, session)),
            }
        }
        for m in fresh["messages"].as_array().cloned().unwrap_or_default() {
            let sid = text_of(m.get("session_id"));
            let Some(when) = parse(c, m.get("timestamp")).filter(|w| *w != 0.0) else { continue };
            if let Some((_, s)) = p.sessions.iter_mut().find(|(k, _)| *k == sid) {
                s.last_message_at = Some(s.last_message_at.unwrap_or(0.0).max(when));
            }
            let s = p.sessions.iter().find(|(k, _)| *k == sid).map(|(_, s)| s.clone());
            let tool = m.get("tool_name").filter(|t| truthy(Some(t))).map(|t| template(Some(t))).filter(|_| m["role"].as_str() == Some("tool"));
            if let Some(tool) = tool {
                match p.last_tool.iter_mut().find(|(k, _)| *k == sid) {
                    Some(slot) => slot.1.clone_from(&tool),
                    None => p.last_tool.push((sid.clone(), tool.clone())),
                }
                if record {
                    if let Some(kind) = tool_kind(&tool) {
                        let target = Self::place_of_session(c, s.as_ref())?;
                        c.call(
                            "",
                            "gather",
                            vec![Arg::text(&p.id), Arg::Obj(vec![("kind".into(), Arg::text(kind)), ("target".into(), value_arg(&target)), ("at".into(), Arg::Num(when))])],
                        )?;
                    }
                }
            } else if record && m["role"].as_str() == Some("assistant") && !truthy(m.get("calls")) {
                if let Some(pl) = s.as_ref().and_then(|s| s.source.as_str()).and_then(platform) {
                    c.call(
                        "",
                        "gather",
                        vec![Arg::text(&p.id), Arg::Obj(vec![("kind".into(), Arg::text("messaged")), ("platform".into(), Arg::text(pl)), ("at".into(), Arg::Num(when))])],
                    )?;
                }
            }
        }
        if !record {
            return Ok(());
        }
        // Its edits: the files, each in a place XenoNet knows, else the
        // session's place.
        for t in fresh["toolPaths"].as_array().cloned().unwrap_or_default() {
            let sid = text_of(t.get("session_id"));
            let s = p.sessions.iter().find(|(k, _)| *k == sid).map(|(_, s)| s.clone());
            let Some(when) = parse(c, t.get("timestamp")).filter(|w| *w != 0.0) else { continue };
            let file = self.absolute(t.get("path").unwrap_or(&Value::Null), s.as_ref());
            let place = match &file {
                Some(f) => c.call("actors", "placeOf", vec![Arg::text(f)])?.to_json(),
                None => Value::Null,
            };
            let target = if truthy(Some(&place)) { place.clone() } else { Self::place_of_session(c, s.as_ref())? };
            let paths = Arg::List(file.filter(|_| truthy(Some(&place))).map(Arg::text).into_iter().collect());
            c.call(
                "",
                "gather",
                vec![
                    Arg::text(&p.id),
                    Arg::Obj(vec![("kind".into(), Arg::text("edited")), ("target".into(), value_arg(&target)), ("at".into(), Arg::Num(when)), ("paths".into(), paths)]),
                ],
            )?;
        }
        // Sessions that ended (not a subagent's, nor one that went on in
        // another), once each.
        for s in &sessions {
            let id = text_of(s.get("id"));
            let Some(k) = p.sessions.iter().find(|(x, _)| *x == id).map(|(_, k)| k.clone()) else { continue };
            let Some(ended) = k.ended_at else { continue };
            if truthy(Some(&k.parent)) || k.source.as_str() == Some("subagent") || at - ended > DAY_MS {
                continue;
            }
            if let Some(kind) = ending_of(&text_of(Some(&k.end_reason))) {
                let target = Self::place_of_session(c, Some(&k))?;
                let o = Arg::Obj(vec![
                    ("ref".into(), Arg::text(format!("session:{}:end", k.id))),
                    ("kind".into(), Arg::text(kind)),
                    ("target".into(), value_arg(&target)),
                    ("subject".into(), Arg::text("session")),
                    ("at".into(), Arg::Num(ended.min(at))),
                ]);
                c.call("", "happened", vec![Arg::text(&p.id), o])?;
            }
        }
        // Sessions long over are let go.
        let gone: Vec<String> = p
            .sessions
            .iter()
            .filter(|(_, s)| s.ended_at.unwrap_or(0.0).max(s.activity_at.unwrap_or(0.0)).max(s.last_message_at.unwrap_or(0.0)).max(s.started_at.unwrap_or(0.0)) < at - DAY_MS)
            .map(|(k, _)| k.clone())
            .collect();
        for id in gone {
            p.sessions.retain(|(k, _)| *k != id);
            p.last_tool.retain(|(k, _)| *k != id);
        }
        Ok(())
    }

    // The Claude Code and Codex sessions Hermes started.
    fn attribute(&self, c: &mut dyn Calls, census: &Census, hosts: &[usize], profile_of_host: &dyn Fn(f64) -> String, read: &[(String, Read)], at: f64) -> Result<()> {
        // Its background processes, which outlive the host that started them.
        let mut background: Vec<(f64, String)> = Vec::new();
        for (name, r) in read {
            for e in &r.processes {
                if self.alive(e.get("pid"), e.get("startTime")) {
                    let pid = n(e.get("pid")).unwrap_or(f64::NAN);
                    match background.iter_mut().find(|(p, _)| *p == pid) {
                        Some(slot) => slot.1.clone_from(name),
                        None => background.push((pid, name.clone())),
                    }
                }
            }
        }
        let host_ids: Vec<f64> = hosts.iter().map(|&i| census.table[i].pid).collect();
        let mut agent_of = |i: usize| Ok(census.agents[i].clone());
        let mut is_root = |i: usize| Ok(host_ids.contains(&census.table[i].pid) || background.iter().any(|(p, _)| *p == census.table[i].pid));
        let started = attribution::started_under(&census.table, &mut agent_of, &mut is_root)?;
        let mut by_actor: Vec<(String, Vec<attribution::Candidate>)> = Vec::new();
        for s in started {
            let profile = background.iter().find(|(p, _)| *p == s.root).map_or_else(|| profile_of_host(census.get(s.root).map_or(f64::NAN, |r| r.pid)), |(_, n)| n.clone());
            let id = self.profile(&profile).id;
            if !c.call("actors", "has", vec![Arg::text(&id)])?.truthy() || !c.call("actors", "enabled", vec![Arg::text(&id)])?.truthy() {
                continue;
            }
            let cand = attribution::Candidate { pid: s.pid, start: s.start, agent: s.agent, cwd: None };
            match by_actor.iter_mut().find(|(a, _)| *a == id) {
                Some((_, l)) => l.push(cand),
                None => by_actor.push((id, vec![cand])),
            }
        }
        for (id, list) in &by_actor {
            attribution::saw(&self.spawns, c, &paths(), id, list, at)?;
        }
        // Which still run, by the same read of the table they were found in.
        let mut alive = |_: &mut dyn Calls, p: &attribution::Pending| Ok(census.get(p.pid).is_some_and(|q| q.start == p.start));
        for s in attribution::settle(&self.spawns, c, &paths(), at, Some(&mut alive))? {
            spawned(c, &s.actor.clone(), &s)?;
        }
        // Codex threads Hermes's own runtime started, and sessions in its
        // codex skill's scratch worktrees: the default profile's.
        let id = self.profile("default").id;
        if !c.call("actors", "has", vec![Arg::text(&id)])?.truthy() || !c.call("actors", "enabled", vec![Arg::text(&id)])?.truthy() {
            return Ok(());
        }
        let scratch = path_key(&join(&self.root, &["cache", "scratch"]));
        for s in list(attribution::opt_call(c, "actors.map", "sessions")?) {
            let Some(created) = s.field("createdAt").as_f64().filter(|_| !s.field("createdAt").is_nullish()) else { continue };
            if at - created > DAY_MS {
                continue;
            }
            let key = |f: &str| s.field(f).as_str().and_then(path_key);
            let under = |k: Option<String>| match (&k, &scratch) {
                (Some(k), Some(sc)) => is_under(k, sc),
                _ => false,
            };
            let mine = (s.field("app").as_str() == Some("codex") && s.field("originator").as_str() == Some("hermes")) || under(key("cwd")) || under(key("worktree"));
            if mine {
                let sid = xenonet_engine::store::js_text(&s.field("id"));
                self.spawns.locked().claimed.add(sid.clone());
                let o = Arg::Obj(vec![
                    ("ref".into(), Arg::text(format!("spawned:{sid}"))),
                    ("kind".into(), Arg::text("spawned")),
                    ("target".into(), s.field("id")),
                    ("app".into(), s.field("app")),
                    ("via".into(), Arg::text("attributed")),
                    ("at".into(), Arg::Num(created)),
                ]);
                c.call("", "happened", vec![Arg::text(&id), o])?;
            }
        }
        Ok(())
    }

    /// What the doctor shows of this observer: whether Hermes is installed,
    /// each source XenoNet reads and whether it could, and the process
    /// check's self-test.
    pub fn status(&self, c: &mut dyn Calls) -> Result<Value> {
        let installed = hf::installed(c, &self.root)?;
        let s = self.s();
        Ok(
            json!({ "product": "hermes-agent", "name": "Hermes Agent", "installed": installed, "sources": s.source_states.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), "recognized": s.recognized }),
        )
    }
}
