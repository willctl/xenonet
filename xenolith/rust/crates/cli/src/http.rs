//! `serve --http`: Streamable HTTP on 127.0.0.1 (stateless, JSON responses), /health, /status, the
//! window (/ui, /api) and sync (/sync/v1). Never a web page: other Hosts and cross-origin requests
//! are refused (DNS rebinding).
//!
//! Who a request is: a request is this machine's own (Local) only when it reaches the loopback
//! listener directly, names a loopback Host and carries no Forwarded, X-Forwarded-* or Via header.
//! Local is any process on this machine, not only the owner's own apps: tokens scope remote and
//! other apps' access. /mcp takes Local or a bearer token; /sync/v1 always needs a token, since sync
//! is remote by nature; /status, /ui and /api are Local only and never take a token; /health is
//! open and says nothing private. Deciding anything in /api also needs the owner key (owner.rs),
//! which this service makes when it starts.

use crate::httpd::{self, Request, Response};
use crate::mcp::{self, Ctx, State, log, rpc_error};
use crate::uiapi;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use xenolith_core::access::{self, Level, Principal};
use xenolith_core::audit;
use xenolith_core::inbox;
use xenolith_core::lock::lock_info;
use xenolith_core::owner::{self, OwnerKey};
use xenolith_core::paths::AppPaths;
use xenolith_core::reqlog::{RequestLog, clip_str, new_request_id};
use xenolith_core::text::{from_ms, local_iso, now_ms, parse_ms, random_uuid, sha256_hex};
use xenolith_core::waker::Waker;
use xenolith_core::{asks, wiki};

const MAX_BATCH: usize = 100;

pub fn build_hash() -> String {
    static B: OnceLock<String> = OnceLock::new();
    B.get_or_init(|| std::env::current_exe().ok().and_then(|p| std::fs::read(p).ok()).map(|b| sha256_hex(&b)[..12].to_string()).unwrap_or_else(|| "unknown".into())).clone()
}

pub fn started_ms() -> i64 {
    static S: OnceLock<i64> = OnceLock::new();
    *S.get_or_init(now_ms)
}

/// Recent errors (5xx, unexpected exceptions) for /status.
pub struct Counters {
    recent: Mutex<VecDeque<i64>>,
    total: AtomicUsize,
    last: Mutex<Option<Value>>,
}

impl Counters {
    fn new() -> Self {
        Counters { recent: Mutex::new(VecDeque::new()), total: AtomicUsize::new(0), last: Mutex::new(None) }
    }
    pub fn error(&self, msg: &str) {
        self.total.fetch_add(1, Ordering::SeqCst);
        *self.last.lock().unwrap() = Some(json!({ "at": mcp::now_iso(), "message": clip_str(msg, 200) }));
        let mut r = self.recent.lock().unwrap();
        r.push_back(now_ms());
        while r.front().is_some_and(|t| now_ms() - t > 15 * 60_000) {
            r.pop_front();
        }
    }
    pub fn snapshot(&self) -> Value {
        let mut r = self.recent.lock().unwrap();
        while r.front().is_some_and(|t| now_ms() - t > 15 * 60_000) {
            r.pop_front();
        }
        json!({ "total": self.total.load(Ordering::SeqCst), "last15m": r.len(), "last": self.last.lock().unwrap().clone() })
    }
}

/// Mcp-Session-Id -> client name, kept in logs/http-sessions.json so names survive restarts.
struct Sessions {
    file: PathBuf,
    map: Mutex<Vec<(String, String)>>,
}

impl Sessions {
    fn load(file: PathBuf) -> Self {
        let map = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Map<String, Value>>(&t).ok())
            .map(|m| m.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        Sessions { file, map: Mutex::new(map) }
    }
    fn get(&self, id: &str) -> Option<String> {
        self.map.lock().unwrap().iter().find(|(k, _)| k == id).map(|(_, v)| v.clone())
    }
    fn set(&self, id: &str, client: &str) {
        let mut m = self.map.lock().unwrap();
        m.retain(|(k, _)| k != id);
        m.push((id.to_string(), client.to_string()));
        while m.len() > 500 {
            m.remove(0);
        }
        let obj: Map<String, Value> = m.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
        let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(&self.file));
        let tmp = PathBuf::from(format!("{}.{}.tmp", self.file.display(), std::process::id()));
        if std::fs::write(&tmp, Value::Object(obj).to_string()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }
}

pub struct App {
    pub state: RwLock<State>,
    pub reqlog: RequestLog,
    sessions: Sessions,
    pub counters: Counters,
    pub port: u16,
    closing: AtomicBool,
    inflight: AtomicUsize,
    /// This start's owner key, or why it could not be written (approvals then wait for a restart).
    owner: Result<OwnerKey, String>,
    bad_tokens: BadTokens,
}

/// Bad tokens per source address: after BAD_TOKENS_PER_MINUTE in a minute, every request with a
/// token from that source gets 429 for BAD_TOKEN_BLOCK. A source is the connection's address and,
/// behind a proxy, the address the proxy last added (Forwarded for=, X-Forwarded-For), so one
/// caller guessing tokens through a gateway doesn't lock out the gateway's other callers.
pub const BAD_TOKENS_PER_MINUTE: u32 = 20;
pub const BAD_TOKEN_BLOCK: Duration = Duration::from_secs(60);

/// One source's minute: when it started, the bad tokens in it, and the block it earned.
type Minute = (Instant, u32, Option<Instant>);

#[derive(Default)]
pub struct BadTokens {
    map: Mutex<HashMap<String, Minute>>,
}

impl BadTokens {
    /// How long a source must still wait, when it is blocked.
    pub fn blocked(&self, source: &str, now: Instant) -> Option<Duration> {
        let m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        m.get(source).and_then(|(_, _, until)| *until).filter(|u| *u > now).map(|u| u - now)
    }

    /// Counts a bad token from `source`; true when that starts a block.
    pub fn failed(&self, source: &str, now: Instant) -> bool {
        let mut m = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if m.len() > 10_000 {
            m.retain(|_, (start, _, until)| now.duration_since(*start) < Duration::from_secs(60) || until.is_some_and(|u| u > now));
        }
        let e = m.entry(source.to_string()).or_insert((now, 0, None));
        if now.duration_since(e.0) >= Duration::from_secs(60) {
            *e = (now, 0, None);
        }
        e.1 += 1;
        if e.1 >= BAD_TOKENS_PER_MINUTE && e.2.is_none_or(|u| u <= now) {
            *e = (now, 0, Some(now + BAD_TOKEN_BLOCK));
            return true;
        }
        false
    }
}

/// Where a request comes from, for BadTokens: the peer, and the client a proxy last named.
fn source_of(req: &Request) -> String {
    let peer = req.peer.map(|p| p.ip().to_string()).unwrap_or_default();
    // The last element is the one the nearest proxy added (read from the end); earlier ones are the
    // client's own say.
    let last = |h: &str, key: Option<&str>| {
        req.header(h).and_then(|v| {
            v.rsplit(',')
                .filter_map(|el| {
                    let el = el.trim();
                    match key {
                        Some(k) => {
                            el.split(';').map(str::trim).find_map(|kv| kv.split_once('=').filter(|(n, _)| n.trim().eq_ignore_ascii_case(k)).map(|(_, v)| v.trim().trim_matches('"').to_string()))
                        }
                        None => Some(el.to_string()).filter(|e| !e.is_empty()),
                    }
                })
                .next()
        })
    };
    match last("forwarded", Some("for")).or_else(|| last("x-forwarded-for", None)) {
        Some(client) => format!("{peer} for {}", clip_str(&client, 80)),
        None => peer,
    }
}

/// The resource this service is to a caller: its MCP endpoint and its RFC 9728 metadata. A local
/// caller sees the loopback address it used; anyone else the configured public URL (config.json
/// "publicUrl", such as https://studio.example/xenolith), when there is one.
struct Resource {
    base: String,
}

impl Resource {
    fn mcp(&self) -> String {
        format!("{}/mcp", self.base)
    }
    /// RFC 9728 3.1: /.well-known/oauth-protected-resource between the origin and the path.
    fn metadata(&self) -> String {
        let after = self.base.find("://").map_or(0, |i| i + 3);
        let (origin, path) = self.base.split_at(self.base[after..].find('/').map_or(self.base.len(), |i| after + i));
        format!("{origin}/.well-known/oauth-protected-resource{path}/mcp")
    }
    /// The metadata's path on this server, as a proxy that keeps the public path sends it.
    fn metadata_path(&self) -> String {
        let m = self.metadata();
        let after = m.find("://").map_or(0, |i| i + 3);
        m[after..].find('/').map(|i| m[after + i..].to_string()).unwrap_or_default()
    }
}

/// config.json's "publicUrl": https:// (or http:// to this machine), no query or fragment.
fn public_url(config: &Value) -> Option<String> {
    config.get("publicUrl").and_then(Value::as_str).map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| xenolith_core::sync::valid_url(u))
}

/// Who is calling /status or the window's API: a local request (http.rs refuses any other).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// It showed this start's owner key: the window the tray opened ("owner"), or `xenolith` in a
    /// terminal ("cli", which says so in X-Xenolith).
    Owner(&'static str),
    /// A process on this machine without the key, with what access.json gives local callers.
    Local(Option<Level>),
}

impl Caller {
    pub fn may(self, need: Level) -> bool {
        match self {
            Caller::Owner(_) => true,
            Caller::Local(l) => l.is_some_and(|l| l >= need),
        }
    }
}

impl App {
    /// Who a local request is: the owner when it carries this start's owner key, else a local process.
    pub fn caller(&self, req: &Request) -> Caller {
        if let (Ok(real), Some(shown)) = (&self.owner, req.header(owner::HEADER)) {
            if owner::matches(&real.key, shown.trim()) {
                return Caller::Owner(if req.header("x-xenolith").as_deref() == Some("cli") { "cli" } else { "owner" });
            }
        }
        Caller::Local(access::load(&AppPaths::current()).local)
    }

    /// Why decisions can't be made in the window this start, if the owner key could not be written.
    pub fn owner_problem(&self) -> Option<&str> {
        self.owner.as_ref().err().map(String::as_str)
    }

    /// The state, retrying setup if it failed (e.g. folder permissions fixed since startup).
    pub fn state(&self) -> State {
        if self.state.read().unwrap().setup_error.is_some() {
            *self.state.write().unwrap() = mcp::setup();
        }
        self.state.read().unwrap().clone()
    }
}

/// The /status report: health, queue, curator, Ask worker, recent activity, lock.
pub fn status_report(app: &App, st: &State) -> Value {
    let started = started_ms();
    let mut s = Map::new();
    s.insert("ok".into(), json!(st.setup_error.is_none()));
    s.insert("name".into(), json!("xenolith"));
    s.insert("version".into(), json!(xenolith_core::VERSION));
    s.insert("build".into(), json!(build_hash()));
    s.insert("pid".into(), json!(std::process::id()));
    s.insert("startedAt".into(), json!(local_iso(&from_ms(started))));
    s.insert("uptimeSec".into(), json!(((now_ms() - started) as f64 / 1000.0).round() as i64));
    s.insert("wikiDir".into(), st.wiki_dir.as_ref().map(|d| json!(wiki::disp(d))).unwrap_or(Value::Null));
    s.insert("mcpUrl".into(), json!(format!("http://127.0.0.1:{}/mcp", app.port)));
    s.insert("writeMode".into(), json!(if st.curated { "curated" } else { "direct" }));
    if let Some(e) = &st.setup_error {
        s.insert("error".into(), json!(e));
    }
    let Some(dir) = st.wiki_dir.as_ref().filter(|_| st.setup_error.is_none()) else {
        s.insert("health".into(), json!("degraded"));
        s.insert("reasons".into(), json!([st.setup_error.clone().unwrap_or_else(|| "wiki unavailable".into())]));
        return Value::Object(s);
    };
    let queue = inbox::queue_stats(dir);
    let headlines = wiki::recent_headlines(dir, 3, 10);
    let curator = inbox::read_curator_status(dir);
    let paused = inbox::is_paused(dir);
    let lock = lock_info(dir, "write");
    let asker = asks::worker_status(dir);
    let heartbeat_age = curator.as_ref().and_then(|c| c.get("heartbeatAt")).and_then(Value::as_str).and_then(parse_ms).map(|t| (now_ms() - t) as f64 / 1000.0);
    let running = heartbeat_age.is_some_and(|a| a < 180.0);
    let errors = app.counters.snapshot();
    let mut reasons: Vec<String> = vec![];
    if let Some(p) = app.owner_problem() {
        reasons.push(format!("approvals need the owner key, which could not be written: {p}"));
    }
    let (audit_health, audit_problem) = audit::health(&AppPaths::current());
    s.insert("audit".into(), audit_health);
    reasons.extend(audit_problem);
    let oldest = queue["oldestPendingAt"].as_str().and_then(parse_ms);
    let wait_min = oldest.map(|t| (now_ms() - t) as f64 / 60_000.0).unwrap_or(0.0);
    let pending = queue["pending"].as_i64().unwrap_or(0);
    if st.curated {
        let dead = queue["dead"].as_i64().unwrap_or(0);
        if dead > 0 {
            reasons.push(format!("{dead} note(s) failed curation"));
        }
        if pending > 0 && wait_min > 30.0 {
            reasons.push(format!("backlog: {pending} note(s), oldest waiting {} min", wait_min.round() as i64));
        }
        if pending > 0 && !running {
            reasons.push("curator is not running".into());
        }
        let cstate = curator.as_ref().and_then(|c| c.get("state")).and_then(Value::as_str).unwrap_or("");
        if running && cstate == "signed_out" {
            reasons.push("curator is signed out of ChatGPT".into());
        }
        if running && (cstate == "rate_limited" || cstate == "error") {
            let last = curator.as_ref().and_then(|c| c.get("lastError")).and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or(cstate);
            reasons.push(format!("curator: {last}"));
        }
        let held = xenolith_core::held::pending_count(dir) + xenolith_core::forget::pending_count(dir);
        s.insert("held".into(), json!(held));
        let (approvals, problem) = xenolith_core::settings::approvals_and_problem(dir);
        s.insert("approvals".into(), json!(approvals.as_str()));
        s.insert("autoApplied".into(), xenolith_core::held::auto_summary(dir));
        // The models in use (for the tray's menu), and a choice this ChatGPT account does not offer.
        let models = xenolith_core::models::overview(dir, &st.config);
        reasons.extend(models["problems"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from));
        s.insert("models".into(), json!({ "curator": { "model": models["curator"]["model"], "reasoningEffort": models["curator"]["reasoningEffort"] }, "ask": { "model": models["ask"]["model"], "reasoningEffort": models["ask"]["reasoningEffort"] } }));
        let cleanup = xenolith_core::lint::schedule_status(dir, xenolith_core::curator::cleanups_off(&st.config));
        reasons.extend(cleanup["problem"].as_str().map(String::from));
        s.insert("cleanup".into(), cleanup);
        // Hybrid search: on, and whether this process can reach the API key (else searches are BM25 alone).
        if let Some(e) = xenolith_core::embed::settings() {
            s.insert("embeddings".into(), json!({ "model": e.model, "keyAvailable": xenolith_core::embed::api_key(&e).is_some() }));
        }
        reasons.extend(problem);
        if held > 0 {
            reasons.push(format!("{held} change(s) need your OK (window > Inbox)"));
        }
        let skipped: Vec<&str> = curator.as_ref().and_then(|c| c["lastRun"]["skipped"].as_array()).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if !skipped.is_empty() {
            reasons.push(format!("curator: {} page(s) did not fit in the last batch's context: {}", skipped.len(), skipped.join(", ")));
        }
    }
    // Followed spaces: notes their curators refused for good, and a curator that refuses this
    // machine's token or answers as another curator. A curator that is just out of reach is normal
    // for a laptop away from home, so that is shown but is not a problem.
    let sync = xenolith_core::sync::status(&AppPaths::current(), dir);
    if sync["spaces"].as_array().is_some_and(|a| !a.is_empty()) {
        let dead = sync["dead"].as_u64().unwrap_or(0);
        if dead > 0 {
            reasons.push(format!("{dead} note(s) refused by a curator for good (xenolith sync status)"));
        }
        for sp in sync["spaces"].as_array().into_iter().flatten() {
            if matches!(sp["last"]["errorKind"].as_str(), Some("auth" | "curator")) {
                reasons.push(format!("sync of {}: {}", sp["space"].as_str().unwrap_or("?"), sp["last"]["error"].as_str().unwrap_or("")));
            }
        }
        s.insert("sync".into(), sync);
    }
    let last15 = errors["last15m"].as_i64().unwrap_or(0);
    if last15 > 0 {
        reasons.push(format!("{last15} server error(s) in the last 15 min"));
    }
    let last_note = queue["newestAt"].as_str().and_then(parse_ms).unwrap_or(0);
    let last = headlines.first();
    let last_log = last.and_then(|h| parse_ms(&format!("{}T{}:00", h.date, h.time))).unwrap_or(0);
    let last_write = if last_note > last_log {
        json!({ "at": queue["newestAt"], "text": "note queued" })
    } else if let Some(h) = last {
        json!({ "at": format!("{} {}", h.date, h.time), "text": h.text })
    } else {
        Value::Null
    };
    s.insert("health".into(), json!(if reasons.is_empty() { "ok" } else { "degraded" }));
    s.insert("reasons".into(), json!(reasons));
    s.insert("queue".into(), queue);
    s.insert("lastWrite".into(), last_write);
    s.insert("recent".into(), serde_json::to_value(&headlines).unwrap_or(Value::Null));
    let cur = match curator {
        Some(Value::Object(mut c)) => {
            c.insert("running".into(), json!(running));
            c.insert("heartbeatAgeSec".into(), heartbeat_age.map(|a| json!(a.round() as i64)).unwrap_or(Value::Null));
            c.insert("paused".into(), json!(paused));
            Value::Object(c)
        }
        _ => json!({ "running": false, "paused": paused }),
    };
    s.insert("curator".into(), cur);
    s.insert("requests".into(), errors);
    s.insert("ask".into(), asker);
    s.insert(
        "lock".into(),
        lock.map(|l| {
            json!({
                "state": l.state.map(|x| x.as_str()),
                "label": l.owner.as_ref().and_then(|o| o.label.clone()),
                "pid": l.owner.as_ref().map(|o| o.pid),
                "ageSec": (l.age_ms as f64 / 1000.0).round() as i64,
            })
        })
        .unwrap_or(Value::Null),
    );
    Value::Object(s)
}

/// /status for a caller that may not read the wiki ("local": "none" in access.json, no owner key):
/// the service's health without what the wiki says.
pub fn status_for(app: &App, st: &State, caller: Caller) -> Value {
    let mut s = status_report(app, st);
    if !caller.may(Level::Read) {
        if let Some(o) = s.as_object_mut() {
            o.remove("recent");
            o.remove("lastWrite");
        }
    }
    s
}

// ---------------------------------------------------------------- responses

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    pub fn json(status: u16, v: &Value) -> Self {
        Reply { status, body: format!("{v}\n").into_bytes(), headers: vec![("Content-Type".into(), "application/json".into()), ("Cache-Control".into(), "no-store".into())] }
    }
    pub fn with(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.into(), v.into()));
        self
    }
}

/// What a request log line records about one HTTP request.
#[derive(Default)]
pub struct Entry {
    pub rid: String,
    pub path: String,
    pub rpc: Option<String>,
    pub sid: Option<String>,
    pub client: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub quiet: bool,
}

fn describe_rpc(body: &Value) -> String {
    let msgs: Vec<&Value> = match body {
        Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    msgs.iter()
        .map(|m| {
            if m.get("method").and_then(Value::as_str) == Some("tools/call") {
                format!("tools/call {}", m["params"]["name"].as_str().unwrap_or("undefined"))
            } else if let Some(method) = m.get("method").and_then(Value::as_str) {
                method.to_string()
            } else if m.get("result").is_some() {
                "response".into()
            } else {
                "?".into()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn sdk_error(status: u16, code: i64, message: &str) -> Reply {
    Reply::json(status, &rpc_error(Value::Null, code, message))
}

// ---------------------------------------------------------------- sync (xenolith_core::sync)

/// /sync/v1/spaces/<kind>/<name>/pages[/<slug>] and /log/<date> (GET, read), and /entries (POST,
/// append or write per entry). A space the caller may not read is "not found", like one that
/// doesn't exist; a space this machine only follows sends the caller to its curator (421).
fn sync_route(app: &App, req: &Request, method: &str, path: &str, who: Principal, resource: &Resource, entry: &mut Entry) -> Reply {
    use xenolith_core::access::Level;
    use xenolith_core::spaces::{self, SpaceId};
    use xenolith_core::sync;
    if path == format!("{}/spaces", sync::API) {
        return spaces_list(app, method, who, resource, entry);
    }
    let rest = path.strip_prefix(&format!("{}/spaces/", sync::API)).unwrap_or("");
    let parts: Vec<&str> = rest.split('/').collect();
    let not_found = || Reply::json(404, &json!({ "error": "not_found", "error_description": "No such space or page here." }));
    let (space, tail) = match parts.as_slice() {
        [kind, name, tail @ ..] if !tail.is_empty() => match SpaceId::parse(&format!("{kind}:{name}")) {
            Ok(id) if id.kind == *kind && id.name == *name => (id, tail.to_vec()),
            _ => return not_found(),
        },
        _ => return not_found(),
    };
    let st = app.state();
    let Some(root) = st.wiki_dir.clone().filter(|_| st.setup_error.is_none()) else {
        return Reply::json(503, &json!({ "error": st.setup_error.clone().unwrap_or_default() }));
    };
    let paths = AppPaths::current();
    let policy = access::load(&paths);
    let now = now_ms();
    let Some(who) = policy.current(&who, now) else {
        return challenge(resource, Some("invalid_token"), None, "The token has been revoked or has expired.");
    };
    if !policy.allows(&who, &space, Level::Read, now) || !spaces::exists(&root, &space) {
        xenolith_core::audit::best_effort(
            &paths,
            "refused",
            &who.name(),
            vec![("space", json!(space.to_string())), ("need", json!("read")), ("why", json!("sync: not granted")), ("transport", json!("sync"))],
        );
        return not_found();
    }
    entry.client = Some(who.name());
    let dir = spaces::space_dir(&root, &space);
    // A copy is not the curator: its pages lag, it has no curator id, and entries go to the curator.
    let cfg = sync::Config::load(&paths);
    if let Some(curator) = cfg.curator_of(&space) {
        // Its URL when another machine could use it; a loopback address means nothing there.
        let url = cfg.peers.get(curator).filter(|u| u.starts_with("https://"));
        let at = url.map(|u| format!(", at {u}")).unwrap_or_default();
        return Reply::json(
            421,
            &json!({ "error": "not_curator", "error_description": format!("This machine follows {space}; its curator is {curator}{at}."), "curator": curator, "curatorUrl": url }),
        );
    }
    match (method, tail.as_slice()) {
        ("GET", ["pages"]) => match sync::page_list(&dir) {
            Ok(v) => Reply::json(200, &v),
            Err(e) => Reply::json(500, &json!({ "error": "server_error", "error_description": format!("Could not list the pages: {e}") })),
        },
        ("GET", ["pages", slug]) => match sync::page_get(&dir, slug) {
            Some(v) => Reply::json(200, &v),
            None => not_found(),
        },
        ("GET", ["log", date]) => match sync::log_get(&dir, date) {
            Some(v) => Reply::json(200, &v),
            None => not_found(),
        },
        ("POST", ["entries"]) => {
            let body: Value = match read_body(req).ok().and_then(|b| serde_json::from_str(&b).ok()) {
                Some(v) => v,
                None => return Reply::json(400, &json!({ "error": "invalid_request", "error_description": "Send {\"entries\": [...]} as JSON." })),
            };
            let entries = body["entries"].as_array().cloned().unwrap_or_default();
            if entries.is_empty() || entries.len() > sync::MAX_ENTRIES {
                return Reply::json(400, &json!({ "error": "invalid_request", "error_description": format!("Send 1 to {} entries.", sync::MAX_ENTRIES) }));
            }
            let can = |l: Level| policy.allows(&who, &space, l, now);
            let (results, conflict) = sync::accept(&dir, &who, &entries, &can);
            let count = |r: &str| results.iter().filter(|x| x["result"] == r).count();
            xenolith_core::audit::best_effort(
                &paths,
                "sync",
                &who.name(),
                vec![
                    ("space", json!(space.to_string())),
                    ("accepted", json!(count("accepted"))),
                    ("duplicates", json!(count("duplicate"))),
                    ("conflicts", json!(count("conflict"))),
                    ("refused", json!(count("refused"))),
                ],
            );
            entry.result = Some(if conflict { "conflict".into() } else { "ok".into() });
            // 409 when any page write came in against an older version: it is a proposal now.
            Reply::json(if conflict { 409 } else { 200 }, &json!({ "results": results }))
        }
        (_, ["entries"]) => Reply::json(405, &json!({ "error": "Use POST" })).with("Allow", "POST"),
        (_, ["pages" | "log", ..]) => Reply::json(405, &json!({ "error": "Use GET" })).with("Allow", "GET"),
        _ => not_found(),
    }
}

/// GET /sync/v1/spaces: the spaces this machine curates that the caller may read (never
/// machine:local), which a satellite follows.
fn spaces_list(app: &App, method: &str, who: Principal, resource: &Resource, entry: &mut Entry) -> Reply {
    use xenolith_core::access::Level;
    if method != "GET" {
        return Reply::json(405, &json!({ "error": "Use GET" })).with("Allow", "GET");
    }
    let st = app.state();
    let Some(root) = st.wiki_dir.clone().filter(|_| st.setup_error.is_none()) else {
        return Reply::json(503, &json!({ "error": st.setup_error.clone().unwrap_or_default() }));
    };
    let policy = access::load(&AppPaths::current());
    let now = now_ms();
    let Some(who) = policy.current(&who, now) else {
        return challenge(resource, Some("invalid_token"), None, "The token has been revoked or has expired.");
    };
    entry.client = Some(who.name());
    // A space this machine only follows is not its to list: its curator lists it.
    let cfg = xenolith_core::sync::Config::load(&AppPaths::current());
    Reply::json(200, &xenolith_core::sync::space_list(&root, &|s| policy.allows(&who, s, Level::Read, now) && cfg.curator_of(s).is_none()))
}

/// POST /sync/v1/pair: a satellite joining with a pairing code (xenolith_core::pair). No token: the
/// proof of the code is in the body. A bad join counts like a bad token against its source.
fn pair_route(app: &App, req: &Request, method: &str, entry: &mut Entry) -> Reply {
    if method != "POST" {
        return Reply::json(405, &json!({ "error": "Use POST" })).with("Allow", "POST");
    }
    let source = source_of(req);
    let now = Instant::now();
    if let Some(wait) = app.bad_tokens.blocked(&source, now) {
        entry.result = Some("rate-limited".into());
        return too_many(wait);
    }
    let body: Value = read_body(req).ok().and_then(|b| serde_json::from_str(&b).ok()).unwrap_or(Value::Null);
    match xenolith_core::pair::accept(&AppPaths::current(), &body, &clip_str(&source, 120), now_ms()) {
        Ok(answer) => {
            entry.result = Some("paired".into());
            Reply::json(200, &answer)
        }
        Err(r) => {
            entry.result = Some("pair-refused".into());
            if r.bad && app.bad_tokens.failed(&source, now) {
                audit::best_effort(
                    &AppPaths::current(),
                    "refused",
                    "an unknown machine",
                    vec![("why", json!("too many bad pairing codes")), ("source", json!(clip_str(&source, 120))), ("transport", json!("sync"))],
                );
            }
            Reply::json(r.status, &json!({ "error": "pair_refused", "error_description": r.why }))
        }
    }
}

fn too_many(wait: Duration) -> Reply {
    let secs = wait.as_secs().max(1);
    Reply::json(429, &json!({ "error": "too_many_requests", "error_description": format!("Too many bad tokens from here. Try again in {secs} s.") })).with("Retry-After", &secs.to_string())
}

// ---------------------------------------------------------------- tokens (OAuth 2.1 resource server, as MCP's authorization spec has it)

/// RFC 9728's metadata for the MCP endpoint: what it is and which scopes it knows.
pub fn protected_resource(base: &str) -> Value {
    json!({
        "resource": format!("{base}/mcp"),
        "scopes_supported": access::scopes_supported(),
        "bearer_methods_supported": ["header"],
        "resource_name": "Xenolith",
    })
}

/// A 401 (or 403 for insufficient_scope) with RFC 6750's WWW-Authenticate challenge, pointing at
/// the resource metadata as MCP clients expect. Without credentials there is no error code.
fn challenge(resource: &Resource, error: Option<&str>, scope: Option<&str>, why: &str) -> Reply {
    let mut parts = vec![];
    if let Some(e) = error {
        parts.push(format!("error=\"{e}\""));
    }
    parts.push(format!("error_description=\"{why}\""));
    if let Some(s) = scope {
        parts.push(format!("scope=\"{s}\""));
    }
    parts.push(format!("resource_metadata=\"{}\"", resource.metadata()));
    let status = if error == Some("insufficient_scope") { 403 } else { 401 };
    Reply::json(status, &json!({ "error": error.unwrap_or("unauthorized"), "error_description": why })).with("WWW-Authenticate", &format!("Bearer {}", parts.join(", ")))
}

/// 127.0.0.1 and localhost name the same server: a token issued for one works at the other.
fn same_resource(a: &str, b: &str) -> bool {
    a.replace("://localhost:", "://127.0.0.1:") == b.replace("://localhost:", "://127.0.0.1:")
}

/// Whether a request is this machine's own: it reached the loopback listener directly (the Host
/// check has already run) and no proxy says it passed it on. A forwarder that adds none of those
/// headers (a plain TCP tunnel) is indistinguishable from a local process: README, Security.
fn is_local(req: &Request) -> bool {
    !req.forwarded() && req.peer.is_none_or(|p| p.ip().is_loopback())
}

/// Who a request to /mcp or /sync comes from, or the challenge to answer it with. Without a token
/// it is this machine's own when `local` (see is_local), unless access.json says "local": "none".
/// `also`: the address the request used, which a token issued for it may name.
fn principal_of(resource: &Resource, also: &str, bearer: &Option<Option<String>>, local: bool, entry: &mut Entry) -> Result<Principal, Reply> {
    let policy = access::load(&AppPaths::current());
    let now = now_ms();
    match bearer {
        None if local && policy.local.is_some() => Ok(Principal::Local),
        None => {
            entry.result = Some("unauthorized".into());
            Err(challenge(resource, None, None, if local { "This server needs a token." } else { "This server needs a token: a forwarded request is never this machine's own." }))
        }
        Some(None) => {
            entry.result = Some("unauthorized".into());
            Err(challenge(resource, Some("invalid_request"), None, "Send the token as Authorization: Bearer <token>."))
        }
        Some(Some(t)) => {
            // A token issued for one server works at no other: this one, by its public URL or the address used.
            let rec = policy.token(t).filter(|r| r.resource.as_deref().is_none_or(|res| same_resource(res, &resource.mcp()) || same_resource(res, &format!("{also}/mcp"))));
            match rec.and_then(|_| policy.principal_for(t, now)) {
                Some(p) => Ok(p),
                None => {
                    entry.result = Some("invalid-token".into());
                    // Never the token itself: a hash of it, enough to tell attempts with one token apart.
                    xenolith_core::audit::best_effort(
                        &AppPaths::current(),
                        "refused",
                        "an unknown token",
                        vec![("why", json!("invalid token")), ("token", json!(&access::hash_token(t)[..12])), ("transport", json!("http"))],
                    );
                    Err(challenge(resource, Some("invalid_token"), None, "The token is unknown, expired, revoked, or for another server."))
                }
            }
        }
    }
}

/// principal_of, behind the bad-token limit: a source that sent BAD_TOKENS_PER_MINUTE bad tokens
/// in a minute gets 429 for every request with a token for BAD_TOKEN_BLOCK.
fn authenticate(app: &App, req: &Request, resource: &Resource, also: &str, bearer: &Option<Option<String>>, local: bool, entry: &mut Entry) -> Result<Principal, Reply> {
    let Some(_) = bearer else { return principal_of(resource, also, bearer, local, entry) };
    let source = source_of(req);
    let now = Instant::now();
    if let Some(wait) = app.bad_tokens.blocked(&source, now) {
        entry.result = Some("rate-limited".into());
        return Err(too_many(wait));
    }
    let r = principal_of(resource, also, bearer, local, entry);
    if r.is_err() && app.bad_tokens.failed(&source, now) {
        audit::best_effort(&AppPaths::current(), "refused", "an unknown token", vec![("why", json!("too many bad tokens")), ("source", json!(clip_str(&source, 120))), ("transport", json!("http"))]);
    }
    r
}

fn handle(app: &App, req: &Request, entry: &mut Entry) -> Reply {
    let host = req.header("host").unwrap_or_default().to_lowercase();
    if host != format!("127.0.0.1:{}", app.port) && host != format!("localhost:{}", app.port) {
        entry.result = Some("forbidden-host".into());
        return Reply::json(403, &json!({ "error": "Forbidden: unexpected Host header" }));
    }
    if let Some(origin) = req.header("origin") {
        // Loopback is not an origin: a page on another local server is still cross-origin.
        // The listener is plain HTTP, and the browser must use this exact host and port.
        let authority = if app.port == 80 { host.trim_end_matches(":80") } else { &host };
        if !origin.eq_ignore_ascii_case(&format!("http://{authority}")) {
            entry.result = Some("forbidden-origin".into());
            return Reply::json(403, &json!({ "error": "Forbidden: cross-origin request" }));
        }
    }
    // Browsers say who sent a request: refuse what another site's page sent (an <img> or a form needs
    // no CORS approval, so the Origin check alone does not stop it). Apps and the tray send no such header.
    if req.header("sec-fetch-site").is_some_and(|s| !matches!(s.to_ascii_lowercase().as_str(), "same-origin" | "none")) {
        entry.result = Some("forbidden-site".into());
        return Reply::json(403, &json!({ "error": "Forbidden: sent by another site" }));
    }
    let url = req.url.clone();
    let (path, query) = url.split_once('?').map(|(p, q)| (p.to_string(), q.to_string())).unwrap_or((url.clone(), String::new()));
    let path = percent_decode_path(&path);
    entry.path = path.clone();
    let method = req.method.as_str();
    let local = is_local(req);
    let also = format!("http://{host}");
    // Forwarded callers see the configured public URL in the resource metadata, when there is one.
    let public = public_url(&app.state.read().unwrap().config).filter(|_| !local);
    let resource = Resource { base: public.clone().unwrap_or_else(|| also.clone()) };
    // Tokens travel in the Authorization header only (RFC 6750 5.3): logs and histories keep URLs.
    if query.split('&').any(|kv| kv.split('=').next().is_some_and(|k| k.eq_ignore_ascii_case("access_token") || k.eq_ignore_ascii_case("token"))) || url.contains(access::TOKEN_PREFIX) {
        entry.result = Some("token-in-url".into());
        return Reply::json(400, &json!({ "error": "invalid_request", "error_description": "Send tokens in the Authorization header, never in a URL." }));
    }
    if path == "/.well-known/oauth-protected-resource" || path == "/.well-known/oauth-protected-resource/mcp" || (public.is_some() && path == resource.metadata_path()) {
        if method != "GET" {
            return Reply::json(405, &json!({ "error": "Use GET" })).with("Allow", "GET");
        }
        return Reply::json(200, &protected_resource(&resource.base));
    }
    // A token is for /mcp and sync: the window's API, /status and /health belong to this machine
    // and show its folders.
    let bearer = access::bearer_of(req.header("authorization").as_deref());
    let is_sync = path.starts_with(&format!("{}/", xenolith_core::sync::API));
    if bearer.is_some() && path != "/mcp" && !is_sync {
        entry.result = Some("forbidden-token".into());
        return Reply::json(403, &json!({ "error": "Forbidden: tokens work at /mcp and /sync only" }));
    }
    if is_sync {
        if app.closing.load(Ordering::SeqCst) {
            return Reply::json(503, &json!({ "error": "xenolith is restarting; retry shortly." }));
        }
        if path == xenolith_core::pair::ROUTE {
            return pair_route(app, req, method, entry);
        }
        // Sync is remote by nature: a token always, even from this machine.
        if bearer.is_none() {
            entry.result = Some("unauthorized".into());
            return challenge(&resource, None, None, "Sync needs a token, even from this machine.");
        }
        let principal = match authenticate(app, req, &resource, &also, &bearer, local, entry) {
            Ok(p) => p,
            Err(r) => return r,
        };
        return sync_route(app, req, method, &path, principal, &resource, entry);
    }
    if path == "/health" {
        if method != "GET" {
            return Reply::json(405, &json!({ "error": "Use GET" })).with("Allow", "GET");
        }
        // Open to anyone who reaches the port, so nothing private: no folders, no setup error text.
        let st = app.state();
        let mut h = Map::new();
        h.insert("ok".into(), json!(st.setup_error.is_none()));
        h.insert("name".into(), json!("xenolith"));
        h.insert("version".into(), json!(xenolith_core::VERSION));
        h.insert("build".into(), json!(build_hash()));
        h.insert("pid".into(), json!(std::process::id()));
        h.insert("uptimeSec".into(), json!(((now_ms() - started_ms()) as f64 / 1000.0).round() as i64));
        h.insert("writeMode".into(), json!(if st.curated { "curated" } else { "direct" }));
        return Reply::json(if st.setup_error.is_some() { 503 } else { 200 }, &Value::Object(h));
    }
    // /status, the window and its API show this machine's folders and decide its changes: its
    // own processes only, never a forwarded request.
    let own = path == "/status" || path == "/ui" || path.starts_with("/ui/") || path.starts_with("/api/");
    if own && !local {
        entry.result = Some("forbidden-forwarded".into());
        return Reply::json(403, &json!({ "error": "Forbidden: this is for this machine's own processes; a forwarded request is never one of them" }));
    }
    if path == "/status" {
        if method != "GET" {
            return Reply::json(405, &json!({ "error": "Use GET" })).with("Allow", "GET");
        }
        let st = app.state();
        entry.quiet = true;
        return Reply::json(200, &status_for(app, &st, app.caller(req)));
    }
    if path == "/ui" || path.starts_with("/ui/") || path.starts_with("/api/") {
        let st = app.state();
        return uiapi::handle(app, &st, req, method, &path, &query, entry);
    }
    if path != "/mcp" {
        return Reply::json(404, &json!({ "error": "Not found: the MCP endpoint is /mcp, the tray window /ui/" }));
    }
    if method != "POST" {
        return Reply::json(405, &rpc_error(Value::Null, -32000, "Method not allowed: this server is stateless, use POST.")).with("Allow", "POST");
    }
    if app.closing.load(Ordering::SeqCst) {
        return Reply::json(503, &rpc_error(Value::Null, -32000, "xenolith is restarting; retry shortly."));
    }
    let principal = match authenticate(app, req, &resource, &also, &bearer, local, entry) {
        Ok(p) => p,
        Err(r) => return r,
    };
    let raw = match read_body(req) {
        Ok(b) => b,
        Err(status) => {
            entry.result = Some("parse-error".into());
            return Reply::json(status, &rpc_error(Value::Null, -32700, "Parse error: request body too large"));
        }
    };
    let body: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            entry.result = Some("parse-error".into());
            return Reply::json(400, &rpc_error(Value::Null, -32700, &format!("Parse error: {e}")));
        }
    };
    entry.rpc = Some(describe_rpc(&body));
    let mut sid = req.header("mcp-session-id").filter(|s| !s.is_empty());
    let messages: Vec<Value> = match &body {
        Value::Array(a) => a.clone(),
        other => vec![other.clone()],
    };
    let init = messages.iter().find(|m| m.get("method").and_then(Value::as_str) == Some("initialize")).cloned();
    let mut reply_headers: Vec<(String, String)> = vec![];
    if let Some(init) = &init {
        let new_sid = random_uuid();
        let ci = &init["params"]["clientInfo"];
        let name = if ci.is_object() { format!("{} {}", ci["name"].as_str().unwrap_or(""), ci["version"].as_str().unwrap_or("")).trim().to_string() } else { "unknown".into() };
        app.sessions.set(&new_sid, &name);
        reply_headers.push(("Mcp-Session-Id".into(), new_sid.clone()));
        sid = Some(new_sid);
    }
    entry.sid = sid.clone();
    entry.client = sid.as_deref().and_then(|s| app.sessions.get(s));
    // The SDK's transport rules.
    let accept = req.header("accept").unwrap_or_default();
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return with_headers(sdk_error(406, -32000, "Not Acceptable: Client must accept both application/json and text/event-stream"), reply_headers);
    }
    let ct = req.header("content-type").unwrap_or_default();
    if !ct.to_lowercase().split(';').next().unwrap_or("").trim().eq("application/json") {
        return with_headers(sdk_error(415, -32000, "Unsupported Media Type: Content-Type must be application/json"), reply_headers);
    }
    if body.is_array() && messages.len() > MAX_BATCH {
        return with_headers(sdk_error(400, -32600, &format!("Invalid Request: Batch must not exceed {MAX_BATCH} messages")), reply_headers);
    }
    if !messages.iter().all(mcp::valid_message) {
        return with_headers(sdk_error(400, -32700, "Parse error: Invalid JSON-RPC message"), reply_headers);
    }
    if init.is_some() && messages.len() > 1 {
        return with_headers(sdk_error(400, -32600, "Invalid Request: Only one initialization request is allowed"), reply_headers);
    }
    if let Some(v) = req.header("mcp-protocol-version").filter(|v| init.is_none() && !mcp::SUPPORTED_VERSIONS.contains(&v.as_str())) {
        return with_headers(sdk_error(400, -32000, &format!("Bad Request: Unsupported protocol version: {v} (supported versions: {})", mcp::SUPPORTED_VERSIONS.join(", "))), reply_headers);
    }
    if !messages.iter().any(mcp::is_request) {
        return with_headers(Reply { status: 202, body: vec![], headers: vec![] }, reply_headers);
    }
    let st = app.state();
    let client = sid.as_deref().and_then(|s| app.sessions.get(s)).unwrap_or_default();
    let on_error = |m: &str| app.counters.error(m);
    let ctx = Ctx::new("http", Some(entry.rid.clone()), client, &app.reqlog, Some(&on_error), principal);
    let ci = Mutex::new(None);
    let responses: Vec<Value> = messages.iter().filter_map(|m| mcp::handle(&st, &ctx, m, &ci)).collect();
    let out = if responses.len() == 1 { responses[0].clone() } else { Value::Array(responses) };
    // A call the token's scopes can't reach at all: 403 insufficient_scope, so the client can ask
    // for the scope (MCP's step-up); the JSON-RPC answer still says what was refused.
    if let (Some(scope), 1) = (ctx.scope_needed.lock().unwrap().clone(), messages.len()) {
        entry.result = Some("insufficient-scope".into());
        let mut r = challenge(&resource, Some("insufficient_scope"), Some(&scope), "The token's scopes don't reach what this call needs.");
        r.body = format!("{out}\n").into_bytes();
        return with_headers(r, reply_headers);
    }
    with_headers(Reply { status: 200, body: out.to_string().into_bytes(), headers: vec![("Content-Type".into(), "application/json".into())] }, reply_headers)
}

fn with_headers(mut r: Reply, extra: Vec<(String, String)>) -> Reply {
    r.headers.extend(extra);
    r
}

fn percent_decode_path(p: &str) -> String {
    // URL.pathname keeps percent-escapes; only %2F-style traversal matters to the /ui check, which rejects them.
    p.to_string()
}

/// The request body as text, or 413 when it was over httpd::MAX_BODY.
pub fn read_body(req: &Request) -> Result<String, u16> {
    req.body().map(|b| String::from_utf8_lossy(b).into_owned())
}

/// Serves until the parent closes stdin (`parent_stdin`), or, with `exit_on_upgrade`, until a new
/// build of this program replaces it (the service manager then starts the new one).
/// XENOLITH_PROCESS must already be set (main does it before any thread starts).
pub fn run(port: u16, parent_stdin: bool, exit_on_upgrade: bool) -> ! {
    // As the user (macOS, Linux, or `serve --http` by hand): the owner key is the user's alone.
    let stop = Waker::new();
    if parent_stdin {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut sink = [0u8; 4096];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            stop.stop();
        });
    }
    match serve(port, stop, exit_on_upgrade, None) {
        Ok(Ended::Stopped) => std::process::exit(0),
        Ok(Ended::Upgraded) => std::process::exit(UPGRADED_EXIT),
        Err(e) => {
            log(&e);
            std::process::exit(1)
        }
    }
}

/// Exit code after an upgrade: not 0, so service managers restart the program.
pub const UPGRADED_EXIT: i32 = 75;

pub enum Ended {
    Stopped,
    Upgraded,
}

/// This program's file, to notice when an install replaces it: which file it is, its size and time.
fn exe_stamp(exe: &std::path::Path) -> Option<((u64, u64), u64, std::time::SystemTime)> {
    let m = std::fs::metadata(exe).ok()?;
    Some((xenolith_core::sys::file_identity(exe)?, m.len(), m.modified().ok()?))
}

/// Serves on 127.0.0.1:`port` until `stop` (graceful: in-flight requests get up to 5 s), or until this
/// program's file changes (`watch_exe`). `owner_sid`: the Windows service's installing user (its
/// ini's `owner`), who may read the owner key this service writes.
pub fn serve(port: u16, stop: Arc<Waker>, watch_exe: bool, owner_sid: Option<String>) -> Result<Ended, String> {
    started_ms();
    let state = mcp::setup();
    let reqlog = mcp::open_request_log("service", &state.config);
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("cannot listen on 127.0.0.1:{port}: {e}"))?;
    let actual = listener.local_addr().map(|a| a.port()).unwrap_or(port);
    let mode = if state.curated { "curated" } else { "direct" };
    let wiki_shown = state.wiki_dir.as_ref().map(|d| wiki::disp(d)).unwrap_or_else(|| "(unavailable)".into());
    // A new owner key each start, for the tray and `xenolith held` (owner.rs).
    let owner = owner::create(&AppPaths::current(), actual, &owner_sid.into_iter().collect::<Vec<_>>()).map_err(|e| e.to_string());
    if let Err(e) = &owner {
        log(&format!("the owner key could not be written ({e}); deciding held changes in the window waits for a restart"));
    }
    let app = Arc::new(App {
        state: RwLock::new(state),
        reqlog,
        sessions: Sessions::load(AppPaths::current().http_sessions()),
        counters: Counters::new(),
        port: actual,
        closing: AtomicBool::new(false),
        inflight: AtomicUsize::new(0),
        owner,
        bad_tokens: BadTokens::default(),
    });
    log(&format!("v{} listening on http://127.0.0.1:{actual}/mcp ({mode} writes); wiki at {wiki_shown}", xenolith_core::VERSION));

    let exe = std::env::current_exe().ok().filter(|_| watch_exe);
    let stamp = exe.as_deref().and_then(exe_stamp);
    let shared = app.clone();
    let handler = move |req: &Request| -> Response {
        app.inflight.fetch_add(1, Ordering::SeqCst);
        let t0 = now_ms();
        let rid = new_request_id();
        let ua = req.header("user-agent");
        let mut entry = Entry { rid: rid.clone(), ..Entry::default() };
        let reply = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle(&app, req, &mut entry))).unwrap_or_else(|_| {
            log("request failed: internal error");
            entry.error = Some("internal error".into());
            Reply::json(500, &rpc_error(Value::Null, -32603, "Internal error"))
        });
        let status = reply.status;
        app.inflight.fetch_sub(1, Ordering::SeqCst);
        if !(entry.quiet && status < 400) {
            if status >= 500 {
                app.counters.error(&format!("HTTP {status} {}", entry.rpc.clone().unwrap_or_else(|| entry.path.clone())));
            }
            let path = if entry.path.is_empty() { req.url.clone() } else { entry.path.clone() };
            app.reqlog.write(vec![
                ("kind", json!("http")),
                ("rid", json!(rid)),
                ("transport", json!("http")),
                ("method", json!(req.method)),
                ("path", json!(path)),
                ("ua", ua.map(|u| json!(clip_str(&u, 80))).unwrap_or(Value::Null)),
                ("rpc", entry.rpc.map(Value::String).unwrap_or(Value::Null)),
                ("sid", entry.sid.map(Value::String).unwrap_or(Value::Null)),
                ("client", entry.client.map(|c| json!(clip_str(&c, 80))).unwrap_or(Value::Null)),
                ("result", entry.result.map(Value::String).unwrap_or(Value::Null)),
                ("error", entry.error.map(Value::String).unwrap_or(Value::Null)),
                ("status", json!(status)),
                ("ms", json!(now_ms() - t0)),
            ]);
        }
        Response { status, headers: reply.headers, body: reply.body }
    };
    let acceptor = {
        let stop = stop.clone();
        std::thread::spawn(move || httpd::serve(listener, Arc::new(handler), stop))
    };
    let mut ended = Ended::Stopped;
    while !stop.wait_stop(2000) {
        let Some(exe) = &exe else { continue };
        let now = exe_stamp(exe);
        // Once the new file has been in place for a second (an install copies, then renames).
        let settled = now.is_some_and(|(_, _, t)| t.elapsed().is_ok_and(|e| e > Duration::from_secs(1)));
        if now.is_some() && now != stamp && settled {
            log("a new build of xenolith was installed; restarting");
            ended = Ended::Upgraded;
            stop.stop();
        }
    }
    shared.closing.store(true, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(5);
    while shared.inflight.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(25));
    }
    log(&format!("shutting down ({})", if matches!(ended, Ended::Upgraded) { "upgraded" } else { "stopped" }));
    if let Ok(k) = &shared.owner {
        owner::remove(&AppPaths::current(), k);
    }
    let _ = acceptor.join();
    Ok(ended)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_bad_tokens_a_minute_then_a_minute_of_429() {
        let b = BadTokens::default();
        let t0 = Instant::now();
        for i in 0..BAD_TOKENS_PER_MINUTE - 1 {
            assert!(!b.failed("127.0.0.1", t0 + Duration::from_secs(u64::from(i))));
            assert_eq!(b.blocked("127.0.0.1", t0), None);
        }
        assert!(b.failed("127.0.0.1", t0 + Duration::from_secs(30)), "the 20th in a minute starts a block");
        let wait = b.blocked("127.0.0.1", t0 + Duration::from_secs(31)).unwrap();
        assert!(wait <= BAD_TOKEN_BLOCK && wait > Duration::from_secs(55), "{wait:?}");
        assert_eq!(b.blocked("127.0.0.1 for 192.0.2.7", t0 + Duration::from_secs(31)), None, "another source is not blocked");
        assert_eq!(b.blocked("127.0.0.1", t0 + Duration::from_secs(91)), None, "a minute later it may try again");
        // Bad tokens spread over more than a minute never add up to a block.
        let slow = BadTokens::default();
        for i in 0..60 {
            assert!(!slow.failed("x", t0 + Duration::from_secs(i * 4)));
        }
    }

    #[test]
    fn the_resource_metadata_follows_the_public_url() {
        let local = Resource { base: "http://127.0.0.1:47850".into() };
        assert_eq!(local.mcp(), "http://127.0.0.1:47850/mcp");
        assert_eq!(local.metadata(), "http://127.0.0.1:47850/.well-known/oauth-protected-resource/mcp");
        let public = Resource { base: "https://studio.example/xenolith".into() };
        assert_eq!(public.mcp(), "https://studio.example/xenolith/mcp");
        assert_eq!(public.metadata(), "https://studio.example/.well-known/oauth-protected-resource/xenolith/mcp", "RFC 9728: between the origin and the path");
        assert_eq!(public.metadata_path(), "/.well-known/oauth-protected-resource/xenolith/mcp");
        assert_eq!(public_url(&json!({ "publicUrl": "https://studio.example/xenolith/" })).as_deref(), Some("https://studio.example/xenolith"));
        for bad in [json!({ "publicUrl": "http://studio.example" }), json!({ "publicUrl": "https://x.example/?a=1" }), json!({ "publicUrl": 4 }), json!({})] {
            assert_eq!(public_url(&bad), None, "{bad}");
        }
    }
}
