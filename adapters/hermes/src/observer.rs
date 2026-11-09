//! Hermes Agent's observer (docs/design/actors/hermes.md): observed and
//! attributed only, with no installs and no change to Hermes's settings.
//! One actor per profile: actor:hermes for the default one,
//! `actor:hermes-<profile>` for each named one.
//!
//! What it reads (`files`, metadata only): Hermes's processes in the
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

use crate::files::{self as hf, ms_of, re, same_start};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use xenonet_actors::names::hermes_slug;
use xenonet_adapter::observe::attribution::{self, census, spawned, Census, Pending, SpawnsState};
use xenonet_adapter::observe::{Core, Gather, Keep, Observer, ObserverHost, Result, Say};
use xenonet_adapter::products::Product;
use xenonet_base::json::{number, number_string};
use xenonet_base::nodepath::{style_of, NATIVE};
use xenonet_base::paths::{is_under, path_key, tilde};
use xenonet_base::sync::Locked;
use xenonet_base::text::{js_string, truthy, JS_SPACE};

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
    let tool = re!(format!(r"(?i)^(?:executing tool|tool completed|sequential tool running \([^)]*\)):[{JS_SPACE}]*([a-z_][a-z0-9_]*)"))
        .captures(d)
        .or_else(|| re!(format!(r"(?i)^executing [0-9]+ tools concurrently:[{JS_SPACE}]*([a-z_][a-z0-9_]*)")).captures(d))
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    if let Some(t) = tool {
        return tool_words(&t);
    }
    if re!(r"(?i)^(waiting for (stream|provider|non-streaming)|receiving stream|local model loading|stream retry|stale stream)").is_match(d) {
        return Some("Thinking");
    }
    if re!(r"(?i)^(context )?compress|compaction").is_match(d) {
        return Some("Tidying its context");
    }
    None
}

/// How a session ended, as an event's kind: failed when its reason says
/// so, nothing for one that went on in another (compaction), else finished.
pub fn ending_of(r: &str) -> Option<&'static str> {
    if re!(r"(?i)compress|compact").is_match(r) {
        return None;
    }
    if re!(r"(?i)error|fail|crash|unreachable|overflow|timed?_?out|out_of_credits|filter").is_match(r) {
        return Some("failed");
    }
    Some("finished")
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

// A file read again only when it changes: its mtime and size, or why it
// can't be had.
fn stamp_of(h: &dyn ObserverHost, paths: &[String]) -> String {
    let mut out = Vec::new();
    for p in paths {
        out.push(match h.files().stat(p) {
            Ok(st) => format!("{}:{}", number_string(st.mtime_ms), number_string(st.size)),
            Err(e) => e.code.unwrap_or_else(|| "?".into()),
        });
    }
    out.join("|")
}

fn join(base: &str, parts: &[&str]) -> String {
    parts.iter().fold(base.to_string(), |a, p| xenonet_base::paths::join(&a, p))
}

#[derive(Clone, Debug, Default)]
struct Db {
    stamp: Option<String>,
    last_id: Value,
    read: bool,
    /// When the usage began to be added up (the first look after the
    /// database was read).
    since: Option<f64>,
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
    /// What its row says it used, and its use by model when the database
    /// keeps that.
    usage: Option<UsageRow>,
    models: Option<Vec<UsageRow>>,
}

/// Hermes's cost_status words for a price it stands behind. "included" (a
/// subscription's) and "unknown" give none, never zero.
const PRICED: [&str; 2] = ["actual", "estimated"];

/// A row's tokens and price, as Hermes totals them.
#[derive(Clone, Debug, Default)]
struct UsageRow {
    provider: Option<String>,
    model: Option<String>,
    mode: Option<String>,
    input: f64,
    output: f64,
    reasoning: f64,
    cache_read: f64,
    cache_write: f64,
    cost_usd: Option<f64>,
    estimated: bool,
}

fn usage_row(r: &Value) -> UsageRow {
    let n = |k: &str| r.get(k).and_then(Value::as_f64).filter(|v| v.is_finite() && *v >= 0.0);
    let name = |k: &str| r.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
    let status = r.get("cost_status").and_then(Value::as_str);
    let (actual, estimate) = (n("actual_cost_usd"), n("estimated_cost_usd"));
    let cost_usd = if status.is_some_and(|s| PRICED.contains(&s)) {
        if status == Some("actual") {
            actual.or(estimate)
        } else {
            estimate.or(actual)
        }
    } else {
        None
    };
    UsageRow {
        provider: name("billing_provider"),
        model: name("model"),
        mode: name("billing_mode"),
        input: n("input_tokens").unwrap_or(0.0),
        output: n("output_tokens").unwrap_or(0.0),
        reasoning: n("reasoning_tokens").unwrap_or(0.0),
        cache_read: n("cache_read_tokens").unwrap_or(0.0),
        cache_write: n("cache_write_tokens").unwrap_or(0.0),
        cost_usd,
        estimated: cost_usd.is_some() && (status == Some("estimated") || actual.is_none()),
    }
}

/// Whether a session row carries any usage at all (the older layouts don't).
fn usage_of_row(s: &Value) -> Option<UsageRow> {
    ["input_tokens", "output_tokens", "cost_status", "billing_provider", "model"].iter().any(|c| s.get(*c).is_some_and(|v| !v.is_null())).then(|| usage_row(s))
}

/// A session's rows of session_model_usage, or None when it has none.
fn models_of(rows: &Value, id: &str) -> Option<Vec<UsageRow>> {
    let mine: Vec<UsageRow> = rows.as_array().map(|l| l.iter().filter(|r| r.get("session_id").and_then(Value::as_str) == Some(id)).map(usage_row).collect()).unwrap_or_default();
    (!mine.is_empty()).then_some(mine)
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

/// Where Hermes keeps its files: HERMES_HOME when given, else ~/.hermes.
struct Roots {
    root: String,
    home: String,
}

pub struct Hermes {
    core: Core,
    state: Mutex<State>,
    spawns: SpawnsState,
    roots: OnceLock<Roots>,
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

impl Default for Hermes {
    fn default() -> Self {
        Self::new()
    }
}

impl Hermes {
    pub fn new() -> Self {
        Hermes {
            core: Core::new(crate::product()),
            state: Mutex::new(State { recognized: Value::Null, ..Default::default() }),
            spawns: SpawnsState::default(),
            roots: OnceLock::new(),
        }
    }

    fn s(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.locked()
    }

    /// Hermes's root folder and the home folder, settled at the first look.
    fn roots(&self, h: &dyn ObserverHost) -> &Roots {
        self.roots.get_or_init(|| {
            let home = h.home().to_string();
            let root = match std::env::var("HERMES_HOME").ok().filter(|r| !r.is_empty()) {
                Some(r) => xenonet_base::paths::resolve(&r),
                None => xenonet_base::paths::join(&home, ".hermes"),
            };
            Roots { root, home }
        })
    }

    // A source read again only when its files change.
    fn stamped(&self, h: &dyn ObserverHost, key: &str, paths: &[String], read: &mut dyn FnMut() -> Value) -> Value {
        let stamp = stamp_of(h, paths);
        if let Some((_, s, v)) = self.s().cache.iter().find(|(k, _, _)| k == key) {
            if *s == stamp {
                return v.clone();
            }
        }
        let value = read();
        let mut st = self.s();
        match st.cache.iter_mut().find(|(k, _, _)| k == key) {
            Some(slot) => *slot = (key.into(), stamp, value.clone()),
            None => st.cache.push((key.into(), stamp, value.clone())),
        }
        value
    }

    fn source(&self, home: &str, name: &str, file: &str, state: &Value) {
        let v = json!({ "name": name, "file": tilde(file, home), "state": state });
        let mut s = self.s();
        match s.source_states.iter_mut().find(|(k, _)| k == name) {
            Some(slot) => slot.1 = v,
            None => s.source_states.push((name.into(), v)),
        }
    }

    // A profile's actor: its id and name, and what XenoNet keeps of it
    // between looks.
    fn profile(&self, root: &str, name: &str) -> Profile {
        let mut s = self.s();
        if let Some(p) = s.profiles.iter().find(|p| p.name == name) {
            return p.clone();
        }
        let slug = hermes_slug(Some(&Value::String(name.into())));
        let title = if name == "default" {
            "Hermes".to_string()
        } else if re!(r"^[A-Za-z0-9][0-9A-Za-z_ .-]{0,31}$").is_match(name) {
            format!("Hermes ({name})")
        } else {
            "Hermes (a profile)".into()
        };
        let p = Profile {
            name: name.into(),
            id: format!("actor:{slug}"),
            home: if name == "default" { root.to_string() } else { join(root, &["profiles", name]) },
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
    fn profiles_now(&self, h: &dyn ObserverHost, root: &str, at: f64, ledger: &[Value]) -> Vec<Profile> {
        if at - self.s().profiles_at.unwrap_or(f64::NEG_INFINITY) >= PROFILES_MS {
            let list = hf::list_profiles(h, root);
            let mut s = self.s();
            s.profiles_at = Some(at);
            s.profile_list = list.as_array().into_iter().flatten().map(|p| text_of(p.get("name"))).collect();
        }
        let mut names = self.s().profile_list.clone();
        // Only a plain name, so a profile's folder stays inside Hermes's.
        for e in ledger {
            let profile = e.get("profile").filter(|p| truthy(Some(p)));
            if profile.is_none_or(|p| re!(r"^[A-Za-z0-9][0-9A-Za-z_.-]{0,63}$").is_match(&js_string(p))) {
                let name = profile.map_or_else(|| "default".to_string(), js_string);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names.iter().map(|n| self.profile(root, n)).collect()
    }

    // Whether a process Hermes recorded still runs, from the last census.
    fn alive(&self, pid: Option<&Value>, start: Option<&Value>) -> bool {
        let s = self.s();
        let Some(pid) = pid.and_then(Value::as_f64).filter(|p| *p != 0.0) else { return false };
        let Some(row) = s.table.as_ref().and_then(|t| t.get(pid)) else { return false };
        start.filter(|s| !s.is_null()).is_none_or(|s| same_start(s, row.start, 2000.0))
    }

    // Where a session works, as a place on the map: its repo's root, else
    // its folder.
    fn place_of_session(h: &dyn ObserverHost, s: Option<&Session>) -> Value {
        let Some(s) = s else { return Value::Null };
        for dir in [&s.repo, &s.cwd] {
            if let Some(d) = dir.as_str().filter(|d| !d.is_empty() && NATIVE.is_absolute(d)) {
                let place = h.actors().place_of(d);
                if truthy(Some(&place)) {
                    return place;
                }
            }
        }
        Value::Null
    }

    // A path from a tool call, made absolute: Hermes takes them absolute,
    // relative to the session's folder, or under ~.
    fn absolute(home: &str, p: &Value, s: Option<&Session>) -> Option<String> {
        let p = p.as_str().filter(|p| !p.is_empty() && !p.contains('\0'))?;
        if p == "~" || p.starts_with("~/") {
            return Some(xenonet_base::paths::join(home, &p[1..]));
        }
        if NATIVE.is_absolute(p) {
            return Some(style_of(p).join(&[p]));
        }
        let cwd = s.and_then(|s| s.cwd.as_str()).filter(|c| NATIVE.is_absolute(c))?;
        let joined = style_of(cwd).join(&[cwd, p]);
        Some(if joined.len() > 1 { joined.trim_end_matches(['/', '\\']).to_string() } else { joined })
    }

    fn exists(&self, h: &dyn ObserverHost, p: &str) -> bool {
        let dir = std::path::Path::new(p).parent().map_or_else(|| ".".into(), |d| d.to_string_lossy().into_owned());
        self.stamped(h, &format!("exists:{p}"), &[dir], &mut || json!(h.files().exists(p))).as_bool().unwrap_or(false)
    }

    // Whether Hermes's computer-use driver is installed among its tools.
    fn cua_driver(&self, h: &dyn ObserverHost, root: &str) -> bool {
        let dir = join(root, &["tools"]);
        self.stamped(h, "cua-driver", std::slice::from_ref(&dir), &mut || json!(h.files().readdir(&dir).is_ok_and(|names| names.iter().any(|n| n.starts_with("cua-driver")))))
            .as_bool()
            .unwrap_or(false)
    }

    // A profile's files, each read again only when it changes, and what's
    // new in its session database.
    fn read_profile(&self, h: &dyn ObserverHost, home: &str, p: &mut Profile, version: Option<&str>, at: f64) -> Read {
        let f = |parts: &[&str]| join(&p.home, parts);
        let phome = p.home.clone();
        let gateway = self.stamped(h, &format!("{}:gateway", p.name), &[f(&["gateway_state.json"])], &mut || hf::read_gateway(h, &phome, version));
        let ticker =
            self.stamped(h, &format!("{}:ticker", p.name), &[f(&["cron", "ticker_heartbeat"]), f(&["cron", "ticker_last_success"])], &mut || hf::read_ticker(h, &phome, version));
        let processes = self.stamped(h, &format!("{}:processes", p.name), &[f(&["processes.json"])], &mut || hf::read_processes(h, &phome, version));
        let jobs = self.stamped(h, &format!("{}:jobs", p.name), &[f(&["cron", "jobs.json"])], &mut || hf::read_jobs(h, &phome, version));
        let label = if p.name == "default" { String::new() } else { format!(" ({})", p.name) };
        self.source(home, &format!("gateway's state{label}"), &f(&["gateway_state.json"]), &gateway["state"]);
        self.source(home, &format!("scheduler's ticker{label}"), &f(&["cron", "ticker_heartbeat"]), &ticker["state"]);
        self.source(home, &format!("background processes{label}"), &f(&["processes.json"]), &processes["state"]);
        self.source(home, &format!("scheduled jobs{label}"), &f(&["cron", "jobs.json"]), &jobs["state"]);
        // The session database, when it or its log changed.
        let db_file = f(&["state.db"]);
        let stamp = stamp_of(h, &[db_file.clone(), format!("{db_file}-wal")]);
        let mut fresh = None;
        if Some(&stamp) != p.db.stamp.as_ref() {
            let since = at - if p.db.read { HISTORY_MS } else { REPLAY_MS };
            let got = hf::read_state_db(h, &db_file, &p.db.last_id, since, &[]);
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
        self.source(home, &format!("session database{label}"), &db_file, &json!(p.db.state.clone().unwrap_or_else(|| "missing".into())));
        let g = gateway["gateway"].clone();
        let t = ticker["ticker"].clone();
        let gateway_alive = g.is_object() && running_gateway(&g["state"]) && self.alive(g.get("pid"), g.get("startTime"));
        let ticker_alive = t.is_object() && self.alive(t.get("pid"), None) && at - n(t.get("at")).unwrap_or(0.0) < TICKER_STALE_MS;
        Read {
            gateway: g,
            gateway_alive,
            ticker: t,
            ticker_alive,
            processes: processes["entries"].as_array().cloned().unwrap_or_default(),
            jobs: jobs["jobs"].as_array().cloned().unwrap_or_default(),
            fresh,
            heartbeats: p.db.heartbeats.as_ref().and_then(Value::as_array).cloned().unwrap_or_default(),
            db_state: p.db.state.clone().unwrap_or_else(|| "missing".into()),
        }
    }

    /// What its sessions' own rows say they used and cost, added up: the
    /// sessions seen at work since the observer began (their whole totals, as
    /// Hermes keeps them). None while the database hasn't been read (the
    /// usage is left unsaid), null when no session carries any.
    fn usage(p: &mut Profile, at: f64) -> Option<Value> {
        if !p.db.read {
            return None;
        }
        let since = *p.db.since.get_or_insert(at);
        let (mut input, mut output, mut reasoning, mut cache_read, mut cache_write) = (0.0, 0.0, 0.0, 0.0, 0.0);
        let mut by_model: Vec<(String, UsageRow, f64, Option<f64>)> = Vec::new();
        let (mut cost_usd, mut estimated, mut unpriced, mut sessions): (Option<f64>, bool, f64, f64) = (None, false, 0.0, 0.0);
        for (_, s) in &p.sessions {
            let Some(u) = &s.usage else { continue };
            if s.hidden {
                continue;
            }
            sessions += 1.0;
            let rows: Vec<&UsageRow> = match &s.models {
                Some(m) if !m.is_empty() => m.iter().collect(),
                _ => vec![u],
            };
            for r in rows {
                input += r.input;
                output += r.output;
                reasoning += r.reasoning;
                cache_read += r.cache_read;
                cache_write += r.cache_write;
                let key = format!("{:?}\u{0}{:?}\u{0}{:?}", r.provider, r.model, r.mode);
                let slot = match by_model.iter().position(|(k, ..)| *k == key) {
                    Some(i) => i,
                    None => {
                        by_model.push((key, r.clone(), 0.0, None));
                        by_model.len() - 1
                    }
                };
                by_model[slot].2 += r.input + r.output;
                if let Some(cost) = r.cost_usd {
                    by_model[slot].3 = Some(by_model[slot].3.unwrap_or(0.0) + cost);
                }
            }
            match u.cost_usd {
                None => unpriced += 1.0,
                Some(cost) => {
                    cost_usd = Some(cost_usd.unwrap_or(0.0) + cost);
                    if u.estimated {
                        estimated = true;
                    }
                }
            }
        }
        if sessions == 0.0 {
            return Some(Value::Null);
        }
        by_model.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        let models: Vec<Value> = by_model
            .iter()
            .take(5)
            .map(|(_, r, tokens, cost)| json!({ "provider": r.provider, "model": r.model, "mode": r.mode, "tokens": number(*tokens), "costUsd": cost.map(number) }))
            .collect();
        Some(json!({
            "since": number(since),
            "sessions": number(sessions),
            "costUsd": cost_usd.map(number),
            "estimated": estimated,
            "unpriced": number(unpriced),
            "tokens": { "input": number(input), "output": number(output), "reasoning": number(reasoning), "cacheRead": number(cache_read), "cacheWrite": number(cache_write) },
            "models": models,
        }))
    }

    // What a profile's database says, its presence and status, and the
    // events and targets they make.
    #[expect(clippy::too_many_arguments, reason = "what one look found of a profile, one argument each")]
    fn observe(
        &self,
        h: &dyn ObserverHost,
        roots: &Roots,
        p: &mut Profile,
        r: &Read,
        processes: usize,
        apps: usize,
        version: Option<&str>,
        at: f64,
        is_installed: bool,
    ) -> Result<()> {
        let actors = h.actors();
        let there = processes > 0 || r.gateway_alive || r.ticker_alive;
        let fresh_ok = r.fresh.as_ref().filter(|f| f["state"].as_str() == Some("ok"));
        // A named profile is an actor once something of it has run; the
        // default one is while Hermes is installed.
        let installed_default = p.name == "default" && is_installed;
        if !there && apps == 0 && !actors.has(&p.id) && !installed_default {
            if let Some(f) = fresh_ok {
                self.take(h, &roots.home, p, f, at, false)?;
            }
            return Ok(());
        }
        let mut capabilities = vec!["run-commands", "edit-repos", "browse", "schedule"];
        if self.exists(h, &join(&p.home, &["skills", "autonomous-ai-agents"])) {
            capabilities.push("spawn-sessions");
        }
        if r.gateway.get("platforms").and_then(Value::as_array).is_some_and(|l| l.iter().any(|x| x["state"].as_str() == Some("connected"))) {
            capabilities.push("message-people");
        }
        if self.cua_driver(h, &roots.root) {
            capabilities.push("use-computer");
        }
        // Installed: Hermes is, and so is the profile's folder.
        let installed = is_installed && self.s().profile_list.contains(&p.name);
        let keep = Keep {
            name: p.title.clone(),
            version: version.map(str::to_string),
            via: vec!["observed", "attributed"],
            heartbeat_ms: Some(HERMES_HEARTBEAT_MS),
            capabilities,
            installed: Some(installed),
            at,
        };
        self.core.keep(h, &p.id, &keep)?;
        if !actors.enabled(&p.id) {
            return Ok(());
        }
        // What the database says that's new.
        if let Some(f) = fresh_ok {
            self.take(h, &roots.home, p, f, at, true)?;
        }
        // The sessions at work: open, not hidden, active in the last two
        // minutes.
        let latest = |s: &Session| s.activity_at.unwrap_or(0.0).max(s.last_message_at.unwrap_or(0.0));
        let mut active: Vec<Session> =
            p.sessions.iter().map(|(_, s)| s.clone()).filter(|s| s.ended_at.is_none() && !s.archived && !s.hidden && latest(s) >= at - ACTIVE_MS).collect();
        xenonet_base::sort::sort_by(&mut active, |a, b| latest(b).partial_cmp(&latest(a)).unwrap_or(std::cmp::Ordering::Equal));
        // A gateway whose heartbeat is stale while its process runs has gone
        // quiet: only its sessions' own activity says what it's doing.
        let quiet = r.gateway_alive && n(r.gateway.get("updatedAt")).is_some_and(|u| at - u > STALE_MS);
        let busy = r.gateway_alive && !quiet && n(r.gateway.get("activeAgents")).unwrap_or(0.0) > 0.0;
        let (presence, status): (&str, Option<String>) = if there && (!active.is_empty() || busy) {
            let status = match active.first() {
                Some(top) => {
                    let doing = if top.activity_at.is_some_and(|a| a >= at - ACTIVE_MS) { doing_of(&text_of(Some(&top.activity))) } else { None };
                    let tool = p.last_tool.iter().find(|(k, _)| *k == top.id).map(|(_, t)| t.clone()).unwrap_or_default();
                    let words = doing.or_else(|| tool_words(&tool)).unwrap_or("Working");
                    let place = Self::place_of_session(h, Some(top));
                    let name = actors.where_of(&place).get("name").cloned().unwrap_or(Value::Null);
                    if truthy(Some(&name)) {
                        format!("{words} in {}", js_string(&name))
                    } else {
                        words.to_string()
                    }
                }
                None => "Working through its gateway".to_string(),
            };
            ("working", Some(status))
        } else if quiet {
            ("away", Some("Its gateway has gone quiet".into()))
        } else if there {
            let s = if r.gateway_alive {
                match r.gateway["state"].as_str() {
                    Some("starting") => Some("Its gateway is starting".to_string()),
                    Some("degraded") => Some("Its gateway is degraded".to_string()),
                    Some("draining") => Some("Its gateway is finishing its work".to_string()),
                    _ => None,
                }
            } else {
                None
            };
            ("idle", s)
        } else if apps > 0 {
            ("idle", Some("Hermes.app runs with no backend on this Mac".into()))
        } else if r.db_state == "denied" || r.db_state == "unreadable" {
            ("unknown", Some("XenoNet can't read its session database".into()))
        } else {
            ("offline", (r.gateway["state"].as_str() == Some("startup_failed")).then(|| "Its gateway failed to start".to_string()))
        };
        let usage = Self::usage(p, at);
        self.core.say(h, &p.id, &Say { presence, status, usage, at })?;
        // Where it works: its sessions' places, and its jobs' folders.
        for s in &active {
            let place = Self::place_of_session(h, Some(s));
            self.core.touch(h, &p.id, &place, "works-on", at)?;
        }
        for j in &r.jobs {
            let workdir = j["workdir"].as_str().filter(|w| !w.is_empty() && NATIVE.is_absolute(w));
            if j["enabled"].as_bool() == Some(true) {
                if let Some(w) = workdir {
                    let place = actors.place_of(w);
                    self.core.touch(h, &p.id, &place, "watches", at)?;
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
                    Some(w) => actors.place_of(w),
                    None => Value::Null,
                };
                let o = json!({
                    "ref": format!("job:{}:{}", text_of(j.get("id")), number_string(last)),
                    "kind": kind,
                    "target": target,
                    "subject": "job",
                    "at": number(last.min(at)),
                });
                self.core.happened(h, &p.id, &o)?;
            }
        }
        Ok(())
    }

    // New rows of a profile's database: sessions kept current, its tools
    // gathered per minute, its replies on messaging platforms, sessions that
    // ended.
    fn take(&self, h: &dyn ObserverHost, home: &str, p: &mut Profile, fresh: &Value, at: f64, record: bool) -> Result<()> {
        let parse = |v: Option<&Value>| -> Option<f64> { v.and_then(ms_of) };
        let sessions = fresh["sessions"].as_array().cloned().unwrap_or_default();
        for s in &sessions {
            let id = text_of(s.get("id"));
            let was = p.sessions.iter().find(|(k, _)| *k == id).and_then(|(_, x)| x.last_message_at);
            let was_models = p.sessions.iter().find(|(k, _)| *k == id).and_then(|(_, x)| x.models.clone());
            let opt = |k: &str| s.get(k).cloned().unwrap_or(Value::Null);
            let session = Session {
                id: id.clone(),
                source: opt("source"),
                cwd: opt("cwd"),
                repo: opt("git_repo_root"),
                parent: opt("parent_session_id"),
                started_at: parse(s.get("started_at")),
                ended_at: parse(s.get("ended_at")),
                end_reason: opt("end_reason"),
                activity_at: parse(s.get("last_activity_at")),
                activity: opt("last_activity_description"),
                archived: truthy(s.get("archived")),
                hidden: truthy(s.get("hidden")),
                last_message_at: was,
                usage: usage_of_row(s),
                models: models_of(&fresh["modelUsage"], &id).or(was_models),
            };
            match p.sessions.iter_mut().find(|(k, _)| *k == id) {
                Some(slot) => slot.1 = session,
                None => p.sessions.push((id, session)),
            }
        }
        for m in fresh["messages"].as_array().cloned().unwrap_or_default() {
            let sid = text_of(m.get("session_id"));
            let Some(when) = parse(m.get("timestamp")).filter(|w| *w != 0.0) else { continue };
            if let Some((_, s)) = p.sessions.iter_mut().find(|(k, _)| *k == sid) {
                s.last_message_at = Some(s.last_message_at.unwrap_or(0.0).max(when));
            }
            let s = p.sessions.iter().find(|(k, _)| *k == sid).map(|(_, s)| s.clone());
            let tool = m.get("tool_name").filter(|t| truthy(Some(t))).map(js_string).filter(|_| m["role"].as_str() == Some("tool"));
            if let Some(tool) = tool {
                match p.last_tool.iter_mut().find(|(k, _)| *k == sid) {
                    Some(slot) => slot.1.clone_from(&tool),
                    None => p.last_tool.push((sid.clone(), tool.clone())),
                }
                if record {
                    if let Some(kind) = tool_kind(&tool) {
                        let target = Self::place_of_session(h, s.as_ref());
                        self.core.gather(&p.id, &Gather::new(kind, target, when));
                    }
                }
            } else if record && m["role"].as_str() == Some("assistant") && !truthy(m.get("calls")) {
                if let Some(pl) = s.as_ref().and_then(|s| s.source.as_str()).and_then(platform) {
                    self.core.gather(&p.id, &Gather { platform: Some(pl.into()), ..Gather::new("messaged", Value::Null, when) });
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
            let Some(when) = parse(t.get("timestamp")).filter(|w| *w != 0.0) else { continue };
            let file = Self::absolute(home, t.get("path").unwrap_or(&Value::Null), s.as_ref());
            let place = match &file {
                Some(f) => h.actors().place_of(f),
                None => Value::Null,
            };
            let target = if truthy(Some(&place)) { place.clone() } else { Self::place_of_session(h, s.as_ref()) };
            let paths: Vec<String> = file.filter(|_| truthy(Some(&place))).into_iter().collect();
            self.core.gather(&p.id, &Gather { paths, ..Gather::new("edited", target, when) });
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
                let target = Self::place_of_session(h, Some(&k));
                let o = json!({
                    "ref": format!("session:{}:end", k.id),
                    "kind": kind,
                    "target": target,
                    "subject": "session",
                    "at": number(ended.min(at)),
                });
                self.core.happened(h, &p.id, &o)?;
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
    fn attribute(&self, h: &dyn ObserverHost, census: &Census, hosts: &[usize], profile_of_host: &dyn Fn(f64) -> String, read: &[(String, Read)], at: f64) -> Result<()> {
        let root = self.roots(h).root.as_str();
        let actors = h.actors();
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
        let mut agent_of = |i: usize| census.agents[i].clone();
        let mut is_root = |i: usize| host_ids.contains(&census.table[i].pid) || background.iter().any(|(p, _)| *p == census.table[i].pid);
        let started = attribution::started_under(&census.table, &mut agent_of, &mut is_root);
        let mut by_actor: Vec<(String, Vec<attribution::Candidate>)> = Vec::new();
        for s in started {
            let profile = background.iter().find(|(p, _)| *p == s.root).map_or_else(|| profile_of_host(census.get(s.root).map_or(f64::NAN, |r| r.pid)), |(_, n)| n.clone());
            let id = self.profile(root, &profile).id;
            if !actors.has(&id) || !actors.enabled(&id) {
                continue;
            }
            let cand = attribution::Candidate { pid: s.pid, start: s.start, agent: s.agent, cwd: None };
            match by_actor.iter_mut().find(|(a, _)| *a == id) {
                Some((_, l)) => l.push(cand),
                None => by_actor.push((id, vec![cand])),
            }
        }
        for (id, list) in &by_actor {
            attribution::saw(&self.spawns, h, id, list, at);
        }
        // Which still run, by the same read of the table they were found in.
        let mut alive = |p: &Pending| census.get(p.pid).is_some_and(|q| q.start == p.start);
        for s in attribution::settle(&self.spawns, h, at, Some(&mut alive))? {
            spawned(h, &self.core, &s.actor.clone(), &s)?;
        }
        // Codex threads Hermes's own runtime started, and sessions in its
        // codex skill's scratch worktrees: the default profile's.
        let id = self.profile(root, "default").id;
        if !actors.has(&id) || !actors.enabled(&id) {
            return Ok(());
        }
        let scratch = path_key(&join(root, &["cache", "scratch"]));
        for s in h.spawns().sessions()? {
            let Some(created) = s.get("createdAt").and_then(Value::as_f64) else { continue };
            if at - created > DAY_MS {
                continue;
            }
            let key = |f: &str| s.get(f).and_then(Value::as_str).and_then(path_key);
            let under = |k: Option<String>| match (&k, &scratch) {
                (Some(k), Some(sc)) => is_under(k, sc),
                _ => false,
            };
            let mine = (s.get("app").and_then(Value::as_str) == Some("codex") && s.get("originator").and_then(Value::as_str) == Some("hermes"))
                || under(key("cwd"))
                || under(key("worktree"));
            if mine {
                let sid = text_of(s.get("id"));
                self.spawns.locked().claimed.add(sid.clone());
                let o = json!({
                    "ref": format!("spawned:{sid}"),
                    "kind": "spawned",
                    "target": s.get("id").cloned().unwrap_or(Value::Null),
                    "app": s.get("app").cloned().unwrap_or(Value::Null),
                    "via": "attributed",
                    "at": number(created),
                });
                self.core.happened(h, &id, &o)?;
            }
        }
        Ok(())
    }
}

impl Observer for Hermes {
    fn name(&self) -> &'static str {
        "hermes-observer"
    }

    fn product(&self) -> &'static Product {
        self.core.product()
    }

    fn core(&self) -> &Core {
        &self.core
    }

    fn look(&self, h: &dyn ObserverHost, at: f64) -> Result<()> {
        let roots = self.roots(h);
        let root = roots.root.as_str();
        let actors = h.actors();
        let mut known = 0;
        for id in actors.of_product("hermes-agent") {
            let info = actors.info(&id);
            if !info.get("via").and_then(Value::as_array).is_some_and(|l| l.iter().any(|v| v.as_str() == Some("demo"))) {
                known += 1;
            }
        }
        let is_installed = hf::installed(h, root);
        if !is_installed && known == 0 {
            return Ok(());
        }
        let mut table_state = None;
        let census = census(h, &mut table_state)?;
        let role = |i: usize| match census.product_role(i) {
            (Some("hermes-agent"), role) => role.map(str::to_string),
            _ => None,
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
        let stamp_file = join(root, &["hermes-agent", "install-stamp.json"]);
        let stamp = self.stamped(h, "stamp", std::slice::from_ref(&stamp_file), &mut || hf::read_stamp(h, root));
        self.source(&roots.home, "build stamp", &stamp_file, &stamp["state"]);
        let version = stamp["version"].as_str().map(str::to_string);
        let ledger_file = join(root, &["spawn-ledger.json"]);
        let ledger = self.stamped(h, "ledger", std::slice::from_ref(&ledger_file), &mut || hf::read_ledger(h, root, version.as_deref()));
        self.source(&roots.home, "spawn ledger", &ledger_file, &ledger["state"]);
        let live: Vec<Value> = ledger["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| {
                let Some(row) = n(e.get("pid")).and_then(|p| census.get(p)) else { return false };
                e.get("createTime").is_none_or(Value::is_null) || same_start(&e["createTime"], row.start, 2000.0)
            })
            .collect();
        // The process check's self-test: every live host the ledger lists
        // should be one the process table knows as Hermes's.
        let expected: Vec<&Value> = live.iter().filter(|e| host_purpose(&e["purpose"])).collect();
        let found = expected.iter().filter(|e| n(e.get("pid")).and_then(role_of_pid).as_deref() == Some("host")).count();
        let purposes: Vec<Value> = expected.iter().filter(|e| n(e.get("pid")).and_then(role_of_pid).as_deref() != Some("host")).map(|e| e["purpose"].clone()).collect();
        self.s().recognized = json!({ "expected": expected.len(), "found": found, "purposes": purposes });
        self.s().table = Some(Census { table: census.table.clone(), agents: census.agents.clone(), products: census.products.clone() });
        let mut profiles = self.profiles_now(h, root, at, &live);
        let mut profile_of: Vec<(f64, String)> = Vec::new();
        let set = |pid: f64, name: &str, to: &mut Vec<(f64, String)>| match to.iter_mut().find(|(p, _)| *p == pid) {
            Some(slot) => slot.1 = name.into(),
            None => to.push((pid, name.into())),
        };
        for e in &live {
            let name = e.get("profile").filter(|p| truthy(Some(p))).map_or_else(|| "default".to_string(), js_string);
            if self.s().profiles.iter().any(|p| p.name == name) {
                set(n(e.get("pid")).unwrap_or(f64::NAN), &name, &mut profile_of);
            }
        }
        let mut read: Vec<(String, Read)> = Vec::new();
        for p in &mut profiles {
            let r = self.read_profile(h, &roots.home, p, version.as_deref(), at);
            self.put(p);
            if r.gateway_alive {
                set(n(r.gateway.get("pid")).unwrap_or(f64::NAN), &p.name, &mut profile_of);
            }
            if r.ticker_alive {
                set(n(r.ticker.get("pid")).unwrap_or(f64::NAN), &p.name, &mut profile_of);
            }
            for hb in &r.heartbeats {
                if at - hb.get("last_heartbeat").and_then(ms_of).unwrap_or(0.0) < TICKER_STALE_MS {
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
                .map_or_else(|| "default".to_string(), js_string);
            add(&mut apps_of, profile, a);
        }
        for p in &mut profiles {
            let r = read.iter().find(|(n, _)| *n == p.name).map(|(_, r)| r).expect("each profile was read above");
            let procs = running.iter().find(|(n, _)| *n == p.name).map_or(0, |(_, l)| l.len());
            let app_count = apps_of.iter().find(|(n, _)| *n == p.name).map_or(0, |(_, l)| l.len());
            self.observe(h, roots, p, r, procs, app_count, version.as_deref(), at, is_installed)?;
            self.put(p);
        }
        // The sessions Hermes started.
        self.attribute(h, &census, &hosts, &profile_of_host, &read, at)
    }

    /// What the doctor shows of this observer: whether Hermes is installed,
    /// each source XenoNet reads and whether it could, and the process
    /// check's self-test.
    fn status(&self, h: &dyn ObserverHost) -> Result<Value> {
        let installed = hf::installed(h, &self.roots(h).root);
        let s = self.s();
        Ok(
            json!({ "product": "hermes-agent", "name": "Hermes Agent", "installed": installed, "sources": s.source_states.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), "recognized": s.recognized }),
        )
    }
}
