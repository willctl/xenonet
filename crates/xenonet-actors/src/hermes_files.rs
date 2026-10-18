//! Hermes Agent's state on this machine, read for metadata only
//! (core/actors/hermes-files.js; docs/design/actors/hermes.md, sections 2
//! and 5): its build stamp, its spawn ledger, and for each profile its
//! gateway's state, its scheduler's ticker, its background processes, its
//! scheduled jobs and its session database. Each source is read against
//! its vendor schema through the lenient readers, so a change in Hermes's
//! formats shows as drift rather than as a quiet observer.
//!
//! What's never read: .env, auth.json, pairing/, mcp-tokens/, the desktop
//! app's connection and token files, logs, memories, cron output, and in
//! state.db anything that holds content or who's on the other end. The
//! database is opened read only for a moment, with named columns in every
//! query; the one thing taken from a tool call's arguments is the path of a
//! file it wrote, which SQLite takes out (json_extract).
//!
//! Every reader answers { state, ... }: ok, missing, denied or unreadable.
//! The files, the folders, the vendor readers and the database are the
//! twin's ("files", "dirs", "vendor", and open/all/get/close).

use crate::registry::value_arg;
use serde_json::{json, Map, Value};
use xenonet_engine::jsre::js;
use xenonet_engine::peer::{list, Calls, Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_schema::drift::clean_version;

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

fn code_of(t: &Thrown) -> Option<&str> {
    t.code.as_deref().and_then(Value::as_str)
}

// What a failed read says, by the error's code.
fn state_of(t: &Thrown) -> &'static str {
    match code_of(t) {
        Some("ENOENT" | "ENOTDIR") => "missing",
        Some("EACCES" | "EPERM") => "denied",
        _ => "unreadable",
    }
}

/// Hermes's times, epoch seconds (floats) or ISO 8601, as epoch
/// milliseconds; None for anything else. A text is read as Date.parse
/// reads it (`date_parse`: the twin's, since a time without an offset is
/// the system zone's).
pub fn ms_of(v: &Value, date_parse: &mut dyn FnMut(&str) -> Option<f64>) -> Option<f64> {
    match v {
        Value::Number(n) => {
            let f = n.as_f64()?;
            (f.is_finite() && f > 0.0).then(|| if f > 1e11 { xenonet_base::units::js_round(f) } else { xenonet_base::units::js_round(f * 1000.0) })
        }
        Value::String(s) if !s.is_empty() => date_parse(s).filter(|t| t.is_finite()),
        _ => None,
    }
}

/// Date.parse, through the twin ("dateParse").
pub fn parse_date(c: &mut dyn Calls) -> impl FnMut(&str) -> Option<f64> + '_ {
    move |s: &str| c.call("", "dateParse", vec![Arg::text(s)]).ok().and_then(|v| v.as_f64())
}

/// Whether a process start Hermes recorded is the one the process table
/// says (to the second): psutil's epoch seconds in the ledger, centiseconds
/// of the epoch in the gateway's and the background processes' records on
/// macOS.
pub fn same_start(recorded: &Value, table_start: &Value, slack: f64) -> bool {
    let Some(r) = recorded.as_f64().filter(|r| r.is_finite()) else { return false };
    let Some(t) = table_start.as_f64().filter(|t| *t != 0.0 && !t.is_nan()) else { return false };
    [r * 1000.0, r * 10.0, r].iter().any(|ms| (ms - t).abs() <= slack)
}

enum Read {
    Ok(String),
    Failed(&'static str, Value),
}

fn read_text(c: &mut dyn Calls, file: &str) -> Read {
    match c.call("files", "readFileSync", vec![Arg::text(file), Arg::text("utf8")]) {
        Ok(v) => Read::Ok(xenonet_engine::store::js_text(&v)),
        Err(t) => Read::Failed(state_of(&t), t.code.as_deref().cloned().unwrap_or(Value::Null)),
    }
}

fn vendor(c: &mut dyn Calls, op: &str, args: Vec<Arg>) -> Result<Arg> {
    c.call("vendor", op, args)
}

fn version_arg(v: Option<&str>) -> Arg {
    v.map_or(Arg::Null, Arg::text)
}

// A JSON file's value, or why it can't be had: (state, code).
type Checked = std::result::Result<Value, (&'static str, Value)>;

// The version a file's own value names.
type VersionOf<'a> = &'a dyn Fn(&Value) -> Option<String>;

// A JSON file read against its source's schema: Ok(value) or Err((state,
// code)).
fn read_json(c: &mut dyn Calls, file: &str, src: &str, version: Option<String>, version_of: Option<VersionOf<'_>>) -> Result<Checked> {
    let text = match read_text(c, file) {
        Read::Ok(t) => t,
        Read::Failed(state, code) => return Ok(Err((state, code))),
    };
    // Hermes reads its files as utf-8-sig, so one may start with a BOM.
    let Ok(value) = xenonet_base::json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)) else {
        vendor(c, "unparsable", vec![Arg::text(src), version_arg(version.as_deref())])?;
        return Ok(Err(("unreadable", json!("not JSON"))));
    };
    let v = match version_of {
        Some(f) => f(&value),
        None => version,
    };
    let checked = vendor(c, "check", vec![Arg::text(src), value_arg(&value), version_arg(v.as_deref())])?;
    Ok(if checked.is_nullish() { Err(("unreadable", json!("not what XenoNet reads"))) } else { Ok(checked.to_json()) })
}

fn num(v: Option<&Value>) -> Value {
    v.filter(|x| x.as_f64().is_some_and(f64::is_finite)).cloned().unwrap_or(Value::Null)
}

fn text(v: Option<&Value>) -> Value {
    v.filter(|x| x.as_str().is_some_and(|s| !s.is_empty())).cloned().unwrap_or(Value::Null)
}

fn ms(c: &mut dyn Calls, v: Option<&Value>) -> Value {
    let mut p = parse_date(c);
    v.and_then(|v| ms_of(v, &mut p)).map_or(Value::Null, xenonet_base::json::number)
}

// ---------- the install ----------

/// The profiles, by name, with their folders: the default one is the root.
pub fn list_profiles(c: &mut dyn Calls, root: &str) -> Result<Value> {
    let mut out = vec![json!({ "name": "default", "home": root })];
    let profiles = join(root, "profiles");
    let mut names: Vec<String> = match c.call("", "dirs", vec![Arg::text(&profiles)]) {
        Ok(l) => list(l).iter().map(xenonet_engine::store::js_text).collect(),
        Err(_) => Vec::new(),
    };
    names.sort_by(|a, b| xenonet_engine::js::utf16_cmp(a, b));
    for name in names {
        if name != "default" && !name.starts_with('.') {
            out.push(json!({ "name": name, "home": join(&profiles, &name) }));
        }
    }
    Ok(Value::Array(out))
}

fn join(a: &str, b: &str) -> String {
    xenonet_base::paths::join(a, b)
}

/// Whether Hermes is installed here: its root, with its checkout or its
/// session database in it.
pub fn installed(c: &mut dyn Calls, root: &str) -> Result<bool> {
    for f in ["hermes-agent", "state.db"] {
        if c.call("files", "existsSync", vec![Arg::text(join(root, f))])?.truthy() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The build stamp: the release this build is of (baseVersion).
pub fn read_stamp(c: &mut dyn Calls, root: &str) -> Result<Value> {
    let file = join(&join(root, "hermes-agent"), "install-stamp.json");
    let of = |v: &Value| {
        v.get("schemaVersion").filter(|s| s.as_f64().is_some_and(f64::is_finite)).and_then(|s| clean_version(&xenonet_engine::js::template(Some(s))).map(str::to_string))
    };
    Ok(match read_json(c, &file, source("stamp"), None, Some(&of))? {
        Ok(v) => json!({ "state": "ok", "version": v.get("baseVersion").and_then(Value::as_str).filter(|s| !s.is_empty()).and_then(clean_version) }),
        Err((state, _)) => json!({ "state": state, "version": null }),
    })
}

/// Hermes's long-lived processes as it keeps them: { pid, createTime,
/// purpose, port, profile, spawnerPid }. Never argv.
pub fn read_ledger(c: &mut dyn Calls, root: &str, version: Option<String>) -> Result<Value> {
    Ok(match read_json(c, &join(root, "spawn-ledger.json"), source("ledger"), version, None)? {
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
    })
}

/// The gateway as its state file says, or null with no file.
pub fn read_gateway(c: &mut dyn Calls, home: &str, version: Option<String>) -> Result<Value> {
    Ok(match read_json(c, &join(home, "gateway_state.json"), source("gateway"), version, None)? {
        Err((state, _)) => json!({ "state": state, "gateway": null }),
        Ok(g) => {
            let platforms: Vec<Value> = match g.get("platforms") {
                Some(Value::Object(p)) => p.iter().filter(|(_, x)| x.is_object()).map(|(name, x)| json!({ "name": name, "state": text(x.get("state")) })).collect(),
                _ => Vec::new(),
            };
            let active = num(g.get("active_agents"));
            json!({
                "state": "ok",
                "gateway": { "state": text(g.get("gateway_state")), "pid": num(g.get("pid")), "startTime": num(g.get("start_time")), "updatedAt": ms(c, g.get("updated_at")), "activeAgents": if active.is_null() { json!(0) } else { active }, "exitReason": text(g.get("exit_reason")), "platforms": platforms },
            })
        }
    })
}

/// The scheduler's ticker: { at, pid, lastSuccess }, or null.
pub fn read_ticker(c: &mut dyn Calls, home: &str, version: Option<String>) -> Result<Value> {
    let cron = join(home, "cron");
    let beat = read_text(c, &join(&cron, "ticker_heartbeat"));
    let success = read_text(c, &join(&cron, "ticker_last_success"));
    let state = |r: &Read| match r {
        Read::Ok(_) => "ok",
        Read::Failed(s, _) => s,
    };
    if let Some(f) = [&beat, &success].into_iter().find(|r| !matches!(state(r), "ok" | "missing")) {
        return Ok(json!({ "state": state(f), "ticker": null }));
    }
    if state(&beat) == "missing" && state(&success) == "missing" {
        return Ok(json!({ "state": "missing", "ticker": null }));
    }
    // "<epoch seconds> <pid>" and "<epoch seconds>".
    let words = |r: &Read| -> Vec<String> {
        match r {
            Read::Ok(t) => js(r"\s+", false).split(xenonet_base::text::js_trim(t)).map(str::to_string).collect(),
            Read::Failed(..) => Vec::new(),
        }
    };
    let number = |w: Option<&String>| -> Option<f64> { w.filter(|w| js(r"^\d+(\.\d+)?$", false).is_match(w)).map(|w| w.parse::<f64>().unwrap_or(f64::NAN)) };
    let (b, s) = (words(&beat), words(&success));
    let (at, pid, last) = (b.first(), b.get(1), s.first());
    if (state(&beat) == "ok" && number(at).is_none()) || (state(&success) == "ok" && number(last).is_none()) {
        vendor(c, "unparsable", vec![Arg::text(source("ticker")), version_arg(version.as_deref())])?;
        return Ok(json!({ "state": "unreadable", "ticker": null }));
    }
    let n = |x: Option<f64>| x.map_or(Value::Null, xenonet_base::json::number);
    let heartbeat = if state(&beat) == "ok" {
        let mut h = json!({ "at": n(number(at)) });
        if let Some(p) = pid {
            h["pid"] = number(Some(p)).map_or(json!(p), xenonet_base::json::number);
        }
        h
    } else {
        Value::Null
    };
    let record = json!({ "heartbeat": heartbeat, "last_success": if state(&success) == "ok" { json!({ "at": n(number(last)) }) } else { Value::Null } });
    let checked = vendor(c, "check", vec![Arg::text(source("ticker")), value_arg(&record), version_arg(version.as_deref())])?.to_json();
    let checked = if checked.is_null() { json!({}) } else { checked };
    let hb = checked.get("heartbeat");
    Ok(
        json!({ "state": "ok", "ticker": { "at": ms(c, hb.and_then(|h| h.get("at"))), "pid": num(hb.and_then(|h| h.get("pid"))), "lastSuccess": ms(c, checked.get("last_success").and_then(|l| l.get("at"))) } }),
    )
}

/// The background processes Hermes's terminal tool runs. Never their
/// commands.
pub fn read_processes(c: &mut dyn Calls, home: &str, version: Option<String>) -> Result<Value> {
    Ok(match read_json(c, &join(home, "processes.json"), source("processes"), version, None)? {
        Err((state, _)) => json!({ "state": state, "entries": [] }),
        Ok(v) => {
            let entries: Vec<Value> = v
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|e| e.is_object() && num(e.get("pid")).as_f64().is_some_and(|p| p != 0.0) && e.get("pid_scope").is_none_or(|s| s.is_null() || s.as_str() == Some("host")))
                .map(|e| json!({ "pid": e["pid"], "startTime": num(e.get("host_start_time")), "cwd": text(e.get("cwd")), "startedAt": ms(c, e.get("started_at")), "session": text(e.get("parent_session_id")) }))
                .collect();
            json!({ "state": "ok", "entries": entries })
        }
    })
}

/// Scheduled jobs. Never names, prompts or destinations.
pub fn read_jobs(c: &mut dyn Calls, home: &str, version: Option<String>) -> Result<Value> {
    Ok(match read_json(c, &join(&join(home, "cron"), "jobs.json"), source("jobs"), version, None)? {
        Err((state, _)) => json!({ "state": state, "jobs": [] }),
        Ok(v) => {
            let jobs: Vec<Value> = v
                .get("jobs")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|j| j.is_object() && !text(j.get("id")).is_null())
                .map(|j| json!({ "id": j["id"], "status": text(j.get("last_status")), "state": text(j.get("state")), "enabled": j.get("enabled") != Some(&Value::Bool(false)), "workdir": text(j.get("workdir")), "lastRunAt": ms(c, j.get("last_run_at")), "nextRunAt": ms(c, j.get("next_run_at")) }))
                .collect();
            json!({ "state": "ok", "jobs": jobs })
        }
    })
}

// ---------- a profile's state.db ----------

const SESSION_COLUMNS: [&str; 12] =
    ["id", "source", "parent_session_id", "started_at", "ended_at", "end_reason", "cwd", "git_repo_root", "last_activity_at", "last_activity_description", "archived", "hidden"];
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

fn rows(c: &mut dyn Calls, sql: &str, args: Vec<Arg>) -> Result<Vec<Value>> {
    Ok(list(c.call("db", "all", vec![Arg::text(sql), Arg::List(args)])?).iter().map(Arg::to_json).collect())
}

fn row(c: &mut dyn Calls, sql: &str, args: Vec<Arg>) -> Result<Value> {
    Ok(c.call("db", "get", vec![Arg::text(sql), Arg::List(args)])?.to_json())
}

fn empty(after_id: &Value) -> Value {
    json!({ "version": null, "sessions": [], "messages": [], "toolPaths": [], "heartbeats": [], "lastId": after_id })
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

/// What's new in a profile's session database since the last read.
/// Through the twin's "db": open(file) opens the file read only,
/// all/get(sql, args) query it, close() closes it.
pub fn read_state_db(c: &mut dyn Calls, file: &str, after_id: &Value, since_ms: &Value, session_ids: &[Value]) -> Result<Value> {
    if !c.call("files", "existsSync", vec![Arg::text(file)])?.truthy() {
        return Ok(with_state("missing", None, empty(after_id)));
    }
    if let Err(t) = c.call("db", "open", vec![Arg::text(file)]) {
        let denied = js(r"permission|EACCES|EPERM|authoriz", true).is_match(&t.message);
        return Ok(with_state(if denied { "denied" } else { "unreadable" }, Some(t.code.as_deref().cloned().unwrap_or(Value::Null)), empty(after_id)));
    }
    let out = (|| -> Result<Value> {
        let src = source("stateDb");
        let mut has: Vec<(&str, Vec<String>)> = Vec::new();
        for table in ["sessions", "messages", "gateway_heartbeats", "schema_version"] {
            let cols = rows(c, &format!("PRAGMA table_info({table})"), vec![])?.iter().filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_string)).collect();
            has.push((table, cols));
        }
        let cols = |t: &str| has.iter().find(|(n, _)| *n == t).map(|(_, c)| c.clone()).unwrap_or_default();
        let has_col = |t: &str, col: &str| cols(t).iter().any(|c| c == col);
        let version = if has_col("schema_version", "version") {
            let v = row(c, "SELECT max(version) AS v FROM schema_version", vec![])?;
            let v = v.get("v").filter(|v| !v.is_null()).map(|v| xenonet_engine::js::template(Some(v))).unwrap_or_default();
            clean_version(&v).map(str::to_string)
        } else {
            None
        };
        for (table, needed) in NEEDED {
            if cols(table).is_empty() {
                vendor(c, "missing", vec![Arg::text(src), Arg::text(table), version_arg(version.as_deref())])?;
            } else {
                for col in needed {
                    if !has_col(table, col) {
                        vendor(c, "missing", vec![Arg::text(src), Arg::text(format!("{table}[].{col}")), version_arg(version.as_deref())])?;
                    }
                }
            }
        }
        if !has_col("sessions", "id") || !has_col("messages", "id") {
            let mut v = with_state("unreadable", Some(json!("not Hermes's layout")), empty(after_id));
            v["version"] = version.clone().map_or(Value::Null, Value::String);
            return Ok(v);
        }
        let since = since_ms.as_f64().unwrap_or(0.0) / 1000.0;
        let pick = |wanted: &[&str], table: &str| -> Vec<String> { wanted.iter().filter(|c| has_col(table, c)).map(ToString::to_string).collect() };
        // The messages since the last read: from the last one read, or on
        // a first read (or after the file was made again) the recent ones
        // among the last few thousand.
        let max_id = row(c, "SELECT max(id) AS n FROM messages", vec![])?.get("n").and_then(Value::as_f64).unwrap_or(0.0);
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
        let mut args = vec![Arg::Num(from)];
        if timed {
            args.push(Arg::Num(since));
        }
        let messages = rows(c, &format!("SELECT {} FROM messages WHERE id > ?{time} ORDER BY id LIMIT {MESSAGES_MAX}", msg_cols.join(", ")), args)?;
        let last_id = match messages.last() {
            Some(m) if messages.len() == MESSAGES_MAX => m["id"].clone(),
            _ => xenonet_base::json::number(max_id),
        };
        let tool_paths = if has_col("messages", "tool_calls") && has_col("messages", "role") {
            let mut args = vec![Arg::Num(from), value_arg(&last_id)];
            if timed {
                args.push(Arg::Num(since));
            }
            rows(c, &tool_paths_sql(timed), args)?
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
        let mut args = vec![Arg::text(xenonet_base::json::stringify(&Value::Array(ids), 0)), Arg::Num(since)];
        if ended {
            args.push(Arg::Num(since));
        }
        let sessions =
            rows(c, &format!("SELECT {} FROM sessions WHERE {} ORDER BY started_at DESC LIMIT 500", pick(&SESSION_COLUMNS, "sessions").join(", "), wheres.join(" OR ")), args)?;
        let heartbeats = if has_col("gateway_heartbeats", "pid") {
            rows(c, &format!("SELECT {} FROM gateway_heartbeats", pick(&HEARTBEAT_COLUMNS, "gateway_heartbeats").join(", ")), vec![])?
        } else {
            Vec::new()
        };
        let schema_version = match &version {
            Some(v) => json!([{ "version": xenonet_base::text::js_number(Some(&Value::String(v.clone()))) }]),
            None => json!([]),
        };
        let record = json!({ "schema_version": schema_version, "sessions": sessions, "messages": messages, "tool_paths": tool_paths, "gateway_heartbeats": heartbeats });
        let checked = vendor(c, "check", vec![Arg::text(src), value_arg(&record), version_arg(version.as_deref())])?.to_json();
        let checked = if checked.is_null() { json!({}) } else { checked };
        let arr = |k: &str| checked.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(json!([]));
        // Archived compaction generations are history, not new work.
        let messages: Vec<Value> = arr("messages").as_array().cloned().unwrap_or_default().into_iter().filter(|m| m.get("active").and_then(Value::as_f64) != Some(0.0)).collect();
        Ok(
            json!({ "state": "ok", "version": version, "sessions": arr("sessions"), "messages": messages, "toolPaths": arr("tool_paths"), "heartbeats": arr("gateway_heartbeats"), "lastId": last_id }),
        )
    })();
    let _ = c.call("db", "close", vec![]);
    match out {
        Ok(v) => Ok(v),
        Err(t) => {
            let busy = js(r"SQLITE_BUSY|locked", true).is_match(&t.message);
            Ok(with_state(if busy { "busy" } else { "unreadable" }, Some(t.code.as_deref().cloned().unwrap_or(Value::Null)), empty(after_id)))
        }
    }
}
