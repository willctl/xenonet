//! What test/golden.test.js pinned of the JS, replayed on the Rust with no
//! Node: the demo's world and the odd transcripts it adds, frozen in
//! tests/fixtures/golden-world, read with the clock fixed at 2026-10-03
//! 12:00 UTC by the live tailer (its state and history, and lines as they
//! come), each session's stats, the search index and the projects scan,
//! and compared with tests/golden/*.json the way golden.test.js compared
//! them: the world's root as <world>, and search scores to 12 digits.
//!
//! The fixture's manifest.json lists every folder, file and link of the
//! world with its mtime: "start", the world the live tailer reads first;
//! "live", the lines written while it follows them; "afterLive", every
//! mtime once they're in; "clients", the second projects folder once its
//! repos are made; "mixed", the made-up projects folder. The files are
//! under world/, clients/ and mixed/, each path component git would drop
//! or ignore escaped (a leading dot as %2E, node_modules as %6Eode_modules),
//! and <world> in a file stands for the root (@world-uri@ in a path, for
//! it encodeURIComponent'd). Of the git folders only what
//! the readers read is kept: HEAD, config, commondir and gitdir.
//!
//! The drivers in src/ read the system clock, so this drives the same
//! cores they drive (the telemetry, the tailer and the scanner, xenonet-
//! state's projects scan) with the fixed clock, as they do; stats and
//! search, whose output doesn't depend on the clock, go through the
//! drivers themselves.
//!
//! What the map's refresh makes of it (test/golden/state.json: the graph,
//! identities, repos and state.json) isn't replayed here: MapState reads
//! the clock throughout and the repos' status runs git on real repos.
//!
//! XENONET_GOLDEN_OUT=<dir> writes what came out to <dir>/golden-<name>.json,
//! to diff with tests/golden/<name>.json.

// Unix only: the world's paths go into its JSON files as they are, and the
// mixed folder has a link to a folder, which golden.test.js made a junction
// on Windows.
#![cfg(unix)]
#![cfg_attr(unix, expect(clippy::panic, reason = "a test fails where its helpers panic"))]

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};
use xenonet_blocks::scanner::{FnRules, Limits, Rules, Scanner};
use xenonet_blocks::tailer::{Listed, Tailer};
use xenonet_blocks::Halt;
use xenonet_os::fs::{DirEntry, Fs, StdFs};
use xenonet_schema::drift::DriftLog;
use xenonet_schema::vendor::Sampling;
use xenonet_sources::claude_usage;
use xenonet_sources::live::telemetry::{Emit, Telemetry, Where};
use xenonet_state::projects::{FactsCache, Scan, SCAN_DEPTH, SCAN_MAX_DIRS, SCAN_MAX_SUBMODULES};
use xenonet_wire_map::search::TranscriptSearch;
use xenonet_wire_map::stats::Stats;
use xenonet_wire_map::util::{fields, num};
use xenonet_wire_map::vendor::Vendor;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/golden-world");
const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
const MIN: f64 = 60e3;

// ---------- the world, laid out again ----------

// A path component as the fixture stores it (the freeze's escape()).
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the freeze's endsWith")]
fn escape(name: &str) -> String {
    const IGNORED: [&str; 5] = ["node_modules", "dist", "build", "release", "target"];
    let s = name.replace('%', "%25");
    if let Some(rest) = s.strip_prefix('.') {
        format!("%2E{rest}")
    } else if IGNORED.contains(&name) || name.ends_with(".log") {
        let first = name.as_bytes()[0];
        format!("%{first:X}{}", &s[1..])
    } else {
        s
    }
}

// encodeURIComponent.
fn uri_component(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

fn set_mtime(p: &Path, ms: f64) {
    let t = UNIX_EPOCH + Duration::from_millis(ms as u64);
    let f = std::fs::File::open(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    f.set_times(std::fs::FileTimes::new().set_accessed(t).set_modified(t)).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
}

struct World {
    root: String,
    m: Value,
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl World {
    fn new() -> World {
        let tmp = std::fs::canonicalize(std::env::temp_dir()).expect("the temp folder");
        let root = tmp.join(format!("xenonet-golden-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the world's root");
        let text = std::fs::read_to_string(Path::new(FIXTURE).join("manifest.json")).expect("the manifest");
        World { root: root.to_string_lossy().into_owned(), m: xenonet_text::json::parse(&text).expect("the manifest's JSON") }
    }

    fn clock(&self) -> f64 {
        self.m["clock"].as_f64().expect("the clock")
    }

    fn path(&self, rel: &str) -> String {
        if rel.is_empty() {
            self.root.clone()
        } else {
            format!("{}/{}", self.root, self.sub(rel))
        }
    }

    // One of demo.js's folders (world.home, world.claudeRoot…).
    fn dir(&self, key: &str) -> String {
        self.path(self.m["dirs"][key].as_str().unwrap_or_else(|| panic!("no folder {key}")))
    }

    // <world> is the root; @world-uri@ the root encodeURIComponent'd, as
    // a Grok Build session's folder is named for its cwd.
    fn sub(&self, text: &str) -> String {
        text.replace("@world-uri@", &uri_component(&self.root)).replace("<world>", &self.root)
    }

    fn list(&self, key: &str) -> &[Value] {
        self.m[key].as_array().map_or(&[], Vec::as_slice)
    }

    // The manifest's entries under `key`, their files from the fixture's
    // `from` folder, and then every mtime.
    fn lay(&self, key: &str, from: &str) {
        let entries = self.list(key);
        for e in entries {
            let rel = e["path"].as_str().expect("a path");
            let at = PathBuf::from(self.path(rel));
            match e["kind"].as_str() {
                Some("dir") => std::fs::create_dir_all(&at).expect("a folder"),
                Some("file") => {
                    let mut src = PathBuf::from(FIXTURE).join(from);
                    for c in rel.split('/') {
                        src.push(escape(c));
                    }
                    let bytes = std::fs::read(&src).unwrap_or_else(|e| panic!("{}: {e}", src.display()));
                    let bytes = if e["binary"] == true { bytes } else { self.sub(&String::from_utf8(bytes).expect("text")).into_bytes() };
                    std::fs::create_dir_all(at.parent().expect("a parent")).expect("a folder");
                    std::fs::write(&at, bytes).expect("a file");
                }
                Some("link") => std::os::unix::fs::symlink(self.sub(e["target"].as_str().expect("a target")), &at).expect("a link"),
                k => panic!("{rel}: {k:?}"),
            }
        }
        for e in entries {
            if let Some(t) = e["mtime"].as_f64() {
                set_mtime(Path::new(&self.path(e["path"].as_str().unwrap_or_default())), t);
            }
        }
    }
}

// ---------- comparing ----------

// A number as JSON.parse gives it back: whole numbers alike however they
// were made.
fn number(n: f64) -> Value {
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        num(n)
    }
}

// golden.test.js's canon: the world's paths as <world>, and a score to
// 12 significant digits.
fn canon(x: &Value, forms: &[String]) -> Value {
    match x {
        Value::String(s) => {
            let mut s = s.clone();
            for f in forms {
                s = s.replace(f.as_str(), "<world>");
            }
            s = s.replace("/private<world>", "<world>");
            Value::String(if s.contains("<world>") || s.contains('~') { s.replace('\\', "/") } else { s })
        }
        Value::Number(n) => number(n.as_f64().unwrap_or(f64::NAN)),
        Value::Array(a) => Value::Array(a.iter().map(|v| canon(v, forms)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::Number(n) if k == "score" => number(format!("{:.11e}", n.as_f64().unwrap_or(f64::NAN)).parse().unwrap_or(f64::NAN)),
                        _ => canon(v, forms),
                    };
                    (canon(&Value::String(k.clone()), forms).as_str().unwrap_or_default().to_string(), v)
                })
                .collect(),
        ),
        _ => x.clone(),
    }
}

fn clip(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 160 {
        format!("{}…", s.chars().take(160).collect::<String>())
    } else {
        s
    }
}

// Where two values differ, as JSON pointers, the first few.
fn diffs(at: &str, want: &Value, got: &Value, out: &mut Vec<String>) {
    if out.len() >= 25 || want == got {
        return;
    }
    match (want, got) {
        (Value::Object(a), Value::Object(b)) => {
            for k in a.keys().chain(b.keys().filter(|k| !a.contains_key(*k))) {
                match (a.get(k), b.get(k)) {
                    (Some(x), Some(y)) => diffs(&format!("{at}/{k}"), x, y, out),
                    (Some(_), None) => out.push(format!("{at}/{k}: in the golden, not here")),
                    _ => out.push(format!("{at}/{k}: here, not in the golden")),
                }
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                out.push(format!("{at}: {} items in the golden, {} here", a.len(), b.len()));
            }
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                diffs(&format!("{at}/{i}"), x, y, out);
            }
        }
        _ => out.push(format!("{at}: golden {}, here {}", clip(want), clip(got))),
    }
}

struct Check {
    forms: Vec<String>,
    failed: Vec<String>,
}

impl Check {
    fn new(root: &str) -> Check {
        let mut forms: Vec<String> = Vec::new();
        for f in [Some(root.to_string()), Some(root.replace('\\', "/")), xenonet_os::paths::path_key(root)].into_iter().flatten() {
            if !forms.contains(&f) {
                forms.push(f);
            }
        }
        Check { forms, failed: Vec::new() }
    }

    fn golden(&mut self, name: &str, actual: &Value) {
        let got = canon(actual, &self.forms);
        let text = std::fs::read_to_string(Path::new(GOLDEN).join(format!("{name}.json"))).expect("the golden");
        // As JSON.parse reads it: serde_json's own parse can be a bit off in
        // a double's last digit.
        let want = canon(&xenonet_text::json::parse(&text).expect("the golden's JSON"), &self.forms);
        if let Ok(dir) = std::env::var("XENONET_GOLDEN_OUT") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(Path::new(&dir).join(format!("golden-{name}.json")), serde_json::to_string_pretty(&got).unwrap_or_default() + "\n");
        }
        if got != want {
            let mut out = Vec::new();
            diffs("", &want, &got, &mut out);
            self.failed.push(format!("{name} differs from tests/golden/{name}.json:\n  {}", out.join("\n  ")));
        }
    }
}

fn vendor() -> Arc<Vendor> {
    Vendor::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())))
}

fn stat(file: &str) -> Option<(f64, u64)> {
    let m = std::fs::metadata(file).ok()?;
    Some((xenonet_os::fs::mtime_ms_of(&m).unwrap_or(f64::NAN), m.len()))
}

// ---------- the live tailer ----------

// A file the tailer follows: what classify said of it.
struct Followed {
    app: &'static str,
    session_id: String,
    agent_id: Value,
    sampling: Sampling,
}

// core/live.js's LiveTelemetry as golden.test.js runs it (no plan usage
// from Anthropic, Claude desktop's samples, no connectors), on the cores
// src/live.rs drives, at the time given.
struct Live {
    scanners: Vec<Scanner>,
    tailer: Tailer,
    followed: HashMap<String, Followed>,
    telemetry: Telemetry,
    vendor: Arc<Vendor>,
    desktop: String,
    lines: u64,
    errors: u64,
    // The events heard: activity, tokens and new-file, once listened to.
    listening: bool,
    events: Vec<Value>,
}

fn with_event(kind: &str, v: Value) -> Value {
    let mut m = Map::new();
    m.insert("event".into(), json!(kind));
    if let Value::Object(o) = v {
        m.extend(o);
    }
    Value::Object(m)
}

impl Live {
    fn new(roots: &[(&str, &str)], desktop: &str, home: &str, vendor: Arc<Vendor>, now: f64) -> Live {
        let roots: Vec<(String, String)> = roots
            .iter()
            .filter(|(d, _)| !d.is_empty() && Path::new(d).exists())
            .map(|(d, app)| (xenonet_os::fs::realpath_native(d).unwrap_or_else(|_| (*d).to_string()), (*app).to_string()))
            .collect();
        let scanners = roots.iter().map(|(dir, _)| Scanner::new(&format!("transcripts:{dir}"), dir, Limits { depth: 6.0, ..Limits::default() }, true, now)).collect();
        Live {
            telemetry: Telemetry::new(Some(home.to_string()), roots),
            scanners,
            tailer: Tailer::new("transcripts", 24.0 * 3600e3, now),
            followed: HashMap::new(),
            vendor,
            desktop: desktop.into(),
            lines: 0,
            errors: 0,
            listening: false,
            events: Vec::new(),
        }
    }

    fn tell(&mut self, out: Vec<Emit>) {
        if !self.listening {
            return;
        }
        for e in out {
            match e {
                Emit::Activity(f) => self.events.push(with_event("activity", fields(&f))),
                Emit::Tokens(v) => self.events.push(with_event("tokens", v)),
                Emit::Refusal(_) => {}
            }
        }
    }

    fn line(&mut self, text: &str, file: &str, f: &mut Followed, prime: bool, now: f64) {
        let Some(raw) = self.vendor.line(f.app, text, &mut f.sampling) else { return };
        self.lines += 1;
        let at = Where { session_id: f.session_id.clone(), agent_id: f.agent_id.clone(), file: Value::String(file.into()) };
        let mut out = Vec::new();
        let r = self.telemetry.line(f.app, &*self.vendor, &at, raw, !prime, now, &mut out);
        self.tell(out);
        if r.is_err() {
            self.errors += 1;
        }
    }

    // What's new in a file, up to `until`.
    fn take(&mut self, file: &str, prime: bool, until: Option<u64>, now: f64) {
        let Some((_, size)) = stat(file) else { return };
        let Some((from, to)) = self.tailer.plan_take(file, size, until) else { return };
        let data = std::fs::read(file).expect("a transcript");
        let end = (to as usize).min(data.len());
        let lines = self.tailer.took(file, &data[from as usize..end], end as u64);
        if let Some(mut f) = self.followed.remove(file) {
            for l in &lines {
                self.line(l, file, &mut f, prime, now);
            }
            self.followed.insert(file.to_string(), f);
        }
    }

    // The tailer's track(file, { prime }): a file read for the first time
    // is classified, and what it held while dormant read as history.
    fn track(&mut self, file: &str, prime: bool, now: f64) {
        if !self.tailer.files.contains(file) {
            let Some(kind) = self.telemetry.classify(file) else { return };
            let f = Followed {
                app: xenonet_adapter::apps::tailed_app(kind.get("app").and_then(Value::as_str).unwrap_or_default(), "claude"),
                session_id: kind.get("sessionId").and_then(Value::as_str).unwrap_or_default().to_string(),
                agent_id: kind.get("agentId").cloned().unwrap_or(Value::Null),
                sampling: Sampling::default(),
            };
            if !prime && self.listening {
                self.events.push(json!({ "event": "new-file", "file": file, "sessionId": f.session_id }));
            }
            self.followed.insert(file.to_string(), f);
            self.tailer.add_file(file);
            if let Some(held) = self.tailer.take_held(file) {
                self.take(file, true, Some(held), now);
            }
        }
        self.take(file, prime, None, now);
    }

    // start(): the last day's transcripts as history, then Claude
    // desktop's plan usage.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the JS's endsWith, which tells .jsonl from .JSONL")]
    fn start(&mut self, now: f64) {
        let _run = self.tailer.life.begin_start(now);
        let mut files = Vec::new();
        let mut rules = FnRules(|name: &str, _: &str| name.ends_with(".jsonl"), |_: &str, _: &str, _: u32| true);
        let mut clock = || now;
        for s in &mut self.scanners {
            files.extend(s.scan(&mut StdFs, &mut rules, &mut clock).unwrap_or_default());
        }
        for file in files {
            if !self.tailer.listed_at_start(&file) || self.tailer.at_start(&file, stat(&file), now) != Listed::Track {
                continue;
            }
            self.track(&file, true, now);
        }
        self.telemetry.primed(now);
        let file = Path::new(&self.desktop).join("plan-usage-history.json");
        if let Some(raw) = std::fs::read_to_string(file).ok().and_then(|t| xenonet_text::json::parse(&t).ok()) {
            let version = claude_usage::desktop_version(Some(&raw));
            let checked = self.vendor.check("anthropic/claude-desktop/plan-usage", raw.clone(), version.as_deref(), None);
            if let Ok(samples) = claude_usage::desktop_samples(Some(&raw), checked.as_ref()) {
                if xenonet_text::text::truthy(Some(&samples)) {
                    self.telemetry.usage_samples("claude", &samples);
                }
            }
        }
        self.telemetry.settle_usage("claude");
    }

    fn status(&self) -> Value {
        // Stopped: no longer watching.
        json!({ "watching": false, "files": self.tailer.files.len(), "lines": self.lines, "errors": self.errors })
    }

    fn snapshot(&mut self, now: f64) -> Value {
        let mut s = self.telemetry.snapshot(now);
        s["status"] = self.status();
        s
    }

    // A session's own tokens, context, tool counts, prompts and messages,
    // as they are now: live.js hands those out, and JS holding them sees
    // them change.
    fn own(&self, id: &str) -> Value {
        self.telemetry.find(id).map_or(Value::Null, |i| self.telemetry.at(i).mirror(-1.0))
    }
}

fn live(w: &World, c: &mut Check) {
    let fixed = w.clock();
    let roots = [(w.dir("claudeRoot"), "claude"), (format!("{}/sessions", w.dir("codexHome")), "codex"), (format!("{}/sessions", w.dir("grokHome")), "grok")];
    let roots: Vec<(&str, &str)> = roots.iter().map(|(d, a)| (d.as_str(), *a)).collect();
    let mut live = Live::new(&roots, &w.dir("claudeDesktopDir"), &w.dir("home"), vendor(), fixed);
    live.start(fixed);
    let snapshot = live.snapshot(fixed);
    let history = Value::Array(live.telemetry.history_since(0.0).iter().map(|e| fields(&e.fields)).collect());
    let mut ids = live.telemetry.ids();
    ids.sort();
    let notes: Map<String, Value> = ids.iter().map(|id| (id.clone(), live.telemetry.session_notes(id, fixed))).collect();
    let collisions: Vec<Value> = live
        .telemetry
        .collisions(fixed)
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|c| {
            let mut c = c.clone();
            let d = live.telemetry.collision_diffs(c["a"].as_str().unwrap_or_default(), c["b"].as_str().unwrap_or_default());
            c["diffs"] = d;
            c
        })
        .collect();

    // Lines as they come, ten seconds on.
    live.listening = true;
    let now = fixed + 10e3;
    for step in w.list("live") {
        let file = w.path(step["file"].as_str().expect("a file"));
        let text = w.sub(step["text"].as_str().expect("its lines"));
        match step["how"].as_str() {
            Some("append") => {
                let mut f = std::fs::OpenOptions::new().append(true).open(&file).expect("a transcript");
                std::io::Write::write_all(&mut f, text.as_bytes()).expect("its lines");
            }
            _ => std::fs::write(&file, text).expect("a new transcript"),
        }
        live.track(&file, false, now);
    }
    let after = live.snapshot(fixed + 30e3);

    // What the first snapshot and notes handed out is the sessions' own,
    // so it says what they are at the end.
    let mut snapshot = snapshot;
    if let Some(list) = snapshot["sessions"].as_array_mut() {
        for s in list {
            let own = live.own(s["id"].as_str().unwrap_or_default());
            for k in ["tokens", "context", "toolCounts"] {
                s[k] = own[k].clone();
            }
        }
    }
    snapshot["status"] = live.status();
    let notes: Map<String, Value> = notes
        .into_iter()
        .map(|(id, mut n)| {
            if n.is_object() {
                let own = live.own(&id);
                for k in ["prompts", "messages", "context"] {
                    n[k] = own[k].clone();
                }
            }
            (id, n)
        })
        .collect();
    let primed = json!({ "snapshot": snapshot, "history": history, "notes": notes, "collisions": collisions });
    c.golden("live", &json!({ "primed": primed, "events": live.events, "after": after }));
}

// The world after the live tailer's lines, with its mtimes set again.
fn after_live(w: &World) {
    for e in w.list("afterLive") {
        let (Some(rel), Some(t)) = (e[0].as_str(), e[1].as_f64()) else { continue };
        set_mtime(Path::new(&w.path(rel)), t);
    }
}

// ---------- stats and search ----------

fn codex_roots(w: &World) -> Vec<String> {
    let home = w.dir("codexHome");
    vec![format!("{home}/sessions"), format!("{home}/archived_sessions")]
}

async fn stats(w: &World, c: &mut Check) {
    let s = Stats::new(&w.dir("claudeRoot"), &codex_roots(w), &format!("{}/sessions", w.dir("grokHome")), vendor());
    let mut out = Map::new();
    for id in w.list("sessionIds").iter().filter_map(Value::as_str) {
        let v = match s.stats(id).await {
            Ok(v) => v.unwrap_or(Value::Null),
            Err(e) => json!({ "error": e }),
        };
        out.insert(id.into(), v);
    }
    c.golden("stats", &Value::Object(out));
}

async fn search(w: &World, c: &mut Check) {
    let db = format!("{}/golden-search.sqlite", w.dir("cacheDir"));
    let s = TranscriptSearch::new(&db, &w.dir("claudeRoot"), &codex_roots(w), &format!("{}/sessions", w.dir("grokHome")), vendor());
    s.index().await;
    let mut found = Map::new();
    for q in ["refunds", "rounding solver", "make", "invoices", "Why is the build"] {
        found.insert(q.into(), Value::Array(s.search(q, 30.0).await.expect("a search")));
    }
    let st = s.status_json();
    let status = json!({ "files": st["files"], "docs": st["docs"], "errors": st.get("errors").cloned().unwrap_or(json!(0)) });
    s.close();
    let conn = rusqlite::Connection::open(&db).expect("the index");
    let mut q = conn.prepare("select session_id, app, role, ts, text from docs order by session_id, ts, rowid").expect("a query");
    let names = ["session_id", "app", "role", "ts", "text"];
    let docs: Vec<Value> = q
        .query_map([], |r| {
            let mut m = Map::new();
            for (i, n) in names.iter().enumerate() {
                let v = match r.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Real(f) => num(f),
                    rusqlite::types::ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    rusqlite::types::ValueRef::Blob(b) => json!(b),
                };
                m.insert((*n).into(), v);
            }
            Ok(Value::Object(m))
        })
        .expect("the docs")
        .collect::<Result<_, _>>()
        .expect("the docs");
    c.golden("search", &json!({ "status": status, "docs": docs, "found": found }));
}

// ---------- the projects scan ----------

struct Enter<'a>(&'a Scan);

impl Rules for Enter<'_> {
    fn pick(&mut self, _: &str, _: &str) -> Result<bool, Halt> {
        Ok(false)
    }
    fn enter(&mut self, name: &str, dir: &str, level: u32) -> Result<bool, Halt> {
        Ok(self.0.enter(name, dir, f64::from(level)))
    }
}

// scanProjects(dir, { sessionDirs, rules, limits }) after forgetScans(),
// as src/projects.rs runs it, at the time given: { projects, scan, folders }.
fn scan(root: &str, session_dirs: &[String], rules: &Value, limits: &Value, now: f64) -> Value {
    let limit = |k: &str, fallback: f64| limits.get(k).and_then(Value::as_f64).unwrap_or(fallback);
    let depth = limit("depth", SCAN_DEPTH);
    let max_dirs = limit("maxDirs", SCAN_MAX_DIRS);
    let max_submodules = limits.get("maxSubmodules").cloned().unwrap_or(json!(SCAN_MAX_SUBMODULES));
    let mut s = Scan::new(root, rules);
    let mut scanner = Scanner::new("projects", root, Limits::default(), false, now);
    let mut clock = || now;
    let mut w = scanner.begin_breadth(root, Limits { depth, folders: max_dirs, ms: f64::INFINITY }, &mut clock);
    while w.more() {
        let level = w.n;
        for read in scanner.plan(&mut w, None) {
            let Some(at) = scanner.begin_read(&mut w, false, &mut clock) else {
                scanner.skip(&mut w, read.slot);
                continue;
            };
            let ents: Vec<DirEntry> = match StdFs.read_dir(&read.dir) {
                Ok(e) => e,
                Err(e) => {
                    if !scanner.read_failed(&mut w, &read.dir, &e) {
                        scanner.skip(&mut w, read.slot);
                        continue;
                    }
                    Vec::new()
                }
            };
            let names: Vec<(String, bool)> = ents.iter().map(|e| (e.name.clone(), e.is_dir)).collect();
            s.look(&read.dir, &names, f64::from(level), false);
            let _ = scanner.finish_read(&mut w, &read, at, &ents, &mut Enter(&s));
        }
        scanner.end_level(&mut w);
    }
    scanner.finish_breadth(w, &mut clock);
    if !s.root_read {
        return json!({ "projects": [], "scan": { "ok": false, "ms": 0 }, "folders": null });
    }
    let (dirs_read, truncated) = scanner.last.clone().map_or((0, false), |l| (l.read, l.truncated.is_some()));
    let sessions: Vec<Value> = session_dirs.iter().map(|d| json!(d)).collect();
    let found = s.finish(&sessions, Some(&max_submodules), &mut FactsCache::default(), now).expect("a scan");
    json!({
        "projects": found.projects,
        "scan": { "ok": true, "at": num(now), "ms": 0, "dirsRead": dirs_read, "truncated": truncated, "depth": num(depth), "maxDirs": num(max_dirs), "groups": found.groups, "projects": found.count },
        "folders": found.folders,
    })
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.as_f64().map_or_else(String::new, |f| if f.fract() == 0.0 { format!("{}", f as i64) } else { f.to_string() }),
        other => other.to_string(),
    }
}

// What a scan with other limits finds, a line each (golden.test.js's brief).
fn brief(found: &Value) -> Value {
    let truthy = |v: &Value| xenonet_text::text::truthy(Some(v));
    let projects = found["projects"].as_array().map_or(&[][..], Vec::as_slice);
    let lines: Vec<Value> = projects
        .iter()
        .map(|p| {
            let mut s = format!("{} {}:{}", text_of(&p["rel"]), text_of(&p["role"]), text_of(&p["detected"]));
            if truthy(&p["marker"]) {
                s.push_str(" [");
                s.push_str(&text_of(&p["marker"]));
                s.push(']');
            }
            if truthy(&p["parent"]) {
                let parent = projects.iter().find(|q| q["key"] == p["parent"]).expect("its parent");
                s.push_str(" in ");
                s.push_str(&text_of(&parent["rel"]));
            }
            json!(s)
        })
        .collect();
    let folders: Vec<Value> = found["folders"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|f| {
            let mut s = format!("{} {}", text_of(&f["rel"]), text_of(&f["call"]));
            if truthy(&f["why"]) {
                s.push(':');
                s.push_str(&text_of(&f["why"]));
            }
            if truthy(&f["repo"]) {
                s += " repo";
            }
            if truthy(&f["sessions"]) {
                s.push(' ');
                s.push_str(&text_of(&f["sessions"]));
                s.push_str(" sessions");
            }
            json!(s)
        })
        .collect();
    json!({ "scan": found["scan"], "projects": lines, "folders": folders })
}

fn rule_of(root: &str, rel: &str, as_: &str, by: &str) -> (String, Value) {
    let p = if rel.is_empty() { root.to_string() } else { format!("{root}/{rel}") };
    (xenonet_os::paths::path_key(&p).expect("a key"), json!({ "path": p, "as": as_, "by": by }))
}

fn rules(list: &[(String, Value)]) -> Value {
    Value::Object(list.iter().cloned().collect())
}

fn projects(w: &World, c: &mut Check) {
    // The live tailer's test left the clock 30 seconds on.
    let now = w.clock() + 30e3;
    let stamped = w.clock() - 120.0 * MIN;
    let none = json!({});
    let mut out = Map::new();
    let session_dirs: Vec<String> = w.list("sessionDirs").iter().filter_map(Value::as_str).map(|d| w.sub(d)).collect();
    let projects_dir = w.dir("projectsDir");
    out.insert("demo".into(), scan(&projects_dir, &session_dirs, &none, &none, now));
    let mut demo_rules: Vec<(String, Value)> =
        w.list("demoPicks").iter().map(|p| rule_of(&projects_dir, p["path"].as_str().unwrap_or_default(), p["as"].as_str().unwrap_or_default(), "codex")).collect();
    demo_rules.extend([rule_of(&projects_dir, "infra", "ignore", "you"), rule_of(&projects_dir, "labs", "project", "you")]);
    out.insert("demoRules".into(), scan(&projects_dir, &session_dirs, &rules(&demo_rules), &none, now));

    // The second folder, once its repos are made (and their mtimes set).
    let clients = w.dir("clientsDir");
    std::fs::remove_dir_all(&clients).expect("the second folder");
    std::fs::create_dir_all(&clients).expect("the second folder");
    w.lay("clients", "clients");
    set_mtime(Path::new(&clients), stamped);
    out.insert("clients".into(), scan(&clients, &session_dirs, &none, &none, now));

    let root = w.path(w.m["mixedRoot"].as_str().expect("the mixed folder"));
    std::fs::create_dir_all(&root).expect("the mixed folder");
    w.lay("mixed", "mixed");
    set_mtime(Path::new(&root), stamped);
    let at = |rel: &str| if rel.is_empty() { root.clone() } else { format!("{root}/{rel}") };
    out.insert("mixed".into(), scan(&root, &[], &none, &none, now));
    let dirs: Vec<String> = [
        "misc",
        "misc/fastapi_demo",
        "misc/scratch/sub",
        "misc/scratch/sub",
        "notes/drafts",
        "studio",
        "studio/app/src",
        "mono/packages/a",
        "clients/shop/web",
        "clients/notes",
        "deep/a/b/c/repo/src",
        "app/.claude/worktrees/fix-1/src",
        "d/node_modules/pkg",
        "hosts/github.com/me",
        "",
    ]
    .iter()
    .map(|r| at(r))
    .chain([w.dir("home")])
    .collect();
    out.insert("mixedSessions".into(), scan(&root, &dirs, &none, &none, now));
    let mixed_rules = rules(&[
        rule_of(&root, "studio", "project", "you"),
        rule_of(&root, "studio/web", "project", "codex"),
        rule_of(&root, "studio/app", "project", "you"),
        rule_of(&root, "ledger", "group", "you"),
        rule_of(&root, "misc/tool-a", "ignore", "you"),
        rule_of(&root, "misc/payloads", "project", "codex"),
        rule_of(&root, "notes", "group", "codex"),
        rule_of(&root, "deep/a/b/c", "project", "you"),
        rule_of(&root, "d", "ignore", "you"),
        rule_of(&root, "gone/folder", "project", "you"),
        rule_of(&root, "", "group", "you"),
        rule_of(&projects_dir, "ledger", "group", "you"),
    ]);
    out.insert("mixedRules".into(), scan(&root, &[at("lab-less"), at("misc/payloads"), at("ledger/src")], &mixed_rules, &none, now));
    for depth in [1, 2, 4] {
        let found = scan(&root, &[at("deep/a/b/c/repo/src")], &none, &json!({ "depth": depth }), now);
        out.insert(format!("mixedDepth{depth}"), brief(&found));
    }
    let first_level = std::fs::read_dir(&root)
        .expect("the mixed folder")
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            !n.starts_with('.') && n != "node_modules"
        })
        .count();
    out.insert("mixedBudget".into(), brief(&scan(&root, &[], &none, &json!({ "maxDirs": first_level }), now)));
    out.insert("mixedSubmodules".into(), brief(&scan(&root, &[], &none, &json!({ "maxSubmodules": 1 }), now)));
    out.insert("missing".into(), scan(&at("not-here"), &[], &none, &none, now));
    c.golden("projects", &Value::Object(out));
}

// ---------- the test ----------

// One test, in golden.test.js's order: each part reads what the one before
// left (the live tailer's lines, the second folder's repos).
#[tokio::test(flavor = "multi_thread")]
async fn the_golden_world_reads_as_the_js_read_it() {
    // golden.test.js ran with TZ=UTC: the day a token counts for is local.
    std::env::set_var("TZ", "UTC");
    // The telemetry and the vendor take the adapters' readers.
    xenonet_platforms::install();
    let w = World::new();
    w.lay("start", "world");
    let mut c = Check::new(&w.root);
    live(&w, &mut c);
    after_live(&w);
    stats(&w, &mut c).await;
    search(&w, &mut c).await;
    projects(&w, &mut c);
    assert!(c.failed.is_empty(), "{}", c.failed.join("\n\n"));
}
