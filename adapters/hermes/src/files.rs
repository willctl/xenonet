//! Hermes Agent's state on this machine, read for metadata only
//! (docs/design/actors/hermes.md, sections 2 and 5): its build stamp, its
//! spawn ledger, and for each profile its gateway's state, its
//! scheduler's ticker, its background processes, its scheduled jobs and
//! its session database. Each source is read against its vendor schema
//! through the lenient readers, so a change in Hermes's formats shows as
//! drift rather than as a quiet observer.
//!
//! What's never read: .env, auth.json, pairing/, mcp-tokens/, the desktop
//! app's connection and token files, logs, memories, cron output, and in
//! state.db anything that holds content or who's on the other end. The
//! database is opened read only for a moment, with named columns in every
//! query; the one thing taken from a tool call's arguments is the path of a
//! file it wrote, which SQLite takes out (json_extract).
//!
//! Every reader answers { state, ... }: ok, missing, denied or unreadable.

use serde_json::{json, Map, Value};
use xenonet_adapter::observe::{Fail, ObserverHost, Result};
use xenonet_base::json::number;
use xenonet_base::text::{js_string, js_trim, JS_SPACE};
use xenonet_schema::drift::clean_version;

/// A pattern compiled once.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| regex::Regex::new(&$p).expect("a pattern that compiles"))
    }};
}
pub(crate) use re;

pub const SOURCE: [(&str, &str); 7] = [
    ("stamp", "nous/hermes-agent/install-stamp"),
    ("ledger", "nous/hermes-agent/spawn-ledger"),
    ("gateway", "nous/hermes-agent/gateway-state"),
    ("processes", "nous/hermes-agent/processes"),
    ("jobs", "nous/hermes-agent/cron-jobs"),
    ("ticker", "nous/hermes-agent/cron-ticker"),
    ("stateDb", "nous/hermes-agent/state-db"),
];

fn source(k: &str) -> &'static str {
    SOURCE.iter().find(|(n, _)| *n == k).map(|(_, s)| *s).expect("one of the sources above")
}

/// The tool names whose `path` argument is a file Hermes changed.
pub const EDIT_TOOLS: [&str; 2] = ["write_file", "patch"];

// What a failed read says, by the error's code.
fn state_of(f: &Fail) -> &'static str {
    match f.code.as_deref() {
        Some("ENOENT" | "ENOTDIR") => "missing",
        Some("EACCES" | "EPERM") => "denied",
        _ => "unreadable",
    }
}

fn code_value(f: &Fail) -> Value {
    f.code.clone().map_or(Value::Null, Value::String)
}

/// Hermes's times, epoch seconds (floats) or ISO 8601, as epoch
/// milliseconds; None for anything else.
pub fn ms_of(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => {
            let f = n.as_f64()?;
            (f.is_finite() && f > 0.0).then(|| if f > 1e11 { xenonet_base::units::js_round(f) } else { xenonet_base::units::js_round(f * 1000.0) })
        }
        Value::String(s) if !s.is_empty() => xenonet_base::time::parse_iso(s).filter(|t| t.is_finite()),
        _ => None,
    }
}

/// Whether a process start Hermes recorded is the one the process table
/// says (to the second): psutil's epoch seconds in the ledger, centiseconds
/// of the epoch in the gateway's and the background processes' records on
/// macOS.
pub fn same_start(recorded: &Value, table_start: f64, slack: f64) -> bool {
    let Some(r) = recorded.as_f64().filter(|r| r.is_finite()) else { return false };
    if table_start == 0.0 || table_start.is_nan() {
        return false;
    }
    [r * 1000.0, r * 10.0, r].iter().any(|ms| (ms - table_start).abs() <= slack)
}

enum Read {
    Ok(String),
    Failed(&'static str, Value),
}

fn read_text(h: &dyn ObserverHost, file: &str) -> Read {
    match h.files().read_text(file) {
        Ok(t) => Read::Ok(t),
        Err(f) => Read::Failed(state_of(&f), code_value(&f)),
    }
}

// A JSON file's value, or why it can't be had: (state, code).
type Checked = std::result::Result<Value, (&'static str, Value)>;

// The version a file's own value names.
type VersionOf<'a> = &'a dyn Fn(&Value) -> Option<String>;

// A JSON file read against its source's schema: Ok(value) or Err((state,
// code)).
fn read_json(h: &dyn ObserverHost, file: &str, src: &str, version: Option<&str>, version_of: Option<VersionOf<'_>>) -> Checked {
    let text = match read_text(h, file) {
        Read::Ok(t) => t,
        Read::Failed(state, code) => return Err((state, code)),
    };
    // Hermes reads its files as utf-8-sig, so one may start with a BOM.
    let Ok(value) = xenonet_base::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)) else {
        h.vendor().unparsable(src, version);
        return Err(("unreadable", json!("not JSON")));
    };
    let v = match version_of {
        Some(f) => f(&value),
        None => version.map(str::to_string),
    };
    let checked = h.vendor().check(src, &value, v.as_deref());
    if checked.is_null() {
        Err(("unreadable", json!("not what XenoNet reads")))
    } else {
        Ok(checked)
    }
}

fn num(v: Option<&Value>) -> Value {
    v.filter(|x| x.as_f64().is_some_and(f64::is_finite)).cloned().unwrap_or(Value::Null)
}

fn text(v: Option<&Value>) -> Value {
    v.filter(|x| x.as_str().is_some_and(|s| !s.is_empty())).cloned().unwrap_or(Value::Null)
}

fn ms(v: Option<&Value>) -> Value {
    v.and_then(ms_of).map_or(Value::Null, number)
}

// ---------- the install ----------

/// The profiles, by name, with their folders: the default one is the root.
pub fn list_profiles(h: &dyn ObserverHost, root: &str) -> Value {
    let mut out = vec![json!({ "name": "default", "home": root })];
    let profiles = join(root, "profiles");
    let mut names: Vec<String> = h.files().dirs(&profiles).unwrap_or_default();
    names.sort_by(|a, b| xenonet_base::jsstr::cmp_units(a, b));
    for name in names {
        if name != "default" && !name.starts_with('.') {
            out.push(json!({ "name": name, "home": join(&profiles, &name) }));
        }
    }
    Value::Array(out)
}

fn join(a: &str, b: &str) -> String {
    xenonet_base::paths::join(a, b)
}

/// Whether Hermes is installed here: its root, with its checkout or its
/// session database in it.
pub fn installed(h: &dyn ObserverHost, root: &str) -> bool {
    ["hermes-agent", "state.db"].iter().any(|f| h.files().exists(&join(root, f)))
}

/// The build stamp: the release this build is of (baseVersion).
pub fn read_stamp(h: &dyn ObserverHost, root: &str) -> Value {
    let file = join(&join(root, "hermes-agent"), "install-stamp.json");
    let of = |v: &Value| v.get("schemaVersion").filter(|s| s.as_f64().is_some_and(f64::is_finite)).and_then(|s| clean_version(&js_string(s)).map(str::to_string));
    match read_json(h, &file, source("stamp"), None, Some(&of)) {
        Ok(v) => json!({ "state": "ok", "version": v.get("baseVersion").and_then(Value::as_str).filter(|s| !s.is_empty()).and_then(clean_version) }),
        Err((state, _)) => json!({ "state": state, "version": null }),
    }
}

/// Hermes's long-lived processes as it keeps them: { pid, createTime,
/// purpose, port, profile, spawnerPid }. Never argv.
pub fn read_ledger(h: &dyn ObserverHost, root: &str, version: Option<&str>) -> Value {
    match read_json(h, &join(root, "spawn-ledger.json"), source("ledger"), version, None) {
        Err((state, _)) => json!({ "state": state, "entries": [] }),
        Ok(v) => {
            let entries: Vec<Value> = v
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_object() && num(e.get("pid")).as_f64().is_some_and(|p| p != 0.0) && !text(e.get("purpose")).is_null())
                .map(|e| json!({ "pid": e["pid"], "createTime": num(e.get("create_time")), "purpose": e["purpose"], "port": num(e.get("port")), "profile": text(e.get("profile")), "spawnerPid": num(e.get("spawner_pid")) }))
                .collect();
            json!({ "state": "ok", "entries": entries })
        }
    }
}

/// The gateway as its state file says, or null with no file.
pub fn read_gateway(h: &dyn ObserverHost, home: &str, version: Option<&str>) -> Value {
    match read_json(h, &join(home, "gateway_state.json"), source("gateway"), version, None) {
        Err((state, _)) => json!({ "state": state, "gateway": null }),
        Ok(g) => {
            let platforms: Vec<Value> = match g.get("platforms") {
                Some(Value::Object(p)) => p.iter().filter(|(_, x)| x.is_object()).map(|(name, x)| json!({ "name": name, "state": text(x.get("state")) })).collect(),
                _ => Vec::new(),
            };
            let active = num(g.get("active_agents"));
            json!({
                "state": "ok",
                "gateway": { "state": text(g.get("gateway_state")), "pid": num(g.get("pid")), "startTime": num(g.get("start_time")), "updatedAt": ms(g.get("updated_at")), "activeAgents": if active.is_null() { json!(0) } else { active }, "exitReason": text(g.get("exit_reason")), "platforms": platforms },
            })
        }
    }
}

/// The scheduler's ticker: { at, pid, lastSuccess }, or null.
pub fn read_ticker(h: &dyn ObserverHost, home: &str, version: Option<&str>) -> Value {
    let cron = join(home, "cron");
    let beat = read_text(h, &join(&cron, "ticker_heartbeat"));
    let success = read_text(h, &join(&cron, "ticker_last_success"));
    let state = |r: &Read| match r {
        Read::Ok(_) => "ok",
        Read::Failed(s, _) => s,
    };
    if let Some(f) = [&beat, &success].into_iter().find(|r| !matches!(state(r), "ok" | "missing")) {
        return json!({ "state": state(f), "ticker": null });
    }
    if state(&beat) == "missing" && state(&success) == "missing" {
        return json!({ "state": "missing", "ticker": null });
    }
    // "<epoch seconds> <pid>" and "<epoch seconds>".
    let words = |r: &Read| -> Vec<String> {
        match r {
            Read::Ok(t) => re!(format!("[{JS_SPACE}]+")).split(js_trim(t)).map(str::to_string).collect(),
            Read::Failed(..) => Vec::new(),
        }
    };
    let number_of = |w: Option<&String>| -> Option<f64> { w.filter(|w| re!(r"^[0-9]+(\.[0-9]+)?$").is_match(w)).map(|w| w.parse::<f64>().unwrap_or(f64::NAN)) };
    let (b, s) = (words(&beat), words(&success));
    let (at, pid, last) = (b.first(), b.get(1), s.first());
    if (state(&beat) == "ok" && number_of(at).is_none()) || (state(&success) == "ok" && number_of(last).is_none()) {
        h.vendor().unparsable(source("ticker"), version);
        return json!({ "state": "unreadable", "ticker": null });
    }
    let n = |x: Option<f64>| x.map_or(Value::Null, number);
    let heartbeat = if state(&beat) == "ok" {
        let mut hb = json!({ "at": n(number_of(at)) });
        if let Some(p) = pid {
            hb["pid"] = number_of(Some(p)).map_or(json!(p), number);
        }
        hb
    } else {
        Value::Null
    };
    let record = json!({ "heartbeat": heartbeat, "last_success": if state(&success) == "ok" { json!({ "at": n(number_of(last)) }) } else { Value::Null } });
    let checked = h.vendor().check(source("ticker"), &record, version);
    let checked = if checked.is_null() { json!({}) } else { checked };
    let hb = checked.get("heartbeat");
    json!({ "state": "ok", "ticker": { "at": ms(hb.and_then(|h| h.get("at"))), "pid": num(hb.and_then(|h| h.get("pid"))), "lastSuccess": ms(checked.get("last_success").and_then(|l| l.get("at"))) } })
}

/// The background processes Hermes's terminal tool runs. Never their
/// commands.
pub fn read_processes(h: &dyn ObserverHost, home: &str, version: Option<&str>) -> Value {
    match read_json(h, &join(home, "processes.json"), source("processes"), version, None) {
        Err((state, _)) => json!({ "state": state, "entries": [] }),
        Ok(v) => {
            let entries: Vec<Value> = v
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_object() && num(e.get("pid")).as_f64().is_some_and(|p| p != 0.0) && e.get("pid_scope").is_none_or(|s| s.is_null() || s.as_str() == Some("host")))
                .map(|e| json!({ "pid": e["pid"], "startTime": num(e.get("host_start_time")), "cwd": text(e.get("cwd")), "startedAt": ms(e.get("started_at")), "session": text(e.get("parent_session_id")) }))
                .collect();
            json!({ "state": "ok", "entries": entries })
        }
    }
}

/// Scheduled jobs. Never names, prompts or destinations.
pub fn read_jobs(h: &dyn ObserverHost, home: &str, version: Option<&str>) -> Value {
    match read_json(h, &join(&join(home, "cron"), "jobs.json"), source("jobs"), version, None) {
        Err((state, _)) => json!({ "state": state, "jobs": [] }),
        Ok(v) => {
            let jobs: Vec<Value> = v
                .get("jobs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|j| j.is_object() && !text(j.get("id")).is_null())
                .map(|j| json!({ "id": j["id"], "status": text(j.get("last_status")), "state": text(j.get("state")), "enabled": j.get("enabled") != Some(&Value::Bool(false)), "workdir": text(j.get("workdir")), "lastRunAt": ms(j.get("last_run_at")), "nextRunAt": ms(j.get("next_run_at")) }))
                .collect();
            json!({ "state": "ok", "jobs": jobs })
        }
    }
}

// ---------- a profile's state.db ----------

// The session columns, then what a session says it used and what that cost,
// as Hermes totals it (never its title, system prompt, billing_base_url,
// which names a host, or a task's name). The usage ones are missing in older
// layouts, and none of them is drift.
const SESSION_COLUMNS: [&str; 24] = [
    "id",
    "source",
    "parent_session_id",
    "started_at",
    "ended_at",
    "end_reason",
    "cwd",
    "git_repo_root",
    "last_activity_at",
    "last_activity_description",
    "archived",
    "hidden",
    "model",
    "billing_provider",
    "billing_mode",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "reasoning_tokens",
    "estimated_cost_usd",
    "actual_cost_usd",
    "cost_status",
    "cost_source",
];
// The same by model and provider within a session, summed over the tasks it
// did, so a task's name is never read. Only in layouts from schema 20.
const MODEL_USAGE_COLUMNS: [&str; 11] = [
    "model",
    "billing_provider",
    "billing_mode",
    "input_tokens",
    "output_tokens",
    "cache_read_tokens",
    "cache_write_tokens",
    "reasoning_tokens",
    "estimated_cost_usd",
    "actual_cost_usd",
    "cost_status",
];

// session_model_usage's rows for some sessions (a JSON array of ids), one per
// session, model, provider and billing mode.
fn model_usage_sql(cols: &[String]) -> String {
    let has = |c: &str| cols.iter().any(|x| x == c);
    let mut group = vec!["session_id", "model"];
    group.extend(["billing_provider", "billing_mode"].into_iter().filter(|c| has(c)));
    let mut select: Vec<String> = group.iter().map(ToString::to_string).collect();
    for c in MODEL_USAGE_COLUMNS.iter().filter(|c| !group.contains(c)) {
        if *c == "cost_status" {
            if has(c) {
                select.push("MAX(cost_status) AS cost_status".into());
            }
        } else if has(c) {
            select.push(format!("SUM({c}) AS {c}"));
        }
    }
    format!(
        "SELECT {} FROM session_model_usage WHERE session_id IN (SELECT value FROM json_each(?)) GROUP BY {} ORDER BY session_id, model LIMIT 2000",
        select.join(", "),
        group.join(", ")
    )
}
const MESSAGE_COLUMNS: [&str; 6] = ["id", "session_id", "role", "tool_name", "timestamp", "active"];
const HEARTBEAT_COLUMNS: [&str; 4] = ["pid", "started_at", "last_heartbeat", "profile"];
// The columns XenoNet can't do without: one that's gone is drift.
const NEEDED: [(&str, &[&str]); 2] =
    [("sessions", &["id", "source", "started_at", "ended_at", "cwd"]), ("messages", &["id", "session_id", "role", "tool_name", "timestamp", "tool_calls"])];
// New messages read at once at most; the rest wait for the next look.
const MESSAGES_MAX: usize = 5000;
// On a first read, how far back to start, at most.
const FIRST_ROWS: f64 = 5000.0;

// The path argument of each write_file and patch call in messages
// (after, upTo], taken out by SQLite: a call's other arguments, and its
// message's content, never leave the database. Only objects are opened,
// by json_each's own type, and in a CASE, which SQLite evaluates in order.
fn tool_paths_sql(with_time: bool) -> String {
    format!(
        "
  WITH calls AS (
    SELECT m.id, m.session_id, m.timestamp, tc.type AS kind, tc.value AS call
    FROM messages m, json_each(CASE WHEN json_valid(m.tool_calls) THEN CASE WHEN json_type(m.tool_calls) = 'array' THEN m.tool_calls END END) tc
    WHERE m.id > ? AND m.id <= ?{} AND m.role = 'assistant' AND m.tool_calls IS NOT NULL
  ), named AS (
    SELECT id, session_id, timestamp,
      CASE WHEN kind = 'object' THEN json_extract(call, '$.function.name') END AS tool,
      CASE WHEN kind = 'object' THEN json_extract(call, '$.function.arguments') END AS args
    FROM calls
  )
  SELECT id, session_id, timestamp, tool,
    CASE WHEN json_valid(args) THEN CASE WHEN json_type(args, '$.path') = 'text' THEN json_extract(args, '$.path') END END AS path
  FROM named WHERE tool IN ({}) ORDER BY id",
        if with_time { " AND m.timestamp > ?" } else { "" },
        EDIT_TOOLS.iter().map(|t| format!("'{t}'")).collect::<Vec<_>>().join(", ")
    )
}

fn empty(after_id: &Value) -> Value {
    json!({ "version": null, "sessions": [], "messages": [], "toolPaths": [], "heartbeats": [], "modelUsage": [], "lastId": after_id })
}

fn with_state(state: &str, code: Option<Value>, rest: Value) -> Value {
    let mut o = Map::new();
    o.insert("state".into(), json!(state));
    if let Some(c) = code {
        o.insert("code".into(), c);
    }
    if let Value::Object(r) = rest {
        for (k, v) in r {
            o.insert(k, v);
        }
    }
    Value::Object(o)
}

/// What's new in a profile's session database since the last read. The
/// database is opened read only, queried, and closed.
pub fn read_state_db(h: &dyn ObserverHost, file: &str, after_id: &Value, since_ms: f64, session_ids: &[Value]) -> Value {
    if !h.files().exists(file) {
        return with_state("missing", None, empty(after_id));
    }
    let db = h.db();
    if let Err(f) = db.open(file) {
        let denied = re!(r"(?i)permission|EACCES|EPERM|authoriz").is_match(&f.message);
        return with_state(if denied { "denied" } else { "unreadable" }, Some(code_value(&f)), empty(after_id));
    }
    let rows = |sql: &str, args: Vec<Value>| db.all(sql, &args);
    let row = |sql: &str, args: Vec<Value>| db.get(sql, &args);
    let out = (|| -> Result<Value> {
        let src = source("stateDb");
        let mut has: Vec<(&str, Vec<String>)> = Vec::new();
        for table in ["sessions", "messages", "gateway_heartbeats", "schema_version", "session_model_usage"] {
            let cols = rows(&format!("PRAGMA table_info({table})"), vec![])?.iter().filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_string)).collect();
            has.push((table, cols));
        }
        let cols = |t: &str| has.iter().find(|(n, _)| *n == t).map(|(_, c)| c.clone()).unwrap_or_default();
        let has_col = |t: &str, col: &str| cols(t).iter().any(|c| c == col);
        let version = if has_col("schema_version", "version") {
            let v = row("SELECT max(version) AS v FROM schema_version", vec![])?;
            let v = v.get("v").filter(|v| !v.is_null()).map(js_string).unwrap_or_default();
            clean_version(&v).map(str::to_string)
        } else {
            None
        };
        for (table, needed) in NEEDED {
            if cols(table).is_empty() {
                h.vendor().missing(src, table, version.as_deref());
            } else {
                for col in needed {
                    if !has_col(table, col) {
                        h.vendor().missing(src, &format!("{table}[].{col}"), version.as_deref());
                    }
                }
            }
        }
        if !has_col("sessions", "id") || !has_col("messages", "id") {
            let mut v = with_state("unreadable", Some(json!("not Hermes's layout")), empty(after_id));
            v["version"] = version.clone().map_or(Value::Null, Value::String);
            return Ok(v);
        }
        let since = since_ms / 1000.0;
        let pick = |wanted: &[&str], table: &str| -> Vec<String> { wanted.iter().filter(|c| has_col(table, c)).map(ToString::to_string).collect() };
        // The messages since the last read: from the last one read, or on
        // a first read (or after the file was made again) the recent ones
        // among the last few thousand.
        let max_id = row("SELECT max(id) AS n FROM messages", vec![])?.get("n").and_then(Value::as_f64).unwrap_or(0.0);
        let from = match after_id.as_f64() {
            Some(a) if a <= max_id => a,
            _ => (max_id - FIRST_ROWS).max(0.0),
        };
        let mut msg_cols = pick(&MESSAGE_COLUMNS, "messages");
        if has_col("messages", "tool_calls") {
            msg_cols.push("(tool_calls IS NOT NULL) AS calls".into());
        }
        let timed = has_col("messages", "timestamp");
        let time = if timed { " AND timestamp > ?" } else { "" };
        let mut args = vec![number(from)];
        if timed {
            args.push(number(since));
        }
        let messages = rows(&format!("SELECT {} FROM messages WHERE id > ?{time} ORDER BY id LIMIT {MESSAGES_MAX}", msg_cols.join(", ")), args)?;
        let last_id = match messages.last() {
            Some(m) if messages.len() == MESSAGES_MAX => m["id"].clone(),
            _ => number(max_id),
        };
        let tool_paths = if has_col("messages", "tool_calls") && has_col("messages", "role") {
            let mut args = vec![number(from), last_id.clone()];
            if timed {
                args.push(number(since));
            }
            rows(&tool_paths_sql(timed), args)?
        } else {
            Vec::new()
        };
        // The sessions those belong to, the ones open and active since
        // then, and the ones that ended since then.
        let mut ids: Vec<Value> = Vec::new();
        for id in session_ids
            .iter()
            .cloned()
            .chain(messages.iter().map(|m| m.get("session_id").cloned().unwrap_or(Value::Null)))
            .chain(tool_paths.iter().map(|t| t.get("session_id").cloned().unwrap_or(Value::Null)))
        {
            if id.is_string() && !ids.contains(&id) {
                ids.push(id);
            }
        }
        let active = if has_col("sessions", "last_activity_at") { "COALESCE(last_activity_at, started_at)" } else { "started_at" };
        let ended = has_col("sessions", "ended_at");
        let mut wheres = vec!["id IN (SELECT value FROM json_each(?))".to_string()];
        if ended {
            wheres.push(format!("(ended_at IS NULL AND {active} > ?)"));
            wheres.push("ended_at > ?".into());
        } else {
            wheres.push(format!("{active} > ?"));
        }
        let mut args = vec![json!(xenonet_base::json::stringify(&Value::Array(ids), 0)), number(since)];
        if ended {
            args.push(number(since));
        }
        let sessions =
            rows(&format!("SELECT {} FROM sessions WHERE {} ORDER BY started_at DESC LIMIT 500", pick(&SESSION_COLUMNS, "sessions").join(", "), wheres.join(" OR ")), args)?;
        // Each of those sessions' use by model and provider, summed over its tasks.
        let model_usage = if !sessions.is_empty() && has_col("session_model_usage", "session_id") && has_col("session_model_usage", "model") {
            let ids: Vec<Value> = sessions.iter().map(|s| s.get("id").cloned().unwrap_or(Value::Null)).collect();
            rows(&model_usage_sql(&cols("session_model_usage")), vec![json!(xenonet_base::json::stringify(&Value::Array(ids), 0))])?
        } else {
            Vec::new()
        };
        let heartbeats = if has_col("gateway_heartbeats", "pid") {
            rows(&format!("SELECT {} FROM gateway_heartbeats", pick(&HEARTBEAT_COLUMNS, "gateway_heartbeats").join(", ")), vec![])?
        } else {
            Vec::new()
        };
        let schema_version = match &version {
            Some(v) => json!([{ "version": xenonet_base::text::js_number(Some(&Value::String(v.clone()))) }]),
            None => json!([]),
        };
        let record = json!({ "schema_version": schema_version, "sessions": sessions, "messages": messages, "tool_paths": tool_paths, "gateway_heartbeats": heartbeats, "session_model_usage": model_usage });
        let checked = h.vendor().check(src, &record, version.as_deref());
        let checked = if checked.is_null() { json!({}) } else { checked };
        let arr = |k: &str| checked.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(json!([]));
        // Archived compaction generations are history, not new work.
        let messages: Vec<Value> = arr("messages").as_array().cloned().unwrap_or_default().into_iter().filter(|m| m.get("active").and_then(Value::as_f64) != Some(0.0)).collect();
        Ok(
            json!({ "state": "ok", "version": version, "sessions": arr("sessions"), "messages": messages, "toolPaths": arr("tool_paths"), "heartbeats": arr("gateway_heartbeats"), "modelUsage": arr("session_model_usage"), "lastId": last_id }),
        )
    })();
    db.close();
    match out {
        Ok(v) => v,
        Err(f) => {
            let busy = re!(r"(?i)SQLITE_BUSY|locked").is_match(&f.message);
            with_state(if busy { "busy" } else { "unreadable" }, Some(code_value(&f)), empty(after_id))
        }
    }
}
