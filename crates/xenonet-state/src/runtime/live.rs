//! The live tailer: it reads the transcripts the apps append to
//! (its decisions are xenonet-blocks' tailer.rs), each line through its app's normalizer into the telemetry
//! (xenonet-sources' live/telemetry.rs), which holds every session's state
//! and says what each line means; and the readings of Claude plan usage,
//! Anthropic's and Claude desktop's.
//!
//! At start the tailer reads the last day's transcripts as history. From
//! then on the watcher pushes what changes (xenonet-adapter's
//! `watch::Sessions`, each adapter's `watch` and `changed`): the lines an
//! adapter read of a transcript the tailer follows are told as live, and a
//! transcript it meets for the first time is read through the tailer,
//! with what it held while dormant as history. The walk the start does
//! is the reconcile, the backstop for what the watcher missed: when it
//! says events were lost, and every 10 minutes. Other owners' watches
//! (a repo's, the wiki's) share the watcher, and what they hear is passed
//! on as it is.

use crate::runtime::blocks::{health_of, Block};
use crate::runtime::gatherer::{Gather, Options};
use crate::runtime::util::{fields, lock, log, num, obj, Clock};
use crate::runtime::vendor::Vendor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use xenonet_adapter::watch::{self, Report, SessionEvent, Sessions, WatchSpec};
use xenonet_adapter::Platform;
use xenonet_blocks::scanner::{FnRules, Limits, Scanner};
use xenonet_blocks::tailer::{Followed, Listed, Swept, Tailer};
use xenonet_os::fs::StdFs;
use xenonet_schema::vendor::Sampling;
use xenonet_sources::claude_usage;
use xenonet_sources::desktop_connectors::Connectors;
use xenonet_sources::live::names::with_declared;
use xenonet_sources::live::telemetry::{Emit, Telemetry, Where};

const PRIME_WINDOW_MS: f64 = 24.0 * 3600e3;
/// How often the walk runs anyway: the backstop for what the watcher
/// missed while it was restarting a watch, or never heard.
pub const RECONCILE_MS: u64 = 10 * 60_000;
const DEPTH: f64 = 6.0;
// The most of a transcript read in one go.
const TAKE_PIECE: u64 = 4 << 20;
const LIMITS_EVERY_MS: u64 = 30_000;

/// What the telemetry says, for the state to pass on: activity, tokens
/// and refusals as the telemetry emits them, and a transcript read for the
/// first time after start; and what the watcher heard that the map reads.
#[derive(Clone, Debug)]
pub enum Event {
    Activity(Value),
    Tokens(Value),
    Refusal(Value),
    NewFile(Value),
    /// An app's sessions changed where the map reads them (a record read
    /// whole, a session gone, lines of a transcript the tailer doesn't
    /// read, or of one it does).
    Changed {
        app: String,
    },
    /// The walk ran: what the watcher missed is read. `app` None: every
    /// app's, as the reconcile every 10 minutes does.
    Reconciled {
        app: Option<String>,
        why: String,
    },
    /// Another owner's watch (`Live::watch`): its changed paths, or that
    /// it should be read again whole (`reconcile`).
    Other {
        key: String,
        paths: Vec<PathBuf>,
        reconcile: Option<String>,
    },
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
    app: &'static str,
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
    /// What Claude desktop calls its connectors (desktop_connectors.rs).
    connectors: Mutex<Connectors>,
    listener: OnceLock<Listener>,
    /// The watcher, once started: the apps' watches and other owners'.
    sessions: Mutex<Option<Sessions>>,
    /// Other owners' specs, by key, watched once the watcher is up.
    wanted: Mutex<std::collections::BTreeMap<String, Vec<WatchSpec>>>,
    // The reports', the reconcile's and the limits' tasks, stopped with it.
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    clock: Clock,
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
    Some((xenonet_os::fs::mtime_ms_of(&m).unwrap_or(f64::NAN), m.len()))
}

#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "a transcript is .jsonl, never .JSONL")]
fn transcript(name: &str) -> bool {
    name.ends_with(".jsonl")
}

// Where the next whole line of a file the tailer follows starts: before
// the half line it holds.
fn line_start(f: &Followed) -> u64 {
    f.offset.saturating_sub(f.rest.len() as u64)
}

impl Live {
    /// `roots`: the folders the apps write transcripts in, each with its
    /// app; `usage`: whether to ask Anthropic for plan usage.
    pub fn new(roots: &[(String, String)], claude_desktop_dir: Option<&str>, usage: bool, home: &str, vendor: Arc<Vendor>) -> Arc<Live> {
        Live::with_clock(roots, claude_desktop_dir, usage, home, vendor, Clock::system())
    }

    /// The same, reading `clock`.
    pub fn with_clock(roots: &[(String, String)], claude_desktop_dir: Option<&str>, usage: bool, home: &str, vendor: Arc<Vendor>, clock: Clock) -> Arc<Live> {
        // Watch canonical long paths: an 8.3 short name on Windows names
        // the same folder another way.
        let roots: Vec<(String, String)> = roots
            .iter()
            .filter(|(d, _)| !d.is_empty() && Path::new(d).exists())
            .map(|(d, app)| (xenonet_os::fs::realpath_native(d).unwrap_or_else(|_| d.clone()), app.clone()))
            .collect();
        let now = clock.now();
        let scanners = roots.iter().map(|(dir, _)| Scanner::new(&format!("transcripts:{dir}"), dir, Limits { depth: DEPTH, ..Limits::default() }, true, now)).collect();
        let telemetry = Telemetry::new(Some(home.to_string()).filter(|h| !h.is_empty()), roots.clone());
        let anthropic = usage.then(|| {
            Arc::new(Gather::new(
                "claude-usage",
                Options { every_ms: Some(claude_usage::USAGE_EVERY_MS), next_in: true, fallback: Value::Null, clock: clock.clone(), ..Default::default() },
            ))
        });
        let desktop = claude_desktop_dir
            .map(|d| (Arc::new(Gather::new("claude-desktop-usage", Options { keep: true, fallback: Value::Null, clock: clock.clone(), ..Default::default() })), d.to_string()));
        Arc::new(Live {
            connectors: Mutex::new(Connectors::new(claude_desktop_dir)),
            roots,
            tail: Mutex::new(Tail { tailer: Tailer::new("transcripts", PRIME_WINDOW_MS, now), scanners, entries: HashMap::new(), started: false }),
            telemetry: Mutex::new(telemetry),
            status: Mutex::new(TailStatus::default()),
            vendor,
            anthropic,
            desktop,
            listener: OnceLock::new(),
            sessions: Mutex::new(None),
            wanted: Mutex::new(std::collections::BTreeMap::new()),
            tasks: Mutex::new(Vec::new()),
            clock,
        })
    }

    /// Who hears what the lines say.
    pub fn listen(&self, l: Listener) {
        let _ = self.listener.set(l);
    }

    /// The MCP server the config names for the shared context
    /// (wikiServer), under which Xenolith's tools write there.
    pub fn context_server(&self, server: Option<&str>) {
        lock(&self.telemetry).context_server = server.map(String::from);
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
        let Some(raw) = self.vendor.line(e.app, text, &mut e.sampling) else { return };
        lock(&self.status).lines += 1;
        let f = Where { session_id: e.session_id.clone(), agent_id: e.agent_id.clone(), file: Value::String(file.into()) };
        let mut emitted = Vec::new();
        let r = lock(&self.telemetry).line(e.app, &*self.vendor, &f, raw, !prime, self.clock.now(), &mut emitted);
        out.extend(emits(emitted));
        if r.is_err() {
            lock(&self.status).errors += 1;
        }
    }

    // A read that worked makes the tailer fine again.
    fn read_ok(&self, t: &mut Tail) {
        if t.tailer.ok_after_read(self.roots.len(), self.roots.len()) {
            t.tailer.life.ok(self.clock.now());
        }
    }

    // Reads what's new in a file, up to `until`, as history (`prime`) or
    // live.
    fn take(&self, t: &mut Tail, file: &str, prime: bool, until: Option<u64>, out: &mut Vec<Event>) -> std::io::Result<()> {
        let Some((_, size)) = stat(file) else { return Ok(()) };
        let Some((from, to)) = t.tailer.plan_take(file, size, until) else { return Ok(()) };
        let mut f = std::fs::File::open(file)?;
        f.seek(SeekFrom::Start(from))?;
        // A piece at a time, so a long transcript read as history never
        // has to be held whole, nor its lines all at once.
        let mut at = from;
        let mut buf = vec![0u8; (to - from).min(TAKE_PIECE) as usize];
        while at < to {
            let want = (to - at).min(TAKE_PIECE) as usize;
            let mut got = 0;
            while got < want {
                let n = f.read(&mut buf[got..want])?;
                if n == 0 {
                    break;
                }
                got += n;
            }
            at += got as u64;
            let lines = t.tailer.took(file, &buf[..got], at);
            if let Some(mut e) = t.entries.remove(file) {
                for l in &lines {
                    self.line(l, file, &mut e, prime, out);
                }
                t.entries.insert(file.to_string(), e);
            }
            if got < want {
                break;
            }
        }
        self.read_ok(t);
        Ok(())
    }

    // Reads what's new in a transcript: one it reads for the first time
    // is classified, and what it held while dormant is read as history
    // before its new lines.
    fn read(&self, t: &mut Tail, file: &str, prime: bool, out: &mut Vec<Event>) -> std::io::Result<()> {
        if !t.tailer.files.contains(file) {
            let Some(kind) = lock(&self.telemetry).classify(file) else { return Ok(()) };
            let entry = Entry {
                // Any app that isn't tailed is read as the one whose reader
                // reads the others (Claude Code).
                app: crate::apps::read_as(kind.get("app").and_then(Value::as_str).unwrap_or_default()).unwrap_or_default(),
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
        t.tailer.life.problem(&e.to_string(), self.clock.now());
    }

    fn walk(&self, t: &mut Tail) -> Vec<String> {
        let mut files = Vec::new();
        let mut rules = FnRules(|name: &str, _: &str| transcript(name), |_: &str, _: &str, _: u32| true);
        for s in &mut t.scanners {
            if let Ok(found) = s.scan(&mut StdFs, &mut rules, &mut || self.clock.now()) {
                files.extend(found);
            }
        }
        files
    }

    /// Reads the last day's transcripts as history: the start's walk.
    fn prime(&self) -> Vec<Event> {
        let mut out = Vec::new();
        let mut t = lock(&self.tail);
        let now = self.clock.now();
        let Some(_run) = t.tailer.life.begin_start(now) else { return out };
        let mut failed = false;
        for file in self.walk(&mut t) {
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
        lock(&self.telemetry).primed(self.clock.now());
        if !failed {
            t.tailer.life.ok(now);
        }
        t.started = true;
        lock(&self.status).watching = !self.roots.is_empty();
        out
    }

    /// The reconcile: what `catch_up` reads, and a walk for new files.
    fn reconcile_now(&self) -> Vec<Event> {
        lock(&self.tail).tailer.life.count("reconciles", 1.0);
        self.read_grown(true)
    }

    /// The files it follows read where they grew, and dormant ones written
    /// to again; then the watcher's cursors are moved to where the tailer
    /// read to, so it reads on from there.
    fn catch_up(&self) -> Vec<Event> {
        self.read_grown(false)
    }

    fn read_grown(&self, walk: bool) -> Vec<Event> {
        let mut out = Vec::new();
        {
            let mut t = lock(&self.tail);
            let mut cursor = xenonet_text::ordered::Cursor::of(None);
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
            // A dormant file written to again: what it held then as
            // history, and its new lines after.
            let woken: Vec<String> = t.tailer.dormant.iter().filter(|(f, held)| stat(f).is_some_and(|(_, size)| size != **held)).map(|(f, _)| f.to_string()).collect();
            for file in woken {
                if let Err(e) = self.read(&mut t, &file, false, &mut out) {
                    self.error(&mut t, &e);
                }
            }
            if walk {
                for file in self.walk(&mut t) {
                    if !t.tailer.may_discover(&file) || lock(&self.telemetry).classify(&file).is_none() {
                        continue;
                    }
                    if t.tailer.discovered(&file, stat(&file), || self.clock.now()) == Listed::Track {
                        if let Err(e) = self.read(&mut t, &file, false, &mut out) {
                            self.error(&mut t, &e);
                        }
                    }
                }
            }
            lock(&self.status).files = t.tailer.files.len();
        }
        self.seed_all();
        out
    }

    // The app whose folder a file is in.
    fn app_of(&self, file: &str) -> Option<&str> {
        self.roots.iter().find(|(d, _)| Path::new(file).starts_with(d)).map(|(_, app)| app.as_str())
    }

    // Tells the watcher's cursor where the tailer read a file to, so the
    // lines it reads next start there.
    fn seed(&self, app: &str, file: &str, at: u64) {
        if let Some(s) = lock(&self.sessions).as_ref() {
            s.seed(app, Path::new(file), at);
        }
    }

    // Every file the tailer follows or keeps dormant, seeded.
    fn seed_all(&self) {
        let marks: Vec<(String, u64)> = {
            let t = lock(&self.tail);
            t.tailer.files.iter().map(|(f, at)| (f.to_string(), line_start(at))).chain(t.tailer.dormant.iter().map(|(f, held)| (f.to_string(), *held))).collect()
        };
        for (file, at) in marks {
            if let Some(app) = self.app_of(&file) {
                self.seed(app, &file, at);
            }
        }
    }

    // ---------- what the watcher heard ----------

    /// Handles what the watcher heard, and says what it means.
    pub fn report_now(&self, r: Report) -> Vec<Event> {
        match r {
            Report::Events { app, events } => self.events(&app, events),
            Report::Reconcile { app, why } => {
                let mut out = self.reconcile_now();
                out.push(Event::Reconciled { app, why });
                out
            }
            Report::Other { key, paths, reconcile } => vec![Event::Other { key, paths, reconcile }],
        }
    }

    /// The same, off the runtime's threads, told to the listener.
    pub async fn report(self: &Arc<Self>, r: Report) {
        let me = self.clone();
        let events = crate::runtime::util::blocking(move || me.report_now(r)).await;
        self.tell(events);
    }

    /// The reconcile, now: what `report` does with the watcher's
    /// Reconcile.
    pub async fn reconcile(self: &Arc<Self>, why: &str) {
        self.report(Report::Reconcile { app: None, why: why.to_string() }).await;
    }

    fn events(&self, app: &str, events: Vec<SessionEvent>) -> Vec<Event> {
        let mut out = Vec::new();
        let mut changed = false;
        for e in events {
            match e {
                SessionEvent::Lines(l) => {
                    if !self.lines(app, l, &mut out) {
                        changed = true;
                    }
                }
                SessionEvent::Changed { .. } => changed = true,
                SessionEvent::Gone { file, .. } => {
                    let f = file.to_string_lossy();
                    let mut t = lock(&self.tail);
                    t.tailer.files.remove(&f);
                    t.tailer.dormant.remove(&f);
                    t.entries.remove(f.as_ref());
                    lock(&self.status).files = t.tailer.files.len();
                    changed = true;
                }
            }
        }
        // The map reads an app's sessions from more than the lines the
        // tailer reads: their records, the sessions gone, the transcripts it
        // doesn't read. (What the tailer reads, it tells as activity.)
        if changed {
            out.push(Event::Changed { app: app.to_string() });
        }
        out
    }

    // Lines an adapter read of a transcript: told as live when they go on
    // from where the tailer read to; otherwise (a file it meets for the
    // first time, one written again, lines it read past already or short
    // of) the tailer reads the file itself, and the watcher's cursor is
    // moved to where it read to. False for a file the tailer doesn't read.
    fn lines(&self, app: &str, l: watch::Lines, out: &mut Vec<Event>) -> bool {
        let file = l.file.to_string_lossy().into_owned();
        let mut t = lock(&self.tail);
        let at = t.tailer.files.get(&file).map(line_start);
        match at {
            Some(start) if l.read != watch::Read::Rewritten && l.from == start => {
                if let Some(f) = t.tailer.files.get_mut(&file) {
                    f.offset = l.to;
                    f.rest.clear();
                }
                if let Some(mut e) = t.entries.remove(&file) {
                    for line in &l.lines {
                        self.line(line, &file, &mut e, false, out);
                    }
                    t.entries.insert(file.clone(), e);
                }
                self.read_ok(&mut t);
                return true;
            }
            Some(start) if l.read != watch::Read::Rewritten && l.to <= start => return true,
            Some(_) => {
                if l.read == watch::Read::Rewritten {
                    if let Some(f) = t.tailer.files.get_mut(&file) {
                        *f = Followed::default();
                    }
                }
            }
            None => {
                if lock(&self.telemetry).classify(&file).is_none() {
                    return false;
                }
                // Copied in, not written: it waits, dormant, like one too
                // old to read at start.
                if !t.tailer.dormant.contains(&file) && t.tailer.discovered(&file, stat(&file), || self.clock.now()) != Listed::Track {
                    return true;
                }
            }
        }
        if let Err(e) = self.read(&mut t, &file, false, out) {
            self.error(&mut t, &e);
        }
        let at = t.tailer.files.get(&file).map(line_start);
        lock(&self.status).files = t.tailer.files.len();
        drop(t);
        if let Some(at) = at {
            self.seed(app, &file, at);
        }
        true
    }

    // ---------- other owners' watches ----------

    /// Watches an owner's specs (a repo's, the wiki's) on the same
    /// watcher, in place of any it had: what changes comes as
    /// `Event::Other` under `key`. Asked before the watcher is up, they're
    /// watched once it is.
    pub fn watch(&self, key: &str, specs: Vec<WatchSpec>) {
        lock(&self.wanted).insert(key.to_string(), specs.clone());
        if let Some(s) = lock(&self.sessions).as_ref() {
            s.watcher().add(key, specs);
        }
    }

    /// Stops watching an owner's specs.
    pub fn unwatch(&self, key: &str) {
        lock(&self.wanted).remove(key);
        if let Some(s) = lock(&self.sessions).as_ref() {
            s.watcher().remove(key);
        }
    }

    /// The owners watched now, the apps' included (`app:<id>`).
    pub fn watched(&self) -> Vec<String> {
        lock(&self.sessions).as_ref().map(|s| s.watcher().keys()).unwrap_or_default()
    }

    /// Hands the watcher paths as if the system had said they changed
    /// (`Watcher::inject`), so a test of what follows an event needn't
    /// wait on a busy Mac's FSEvents. Nothing before the watcher is up.
    #[cfg(test)]
    pub(crate) fn inject(&self, paths: &[PathBuf]) {
        if let Some(s) = lock(&self.sessions).as_ref() {
            s.watcher().inject(paths);
        }
    }

    // ---------- running ----------

    /// Watches the apps' files (`watches`, as the registry's `watches`
    /// gives them), reads the transcripts written lately as history, then
    /// tells what the watcher hears, one report at a time, with the
    /// reconcile every 10 minutes.
    pub async fn start(self: &Arc<Self>, watches: Vec<(Arc<dyn Platform>, Vec<WatchSpec>)>) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Report>();
        let me = self.clone();
        let events = crate::runtime::util::blocking(move || me.prime()).await;
        self.tell(events);
        // The watcher comes up beside the map's first refresh, since its
        // system watches take a while to set up (FSEvents starts its stream
        // again for each folder). Then what was written since the history
        // was read is caught up, and the watcher's cursors seeded.
        let me = self.clone();
        let heard = tx.clone();
        let reports = tokio::spawn(async move {
            let m = me.clone();
            let events = crate::runtime::util::blocking(move || {
                match Sessions::start(watches, move |r| {
                    let _ = heard.send(r);
                }) {
                    Ok(s) => {
                        for (key, specs) in lock(&m.wanted).iter() {
                            s.watcher().add(key, specs.clone());
                        }
                        *lock(&m.sessions) = Some(s);
                    }
                    Err(e) => {
                        let why = format!("can't watch the transcripts: {e}");
                        log("warn", "live.watch", &why);
                        lock(&m.tail).tailer.life.problem(&why, m.clock.now());
                    }
                }
                m.catch_up()
            })
            .await;
            me.tell(events);
            // A reconcile the watcher asked for as it came up (a folder that
            // isn't there to watch yet) is what was just read.
            let mut early = Vec::new();
            while let Ok(r) = rx.try_recv() {
                if !matches!(r, Report::Reconcile { .. }) {
                    early.push(r);
                }
            }
            for r in early {
                me.report(r).await;
            }
            while let Some(r) = rx.recv().await {
                if let Report::Reconcile { why, .. } = &r {
                    log("info", "live.reconcile", why);
                }
                me.report(r).await;
            }
        });
        let reconcile = tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_millis(RECONCILE_MS));
            every.tick().await;
            loop {
                every.tick().await;
                if tx.send(Report::Reconcile { app: None, why: "the reconcile every 10 minutes".into() }).is_err() {
                    break;
                }
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
        lock(&self.tasks).extend([reports, reconcile, limits]);
        self.read_claude_limits().await;
    }

    pub fn stop(&self) {
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        let sessions = lock(&self.sessions).take();
        drop(sessions);
        let mut t = lock(&self.tail);
        t.tailer.life.begin_stop();
        t.tailer.life.set_state(xenonet_blocks::lifecycle::State::Stopped, self.clock.now());
        lock(&self.status).watching = false;
    }

    // ---------- Claude plan usage ----------

    /// Anthropic's answer every 5 minutes (later when it asks for a
    /// pause), and Claude desktop's own samples; the newer reading wins.
    pub async fn read_claude_limits(&self) {
        let now = self.clock.now();
        if let Some(a) = &self.anthropic {
            let before = a.last().map(|l| l.at);
            let vendor = self.vendor.clone();
            let at = self.clock.now();
            let r = if a.begin(false, at, None).is_ok() {
                let t0 = self.clock.now();
                let r = fetch_claude_usage(&vendor, now).await;
                let wait = claude_usage::next_in(Some(&r)).and_then(|w| w.as_f64());
                a.done(r, at, self.clock.now() - t0, None, wait)
            } else {
                a.value()
            };
            if a.last().map(|l| l.at) != before {
                let _ = lock(&self.telemetry).usage_reading(crate::apps::anthropic_usage_app(), Some(&r), now);
            }
        }
        if let Some((d, dir)) = &self.desktop {
            let file = Path::new(dir).join("plan-usage-history.json");
            let vendor = self.vendor.clone();
            let samples = d.get(false, None, move || desktop_samples(&vendor, &file));
            if xenonet_text::text::truthy(Some(&samples)) {
                lock(&self.telemetry).usage_samples(crate::apps::anthropic_usage_app(), &samples);
            }
        }
        lock(&self.telemetry).settle_usage(crate::apps::anthropic_usage_app());
    }

    // ---------- what the state reads ----------

    /// The snapshot, with the tailer's status.
    pub fn snapshot(&self) -> Value {
        let mut snap = lock(&self.telemetry).snapshot(self.clock.now());
        let st = lock(&self.status).clone();
        snap["status"] = json!({ "watching": st.watching, "files": st.files, "lines": st.lines, "errors": st.errors });
        // What Claude desktop calls its connectors beats what the lines suggest.
        let declared = lock(&self.connectors).get(self.clock.now()).to_vec();
        with_declared(&mut snap["resources"], &declared);
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
        let r = lock(&self.telemetry).note_hook(id, Some(body), self.clock.now(), &mut out);
        self.tell(emits(out));
        r.map_err(|e| e.0)
    }

    /// An event the console's simulations make for a session.
    pub fn activity(&self, id: &str, ev: Vec<(String, Option<Value>)>) {
        let mut out = Vec::new();
        {
            let mut t = lock(&self.telemetry);
            let Some(i) = t.find(id) else { return };
            t.note_activity(i, ev, true, self.clock.now(), &mut out);
        }
        self.tell(emits(out));
    }

    pub fn session_notes(&self, id: &str) -> Value {
        lock(&self.telemetry).session_notes(id, self.clock.now())
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
        // What the other consoles in a network may show of it (network.md,
        // section 7): the text of what waits on you, what it said last and
        // why it seems stuck. Presence reads them; the engine's view of an
        // agent, which agents read, never carries them.
        if matches!(needs.get("kind").and_then(Value::as_str), Some("permission" | "question")) {
            if let Some(t) = needs.get("text").filter(|t| t.is_string()) {
                m.insert("question".into(), t.clone());
            }
        }
        m.insert("lastMessage".into(), s.last_message());
        if let Some(r) = s.stuck(now).get("reasons").filter(|r| r.is_array()) {
            m.insert("stuck".into(), r.clone());
        }
        m.insert("edits".into(), Value::Array(edits));
        Some(Value::Object(m))
    }

    /// Whether a live session waits on you now.
    pub fn someone_waits(&self, now: f64) -> bool {
        let t = lock(&self.telemetry);
        t.ids().iter().filter_map(|id| t.find(id)).any(|i| xenonet_text::text::truthy(Some(&t.at(i).needs_you(now))))
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
        let watchers = if t.started { self.0.watched().iter().filter(|k| k.starts_with("app:")).count() as f64 } else { 0.0 };
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
    let dir = std::env::var("CLAUDE_CONFIG_DIR").ok().unwrap_or_else(|| Path::new(&std::env::var("HOME").unwrap_or_default()).join(".claude").to_string_lossy().into_owned());
    texts.push(std::fs::read_to_string(Path::new(&dir).join(".credentials.json")).ok());
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
    let res = match xenonet_os::net::fetch("GET", claude_usage::USAGE_URL, &headers, None, Duration::from_secs(8)).await {
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
    let body = xenonet_text::json::parse(&res.text()).ok();
    if body.is_none() {
        vendor.unparsable("anthropic/oauth/usage", None);
    }
    let body = body.and_then(|b| vendor.check("anthropic/oauth/usage", b, None, None));
    claude_usage::reading(body.as_ref(), now)
}

/// Claude desktop's own samples: null while there's no file.
fn desktop_samples(vendor: &Vendor, file: &Path) -> Result<Value, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Value::Null),
        Err(e) => return Err(e.to_string()),
    };
    let raw = xenonet_text::json::parse(&text).map_err(|e| e.to_string())?;
    let version = claude_usage::desktop_version(Some(&raw));
    let checked = vendor.check("anthropic/claude-desktop/plan-usage", raw.clone(), version.as_deref(), None);
    claude_usage::desktop_samples(Some(&raw), checked.as_ref()).map_err(|e| match e {
        claude_usage::DesktopError::Plain(m) | claude_usage::DesktopError::Type(m) => m,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::util::now_ms;
    use serde_json::Map;
    use std::io::Write;
    use xenonet_adapter::watch::{Lines, Read as Took};
    use xenonet_schema::drift::DriftLog;

    fn vendor() -> Arc<Vendor> {
        xenonet_platforms::install();
        Vendor::new(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())))
    }

    const ID: &str = "0190d3e0-0000-7000-8000-000000000001";

    fn line(o: Value) -> String {
        format!("{}\n", xenonet_text::json::stringify(&o, 0))
    }

    fn append(file: &Path, text: &str) {
        std::fs::OpenOptions::new().append(true).open(file).unwrap().write_all(text.as_bytes()).unwrap();
    }

    fn len(file: &Path) -> u64 {
        std::fs::metadata(file).unwrap().len()
    }

    // What the claude adapter would tell of lines written to a file.
    fn lines(file: &Path, from: u64, to: u64, read: Took, lines: &[&str]) -> Report {
        let session = file.file_stem().unwrap().to_string_lossy().into_owned();
        let l = Lines { session, agent: None, file: file.to_path_buf(), from, to, read, lines: lines.iter().map(|l| (*l).to_string()).collect() };
        Report::Events { app: "claude".into(), events: vec![SessionEvent::Lines(l)] }
    }

    fn real(p: PathBuf) -> PathBuf {
        PathBuf::from(xenonet_os::fs::realpath_native(&p.to_string_lossy()).unwrap())
    }

    #[tokio::test]
    async fn history_at_start_then_what_the_watcher_pushes() {
        let home = std::env::temp_dir().join(format!("xenonet-state-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let root = home.join(".claude").join("projects").join("demo-x");
        std::fs::create_dir_all(&root).unwrap();
        let root = real(root);
        let file = root.join(format!("{ID}.jsonl"));
        let ts = |ago: f64| xenonet_text::time::iso(now_ms() - ago);
        std::fs::write(&file, line(json!({ "type": "user", "timestamp": ts(60e3), "message": { "role": "user", "content": "Add a test" } }))).unwrap();
        let live = Live::new(&[(home.join(".claude").join("projects").to_string_lossy().into_owned(), "claude".into())], None, false, &home.to_string_lossy(), vendor());
        let heard: Arc<Mutex<Vec<Event>>> = Arc::default();
        let h = heard.clone();
        live.listen(Arc::new(move |e| lock(&h).extend(e)));
        live.start(Vec::new()).await;
        let snap = live.snapshot();
        assert_eq!(snap["sessions"][0]["id"], format!("claude:{ID}"));
        assert_eq!(snap["status"]["lines"], 1);
        assert!(lock(&heard).iter().all(|e| !matches!(e, Event::Activity(_))), "history isn't told as it happens");
        // A tool call, as the watcher reads it on from where the tailer
        // read to: told as live.
        let call = line(
            json!({ "type": "assistant", "timestamp": ts(0.0), "message": { "id": "msg_1", "model": "claude-sonnet-4-5", "stop_reason": "tool_use", "usage": { "input_tokens": 10, "output_tokens": 5 }, "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "npm test" } }] } }),
        );
        let before = len(&file);
        append(&file, &call);
        live.report(lines(&file, before, len(&file), Took::Appended, &[call.trim_end()])).await;
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::Activity(a) if a["kind"] == "tool")), "{:?}", lock(&heard));
        assert_eq!(live.snapshot()["sessions"][0]["status"], "tool");
        assert_eq!(live.snapshot()["status"]["lines"], 2);
        assert!(live.info(&format!("claude:{ID}"), now_ms()).is_some());
        // The same lines told again (a cursor behind the tailer) read
        // nothing twice.
        live.report(lines(&file, before, len(&file), Took::Appended, &[call.trim_end()])).await;
        assert_eq!(live.snapshot()["status"]["lines"], 2);
        // Lines that don't go on from where the tailer read (a cursor
        // ahead of it): the tailer reads the file itself.
        let done = line(json!({ "type": "user", "timestamp": ts(0.0), "message": { "role": "user", "content": "Thanks" } }));
        let at = len(&file);
        append(&file, &done);
        append(&file, &done);
        live.report(lines(&file, at + 3, len(&file), Took::Appended, &[done.trim_end()])).await;
        assert_eq!(live.snapshot()["status"]["lines"], 4, "both lines, once each");
        // A new transcript, met for the first time.
        let other = root.join("0190d3e0-0000-7000-8000-000000000002.jsonl");
        let hi = line(json!({ "type": "user", "timestamp": ts(0.0), "message": { "role": "user", "content": "Hi" } }));
        std::fs::write(&other, &hi).unwrap();
        live.report(lines(&other, 0, len(&other), Took::New, &[hi.trim_end()])).await;
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::NewFile(f) if f["sessionId"] == "claude:0190d3e0-0000-7000-8000-000000000002")));
        let h = live.block().health();
        assert_eq!((h["state"].as_str(), h["counters"]["files"].as_f64()), (Some("running"), Some(2.0)));
        // One the watcher missed, found by the reconcile.
        let third = root.join("0190d3e0-0000-7000-8000-000000000004.jsonl");
        std::fs::write(&third, &hi).unwrap();
        live.reconcile("a test").await;
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::NewFile(f) if f["sessionId"] == "claude:0190d3e0-0000-7000-8000-000000000004")));
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::Reconciled { app: None, .. })));
        // A transcript too old to read at start waits, dormant, until it's
        // written to: then what it held is history, and what's new is live.
        let old = root.join("0190d3e0-0000-7000-8000-000000000003.jsonl");
        assert_eq!(lock(&live.tail).tailer.dormant.len(), 0);
        {
            let mut t = lock(&live.tail);
            t.tailer.dormant.insert(&old.to_string_lossy(), 0);
        }
        let back = line(json!({ "type": "user", "timestamp": ts(0.0), "message": { "role": "user", "content": "Back again" } }));
        std::fs::write(&old, &back).unwrap();
        live.report(lines(&old, 0, len(&old), Took::New, &[back.trim_end()])).await;
        assert!(live.snapshot()["sessions"].as_array().unwrap().iter().any(|s| s["id"] == "claude:0190d3e0-0000-7000-8000-000000000003"), "followed once it's written to");
        // Gone: forgotten, and the map hears it.
        lock(&heard).clear();
        std::fs::remove_file(&old).unwrap();
        let gone = SessionEvent::Gone { session: "0190d3e0-0000-7000-8000-000000000003".into(), agent: None, file: old.clone() };
        live.report(Report::Events { app: "claude".into(), events: vec![gone] }).await;
        assert!(!lock(&live.tail).tailer.files.contains(&old.to_string_lossy()));
        assert!(lock(&heard).iter().any(|e| matches!(e, Event::Changed { .. })));
        live.stop();
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn the_watcher_pushes_a_transcripts_lines() {
        let home = std::env::temp_dir().join(format!("xenonet-state-live-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let projects = home.join(".claude").join("projects");
        std::fs::create_dir_all(projects.join("demo-x")).unwrap();
        let projects = real(projects);
        let file = projects.join("demo-x").join(format!("{ID}.jsonl"));
        let ts = |ago: f64| xenonet_text::time::iso(now_ms() - ago);
        std::fs::write(&file, line(json!({ "type": "user", "timestamp": ts(60e3), "message": { "role": "user", "content": "Add a test" } }))).unwrap();
        let vendor = vendor();
        let claude = xenonet_adapter::platforms().get("claude").expect("claude's adapter").clone();
        let env = Map::new();
        let specs = claude.watch(&xenonet_adapter::reader::ListerEnv {
            home_dir: &home.to_string_lossy(),
            root: &projects.to_string_lossy(),
            desktop_dir: None,
            state_dir: "",
            env: &env,
            version: "0",
            handle: tokio::runtime::Handle::current(),
        });
        let live = Live::new(&[(projects.to_string_lossy().into_owned(), "claude".into())], None, false, &home.to_string_lossy(), vendor);
        let heard: Arc<Mutex<Vec<Event>>> = Arc::default();
        let h = heard.clone();
        live.listen(Arc::new(move |e| lock(&h).extend(e)));
        live.start(vec![(claude, specs)]).await;
        let up = std::time::Instant::now();
        while !live.watched().contains(&"app:claude".to_string()) {
            assert!(up.elapsed() < Duration::from_secs(30), "the watcher came up");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let call = json!({ "type": "assistant", "timestamp": ts(0.0), "message": { "id": "msg_1", "model": "claude-sonnet-4-5", "stop_reason": "tool_use", "usage": { "input_tokens": 10, "output_tokens": 5 }, "content": [{ "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "command": "npm test" } }] } });
        let wrote = std::time::Instant::now();
        append(&file, &line(call));
        // The write handed to the watcher as its folder's watch would say
        // it: FSEvents on a busy Mac can hold it back for tens of seconds.
        // The half minute is only a backstop.
        live.inject(std::slice::from_ref(&file));
        while !lock(&heard).iter().any(|e| matches!(e, Event::Activity(a) if a["kind"] == "tool")) {
            assert!(wrote.elapsed() < Duration::from_secs(30), "the line came: {:?}", lock(&heard));
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        eprintln!("a transcript's line reached the telemetry in {:?}", wrote.elapsed());
        assert_eq!(live.snapshot()["status"]["lines"], 2, "the history's line, then the new one, once");
        live.stop();
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[tokio::test]
    async fn a_grok_sessions_tokens_from_its_usage() {
        let home = std::env::temp_dir().join(format!("xenonet-state-live-grok-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let sessions = home.join(".grok").join("sessions");
        let dir = sessions.join("%2Fa").join(ID);
        std::fs::create_dir_all(&dir).unwrap();
        let ts = |ago: f64| (now_ms() - ago) / 1000.0;
        let chunk = json!({ "timestamp": ts(60e3).floor(), "method": "session/update", "params": { "update": { "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "Add a test" } } } });
        std::fs::write(dir.join("updates.jsonl"), line(chunk)).unwrap();
        std::fs::write(dir.join("summary.json"), json!({ "info": { "id": ID }, "current_model_id": "grok-build" }).to_string()).unwrap();
        let turn = |n: u64, ended: f64, input: u64, ticks: Option<u64>| {
            let mut row = json!({ "inputTokens": input, "outputTokens": 100, "cachedReadTokens": 1000, "modelCalls": 2 });
            if let Some(t) = ticks {
                row["costUsdTicks"] = json!(t);
            }
            json!({ "turnNumber": n, "endedAt": xenonet_text::time::iso(ended), "modelUsage": { "grok-build": row } })
        };
        std::fs::write(dir.join("usage.json"), json!({ "turns": [turn(1, now_ms() - 50e3, 3000, None)] }).to_string()).unwrap();
        let live = Live::new(&[(sessions.to_string_lossy().into_owned(), "grok".into())], None, false, &home.to_string_lossy(), vendor());
        live.start(Vec::new()).await;
        let grok = |live: &Arc<Live>| live.snapshot()["sessions"].as_array().unwrap().iter().find(|s| s["app"] == "grok").cloned().unwrap();
        assert_eq!((grok(&live)["tokens"]["processed"].as_f64(), grok(&live)["model"].as_str()), (Some(3100.0), Some("grok-build")));
        assert_eq!((live.snapshot()["costs"]["byApp"]["grok"].clone(), live.snapshot()["costs"]["today"].clone()), (Value::Null, Value::Null), "no cost recorded isn't $0");
        // A turn with a cost, read once the updates say something new.
        std::fs::write(dir.join("usage.json"), json!({ "turns": [turn(1, now_ms() - 50e3, 3000, None), turn(2, now_ms(), 2000, Some(50_000_000))] }).to_string()).unwrap();
        let end = json!({ "timestamp": ts(0.0).floor(), "method": "_x.ai/session/update", "params": { "update": { "sessionUpdate": "turn_completed" } } });
        std::fs::OpenOptions::new().append(true).open(dir.join("updates.jsonl")).unwrap().write_all(line(end).as_bytes()).unwrap();
        live.reconcile("a test").await;
        assert_eq!(grok(&live)["tokens"]["processed"].as_f64(), Some(5200.0));
        assert_eq!(live.snapshot()["costs"]["byApp"]["grok"].as_f64(), Some(0.005));
        live.stop();
        std::fs::remove_dir_all(&home).unwrap();
    }
}
