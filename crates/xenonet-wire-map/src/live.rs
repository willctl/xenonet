//! core/live.js's runtime: the tailer that reads the transcripts both apps
//! append to (core/blocks/tailer.js, whose decisions are xenonet-blocks'
//! tailer.rs), each line through its app's normalizer into the telemetry
//! (xenonet-sources' live/telemetry.rs), which holds every session's state
//! and says what each line means; and the readings of Claude plan usage
//! (core/claude-usage.js), Anthropic's and Claude desktop's.
//!
//! At start the tailer reads the last day's transcripts as history, then
//! follows them: a sweep every 2 seconds reads what was added to the files
//! it follows, and every 15th sweep walks the folders for new ones. The JS
//! watches the folders too, for lines within 30 ms; the binary has no
//! file watcher, so the sweep is how it follows them.

use crate::blocks::{health_of, Block};
use crate::gatherer::{Gather, Options};
use crate::util::{fields, lock, now_ms, num, obj};
use crate::vendor::Vendor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_base::fs::StdFs;
use xenonet_blocks::scanner::{FnRules, Limits, Scanner};
use xenonet_blocks::tailer::{Listed, Swept, Tailer};
use xenonet_schema::vendor::Sampling;
use xenonet_sources::claude_usage;
use xenonet_sources::live::telemetry::{Emit, Telemetry, Where};

const PRIME_WINDOW_MS: f64 = 24.0 * 3600e3;
const SWEEP_MS: u64 = 2000;
// Sweeps: a walk for new files every 30 seconds.
const DISCOVER_EVERY: u64 = 15;
const DEPTH: f64 = 6.0;
const LIMITS_EVERY_MS: u64 = 30_000;

/// What the telemetry says, for the state to pass on: activity, tokens
/// and refusals as live.js emits them, and a transcript read for the
/// first time after start.
#[derive(Clone, Debug)]
pub enum Event {
    Activity(Value),
    Tokens(Value),
    Refusal(Value),
    NewFile(Value),
}

pub type Listener = Arc<dyn Fn(Vec<Event>) + Send + Sync>;

/// The tailer's counters, shared with its owner: { watching, files, lines,
/// errors }.
#[derive(Clone, Debug, Default)]
pub struct TailStatus {
    pub watching: bool,
    pub files: usize,
    pub lines: u64,
    pub errors: u64,
}

// What a file the tailer follows is: what classify said, and how its
// lines are sampled for the drift check.
struct Entry {
    codex: bool,
    session_id: String,
    agent_id: Value,
    sampling: Sampling,
}

struct Tail {
    tailer: Tailer,
    scanners: Vec<Scanner>,
    entries: HashMap<String, Entry>,
    started: bool,
}

pub struct Live {
    /// The folders it reads, as real paths, with their app.
    pub roots: Vec<(String, String)>,
    tail: Mutex<Tail>,
    pub telemetry: Mutex<Telemetry>,
    pub status: Mutex<TailStatus>,
    vendor: Arc<Vendor>,
    pub anthropic: Option<Arc<Gather>>,
    pub desktop: Option<(Arc<Gather>, String)>,
    listener: OnceLock<Listener>,
    // The sweeps' and the limits' tasks, stopped with it.
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    sweeping: tokio::sync::Mutex<()>,
}

fn emits(out: Vec<Emit>) -> Vec<Event> {
    out.into_iter()
        .map(|e| match e {
            Emit::Activity(f) => Event::Activity(fields(&f)),
            Emit::Tokens(v) => Event::Tokens(v),
            Emit::Refusal(v) => Event::Refusal(v),
        })
        .collect()
}

fn stat(file: &str) -> Option<(f64, u64)> {
    let m = std::fs::metadata(file).ok()?;
    Some((xenonet_base::fs::mtime_ms_of(&m).unwrap_or(f64::NAN), m.len()))
}

fn transcript(name: &str) -> bool {
    name.ends_with(".jsonl")
}

impl Live {
    /// `claude_root` and `codex_root`: the folders the apps write
    /// transcripts in; `usage`: whether to ask Anthropic for plan usage.
    pub fn new(claude_root: &str, codex_root: &str, claude_desktop_dir: Option<&str>, usage: bool, home: &str, vendor: Arc<Vendor>) -> Arc<Live> {
        // Watch canonical long paths: an 8.3 short name on Windows names
        // the same folder another way.
        let roots: Vec<(String, String)> = [(claude_root, "claude"), (codex_root, "codex")]
            .into_iter()
            .filter(|(d, _)| !d.is_empty() && std::path::Path::new(d).exists())
            .map(|(d, app)| (std::fs::canonicalize(d).map_or_else(|_| d.to_string(), |p| p.to_string_lossy().into_owned()), app.to_string()))
            .collect();
        let now = now_ms();
        let scanners = roots.iter().map(|(dir, _)| Scanner::new(&format!("transcripts:{dir}"), dir, Limits { depth: DEPTH, ..Limits::default() }, true, now)).collect();
        let telemetry = Telemetry::new(Some(home.to_string()).filter(|h| !h.is_empty()), roots.clone());
        let anthropic = usage
            .then(|| Arc::new(Gather::new("claude-usage", Options { every_ms: Some(claude_usage::USAGE_EVERY_MS), next_in: true, fallback: Value::Null, ..Default::default() })));
        let desktop =
            claude_desktop_dir.map(|d| (Arc::new(Gather::new("claude-desktop-usage", Options { keep: true, fallback: Value::Null, ..Default::default() })), d.to_string()));
        Arc::new(Live {
            roots,
            tail: Mutex::new(Tail { tailer: Tailer::new("transcripts", PRIME_WINDOW_MS, now), scanners, entries: HashMap::new(), started: false }),
            telemetry: Mutex::new(telemetry),
            status: Mutex::new(TailStatus::default()),
            vendor,
            anthropic,
            desktop,
            listener: OnceLock::new(),
            tasks: Mutex::new(Vec::new()),
            sweeping: tokio::sync::Mutex::new(()),
        })
    }

    /// Who hears what the lines say.
    pub fn listen(&self, l: Listener) {
        let _ = self.listener.set(l);
    }

    fn tell(&self, events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        if let Some(l) = self.listener.get() {
            l(events);
        }
    }

    // ---------- reading ----------

    // A line the tailer read, through its app's normalizer: lines that
    // aren't JSON are skipped, and one that breaks what the telemetry
    // makes of it is counted as an error, so the lines after it are read.
    fn line(&self, text: &str, file: &str, e: &mut Entry, prime: bool, out: &mut Vec<Event>) {
        let Some(raw) = self.vendor.line(e.codex, text, &mut e.sampling) else { return };
        lock(&self.status).lines += 1;
        let f = Where { session_id: e.session_id.clone(), agent_id: e.agent_id.clone(), file: Value::String(file.into()) };
        let mut emitted = Vec::new();
        let r = lock(&self.telemetry).line(e.codex, &f, raw, !prime, now_ms(), &mut emitted);
        out.extend(emits(emitted));
        if r.is_err() {
            lock(&self.status).errors += 1;
        }
    }

    // Reads what's new in a file, up to `until`, as history (`prime`) or
    // live.
    fn take(&self, t: &mut Tail, file: &str, prime: bool, until: Option<u64>, out: &mut Vec<Event>) -> std::io::Result<()> {
        let Some((_, size)) = stat(file) else { return Ok(()) };
        let Some((from, to)) = t.tailer.plan_take(file, size, until) else { return Ok(()) };
        let mut f = std::fs::File::open(file)?;
        f.seek(SeekFrom::Start(from))?;
        let mut buf = vec![0u8; (to - from) as usize];
        let mut got = 0;
        while got < buf.len() {
            let n = f.read(&mut buf[got..])?;
            if n == 0 {
                break;
            }
            got += n;
        }
        buf.truncate(got);
        let lines = t.tailer.took(file, &buf, from + got as u64);
        if let Some(mut e) = t.entries.remove(file) {
            for l in &lines {
                self.line(l, file, &mut e, prime, out);
            }
            t.entries.insert(file.to_string(), e);
        }
        // The sweep stands in for the watches, so every root counts as
        // watched once it has started.
        if t.tailer.ok_after_read(self.roots.len(), self.roots.len()) {
            t.tailer.life.ok(now_ms());
        }
        Ok(())
    }

    // Reads what's new in a transcript: one it reads for the first time
    // is classified, and what it held while dormant is read as history
    // before its new lines.
    fn read(&self, t: &mut Tail, file: &str, prime: bool, out: &mut Vec<Event>) -> std::io::Result<()> {
        if !t.tailer.files.contains(file) {
            let Some(kind) = lock(&self.telemetry).classify(file) else { return Ok(()) };
            let entry = Entry {
                codex: kind.get("app").and_then(Value::as_str) == Some("codex"),
                session_id: kind.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_string(),
                agent_id: kind.get("agentId").cloned().unwrap_or(Value::Null),
                sampling: Sampling::default(),
            };
            let session = entry.session_id.clone();
            t.entries.insert(file.to_string(), entry);
            t.tailer.add_file(file);
            lock(&self.status).files = t.tailer.files.len();
            if !prime {
                out.push(Event::NewFile(json!({ "file": file, "sessionId": session })));
            }
            if let Some(held) = t.tailer.take_held(file) {
                if let Err(e) = self.take(t, file, true, Some(held), out) {
                    self.park(t, file, held);
                    return Err(e);
                }
            }
        }
        self.take(t, file, prime, None, out)
    }

    fn park(&self, t: &mut Tail, file: &str, size: u64) {
        t.tailer.park(file, size);
        if !t.tailer.files.contains(file) {
            t.entries.remove(file);
        }
        lock(&self.status).files = t.tailer.files.len();
    }

    fn error(&self, t: &mut Tail, e: &std::io::Error) {
        lock(&self.status).errors += 1;
        t.tailer.life.problem(&e.to_string(), now_ms());
    }

    fn walk(t: &mut Tail) -> Vec<String> {
        let mut files = Vec::new();
        let mut rules = FnRules(|name: &str, _: &str| transcript(name), |_: &str, _: &str, _: u32| true);
        for s in t.scanners.iter_mut() {
            if let Ok(found) = s.scan(&mut StdFs, &mut rules, &mut now_ms) {
                files.extend(found);
            }
        }
        files
    }

    /// Reads the last day's transcripts as history: the start's walk.
    fn prime(&self) -> Vec<Event> {
        let mut out = Vec::new();
        let mut t = lock(&self.tail);
        let now = now_ms();
        let Some(_run) = t.tailer.life.begin_start(now) else { return out };
        let mut failed = false;
        for file in Self::walk(&mut t) {
            if !t.tailer.listed_at_start(&file) {
                continue;
            }
            let st = stat(&file);
            if t.tailer.at_start(&file, st, now) != Listed::Track {
                continue;
            }
            if let Err(e) = self.read(&mut t, &file, true, &mut out) {
                failed = true;
                self.park(&mut t, &file, st.map_or(0, |s| s.1));
                self.error(&mut t, &e);
            }
        }
        // History is read file by file, so it's put in time order once.
        lock(&self.telemetry).primed(now_ms());
        if !failed {
            t.tailer.life.ok(now);
        }
        t.started = true;
        lock(&self.status).watching = !self.roots.is_empty();
        out
    }

    /// Catches up on what was added to the files it follows, and with
    /// `discover`, walks the folders for new ones.
    fn sweep_now(&self, discover: bool) -> Vec<Event> {
        let mut out = Vec::new();
        let mut t = lock(&self.tail);
        t.tailer.life.count("sweeps", 1.0);
        let mut cursor = xenonet_base::ordered::Cursor::of(None);
        while let Some((c, file)) = t.tailer.next_followed(cursor) {
            cursor = c;
            let size = stat(&file).map(|s| s.1);
            match t.tailer.in_sweep(&file, size) {
                Swept::Drop => {
                    t.entries.remove(&file);
                }
                Swept::Read => {
                    if let Err(e) = self.read(&mut t, &file, false, &mut out) {
                        self.error(&mut t, &e);
                    }
                }
                Swept::Nothing => {}
            }
        }
        lock(&self.status).files = t.tailer.files.len();
        if discover {
            t.tailer.life.count("walks", 1.0);
            for file in Self::walk(&mut t) {
                if !t.tailer.may_discover(&file) || lock(&self.telemetry).classify(&file).is_none() {
                    continue;
                }
                if t.tailer.discovered(&file, stat(&file), now_ms) == Listed::Track {
                    if let Err(e) = self.read(&mut t, &file, false, &mut out) {
                        self.error(&mut t, &e);
                    }
                }
            }
        }
        out
    }

    /// Reads now what's new in the files it follows (and new files with
    /// `discover`): what a sweep does, one at a time.
    pub async fn sweep(self: &Arc<Self>, discover: bool) {
        let _one = self.sweeping.lock().await;
        let me = self.clone();
        let events = crate::util::blocking(move || me.sweep_now(discover)).await;
        self.tell(events);
    }

    /// Reads the transcripts written lately as history, then follows them.
    pub async fn start(self: &Arc<Self>) {
        let me = self.clone();
        let events = crate::util::blocking(move || me.prime()).await;
        self.tell(events);
        let me = self.clone();
        let sweeps = tokio::spawn(async move {
            let mut n = 0u64;
            let mut every = tokio::time::interval(Duration::from_millis(SWEEP_MS));
            every.tick().await;
            loop {
                every.tick().await;
                me.sweep(n.is_multiple_of(DISCOVER_EVERY)).await;
                n += 1;
            }
        });
        let me = self.clone();
        let limits = tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_millis(LIMITS_EVERY_MS));
            every.tick().await;
            loop {
                every.tick().await;
                me.read_claude_limits().await;
            }
        });
        lock(&self.tasks).extend([sweeps, limits]);
        self.read_claude_limits().await;
    }

    pub fn stop(&self) {
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        let mut t = lock(&self.tail);
        t.tailer.life.begin_stop();
        t.tailer.life.set_state(xenonet_blocks::lifecycle::State::Stopped, now_ms());
        lock(&self.status).watching = false;
    }

    // ---------- Claude plan usage ----------

    /// Anthropic's answer every 5 minutes (later when it asks for a
    /// pause), and Claude desktop's own samples; the newer reading wins.
    pub async fn read_claude_limits(&self) {
        let now = now_ms();
        if let Some(a) = &self.anthropic {
            let before = a.last().map(|l| l.at);
            let vendor = self.vendor.clone();
            let at = now_ms();
            let r = if a.begin(false, at, None).is_ok() {
                let t0 = now_ms();
                let r = fetch_claude_usage(&vendor, now).await;
                let wait = claude_usage::next_in(Some(&r)).and_then(|w| w.as_f64());
                a.done(r, at, now_ms() - t0, None, wait)
            } else {
                a.value()
            };
            if a.last().map(|l| l.at) != before {
                let _ = lock(&self.telemetry).claude_asked(Some(&r), now);
            }
        }
        if let Some((d, dir)) = &self.desktop {
            let file = std::path::Path::new(dir).join("plan-usage-history.json");
            let vendor = self.vendor.clone();
            let samples = d.get(false, None, move || desktop_samples(&vendor, &file));
            if xenonet_base::text::truthy(Some(&samples)) {
                lock(&self.telemetry).desktop_read(&samples);
            }
        }
        lock(&self.telemetry).settle_claude();
    }

    // ---------- what the state reads ----------

    /// The snapshot, with the tailer's status.
    pub fn snapshot(&self) -> Value {
        let mut snap = lock(&self.telemetry).snapshot(now_ms());
        let st = lock(&self.status).clone();
        snap["status"] = json!({ "watching": st.watching, "files": st.files, "lines": st.lines, "errors": st.errors });
        snap
    }

    pub fn history_since(&self, since: f64) -> Value {
        Value::Array(lock(&self.telemetry).history_since(since).iter().map(|e| fields(&e.fields)).collect())
    }

    pub fn history(&self) -> Vec<Value> {
        lock(&self.telemetry).history().iter().map(|e| fields(&e.fields)).collect()
    }

    /// A Claude Code hook's event.
    pub fn note_hook(&self, id: &str, body: &Value) -> Result<(), String> {
        let mut out = Vec::new();
        let r = lock(&self.telemetry).note_hook(id, Some(body), now_ms(), &mut out);
        self.tell(emits(out));
        r.map_err(|e| e.0)
    }

    /// An event the console's simulations make for a session.
    pub fn activity(&self, id: &str, ev: Vec<(String, Option<Value>)>) {
        let mut out = Vec::new();
        {
            let mut t = lock(&self.telemetry);
            let Some(i) = t.find(id) else { return };
            t.note_activity(i, ev, true, now_ms(), &mut out);
        }
        self.tell(emits(out));
    }

    pub fn session_notes(&self, id: &str) -> Value {
        lock(&self.telemetry).session_notes(id, now_ms())
    }

    pub fn collision_diffs(&self, a: &str, b: &str) -> Value {
        lock(&self.telemetry).collision_diffs(a, b)
    }

    /// The live sessions' ids, each with its last event.
    pub fn ids(&self) -> Vec<(String, Value)> {
        let t = lock(&self.telemetry);
        t.ids().into_iter().map(|id| (id.clone(), t.find(&id).map_or(Value::Null, |i| t.at(i).get("lastEventAt").flatten().unwrap_or(Value::Null)))).collect()
    }

    /// A live session as the engine reads it, or None.
    pub fn info(&self, id: &str, now: f64) -> Option<Value> {
        let mut t = lock(&self.telemetry);
        let i = t.find(id)?;
        let s = t.at_mut(i);
        let edits: Vec<Value> = match s.map_op("edits", xenonet_sources::live::telemetry::MapOp::Entries) {
            Some(Some(Value::Array(list))) => list
                .iter()
                .filter_map(|kv| kv.get(1).and_then(Value::as_array).and_then(|l| l.last()).cloned())
                .map(|e| obj(vec![("path", e.get("path").cloned().unwrap_or(Value::Null)), ("ts", e.get("ts").cloned().unwrap_or(Value::Null))]))
                .collect(),
            _ => Vec::new(),
        };
        let needs = s.needs_you(now);
        let mut m = serde_json::Map::new();
        m.insert("lastEventAt".into(), s.get("lastEventAt").flatten().unwrap_or(Value::Null));
        m.insert("status".into(), Value::String(s.status(now).into()));
        if let Some(k) = needs.get("kind") {
            m.insert("needsYou".into(), k.clone());
        }
        m.insert("edits".into(), Value::Array(edits));
        Some(Value::Object(m))
    }

    /// Whether a live session waits on you now.
    pub fn someone_waits(&self, now: f64) -> bool {
        let t = lock(&self.telemetry);
        t.ids().iter().filter_map(|id| t.find(id)).any(|i| xenonet_base::text::truthy(Some(&t.at(i).needs_you(now))))
    }

    /// The tailer's health, as a block.
    pub fn block(self: &Arc<Self>) -> Arc<dyn Block> {
        Arc::new(TailBlock(self.clone()))
    }
}

struct TailBlock(Arc<Live>);

impl Block for TailBlock {
    fn name(&self) -> String {
        "transcripts".into()
    }
    fn health(&self) -> Value {
        let t = lock(&self.0.tail);
        let lines = lock(&self.0.status).lines as f64;
        let watchers = if t.started { self.0.roots.len() as f64 } else { 0.0 };
        health_of(
            "transcripts",
            "tailer",
            &t.tailer.life,
            &[("files", t.tailer.files.len() as f64), ("dormant", t.tailer.dormant.len() as f64), ("lines", lines), ("watchers", watchers)],
            Value::Null,
        )
    }
}

// ---------- Claude plan usage's sources ----------

/// Claude Code's sign-in, { token, expiresAt }: the keychain's on a Mac,
/// else its credentials file. Read for each request, and never kept.
async fn claude_token() -> Option<Value> {
    let mut texts: Vec<Option<String>> = Vec::new();
    if cfg!(target_os = "macos") {
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("security")
                .args(["find-generic-password", "-s", claude_usage::KEYCHAIN_SERVICE, "-w"])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output(),
        )
        .await;
        texts.push(match out {
            Ok(Ok(o)) if o.status.success() => Some(String::from_utf8_lossy(&o.stdout).into_owned()),
            _ => None,
        });
    }
    let dir =
        std::env::var("CLAUDE_CONFIG_DIR").ok().unwrap_or_else(|| std::path::Path::new(&std::env::var("HOME").unwrap_or_default()).join(".claude").to_string_lossy().into_owned());
    texts.push(std::fs::read_to_string(std::path::Path::new(&dir).join(".credentials.json")).ok());
    for t in texts.into_iter().flatten() {
        if let Some((token, expires)) = claude_usage::token_of(Some(&t)) {
            return Some(json!({ "token": token, "expiresAt": expires.map_or(Value::Null, num) }));
        }
    }
    None
}

/// One reading of Anthropic's: { usage } or { reason, retryAfterMs? }.
async fn fetch_claude_usage(vendor: &Vendor, now: f64) -> Value {
    let cred = claude_token().await;
    if let Some(problem) = claude_usage::sign_in_problem(cred.as_ref(), now) {
        return problem;
    }
    let token = cred.as_ref().and_then(|c| c.get("token")).and_then(Value::as_str).unwrap_or_default().to_string();
    let headers = vec![
        ("authorization".to_string(), format!("Bearer {token}")),
        ("anthropic-beta".to_string(), "oauth-2025-04-20".to_string()),
        ("content-type".to_string(), "application/json".to_string()),
        ("user-agent".to_string(), format!("xenonet/{}", env!("CARGO_PKG_VERSION"))),
    ];
    let res = match xenonet_service::net::fetch("GET", claude_usage::USAGE_URL, &headers, None, Duration::from_secs(8)).await {
        Ok(r) => r,
        Err(e) => {
            let name = if e.contains("timeout") { json!("TimeoutError") } else { json!("TypeError") };
            return claude_usage::unreachable(Some(&name), Some(&json!(e)));
        }
    };
    let pause = if res.status == 429 || res.status == 403 { res.header("retry-after").map(|h| json!(h)) } else { None };
    if let Some(p) = claude_usage::status_problem(Some(&json!(res.status)), res.ok(), pause.as_ref()) {
        return p;
    }
    let body = xenonet_base::json::parse(&res.text()).ok();
    if body.is_none() {
        vendor.unparsable("anthropic/oauth/usage", None);
    }
    let body = body.and_then(|b| vendor.check("anthropic/oauth/usage", b, None, None));
    claude_usage::reading(body.as_ref(), now)
}

/// Claude desktop's own samples: null while there's no file.
fn desktop_samples(vendor: &Vendor, file: &std::path::Path) -> Result<Value, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(e) => return Err(e.to_string()),
    };
    let raw = xenonet_base::json::parse(&text).map_err(|e| e.to_string())?;
    let version = claude_usage::desktop_version(Some(&raw));
    let checked = vendor.check("anthropic/claude-desktop/plan-usage", raw.clone(), version.as_deref(), None);
    claude_usage::desktop_samples(Some(&raw), checked.as_ref()).map_err(|e| match e {
        claude_usage::DesktopError::Plain(m) | claude_usage::DesktopError::Type(m) => m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use xenonet_schema::drift::DriftLog;

    fn vendor() -> Arc<Vendor> {
        Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())))
    }

    const ID: &str = "0190d3e0-0000-7000-8000-000000000001";

    fn line(o: Value) -> String {
        format!("{}\n", xenonet_base::json::stringify(&o, 0))
    }

    #[tokio::test]
    async fn history_at_start_then_what_comes_live() {
        let home = std::env::temp_dir().join(format!("xenonet-wire-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join(".claude").join("projects").join("demo-x");
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join(format!("{ID}.jsonl"));
        let ts = |ago: f64| xenonet_base::time::iso(now_ms() - ago);
        std::fs::write(&file, line(json!({ "type": "user", "timestamp": ts(60e3), "message": { "role": "user", "content": "Add a test" } }))).unwrap();
        let live = Live::new(&home.join(".claude").join("projects").to_string_lossy(), "", None, false, &home.to_string_lossy(), vendor());
        let heard: Arc<Mutex<Vec<Event>>> = Arc::default();
        let h = heard.clone();
        live.listen(Arc::new(move |e| lock(&h).extend(e)));
        live.start().await;
        let snap = live.snapshot();
        assert_eq!(snap["sessions"][0]["id"], format!("claude:{ID}"));
        assert_eq!(snap["status"]["lines"], 1);
        assert!(lock(&heard).iter().all(|e| !matches!(e, Event::Activity(_))), "history isn't told as it happens");
        // A tool call, read by the next sweep, as live.
        let call = json!({ "type": "assistant", "timestamp": ts(0.0), "message": { "id": "msg_1", "model": "claude-sonnet-4-5", "stop_reason": "tool_use", "usage": { "input_tokens": 10, "output_tokens": 5 }, "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "npm test" } }] } });
        std::fs::OpenOptions::new().append(true).open(&file).unwrap().write_all(line(call).as_bytes()).unwrap();
        live.sweep(false).await;
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::Activity(a) if a["kind"] == "tool")), "{:?}", lock(&heard));
        assert_eq!(live.snapshot()["sessions"][0]["status"], "tool");
        assert!(live.info(&format!("claude:{ID}"), now_ms()).is_some());
        // A new transcript, found by the walk.
        let other = root.join("0190d3e0-0000-7000-8000-000000000002.jsonl");
        std::fs::write(&other, line(json!({ "type": "user", "timestamp": ts(0.0), "message": { "role": "user", "content": "Hi" } }))).unwrap();
        live.sweep(true).await;
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::NewFile(f) if f["sessionId"] == "claude:0190d3e0-0000-7000-8000-000000000002")));
        let h = live.block().health();
        assert_eq!((h["state"].as_str(), h["counters"]["files"].as_f64()), (Some("running"), Some(2.0)));
        live.stop();
        std::fs::remove_dir_all(&home).unwrap();
    }
}
