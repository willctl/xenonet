//! The MCP side of the server (src/server.mjs): state, the five tools, JSON-RPC message handling.
//! The tools are defined once, in mcp/tools.json (curated writes, the default). With writes "direct"
//! two of them do something else, so they read differently: mcp/tools-direct-text.json replaces
//! only their title and description.

use xenolith_core::access::{self, Level, Policy, Principal};
use xenolith_core::audit;
use xenolith_core::inbox::{self, NoteInput, PageInput};
use xenolith_core::lock::cleanup_locks;
use xenolith_core::paths::{AppPaths, Env};
use xenolith_core::reqlog::{RequestLog, clip, clip_str, new_request_id, summarize_args};
use xenolith_core::spaces::{self, SpaceId};
use xenolith_core::text::{defang_remote_images, local_iso, now, now_ms};
use xenolith_core::wiki::{self, Error, UpsertInput};
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const SUPPORTED_VERSIONS: [&str; 5] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];
const ASK_TOOLS: [&str; 2] = ["context_search", "context_read"];
const TOOLS: &str = include_str!("mcp/tools.json");
const TOOLS_DIRECT_TEXT: &str = include_str!("mcp/tools-direct-text.json");

pub fn log(msg: &str) {
    eprintln!("[xenolith {}] {msg}", chrono_utc_iso());
}

fn chrono_utc_iso() -> String {
    let n = now();
    n.with_timezone(&chrono::Utc).format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

#[derive(Clone)]
pub struct State {
    pub wiki_dir: Option<PathBuf>,
    pub setup_error: Option<String>,
    pub config: Value,
    pub curated: bool,
    pub read_only: bool,
}

pub fn env() -> Env {
    xenolith_core::paths::process_env()
}

pub fn setup() -> State {
    let env = env();
    let mut st = State { wiki_dir: None, setup_error: None, config: json!({}), curated: true, read_only: false };
    st.config = wiki::read_config(&env).unwrap_or_else(|_| json!({}));
    xenolith_core::embed::configure(&st.config);
    let mode = env.get("XENOLITH_WRITE_MODE").cloned().or_else(|| st.config.get("writeMode").and_then(Value::as_str).map(String::from));
    st.curated = mode.as_deref() != Some("direct");
    let r = (|| -> wiki::Result<PathBuf> {
        let (dir, _) = wiki::resolve_wiki_dir(&env)?;
        wiki::ensure_wiki(&dir)?;
        cleanup_locks(&dir);
        wiki::with_lock(&dir, || wiki::refresh_index(&dir, None).map(|_| ()))?;
        Ok(dir)
    })();
    match r {
        Ok(d) => st.wiki_dir = Some(d),
        Err(e) => {
            log(&format!("setup failed: {e}"));
            st.setup_error = Some(e.to_string());
            st.wiki_dir = wiki::resolve_wiki_dir(&env).ok().map(|(d, _)| d);
        }
    }
    st
}

/// setup() for --read-only: resolves the wiki and writes nothing.
pub fn setup_read_only() -> State {
    let env = env();
    let mut st = State { wiki_dir: None, setup_error: None, config: json!({}), curated: true, read_only: true };
    st.config = wiki::read_config(&env).unwrap_or_else(|_| json!({}));
    xenolith_core::embed::configure(&st.config);
    match wiki::resolve_wiki_dir(&env) {
        Ok((d, _)) if d.exists() => st.wiki_dir = Some(d),
        Ok((d, _)) => {
            let m = format!("There is no wiki at {}.", wiki::disp(&d));
            log(&format!("setup failed: {m}"));
            st.setup_error = Some(m);
        }
        Err(e) => {
            log(&format!("setup failed: {e}"));
            st.setup_error = Some(e.to_string());
        }
    }
    st
}

pub fn open_request_log(proc_name: &str, config: &Value) -> RequestLog {
    let days = config.get("logs").and_then(|l| l.get("retentionDays")).and_then(Value::as_i64).filter(|d| *d > 0).unwrap_or(30);
    RequestLog::new(&AppPaths::current().log_dir, proc_name, days)
}

fn lead_in(wiki_dir: &str, curated: bool) -> String {
    let writes = if curated {
        "Tell it what happened with context_log (and context_upsert_page for page content): a curator agent organizes notes into pages, so hand over the facts and skip the formatting."
    } else {
        "Record decisions, outcomes, preferences, facts and follow-ups worth keeping with context_log, and keep topic pages current with context_upsert_page."
    };
    format!(
        "Shared long-term memory for this user, for every app and agent they use (wiki folder: {wiki_dir}). Call context_start once at the beginning of EVERY conversation, before your first substantive reply, and follow the protocol it returns. Search the wiki (context_search) before asking the user for context they may have given before. {writes} Never store secrets."
    )
}

/// The instructions a client gets at initialize. A token's holder may be elsewhere: it sees no
/// folder of this machine's, only that the wiki is Xenolith's.
pub fn instructions_for(st: &State, who: &Principal) -> String {
    let dir = match who {
        Principal::Local => st.wiki_dir.as_ref().map(|d| wiki::disp(d)).unwrap_or_else(|| "~/Xenolith".into()),
        Principal::Token { .. } => "shared through Xenolith".into(),
    };
    if st.read_only {
        return format!("Read-only access to this user's shared wiki ({dir}): context_search finds pages, log entries and notes not yet filed; context_read opens one.");
    }
    let protocol = st.wiki_dir.as_ref().map(|d| wiki::read_protocol(d)).unwrap_or_else(|| wiki::DEFAULT_PROTOCOL.to_string());
    format!("{}\n\n{}", lead_in(&dir, st.curated), protocol.trim())
}

fn tool_list(curated: bool) -> Vec<Value> {
    let mut all: Vec<Value> = serde_json::from_str(TOOLS).unwrap_or_default();
    if !curated {
        let text: Value = serde_json::from_str(TOOLS_DIRECT_TEXT).unwrap_or_default();
        for t in &mut all {
            if let Some(o) = text[t["name"].as_str().unwrap_or("")].as_object() {
                for (k, v) in o {
                    t[k] = v.clone();
                }
            }
        }
    }
    all
}

pub fn tools(st: &State) -> Vec<Value> {
    tool_list(st.curated).into_iter().filter(|t| !st.read_only || ASK_TOOLS.contains(&t["name"].as_str().unwrap_or(""))).collect()
}

/// Per-connection (stdio) or per-request (HTTP) context.
pub struct Ctx<'a> {
    pub transport: &'a str,
    pub rid: Option<String>,
    pub client: String,
    pub reqlog: &'a RequestLog,
    pub on_error: Option<&'a (dyn Fn(&str) + Sync)>,
    /// Who is asking: you (stdio, or HTTP from this machine without a token), or a token's holder.
    pub principal: Principal,
    /// Set when a call needed a level the token's scopes never reach: over HTTP that is a 403
    /// with insufficient_scope, so the client can ask for more (MCP's scope step-up).
    pub scope_needed: Mutex<Option<String>>,
}

impl<'a> Ctx<'a> {
    pub fn new(transport: &'a str, rid: Option<String>, client: String, reqlog: &'a RequestLog, on_error: Option<&'a (dyn Fn(&str) + Sync)>, principal: Principal) -> Self {
        Ctx { transport, rid, client, reqlog, on_error, principal, scope_needed: Mutex::new(None) }
    }
}

// ---------------------------------------------------------------- spaces and access

/// The level a call needs: reads read, a note or an appended page appends, a replaced page writes.
pub fn needed_level(name: &str, args: &Value) -> Level {
    match name {
        "context_log" => Level::Append,
        "context_upsert_page" if args.get("mode").and_then(Value::as_str) == Some("replace") => Level::Write,
        "context_upsert_page" => Level::Append,
        _ => Level::Read,
    }
}

/// The same answer for a space that doesn't exist and one the caller may not use, so a token
/// can't learn which spaces exist.
fn no_space(id: &SpaceId, need: Level) -> Error {
    Error::Wiki(format!("Refused: no space \"{id}\" that you can {}.", need.as_str()))
}

/// A call's space when it names none: this machine's own, when the caller may use it; otherwise
/// the first one it may (a token holder's own actor space, say).
fn default_space(root: &Path, policy: &Policy, who: &Principal, need: Level, now: i64) -> Option<SpaceId> {
    let local = SpaceId::local();
    if policy.allows(who, &local, need, now) {
        return Some(local);
    }
    spaces::list(root).into_iter().find(|s| policy.allows(who, s, need, now))
}

/// The space a call works in, after the access check (deny by default): its id and folder. You
/// start a space by writing to it; a token needs admin there to do the same.
pub fn open_space(st: &State, ctx: &Ctx, root: &Path, args: &Value, need: Level) -> Result<(SpaceId, PathBuf), Error> {
    let paths = AppPaths::current();
    let policy = access::load(&paths);
    let now = now_ms();
    // Every refusal goes in the audit chain: who, where, what for and why (never the call's text).
    let refuse = |space: Option<&SpaceId>, why: &str, e: Error| {
        audit::best_effort(&paths, "refused", &ctx.principal.name(), vec![("space", space.map(|s| json!(s.to_string())).unwrap_or(Value::Null)), ("need", json!(need.as_str())), ("why", json!(why)), ("transport", json!(ctx.transport))]);
        e
    };
    let Some(who) = policy.current(&ctx.principal, now) else {
        return Err(refuse(None, "revoked", Error::Wiki("Refused: this token has been revoked or has expired. Ask the user for a new one.".into())));
    };
    let who = &who;
    if Policy::scope_ceiling(who).is_none_or(|c| c < need) {
        *ctx.scope_needed.lock().unwrap() = Some(format!("ctx:{}", need.as_str()));
        return Err(refuse(None, "scope", Error::Wiki(format!("Refused: this token's scopes don't include ctx:{}. Ask the user for a token that does.", need.as_str()))));
    }
    let id = match opt_s(args, "space").filter(|s| !s.trim().is_empty()) {
        Some(s) => SpaceId::parse(&s)?,
        None => default_space(root, &policy, who, need, now).ok_or_else(|| {
            refuse(
                None,
                "no space",
                match who {
                    Principal::Local => Error::Wiki(format!("Refused: you may not {} in any space (access.json says \"local\": \"none\").", need.as_str())),
                    Principal::Token { .. } => Error::Wiki(format!("Refused: no space that this token can {}. Ask the user to grant it one.", need.as_str())),
                },
            )
        })?,
    };
    if !policy.allows(who, &id, need, now) {
        return Err(refuse(Some(&id), "not granted", no_space(&id, need)));
    }
    if spaces::exists(root, &id) {
        return Ok((id.clone(), spaces::space_dir(root, &id)));
    }
    if need >= Level::Append && !st.read_only && policy.allows(who, &id, Level::Admin, now) {
        let dir = spaces::create(root, &id)?;
        audit::best_effort(&paths, "space", &who.name(), vec![("space", json!(id.to_string())), ("event", json!("created"))]);
        return Ok((id, dir));
    }
    Err(match who {
        Principal::Local => Error::Wiki(format!("No space \"{id}\" yet. Logging a note to it starts it.")),
        Principal::Token { .. } => no_space(&id, need),
    })
}

/// context_start's line about spaces, when there is more than one or the caller holds a token:
/// which space this is, and the others the caller can use, with its level in each.
fn spaces_line(root: &Path, who: &Principal, current: &SpaceId) -> Option<String> {
    let policy = access::load(&AppPaths::current());
    let now = now_ms();
    let who = &policy.current(who, now)?;
    let mine: Vec<(SpaceId, Level)> = spaces::list(root).into_iter().filter_map(|s| policy.level(who, &s, now).map(|l| (s, l))).collect();
    if mine.len() <= 1 && *who == Principal::Local {
        return None;
    }
    let others: Vec<String> = mine.iter().filter(|(s, _)| s != current).map(|(s, l)| format!("{s} ({})", l.as_str())).collect();
    Some(if others.is_empty() {
        format!("Space: {current}. No other space is open to you.")
    } else {
        format!("Space: {current}. Other spaces you can use: {}. Pass `space` to any tool to work in one.", others.join(", "))
    })
}

// ---------------------------------------------------------------- argument validation (the zod schemas)

fn type_name(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// Checks `args` against a tool's inputSchema; returns zod-like messages.
fn validate(schema: &Value, args: &Value) -> Vec<String> {
    let mut problems = vec![];
    let empty = Map::new();
    let obj = match args {
        Value::Object(o) => o,
        Value::Null => &empty,
        other => return vec![format!("Invalid input: expected object, received {}", type_name(Some(other)))],
    };
    let props = schema.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
    let required: Vec<&str> = schema.get("required").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    for (name, spec) in &props {
        let v = obj.get(name);
        if v.is_none() || v == Some(&Value::Null) && !required.contains(&name.as_str()) {
            if required.contains(&name.as_str()) {
                problems.push(format!("Invalid input: expected {}, received undefined at {name}", spec["type"].as_str().unwrap_or("value")));
            }
            continue;
        }
        let v = v.unwrap();
        let want = spec["type"].as_str().unwrap_or("");
        let ok = match want {
            "string" => v.is_string(),
            "integer" => v.as_f64().is_some_and(|f| f.fract() == 0.0),
            "number" => v.is_number(),
            "array" => v.as_array().is_some_and(|a| {
                let item = spec["items"]["type"].as_str().unwrap_or("");
                item != "string" || a.iter().all(Value::is_string)
            }),
            "boolean" => v.is_boolean(),
            _ => true,
        };
        if !ok {
            let want_label = if want == "integer" && v.is_number() { "int".to_string() } else { want.to_string() };
            problems.push(format!("Invalid input: expected {want_label}, received {} at {name}", type_name(Some(v))));
            continue;
        }
        if let Some(options) = spec.get("enum").and_then(Value::as_array).filter(|o| !o.contains(v)) {
            let list = options.iter().map(|o| o.to_string()).collect::<Vec<_>>().join("|");
            problems.push(format!("Invalid option: expected one of {list} at {name}"));
        }
        if let Some(p) = spec.get("pattern").and_then(Value::as_str).filter(|p| v.as_str().is_some_and(|s| !regex::Regex::new(p).is_ok_and(|re| re.is_match(s)))) {
            problems.push(format!("Invalid string: must match pattern /{p}/ at {name}"));
        }
        if let Some(n) = v.as_f64() {
            if let Some(min) = spec.get("minimum").and_then(Value::as_f64).filter(|m| n < *m) {
                problems.push(format!("Too small: expected number to be >={} at {name}", wiki::js_number(min)));
            }
            if let Some(max) = spec.get("maximum").and_then(Value::as_f64).filter(|m| n > *m) {
                problems.push(format!("Too big: expected number to be <={} at {name}", wiki::js_number(max)));
            }
        }
    }
    problems
}

fn text_result(text: String, is_error: bool) -> Value {
    let mut r = json!({ "content": [{ "type": "text", "text": text }] });
    if is_error {
        r["isError"] = Value::Bool(true);
    }
    r
}

fn s(args: &Value, k: &str) -> String {
    args.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn opt_s(args: &Value, k: &str) -> Option<String> {
    args.get(k).and_then(Value::as_str).map(String::from)
}

fn queued(r: &inbox::Submitted) -> String {
    if r.duplicate {
        format!("Already saved as note {} ({}); nothing new was queued.", r.id, r.status)
    } else {
        format!(
            "Saved note {} ({}). The curator will file it into the wiki shortly; until then it is listed under \"Pending notes\" in context_start and found by context_search. No need to check on it.",
            r.id, r.rel
        )
    }
}

/// The tools' names before Xenolith, from agent-wiki. A call under one still works for this release,
/// so sessions that listed the tools before an upgrade, and protocols written for them, keep working;
/// tools/list shows only the new names. Remove them in the next release.
pub const OLD_NAMES: [(&str, &str); 5] =
    [("wiki_start", "context_start"), ("wiki_search", "context_search"), ("wiki_read", "context_read"), ("wiki_log", "context_log"), ("wiki_upsert_page", "context_upsert_page")];

/// A tool's current name: itself, or the one an old name stands for.
pub fn current_name(name: &str) -> &str {
    OLD_NAMES.iter().find(|(old, _)| *old == name).map_or(name, |(_, new)| new)
}

/// Runs one tool; failures become isError results. Logs the call, under the name it was called by.
pub fn call_tool(st: &State, ctx: &Ctx, called: &str, args: &Value) -> Value {
    let name = current_name(called);
    let list = tools(st);
    let Some(def) = list.iter().find(|t| t["name"] == name) else {
        return text_result(format!("MCP error -32602: Tool {called} not found"), true);
    };
    let problems = validate(&def["inputSchema"], args);
    if !problems.is_empty() {
        return text_result(format!("MCP error -32602: Input validation error: Invalid arguments for tool {called}: {}", problems.join("; ")), true);
    }
    let t0 = now_ms();
    let mut used_space: Option<String> = None;
    let outcome: Result<String, Error> = (|| {
        let Some(root) = st.wiki_dir.as_ref().filter(|_| st.setup_error.is_none()) else {
            return Err(Error::Wiki(st.setup_error.clone().unwrap_or_else(|| "The wiki folder could not be resolved.".into())));
        };
        let (space, dir) = open_space(st, ctx, root, args, needed_level(name, args))?;
        used_space = Some(space.to_string());
        let dir = &dir;
        match name {
            // Read results never carry a remote image an app would fetch on its own (M2).
            "context_start" => {
                let mut text = wiki::start_context(dir, &s(args, "app"), &s(args, "topic"))?;
                if let Principal::Token { .. } = ctx.principal {
                    // No folder of this machine's for a token's holder, who may be elsewhere.
                    text = text.replacen(&format!(" Wiki folder: {}.", wiki::disp(dir)), "", 1);
                }
                if let (Some(line), Some(i)) = (spaces_line(root, &ctx.principal, &space), text.find("\n\n---")) {
                    text.insert_str(i, &format!("\n{line}"));
                }
                Ok(defang_remote_images(&text).into_owned())
            }
            "context_search" => {
                let query = s(args, "query");
                let scope = opt_s(args, "scope").unwrap_or_else(|| "all".into());
                let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(8);
                let filters = xenolith_core::search::Filters { since: opt_s(args, "since"), until: opt_s(args, "until"), app: opt_s(args, "app").filter(|a| !a.trim().is_empty()) };
                // Other wordings searched with it and fused (at most 5, each clipped like a query).
                let mut queries = vec![query.clone()];
                for q in args.get("queries").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(|q| clip_str(q.trim(), 300)) {
                    if !q.is_empty() && !queries.contains(&q) && queries.len() < 6 {
                        queries.push(q);
                    }
                }
                let hits = xenolith_core::search::search_many(dir, &queries, &scope, limit, &filters)?;
                let what = if queries.len() > 1 { format!("\"{query}\" and {} other wording(s)", queries.len() - 1) } else { format!("\"{query}\"") };
                if hits.is_empty() {
                    return Ok(format!("No results for {what}. The wiki has nothing on this yet, or it uses other words: try names, the broader topic, or what the answer would say."));
                }
                Ok(defang_remote_images(&format!("{} result(s) for {what}:\n\n{}", hits.len(), wiki::format_search_results(&hits))).into_owned())
            }
            "context_read" => {
                let (rel, text) = wiki::read_section(dir, &s(args, "target"), opt_s(args, "section").as_deref())?;
                Ok(format!("File: {rel}\n\n{}", defang_remote_images(&text)))
            }
            "context_log" => {
                if st.curated {
                    let note = NoteInput {
                        kind: "log".into(),
                        source: s(args, "source"),
                        app: s(args, "app"),
                        title: s(args, "title"),
                        body: s(args, "body"),
                        tags: args.get("tags").cloned().unwrap_or(Value::Null),
                        pages: args.get("pages").cloned().unwrap_or(Value::Null),
                        page: None,
                        idempotency_key: opt_s(args, "idempotency_key"),
                    };
                    return Ok(queued(&inbox::submit_note(dir, &note, ctx.transport, &ctx.client)?));
                }
                let (rel, heading) = wiki::append_log(dir, &s(args, "app"), &s(args, "title"), &s(args, "body"), args.get("tags").unwrap_or(&Value::Null), args.get("pages").unwrap_or(&Value::Null))?;
                Ok(format!("Logged to {rel}: {}", heading.trim_start_matches("## ")))
            }
            "context_upsert_page" => {
                if st.curated {
                    let slug = opt_s(args, "slug");
                    let note = NoteInput {
                        kind: "page".into(),
                        source: s(args, "source"),
                        app: s(args, "app"),
                        title: s(args, "title"),
                        body: s(args, "content"),
                        tags: args.get("tags").cloned().unwrap_or(Value::Null),
                        pages: slug.clone().filter(|s| !s.is_empty()).map(|s| json!([s])).unwrap_or(json!([])),
                        page: Some(PageInput {
                            // A replace remembers the page it was written against, so a curator on
                            // another machine can tell a stale one (sync).
                            base: (opt_s(args, "mode").as_deref() == Some("replace")).then(|| {
                                let slug = slug.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| wiki::slugify(&s(args, "title")));
                                xenolith_core::sync::page_base(dir, &slug)
                            }),
                            slug,
                            title: opt_s(args, "title"),
                            kind: opt_s(args, "type"),
                            summary: opt_s(args, "summary"),
                            mode: opt_s(args, "mode"),
                        }),
                        idempotency_key: opt_s(args, "idempotency_key"),
                    };
                    return Ok(queued(&inbox::submit_note(dir, &note, ctx.transport, &ctx.client)?));
                }
                let r = wiki::upsert_page(
                    dir,
                    &UpsertInput {
                        app: s(args, "app"),
                        title: s(args, "title"),
                        slug: opt_s(args, "slug"),
                        kind: opt_s(args, "type"),
                        summary: opt_s(args, "summary"),
                        tags: args.get("tags").cloned().filter(|t| !t.is_null()),
                        content: s(args, "content"),
                        mode: opt_s(args, "mode"),
                    },
                )?;
                let verb = match r.action {
                    "created" => "Created",
                    "updated" => "Appended to",
                    _ => "Rewrote",
                };
                let hist = r.history_rel.map(|h| format!(" (previous version kept at {h})")).unwrap_or_default();
                Ok(format!("{verb} {}{hist}. index.md regenerated.", r.rel))
            }
            _ => Err(Error::Wiki(format!("Tool {name} not found"))),
        }
    })();
    // Every write goes in the audit chain: who, which space, which note, and a hash of the text.
    if let (Ok(text), Some(space)) = (&outcome, &used_space) {
        if needed_level(name, args) >= Level::Append {
            let note = text.strip_prefix("Saved note ").or_else(|| text.strip_prefix("Already saved as note ")).and_then(|t| t.split_whitespace().next());
            let content = format!("{}\n{}", s(args, "title"), if name == "context_log" { s(args, "body") } else { s(args, "content") });
            audit::best_effort(
                &AppPaths::current(),
                "write",
                &ctx.principal.name(),
                vec![
                    ("space", json!(space)),
                    ("tool", json!(name)),
                    ("note", note.map(|n| json!(n)).unwrap_or(Value::Null)),
                    ("duplicate", if text.starts_with("Already saved") { json!(true) } else { Value::Null }),
                    ("content", json!(audit::content_hash(&content))),
                    ("transport", json!(ctx.transport)),
                ],
            );
        }
    }
    let (text, is_error, result, error) = match outcome {
        Ok(t) => (t, false, "ok", None),
        Err(Error::Wiki(m)) => {
            let r = if m.starts_with("Refused") { "refused" } else { "rejected" };
            (m.clone(), true, r, Some(m))
        }
        Err(e) => {
            let m = format!("xenolith error: {e}");
            log(&m);
            if let Some(f) = ctx.on_error {
                f(&m);
            }
            (m.clone(), true, "error", Some(m))
        }
    };
    ctx.reqlog.write(vec![
        ("kind", json!("tool")),
        ("rid", json!(ctx.rid.clone().unwrap_or_else(new_request_id))),
        ("transport", json!(ctx.transport)),
        ("client", json!(clip_str(&ctx.client, 80))),
        ("app", clip(args.get("app").unwrap_or(&Value::Null), 40).map(Value::String).unwrap_or(Value::Null)),
        ("ask", std::env::var("XENOLITH_ASK").ok().filter(|s| !s.is_empty()).map(Value::String).unwrap_or(Value::Null)),
        ("tool", json!(called)),
        ("space", used_space.map(Value::String).unwrap_or(Value::Null)),
        ("who", if ctx.principal == Principal::Local { Value::Null } else { json!(ctx.principal.name()) }),
        ("args", summarize_args(args).unwrap_or(Value::Null)),
        ("ms", json!(now_ms() - t0)),
        ("result", json!(result)),
        ("error", error.map(|e| Value::String(clip_str(&e, 300))).unwrap_or(Value::Null)),
    ]);
    text_result(text, is_error)
}

// ---------------------------------------------------------------- JSON-RPC

pub fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

pub fn is_request(m: &Value) -> bool {
    m.get("method").is_some() && m.get("id").is_some_and(|i| !i.is_null())
}

/// A JSON-RPC message the SDK accepts: request, notification or response.
pub fn valid_message(m: &Value) -> bool {
    let Some(o) = m.as_object() else { return false };
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return false;
    }
    if let Some(method) = o.get("method") {
        return method.is_string() && o.get("id").is_none_or(|i| i.is_string() || i.is_number());
    }
    o.get("id").is_some() && (o.contains_key("result") || o.contains_key("error"))
}

/// Handles one message; returns the response for requests. `initialized`: called on notifications/initialized.
pub fn handle(st: &State, ctx: &Ctx, msg: &Value, client_info: &Mutex<Option<String>>) -> Option<Value> {
    let method = msg.get("method").and_then(Value::as_str)?;
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let id = id?; // notifications get no response
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if SUPPORTED_VERSIONS.contains(&asked) { asked } else { SUPPORTED_VERSIONS[0] };
            if let Some(ci) = params.get("clientInfo") {
                let name = ci.get("name").and_then(Value::as_str).unwrap_or("");
                let v = ci.get("version").and_then(Value::as_str).unwrap_or("");
                *client_info.lock().unwrap() = Some(format!("{name} {v}").trim().to_string());
            }
            // Pages as resources and the start prompt (resources.rs); subscribing needs a connection
            // that lasts, which stateless HTTP doesn't have. The read-only server (Ask) offers tools only.
            let mut capabilities = json!({ "tools": { "listChanged": true } });
            if !st.read_only {
                let live = ctx.transport == "stdio";
                capabilities["resources"] = json!({ "subscribe": live, "listChanged": live });
                capabilities["prompts"] = json!({ "listChanged": false });
            }
            json!({
                "protocolVersion": version,
                "capabilities": capabilities,
                "serverInfo": { "name": "xenolith", "title": "Xenolith", "version": xenolith_core::VERSION },
                "instructions": instructions_for(st, &ctx.principal),
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools(st) }),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            call_tool(st, ctx, name, params.get("arguments").unwrap_or(&Value::Null))
        }
        m @ ("resources/list" | "resources/templates/list" | "resources/read" | "prompts/list" | "prompts/get") if !st.read_only => {
            let r = match m {
                "resources/list" => crate::resources::list(st, ctx, &params),
                "resources/templates/list" => Ok(crate::resources::templates()),
                "resources/read" => crate::resources::read(st, ctx, &params),
                "prompts/list" => Ok(crate::resources::prompts_list()),
                _ => crate::resources::prompt_get(st, ctx, &params),
            };
            match r {
                Ok(v) => v,
                Err((code, message)) => return Some(rpc_error(id, code, &message)),
            }
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({ "result": result, "jsonrpc": "2.0", "id": id }))
}

pub fn now_iso() -> String {
    local_iso(&now())
}

#[cfg(test)]
mod tool_tests {
    use super::*;

    #[test]
    fn one_tool_list_with_the_direct_texts_and_the_core_sources() {
        let (curated, direct) = (tool_list(true), tool_list(false));
        assert_eq!(curated.len(), 5);
        let text: Value = serde_json::from_str(TOOLS_DIRECT_TEXT).unwrap();
        for (name, o) in text.as_object().unwrap() {
            assert!(curated.iter().any(|t| t["name"] == *name), "tools-direct-text.json names {name}, which tools.json does not have");
            assert!(o.as_object().unwrap().keys().all(|k| k == "title" || k == "description"), "only titles and descriptions differ: {name}");
        }
        for (c, d) in curated.iter().zip(&direct) {
            assert_eq!(c["inputSchema"], d["inputSchema"], "{}", c["name"]);
        }
        // The `source` enum the tools offer is the list the inbox accepts.
        for t in curated.iter().filter(|t| !t["inputSchema"]["properties"]["source"].is_null()) {
            let offered: Vec<&str> = t["inputSchema"]["properties"]["source"]["enum"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert_eq!(offered, xenolith_core::inbox::SOURCES, "{}: keep the source enum equal to xenolith_core::inbox::SOURCES", t["name"]);
        }
    }

    #[test]
    fn each_old_name_stands_for_a_listed_tool_and_none_is_listed() {
        let listed: Vec<String> = tool_list(true).iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
        for (old, new) in OLD_NAMES {
            assert!(listed.iter().any(|n| n == new), "{old} stands for {new}, which tools.json does not have");
            assert!(!listed.iter().any(|n| n == old), "{old} must not be listed");
            assert_eq!(current_name(old), new);
        }
        assert_eq!(current_name("context_read"), "context_read");
        assert_eq!(current_name("nope"), "nope");
    }
}
