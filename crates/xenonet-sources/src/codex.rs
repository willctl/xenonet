//! core/codex.js: Codex's threads, projects and rollouts.
//!
//! What a `codex app-server` says over stdio (its lines, the requests it
//! makes of XenoNet, its answers and errors), a thread as a session, a
//! thread's items as the console shows them, from the app-server or from
//! the thread's rollout, Codex desktop's projects from its state database,
//! and the pages of a thread. Spawning the app-server, the requests'
//! promises and time limits, and the lenient checks of what it answers
//! (vendor.js's readers) stay with the caller.

use crate::lines::{opt_prop, prop, CodexLine, Js, TypeError};
use crate::live::command_text;
use crate::record::{Obj, F};
use crate::session::{CodexSession, Parent, ParentKind};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Map, Value};
use xenonet_base::text::{js_number, js_string, js_trim, truthy};
use xenonet_base::{json, time, units};

/// The kinds of thread the listing asks for.
pub const SOURCE_KINDS: [&str; 9] = ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther"];

/// The state database XenoNet reads, and its format's version.
pub const STATE_DB: u32 = 5;

pub const REQUEST_TIMEOUT_MS: f64 = 20000.0;

pub(crate) fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

/// `${v}` in a template: undefined as "undefined".
pub(crate) fn tpl(v: Js<'_>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

/// `Object.keys(v)[0]`: an object's first key, an array's first index.
fn first_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => m.keys().next().cloned(),
        Value::Array(a) if !a.is_empty() => Some("0".into()),
        Value::String(s) if !s.is_empty() => Some("0".into()),
        _ => None,
    }
}

fn is_object(v: Js<'_>) -> bool {
    matches!(v, Some(Value::Object(_) | Value::Array(_)))
}

/// excerpt(s, n) on any value, as units.js does it: a long string cut and
/// said so, nothing as "", and anything else as it is.
pub(crate) fn excerpt(s: Js<'_>, n: usize) -> F {
    match s {
        None | Some(Value::Null) => F::s(""),
        Some(Value::String(t)) => F::s(units::excerpt(t, n)),
        Some(Value::Array(a)) if a.len() > n => F::s(format!("{}\n… ({} more chars)", js_string(&Value::Array(a[..n].to_vec())), a.len() - n)),
        Some(v) => F::V(v.clone()),
    }
}

/// excerpt(JSON.stringify(v, null, 2), n): JSON.stringify of nothing is
/// undefined, which excerpt makes "".
pub(crate) fn excerpt_json(v: Js<'_>, n: usize) -> F {
    match v {
        None => F::s(""),
        Some(v) => F::s(units::excerpt(&json::stringify(v, 2), n)),
    }
}

// When a thread was made or last ran, in ms: seconds when it's under 1e12,
// and a time written as a string read as one.
fn to_ms(t: Js<'_>) -> F {
    match t {
        None | Some(Value::Null) => F::null(),
        Some(Value::String(s)) => match time::parse_iso(s) {
            Some(ms) if ms != 0.0 => F::V(json::number(ms)),
            _ => F::null(),
        },
        Some(v) => {
            let n = js_number(Some(v));
            if n < 1e12 {
                let ms = n * 1000.0;
                if ms.is_finite() {
                    F::V(json::number(ms))
                } else {
                    F::Num(ms)
                }
            } else {
                F::V(v.clone())
            }
        }
    }
}

/// What kind of agent ran a thread (subAgentOf): a spawned sub-agent
/// records its parent and nickname; guardian checks, reviews and
/// compaction are background work.
pub fn sub_agent_of(t: Js<'_>) -> Result<(Value, Value, Value), TypeError> {
    let source = prop(t, "source")?;
    let sub = if truthy(source) && is_object(source) { prop(source, "subAgent")? } else { None };
    let spawn = if truthy(sub) && is_object(sub) { nn(prop(sub, "thread_spawn")?) } else { None };
    let kind = if truthy(spawn) {
        Value::String("spawned".into())
    } else {
        match sub {
            None | Some(Value::Null) => Value::Null,
            Some(Value::String(s)) => Value::String(s.clone()),
            Some(v) => match nn(prop(sub, "other")?) {
                Some(o) => o.clone(),
                None => Value::String(first_key(v).unwrap_or_else(|| "other".into())),
            },
        }
    };
    let parent = nn(opt_prop(spawn, "parent_thread_id")).cloned().unwrap_or(Value::Null);
    let nickname = nn(opt_prop(spawn, "agent_nickname")).cloned().unwrap_or(Value::Null);
    Ok((parent, nickname, kind))
}

/// A thread as a session (threadToSession).
pub fn thread_to_session(t: Js<'_>, archived: bool) -> Result<Obj, TypeError> {
    let get = |k: &str| prop(t, k);
    let status = match get("status")? {
        Some(Value::String(s)) => Value::String(s.clone()),
        st => nn(opt_prop(st, "type")).cloned().unwrap_or(Value::Null),
    };
    let (parent, nickname, kind) = sub_agent_of(t)?;
    let parent_thread = match nn(get("parentThreadId")?) {
        Some(p) => p.clone(),
        None => parent,
    };
    let preview = get("preview")?;
    let title = match get("name")? {
        Some(n) if truthy(Some(n)) => n.clone(),
        _ => {
            let first = match nn(preview) {
                None => None,
                Some(Value::String(p)) => Some(Value::String(p.split('\n').next().unwrap_or("").to_string())),
                Some(_) => return Err(TypeError("t.preview?.split is not a function".into())),
            };
            first.filter(|f| truthy(Some(f))).unwrap_or_else(|| Value::String("(untitled thread)".into()))
        }
    };
    let git = get("gitInfo")?;
    let origin = nn(opt_prop(git, "originUrl")).or(nn(opt_prop(git, "origin_url"))).cloned().unwrap_or(Value::Null);
    let source = get("source")?;
    let source_f = match source {
        Some(Value::String(s)) => F::s(s.clone()),
        Some(v) if truthy(Some(v)) => first_key(v).map_or(F::Undef, F::s),
        _ => F::null(),
    };
    let forked = get("forkedFromId")?;
    let parent = if truthy(forked) {
        Some(Parent { id: format!("codex:{}", tpl(forked)), kind: ParentKind::Fork })
    } else if truthy(Some(&parent_thread)) {
        Some(Parent { id: format!("codex:{}", js_string(&parent_thread)), kind: ParentKind::Subagent })
    } else {
        None
    };
    let nick = match nn(get("agentNickname")?) {
        Some(n) => n.clone(),
        None => nickname,
    };
    let background =
        source.and_then(Value::as_str) == Some("exec") || get("originator")?.and_then(Value::as_str) == Some("codex_exec") || (!kind.is_null() && kind.as_str() != Some("spawned"));
    let recency = nn(get("recencyAt")?).or(get("updatedAt")?);
    let or_null = |v: Js<'_>| nn(v).cloned().unwrap_or(Value::Null);
    Ok(CodexSession {
        raw_id: get("id")?.cloned(),
        title,
        preview: or_null(preview),
        cwd: or_null(get("cwd")?),
        created_at: to_ms(get("createdAt")?),
        updated_at: to_ms(recency),
        archived,
        status,
        model: or_null(get("model")?),
        provider: or_null(get("modelProvider")?),
        effort: or_null(get("reasoningEffort")?),
        branch: or_null(opt_prop(git, "branch")),
        origin,
        source: source_f,
        originator: or_null(get("originator")?),
        native_project_id: or_null(get("projectId")?),
        parent,
        agent_nickname: nick,
        agent_kind: kind,
        background,
    }
    .record())
}

// ---------- a thread's items ----------

// What a message's content says: its parts' text, an image as "[image]".
fn text_of(content: Js<'_>) -> String {
    match content {
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for c in parts {
                let text = nn(opt_prop(Some(c), "text"));
                out.push(match text {
                    Some(t) => js_string(t),
                    None => {
                        let ty = opt_prop(Some(c), "type").and_then(Value::as_str);
                        (if matches!(ty, Some("image" | "localImage" | "local_image")) { "[image]" } else { "" }).to_string()
                    }
                });
            }
            out.join("\n")
        }
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

// [...a, ...b]: what spreading a value gives, or the TypeError JS throws.
fn spread(v: Js<'_>, what: &str) -> Result<Vec<Value>, TypeError> {
    match nn(v) {
        None => Ok(Vec::new()),
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::String(s)) => Ok(s.chars().map(|c| Value::String(c.to_string())).collect()),
        Some(_) => Err(TypeError(format!("{what} is not iterable"))),
    }
}

// Reasoning's parts joined: a string as it is, else its text.
fn reasoning(parts: Vec<Value>, optional: bool) -> Result<String, TypeError> {
    let mut out = Vec::with_capacity(parts.len());
    for x in &parts {
        out.push(match x {
            Value::String(s) => s.clone(),
            _ => {
                let t = if optional { opt_prop(Some(x), "text") } else { prop(Some(x), "text")? };
                nn(t).map_or_else(String::new, js_string)
            }
        });
    }
    Ok(out.join("\n"))
}

fn base(id: F, ts: F, kind: Js<'_>) -> Obj {
    vec![("id", id), ("ts", ts), ("kind", F::of(kind))]
}

fn with(mut o: Obj, fields: Obj) -> Obj {
    o.extend(fields);
    o
}

// error ?? result?.content ?? result ?? arguments
fn mcp_text(it: Js<'_>) -> Result<F, TypeError> {
    let result = prop(it, "result")?;
    let v = nn(prop(it, "error")?).or(nn(opt_prop(result, "content"))).or(nn(result)).or(prop(it, "arguments")?);
    Ok(excerpt_json(v, 4000))
}

/// An app-server's item as the console shows it (normalizeItem).
pub fn normalize_item(entry: Js<'_>) -> Result<Obj, TypeError> {
    static EMPTY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(Map::new()));
    let it = nn(prop(entry, "item")?).or(Some(&*EMPTY));
    let ts = nn(prop(entry, "startedAtMs")?).or(nn(prop(entry, "completedAtMs")?));
    let get = |k: &str| prop(it, k);
    let ty = get("type")?;
    let b = base(F::of(get("id")?), F::or_null(ts), ty);
    Ok(match ty.and_then(Value::as_str) {
        Some("userMessage") => with(b, vec![("role", F::s("user")), ("text", F::s(text_of(get("content")?)))]),
        Some("agentMessage") => with(b, vec![("role", F::s("assistant")), ("text", F::V(nn(get("text")?).cloned().unwrap_or(Value::String(String::new()))))]),
        Some("reasoning") => {
            let mut parts = spread(get("summary")?, "it.summary")?;
            parts.extend(spread(get("content")?, "it.content")?);
            with(b, vec![("role", F::s("reasoning")), ("text", F::s(reasoning(parts, false)?))])
        }
        Some("plan") => with(b, vec![("role", F::s("assistant")), ("title", F::s("plan")), ("text", F::V(nn(get("text")?).cloned().unwrap_or(Value::String(String::new()))))]),
        Some("commandExecution") => {
            let code = get("exitCode")?;
            let meta = nn(code).map_or(F::null(), |c| F::s(format!("exit {}", js_string(c))));
            with(
                b,
                vec![
                    ("role", F::s("tool")),
                    ("title", F::of(get("command")?)),
                    ("status", F::of(get("status")?)),
                    ("meta", meta),
                    ("text", excerpt(get("aggregatedOutput")?, 4000)),
                ],
            )
        }
        Some("mcpToolCall") => {
            let title = format!("{}.{}", tpl(get("server")?), tpl(get("tool")?));
            let text = mcp_text(it)?;
            with(b, vec![("role", F::s("tool")), ("title", F::s(title)), ("status", F::of(get("status")?)), ("text", text)])
        }
        Some("dynamicToolCall") => {
            with(b, vec![("role", F::s("tool")), ("title", F::of(get("tool")?)), ("status", F::of(get("status")?)), ("text", excerpt_json(get("arguments")?, 4000))])
        }
        Some("fileChange") => {
            let changes = get("changes")?;
            let n = match nn(changes) {
                None => F::V(json::number(0.0)),
                Some(Value::Array(a)) => F::V(json::number(a.len() as f64)),
                Some(_) => return Err(TypeError("(it.changes ?? []).map is not a function".into())),
            };
            let mut lines = Vec::new();
            for c in nn(changes).and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
                let kind = prop(Some(c), "kind")?;
                let k = nn(opt_prop(kind, "type")).or(nn(kind)).map_or_else(String::new, js_string);
                lines.push(format!("{k} {}", tpl(prop(Some(c), "path")?)));
            }
            let count = match n {
                F::V(v) => js_string(&v),
                _ => "0".into(),
            };
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("edit {count} file(s)"))), ("status", F::of(get("status")?)), ("text", F::s(lines.join("\n")))])
        }
        Some("webSearch") => {
            let q = nn(get("query")?).map_or_else(String::new, js_string);
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("search: {q}"))), ("text", F::s(""))])
        }
        Some("contextCompaction") => with(b, vec![("role", F::s("system")), ("text", F::s("context compacted"))]),
        _ => with(b, vec![("role", F::s("system")), ("title", F::of(ty)), ("text", excerpt_json(it, 1500))]),
    })
}

/// A page of the app-server's items, oldest first.
pub fn normalize_items(data: Js<'_>) -> Result<Vec<Obj>, TypeError> {
    let items = match nn(data) {
        None => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err(TypeError("(page.data ?? []).map is not a function".into())),
    };
    let mut out = items.iter().map(|e| normalize_item(Some(e))).collect::<Result<Vec<_>, _>>()?;
    out.reverse();
    Ok(out)
}

// A finished item as a rollout writes it (event_msg's item_completed: the
// app-server's items, in Codex's own names), as the console shows it.
fn rollout_item(it: &Value, ts: F, n: usize) -> Result<Obj, TypeError> {
    let it = Some(it);
    let get = |k: &str| prop(it, k);
    let ty = get("type")?;
    let id = match nn(get("id")?) {
        Some(v) => F::V(v.clone()),
        None => F::s(format!("item-{n}")),
    };
    let b = base(id, ts, ty);
    Ok(match ty.and_then(Value::as_str) {
        Some("UserMessage") => with(b, vec![("role", F::s("user")), ("text", F::s(text_of(get("content")?)))]),
        Some("AgentMessage") => with(b, vec![("role", F::s("assistant")), ("text", F::s(text_of(get("content")?)))]),
        Some("Reasoning") => {
            let mut parts = spread(get("summary_text")?, "it.summary_text")?;
            parts.extend(spread(get("raw_content")?, "it.raw_content")?);
            with(b, vec![("role", F::s("reasoning")), ("text", F::s(reasoning(parts, true)?))])
        }
        Some("CommandExecution") => {
            let code = get("exit_code")?;
            let meta = nn(code).map_or(F::null(), |c| F::s(format!("exit {}", js_string(c))));
            with(
                b,
                vec![
                    ("role", F::s("tool")),
                    ("title", F::s(command_text(get("command")?))),
                    ("status", F::of(get("status")?)),
                    ("meta", meta),
                    ("text", excerpt(get("aggregated_output")?, 4000)),
                ],
            )
        }
        Some("McpToolCall") => {
            let title = format!("{}.{}", tpl(get("server")?), tpl(get("tool")?));
            let text = mcp_text(it)?;
            with(b, vec![("role", F::s("tool")), ("title", F::s(title)), ("status", F::of(get("status")?)), ("text", text)])
        }
        Some("FileChange") => {
            // Object.entries(it.changes ?? {})
            let entries: Vec<(String, Value)> = match nn(get("changes")?) {
                Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Some(Value::Array(a)) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
                Some(Value::String(s)) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::String(c.to_string()))).collect(),
                _ => Vec::new(),
            };
            let text: Vec<String> = entries.iter().map(|(p, c)| format!("{} {p}", nn(opt_prop(Some(c), "type")).map_or_else(String::new, js_string))).collect();
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("edit {} file(s)", entries.len()))), ("status", F::of(get("status")?)), ("text", F::s(text.join("\n")))])
        }
        Some("WebSearch") => {
            let q = get("query")?.filter(|q| truthy(Some(q))).or_else(|| opt_prop(get("action").ok().flatten(), "query").filter(|q| truthy(Some(q))));
            let q = q.map_or_else(String::new, js_string);
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("search: {q}"))), ("text", F::s(""))])
        }
        Some("ContextCompaction") => with(b, vec![("role", F::s("system")), ("text", F::s("context compacted"))]),
        _ => with(b, vec![("role", F::s("system")), ("title", F::of(ty)), ("text", excerpt_json(it, 1500))]),
    })
}

fn tool_call(t: &str, output: bool) -> bool {
    let t = if output {
        match t.strip_suffix("_output") {
            Some(t) => t,
            None => return false,
        }
    } else {
        t
    };
    matches!(t, "function_call" | "custom_tool_call" | "local_shell_call")
}

/// A thread's items from its rollout, oldest first, as its lines are read:
/// its finished items, each at the time it started (or finished, or was
/// written). A rollout with none, as `codex exec` writes them, has the
/// older lines: what you said and the agent said as events, and each call
/// and its output, kept only until a finished item turns up.
#[derive(Default, Clone, Debug)]
pub struct RolloutEntries {
    items: Vec<Obj>,
    older: Vec<Obj>,
}

impl RolloutEntries {
    pub fn list(&self) -> &[Obj] {
        if self.items.is_empty() {
            &self.older
        } else {
            &self.items
        }
    }

    /// A rollout's line (its record, as lines.js read it).
    pub fn add(&mut self, raw: Value) -> Result<(), TypeError> {
        let o = CodexLine { raw };
        let p = o.payload()?.clone();
        let p = Some(&p);
        let ts = o.ts()?;
        let at = if ts.is_finite() { F::V(json::number(ts)) } else { F::null() };
        let kind = o.kind()?.cloned();
        let is = |k: &str| kind.as_ref().and_then(Value::as_str) == Some(k);
        let ptype = prop(p, "type")?;
        let item = prop(p, "item")?;
        if is("event_msg") && ptype.and_then(Value::as_str) == Some("item_completed") && truthy(item) && is_object(item) {
            let when = match nn(prop(p, "started_at_ms")?).or(nn(prop(p, "completed_at_ms")?)) {
                Some(v) => F::V(v.clone()),
                None => at,
            };
            let entry = rollout_item(item.expect("checked truthy above"), when, self.items.len())?;
            self.items.push(entry);
            self.older.clear();
            return Ok(());
        }
        if !self.items.is_empty() {
            return Ok(());
        }
        let id = match nn(prop(p, "id")?).or(nn(prop(p, "call_id")?)) {
            Some(v) => F::V(v.clone()),
            None => F::s(format!("item-{}", self.older.len())),
        };
        let b = base(id, at, ptype);
        let pt = tpl(ptype);
        if is("event_msg") && (pt == "user_message" || pt == "agent_message") && ptype.is_some_and(Value::is_string) {
            let text = nn(prop(p, "message")?).map_or_else(String::new, js_string);
            self.older.push(with(b, vec![("role", F::s(if pt == "user_message" { "user" } else { "assistant" })), ("text", F::s(text))]));
        } else if is("response_item") && tool_call(&pt, false) {
            let input = nn(prop(p, "input")?).or(nn(prop(p, "arguments")?)).or(nn(prop(p, "action")?));
            let text = match input {
                Some(Value::String(s)) => F::s(units::excerpt(s, 2500)),
                Some(v) => F::s(units::excerpt(&json::stringify(v, 2), 2500)),
                None => F::s("null"),
            };
            let title = nn(prop(p, "name")?).or(ptype).cloned().map_or(F::Undef, F::V);
            self.older.push(with(b, vec![("role", F::s("tool")), ("title", title), ("text", text)]));
        } else if is("response_item") && tool_call(&pt, true) {
            let text = match prop(p, "output")? {
                Some(Value::String(s)) => F::s(units::excerpt(s, 4000)),
                out => F::s(units::excerpt(&json::stringify(nn(out).unwrap_or(&Value::Null), 2), 4000)),
            };
            self.older.push(with(b, vec![("role", F::s("tool")), ("title", F::s("result")), ("text", text)]));
        }
        Ok(())
    }
}

// `xs.slice(start, end)`'s index, as JS reads a number.
pub(crate) fn slice_index(i: f64, len: usize) -> usize {
    if i.is_nan() {
        return 0;
    }
    let i = i.trunc();
    if i < 0.0 {
        (len as f64 + i).max(0.0) as usize
    } else {
        i.min(len as f64) as usize
    }
}

/// A page of `len` entries, newest first: the range to show, and the
/// cursor further back. `cursor` ("`rollout:<n>`") counts the items already
/// shown from the end.
pub fn rollout_page(len: usize, cursor: Js<'_>, limit: f64) -> (usize, usize, Option<String>) {
    let c = match nn(cursor) {
        Some(v) => js_string(v),
        None => String::new(),
    };
    let shown = c.strip_prefix("rollout:").filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).map_or(0.0, |d| d.parse::<f64>().unwrap_or(0.0));
    let end = (len as f64 - shown).max(0.0);
    let start = js_max0(end - limit);
    let (a, b) = (slice_index(start, len), slice_index(end, len));
    let cursor = (start > 0.0).then(|| format!("rollout:{}", json::number_string(len as f64 - start)));
    (a, b.max(a), cursor)
}

/// Math.max(0, x): NaN stays NaN.
pub(crate) fn js_max0(x: f64) -> f64 {
    if x.is_nan() {
        x
    } else {
        x.max(0.0)
    }
}

/// Whether the app-server's error says it can't list a thread's items:
/// an unknown method, or words to say it isn't supported.
pub fn says_unsupported(rpc: Js<'_>) -> bool {
    if !truthy(rpc) {
        return false;
    }
    if matches!(opt_prop(rpc, "code"), Some(Value::Number(n)) if n.as_f64() == Some(-32601.0)) {
        return true;
    }
    let m = nn(opt_prop(rpc, "message")).map_or_else(String::new, js_string).to_ascii_lowercase();
    ["not supported", "not yet supported", "unsupported", "not implemented", "unknown method", "method not found"].iter().any(|w| m.contains(w))
}

// ---------- the app-server's stdio ----------

/// What a line from the app-server is.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A request of its own (an approval): XenoNet declines, with this line.
    Decline(String),
    /// An answer to request `id`: its result, or its error's message and
    /// the error itself.
    Answer { id: Value, result: Box<F>, error: Option<Box<(String, Value)>> },
    /// A notification, or what isn't JSON.
    Other,
}

/// One app-server's stdout, read into lines.
#[derive(Default, Clone, Debug)]
pub struct Stdout {
    buf: String,
}

impl Stdout {
    /// What a chunk of stdout completes: each whole line, trimmed, that has
    /// something in it, as a message.
    pub fn take(&mut self, chunk: &str) -> Vec<Message> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.find('\n') {
            let line = js_trim(&self.buf[..nl]).to_string();
            self.buf.drain(..=nl);
            if !line.is_empty() {
                out.push(message(&line));
            }
        }
        out
    }
}

/// The last 20 lines the client's app-servers wrote to stderr.
#[derive(Default, Clone, Debug)]
pub struct Stderr {
    pub lines: Vec<String>,
}

impl Stderr {
    pub fn push(&mut self, chunk: &str) {
        for l in crate::git::split_lines(chunk).into_iter().filter(|l| !l.is_empty()) {
            self.lines.push(l.to_string());
        }
        if self.lines.len() > 20 {
            self.lines.drain(..self.lines.len() - 20);
        }
    }

    /// Why the app-server's end failed what waited on it.
    pub fn exited(&self, code: Js<'_>) -> String {
        let tail = &self.lines[self.lines.len().saturating_sub(3)..];
        format!("codex app-server exited ({}): {}", tpl(code), tail.join(" | "))
    }
}

/// What one line from the app-server is.
pub fn message(line: &str) -> Message {
    let Ok(msg) = json::parse(line) else { return Message::Other };
    let m = Some(&msg);
    let id = opt_prop(m, "id");
    let Some(id) = id else { return Message::Other };
    if truthy(opt_prop(m, "method")) {
        // A request of the app-server's own (approvals and the like):
        // XenoNet never starts turns, so it declines.
        let mut o = Map::new();
        o.insert("jsonrpc".into(), Value::String("2.0".into()));
        o.insert("id".into(), id.clone());
        o.insert("error".into(), serde_json::json!({ "code": -32601, "message": "XenoNet is read-only" }));
        return Message::Decline(format!("{}\n", json::stringify(&Value::Object(o), 0)));
    }
    let error = opt_prop(m, "error");
    if truthy(error) {
        let message = nn(opt_prop(error, "message")).map_or_else(|| "app-server error".to_string(), js_string);
        return Message::Answer { id: id.clone(), result: Box::new(F::Undef), error: Some(Box::new((message, error.cloned().unwrap_or(Value::Null)))) };
    }
    Message::Answer { id: id.clone(), result: Box::new(F::of(opt_prop(m, "result"))), error: None }
}

// ---------- Codex desktop's projects ----------

/// The numbers of the state databases in Codex's folder that aren't the
/// one XenoNet reads (state_6.sqlite), as they're written.
pub fn other_state_dbs(names: &[String]) -> Vec<String> {
    names
        .iter()
        .filter_map(|n| n.strip_prefix("state_")?.strip_suffix(".sqlite").map(str::to_string))
        .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        .filter(|n| n.parse::<f64>().is_ok_and(|x| x != f64::from(STATE_DB)))
        .collect()
}

/// A value from SQLite, as node:sqlite gives it to JS.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    V(Value),
    Bytes(Vec<u8>),
}

/// What the state database says: the projects' rows and their roots', or
/// the tables and columns it reads that aren't there.
#[derive(Clone, Debug, PartialEq)]
pub enum StateRows {
    Rows { projects: Vec<Vec<(&'static str, Cell)>>, roots: Vec<Vec<(&'static str, Cell)>> },
    Missing(Vec<String>),
}

fn rows(db: &Connection, sql: &str, cols: [&'static str; 2]) -> rusqlite::Result<Vec<Vec<(&'static str, Cell)>>> {
    let mut st = db.prepare(sql)?;
    let mut rs = st.query([])?;
    let mut out = Vec::new();
    while let Some(r) = rs.next()? {
        let mut row = Vec::new();
        for (i, c) in cols.iter().enumerate() {
            let cell = match r.get_ref(i)? {
                ValueRef::Null => Cell::V(Value::Null),
                ValueRef::Integer(n) => {
                    // node:sqlite refuses a whole number JS can't hold.
                    if n.unsigned_abs() > 9_007_199_254_740_991 {
                        return Err(rusqlite::Error::IntegralValueOutOfRange(i, n));
                    }
                    Cell::V(json::number(n as f64))
                }
                ValueRef::Real(x) => Cell::V(json::number(x)),
                ValueRef::Text(t) => Cell::V(Value::String(String::from_utf8_lossy(t).into_owned())),
                ValueRef::Blob(b) => Cell::Bytes(b.to_vec()),
            };
            row.push((*c, cell));
        }
        out.push(row);
    }
    Ok(out)
}

// A query that failed in a database that opened: the tables and columns
// it reads that aren't there. A file that isn't a database has none.
fn missing_columns(db: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for (table, columns) in [("projects", ["id", "name"]), ("project_roots", ["project_id", "path"])] {
        let have: rusqlite::Result<Vec<String>> = (|| {
            let mut st = db.prepare(&format!("pragma table_info({table})"))?;
            let names = st.query_map([], |r| r.get::<_, String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(names)
        })();
        let Ok(have) = have else { return out };
        if have.is_empty() {
            out.push(table.to_string());
        } else {
            for c in columns {
                if !have.iter().any(|h| h == c) {
                    out.push(format!("{table}[].{c}"));
                }
            }
        }
    }
    out
}

/// Reads Codex desktop's projects from its state database, read-only.
/// None when there's no database (or it can't be opened).
pub fn read_state_db(file: &str) -> Option<StateRows> {
    if !std::path::Path::new(file).exists() {
        return None;
    }
    let db = Connection::open_with_flags(file, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX).ok()?;
    let read = rows(&db, "select id, name from projects", ["id", "name"])
        .and_then(|p| Ok((p, rows(&db, "select project_id, path from project_roots order by position", ["project_id", "path"])?)));
    Some(match read {
        Ok((projects, roots)) => StateRows::Rows { projects, roots },
        Err(_) => StateRows::Missing(missing_columns(&db)),
    })
}

// a === b, for values from JSON: objects are never the same one.
fn strict_eq(a: Js<'_>, b: Js<'_>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// Codex desktop's projects from the state database's rows, as the
/// lenient reader left them: each with its roots, in their order.
pub fn native_projects(rows: Js<'_>) -> Result<Vec<Obj>, TypeError> {
    let roots = nn(prop(rows, "project_roots")?).cloned();
    let projects = match nn(prop(rows, "projects")?) {
        None => return Ok(Vec::new()),
        Some(Value::Array(a)) => a.clone(),
        Some(_) => return Err(TypeError("(rows.projects ?? []).map is not a function".into())),
    };
    let mut out = Vec::new();
    for p in &projects {
        let id = prop(Some(p), "id")?;
        let roots: &[Value] = match &roots {
            None => &[],
            Some(Value::Array(a)) => a,
            Some(_) => return Err(TypeError("roots.filter is not a function".into())),
        };
        let mut mine = Vec::new();
        for r in roots {
            if strict_eq(prop(Some(r), "project_id")?, id) {
                mine.push(F::of(prop(Some(r), "path")?));
            }
        }
        out.push(vec![("id", F::of(id)), ("name", F::of(prop(Some(p), "name")?)), ("roots", F::Arr(mine))]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get<'a>(o: &'a Obj, k: &str) -> &'a F {
        &o.iter().find(|(n, _)| *n == k).unwrap().1
    }

    #[test]
    fn a_thread_is_a_session() {
        let t = json!({ "id": "t1", "preview": "Fix the build\nand more", "createdAt": 1790000000, "status": "active", "gitInfo": { "origin_url": "o" }, "source": "cli" });
        let s = thread_to_session(Some(&t), false).unwrap();
        assert_eq!(get(&s, "title"), &F::s("Fix the build"));
        assert_eq!(get(&s, "createdAt"), &F::V(json!(1790000000000u64)));
        assert_eq!(get(&s, "origin"), &F::s("o"));
        let spawned = json!({ "id": "t2", "source": { "subAgent": { "thread_spawn": { "parent_thread_id": "t1", "agent_nickname": "b" } } } });
        let s = thread_to_session(Some(&spawned), true).unwrap();
        assert_eq!([get(&s, "parentId"), get(&s, "agentKind"), get(&s, "source")], [&F::s("codex:t1"), &F::s("spawned"), &F::s("subAgent")]);
        assert_eq!(sub_agent_of(Some(&json!({ "source": { "subAgent": { "compact": {} } } }))).unwrap().2, json!("compact"));
        assert_eq!(sub_agent_of(Some(&json!({ "source": { "subAgent": 5 } }))).unwrap().2, json!("other"));
        assert!(thread_to_session(Some(&Value::Null), false).is_err());
    }

    #[test]
    fn reads_items() {
        let page = json!([{ "item": { "id": "i1", "type": "agentMessage", "text": "Hello" } }, { "item": { "id": "i0", "type": "userMessage", "content": [{ "type": "text", "text": "Hi" }, { "type": "localImage" }] }, "startedAtMs": 10 }]);
        let items = normalize_items(Some(&page)).unwrap();
        assert_eq!(get(&items[0], "text"), &F::s("Hi\n[image]"));
        assert_eq!(get(&items[0], "ts"), &F::V(json!(10)));
        let fc =
            normalize_item(Some(&json!({ "item": { "type": "fileChange", "changes": [{ "path": "a", "kind": { "type": "update" } }, { "path": "b", "kind": "add" }] } }))).unwrap();
        assert_eq!(get(&fc, "text"), &F::s("update a\nadd b"));
        assert_eq!(get(&fc, "id"), &F::Undef);
    }

    #[test]
    fn reads_a_rollout() {
        let mut r = RolloutEntries::default();
        r.add(json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": "Hi" } })).unwrap();
        r.add(json!({ "type": "response_item", "timestamp": "2026-10-05T13:00:01Z", "payload": { "type": "custom_tool_call", "call_id": "c", "name": "exec", "input": "{}" } }))
            .unwrap();
        assert_eq!(r.list().len(), 2);
        r.add(
            json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "type": "CommandExecution", "command": ["/bin/zsh", "-lc", "make"], "exit_code": 2 } } }),
        )
        .unwrap();
        assert_eq!(r.list().len(), 1);
        assert_eq!(get(&r.list()[0], "title"), &F::s("make"));
        assert_eq!(get(&r.list()[0], "id"), &F::s("item-0"));
        assert_eq!(rollout_page(10, Some(&json!("rollout:4")), 4.0), (2, 6, Some("rollout:8".into())));
        assert_eq!(rollout_page(10, None, 120.0), (0, 10, None));
    }

    #[test]
    fn reads_the_app_servers_lines() {
        let mut io = Stdout::default();
        let ms = io.take("not json\n{\"id\":\"a\",\"method\":\"x\"}\n{\"id\":1,\"result\":{}}\n{\"id\":2,\"error\":{\"code\":1}}\n{\"method\":\"n\"}\n{\"id\":3");
        assert_eq!(ms.len(), 5);
        assert_eq!(ms[0], Message::Other);
        assert_eq!(ms[1], Message::Decline("{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"error\":{\"code\":-32601,\"message\":\"XenoNet is read-only\"}}\n".into()));
        assert!(matches!(&ms[3], Message::Answer { error: Some(e), .. } if e.0 == "app-server error"));
        assert_eq!(io.take(",\"result\":5}\n").len(), 1);
        let mut err = Stderr::default();
        err.push("a\nb\r\n\nc");
        assert_eq!(err.exited(Some(&json!(2))), "codex app-server exited (2): a | b | c");
        err.push("d\r");
        assert_eq!(err.lines.last().map(String::as_str), Some("d\r"));
        assert!(says_unsupported(Some(&json!({ "code": -32603, "message": "thread/items/list is Not Supported yet" }))));
        assert!(!says_unsupported(Some(&json!({ "code": "-32601" }))));
    }

    #[test]
    fn lists_other_state_dbs() {
        let names: Vec<String> = ["state_5.sqlite", "state_6.sqlite", "state_05.sqlite", "state_x.sqlite", "notes"].iter().map(ToString::to_string).collect();
        assert_eq!(other_state_dbs(&names), ["6"]);
    }
}
