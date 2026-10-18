//! The console's side of hosts (core/hosts/hub.js and link.js;
//! crates/xenonet-hosts/src/hub.rs, link.rs, merge.rs, cache.rs): a link to
//! each host in hosts.json, each host's last snapshot kept in the cache
//! folder, and the merged view the service sends in place of this
//! machine's own. How a host stands, what the page hears of it, the links'
//! states, the merging and the cache are the Rust's. The programs
//! (`ssh <host> xenonet relay`, or another XenoNet's relay on this
//! machine), their pipes, the timers and the reads' waits are here.
//!
//! Everything sits behind one lock, as the JS ran on one thread: a link's
//! effects, the hub's handlers of its events, and the merges. The map and
//! the live state are read from the other cores outside it, so a core that
//! tells the hub of a change while it holds its own lock can't wait on the
//! hub while the hub waits on it.

use super::config::HostsStore;
use super::schemas::{HostSchemas, MergeSchemas};
use crate::fx::{env_map, now_ms, Locked};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use xenonet_hosts::{aliases, cache, frames, hub as core, link, merge};
use xenonet_service::cores::hosts::HubAnswer;
use xenonet_service::cores::{CoreError, Hub, State, Store, R};
use xenonet_service::glue::From;
use xenonet_service::wire::Emitter;

/// What the hub reads of this machine, and where it says what changed.
pub struct HubParts {
    pub hosts: Arc<HostsStore>,
    pub schemas: Arc<HostSchemas>,
    pub state: Arc<dyn State>,
    pub store: Arc<dyn Store>,
    pub cache_dir: String,
    pub root: String,
    pub home: String,
    pub demo: bool,
    pub windows: Value,
    pub emitter: Emitter,
    /// The program a local link runs cli/xenonet.js with, and whether it's
    /// Electron run as Node.
    pub exec: String,
    pub run_as_node: bool,
    pub ssh_bin: Vec<String>,
}

/// Which link an event is for: a host's (by its id and which start of it),
/// or a test's.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LinkRef {
    Host(String, u64),
    Probe(u64),
}

struct Child {
    stdin: mpsc::UnboundedSender<String>,
    kill: Option<oneshot::Sender<()>>,
}

type GetReply = Result<Value, (String, u16)>;

/// One link: the Rust's core, and its programs, timers and reads.
struct LinkRt {
    core: link::Link,
    children: HashMap<u64, Child>,
    lines: HashMap<u64, frames::LineSplitter>,
    timers: HashMap<String, JoinHandle<()>>,
    gets: HashMap<u64, (oneshot::Sender<GetReply>, f64)>,
    unstarted: HashSet<u64>,
}

impl LinkRt {
    fn abandon(&mut self) {
        for (_, t) in self.timers.drain() {
            t.abort();
        }
        for (_, mut c) in self.children.drain() {
            if let Some(k) = c.kill.take() {
                let _ = k.send(());
            }
        }
    }
}

struct Rt {
    gen: u64,
    link: LinkRt,
    simulated: bool,
    clean: merge::CleanOnce,
    snapshot: Option<Value>,
    cached: bool,
    cached_hello: Value,
    doctor: Value,
    last_cache_at: Option<f64>,
    cache_timer: Option<JoinHandle<()>>,
    cache_key: Option<String>,
    skew_ms: f64,
}

struct Probe {
    link: LinkRt,
    id: String,
    started: f64,
    got: Option<Value>,
    timer: Option<JoinHandle<()>>,
    done: Option<oneshot::Sender<Value>>,
}

#[derive(Default)]
struct HubState {
    runtimes: Vec<(String, Rt)>,
    probes: HashMap<u64, Probe>,
    next: u64,
    soon_graph: Option<JoinHandle<()>>,
    soon_live: Option<JoinHandle<()>>,
    last_graph: f64,
    last_live: f64,
    aliases: Option<aliases::AliasReader>,
    alias_map: HashMap<String, String>,
}

pub struct HostHub {
    parts: HubParts,
    st: Mutex<HubState>,
    me: Weak<HostHub>,
}

// A random number for the links' jitter: xorshift on the clock, which is
// all a backoff's spread needs.
fn random() -> f64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEED: AtomicU64 = AtomicU64::new(0);
    let mut x = SEED.load(Ordering::Relaxed);
    if x == 0 {
        x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64) | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    SEED.store(x, Ordering::Relaxed);
    (x >> 11) as f64 / (1u64 << 53) as f64
}

struct Asks<'a> {
    schemas: &'a HostSchemas,
    home: &'a str,
}

impl link::Asks for Asks<'_> {
    fn random(&mut self) -> f64 {
        random()
    }
    fn check_hello(&mut self, frame: &Value) -> Option<String> {
        self.schemas.check_hello(frame)
    }
    fn tilde(&self, p: &str) -> String {
        if self.home.is_empty() {
            p.to_string()
        } else {
            xenonet_base::paths::tilde(p, self.home)
        }
    }
}

fn fail(message: impl Into<String>, status: u16) -> CoreError {
    CoreError::status(status, message)
}

fn s(v: Option<&Value>) -> Option<String> {
    v.filter(|x| !x.is_null()).map(xenonet_base::text::js_string)
}

impl HostHub {
    pub fn new(parts: HubParts) -> Arc<Self> {
        let home = parts.home.clone();
        Arc::new_cyclic(|me| HostHub {
            parts,
            st: Mutex::new(HubState { aliases: Some(aliases::AliasReader::new((!home.is_empty()).then_some(home.as_str()))), ..Default::default() }),
            me: me.clone(),
        })
    }

    fn asks(&self) -> Asks<'_> {
        Asks { schemas: &self.parts.schemas, home: &self.parts.home }
    }

    fn emit(&self, name: &str, data: Value) {
        self.parts.emitter.emit(From::Hub, name, data);
    }

    fn options(&self) -> link::Options {
        link::Options {
            ssh_config: Some(self.parts.hosts.ssh_config_file()),
            ssh_bin: self.parts.ssh_bin.clone(),
            exec: self.parts.exec.clone(),
            run_as_node: self.parts.run_as_node,
            root: self.parts.root.clone(),
            env: env_map(),
        }
    }

    fn new_link(&self, entry: Value, once: bool) -> LinkRt {
        let core = link::Link::new(entry, self.options(), now_ms(), self.parts.demo, once, link::HELLO_MS, link::STALE_MS, (link::BACKOFF_MIN, link::BACKOFF_MAX), link::GET_MS);
        LinkRt { core, children: HashMap::new(), lines: HashMap::new(), timers: HashMap::new(), gets: HashMap::new(), unstarted: HashSet::new() }
    }

    // ---------- the links' effects ----------

    fn link_mut<'a>(st: &'a mut HubState, r: &LinkRef) -> Option<&'a mut LinkRt> {
        match r {
            LinkRef::Host(id, gen) => st.runtimes.iter_mut().find(|(k, rt)| k == id && rt.gen == *gen).map(|(_, rt)| &mut rt.link),
            LinkRef::Probe(p) => st.probes.get_mut(p).map(|p| &mut p.link),
        }
    }

    /// Does what a link's core says, in order.
    fn apply(&self, st: &mut HubState, r: &LinkRef, fx: Vec<link::Effect>) {
        for e in fx {
            match e {
                link::Effect::Emit { name, data } => self.on_event(st, r, name, data),
                e => {
                    let Some(l) = Self::link_mut(st, r) else { return };
                    let more = self.effect(l, r, e);
                    if !more.is_empty() {
                        self.apply(st, r, more);
                    }
                }
            }
        }
    }

    // One effect on its link; what the link says after (a program that
    // couldn't start ends at once).
    fn effect(&self, l: &mut LinkRt, r: &LinkRef, e: link::Effect) -> Vec<link::Effect> {
        match e {
            link::Effect::Spawn { run, file, args, env } => self.spawn(l, r, run, &file, &args, &env),
            link::Effect::Write { run, text } => {
                if let Some(c) = l.children.get(&run) {
                    let _ = c.stdin.send(text);
                }
                Vec::new()
            }
            link::Effect::Kill { run } => {
                if let Some(k) = l.children.get_mut(&run).and_then(|c| c.kill.take()) {
                    let _ = k.send(());
                }
                Vec::new()
            }
            link::Effect::Fail { run } => match l.children.get_mut(&run).and_then(|c| c.kill.take()) {
                Some(k) => {
                    let _ = k.send(());
                    Vec::new()
                }
                None => self.end(l, run, None, None, None),
            },
            link::Effect::Timer { name, ms } | link::Effect::Interval { name, ms }
                if name.strip_prefix("hello:").and_then(|n| n.parse::<u64>().ok()).is_some_and(|n| l.unstarted.remove(&n)) =>
            {
                let _ = ms;
                Vec::new()
            }
            link::Effect::Timer { name, ms } => {
                self.set_timer(l, r, name, ms, false);
                Vec::new()
            }
            link::Effect::Interval { name, ms } => {
                self.set_timer(l, r, name, ms, true);
                Vec::new()
            }
            link::Effect::Clear { name } => {
                if let Some(t) = l.timers.remove(&name) {
                    t.abort();
                }
                Vec::new()
            }
            link::Effect::Resolve { id, value } => {
                if let Some((tx, _)) = l.gets.remove(&id) {
                    let _ = tx.send(Ok(value));
                }
                Vec::new()
            }
            link::Effect::Reject { id, message, status } => {
                if let Some((tx, _)) = l.gets.remove(&id) {
                    let _ = tx.send(Err((message, status)));
                }
                Vec::new()
            }
            link::Effect::Emit { .. } => Vec::new(),
        }
    }

    fn end(&self, l: &mut LinkRt, run: u64, exit_code: Option<f64>, signal: Option<String>, spawn_error: Option<&str>) -> Vec<link::Effect> {
        let e = link::Ended { exit_code, signal, stderr: "", bye: None, spawn_error };
        l.core.end(run, &e, now_ms(), &mut self.asks())
    }

    fn set_timer(&self, l: &mut LinkRt, r: &LinkRef, name: String, ms: f64, interval: bool) {
        if let Some(t) = l.timers.remove(&name) {
            t.abort();
        }
        let (me, r2, n) = (self.me.clone(), r.clone(), name.clone());
        let every = Duration::from_millis(ms.max(0.0) as u64);
        let t = tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(hub) = me.upgrade() else { return };
                hub.fire(&r2, &n, !interval);
                if !interval {
                    return;
                }
            }
        });
        l.timers.insert(name, t);
    }

    fn fire(&self, r: &LinkRef, name: &str, once: bool) {
        let mut st = self.st.locked();
        let now = now_ms();
        let Some(l) = Self::link_mut(&mut st, r) else { return };
        if once {
            l.timers.remove(name);
        }
        let (kind, n) = name.split_once(':').unwrap_or((name, ""));
        let n: u64 = n.parse().unwrap_or(0);
        let fx = match kind {
            "stale" => l.core.check_stale(now),
            "retry" => l.core.retry(now, &mut self.asks()),
            "hello" => l.core.hello_timeout(n, now, &mut self.asks()),
            "time" => l.core.time_tick(n, now),
            "get" => {
                let ms = l.gets.get(&n).map_or(l.core.get_ms, |(_, ms)| *ms);
                l.core.get_timeout(n, ms)
            }
            _ => Vec::new(),
        };
        self.apply(&mut st, r, fx);
    }

    fn spawn(&self, l: &mut LinkRt, r: &LinkRef, run: u64, file: &str, args: &[String], env: &serde_json::Map<String, Value>) -> Vec<link::Effect> {
        let mut cmd = tokio::process::Command::new(file);
        cmd.args(args)
            .env_clear()
            .envs(crate::fx::env_pairs(env))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                l.unstarted.insert(run);
                let code = xenonet_base::fs::code_of(&e).map_or_else(|| e.to_string(), str::to_string);
                return self.end(l, run, None, None, Some(&code));
            }
        };
        let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
        let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
        l.children.insert(run, Child { stdin: stdin_tx, kill: Some(kill_tx) });
        l.lines.insert(run, frames::LineSplitter::new(frames::MAX_FRAME));
        if let Some(mut stdin) = child.stdin.take() {
            tokio::spawn(async move {
                while let Some(text) = stdin_rx.recv().await {
                    if stdin.write_all(text.as_bytes()).await.is_err() {
                        break;
                    }
                }
            });
        }
        let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
        let (me, r) = (self.me.clone(), r.clone());
        tokio::spawn(async move {
            let (mut obuf, mut ebuf) = (vec![0u8; 64 * 1024], vec![0u8; 8192]);
            let (mut out_open, mut err_open, mut killed) = (out.is_some(), err.is_some(), false);
            while out_open || err_open {
                tokio::select! {
                    n = async { match out.as_mut() { Some(o) => o.read(&mut obuf).await.unwrap_or(0), None => 0 } }, if out_open => {
                        if n == 0 { out_open = false; continue; }
                        let Some(hub) = me.upgrade() else { return };
                        hub.output(&r, run, &obuf[..n]);
                    }
                    n = async { match err.as_mut() { Some(e) => e.read(&mut ebuf).await.unwrap_or(0), None => 0 } }, if err_open => {
                        if n == 0 { err_open = false; continue; }
                        let Some(hub) = me.upgrade() else { return };
                        let text = String::from_utf8_lossy(&ebuf[..n]).into_owned();
                        let mut st = hub.st.locked();
                        if let Some(l) = Self::link_mut(&mut st, &r) {
                            l.core.stderr(run, &text);
                        }
                    }
                    _ = &mut kill_rx, if !killed => {
                        killed = true;
                        let _ = child.start_kill();
                    }
                }
            }
            let status = child.wait().await.ok();
            let Some(hub) = me.upgrade() else { return };
            #[cfg(unix)]
            let signal = {
                use std::os::unix::process::ExitStatusExt;
                status.and_then(|s| s.signal()).map(|n| {
                    if n == 15 {
                        "SIGTERM".to_string()
                    } else if n == 9 {
                        "SIGKILL".to_string()
                    } else {
                        format!("SIG{n}")
                    }
                })
            };
            #[cfg(not(unix))]
            let signal: Option<String> = None;
            let code = status.and_then(|s| s.code()).map(f64::from);
            let mut st = hub.st.locked();
            let Some(l) = Self::link_mut(&mut st, &r) else { return };
            l.children.remove(&run);
            l.lines.remove(&run);
            let fx = hub.end(l, run, code, signal, None);
            hub.apply(&mut st, &r, fx);
        });
        Vec::new()
    }

    // What a program printed: its lines, each a frame for the link.
    fn output(&self, r: &LinkRef, run: u64, bytes: &[u8]) {
        let mut st = self.st.locked();
        let Some(l) = Self::link_mut(&mut st, r) else { return };
        let Some(split) = l.lines.get_mut(&run).map(|s| s.push(bytes)) else { return };
        let mut lines = split.lines.into_iter();
        for is_line in split.order {
            let now = now_ms();
            let Some(l) = Self::link_mut(&mut st, r) else { return };
            if !is_line {
                l.core.too_big(run, now);
                continue;
            }
            let Some(line) = lines.next() else { break };
            let fx = l.core.line(run, &line, now, &mut self.asks());
            self.apply(&mut st, r, fx);
        }
    }

    // ---------- the hub's handlers of its links' events ----------

    fn rt_mut<'a>(st: &'a mut HubState, id: &str, gen: u64) -> Option<&'a mut Rt> {
        st.runtimes.iter_mut().find(|(k, rt)| k == id && rt.gen == gen).map(|(_, rt)| rt)
    }

    fn on_event(&self, st: &mut HubState, r: &LinkRef, name: &str, data: Value) {
        match r {
            LinkRef::Probe(p) => self.on_probe(st, *p, name, &data),
            LinkRef::Host(id, gen) => {
                let gen = *gen;
                match name {
                    "state" => self.hosts_changed(st),
                    "learn" => {
                        let Some(rt) = Self::rt_mut(st, id, gen) else { return };
                        if rt.simulated {
                            return;
                        }
                        // hosts.json can't be written (a newer XenoNet's, say): the
                        // link keeps what it learned until XenoNet quits.
                        if let Err(e) = self.parts.hosts.learn(id, data.get("key"), data.get("name")) {
                            self.parts.emitter.log("warn", "hosts.learn", &format!("couldn't keep {id}'s key: {}", e.message));
                        }
                    }
                    "snapshot" => {
                        let Some(rt) = Self::rt_mut(st, id, gen) else { return };
                        rt.snapshot = Some(json!({ "graph": data.get("graph"), "live": data.get("live"), "engine": data.get("engine") }));
                        rt.cached = false;
                        self.save_soon(rt, id);
                        self.hosts_changed(st);
                    }
                    "event" => {
                        let (ename, edata) = (data.get("name").and_then(Value::as_str).unwrap_or("").to_string(), data.get("data").cloned().unwrap_or(Value::Null));
                        let Some(rt) = Self::rt_mut(st, id, gen) else { return };
                        if ename == "graph" && edata.get("nodes").is_some_and(Value::is_array) {
                            Self::snapshot_part(rt, "graph", edata);
                            self.save_soon(rt, id);
                            return self.graph_soon(st);
                        }
                        if ename == "live" {
                            Self::snapshot_part(rt, "live", edata);
                            self.save_soon(rt, id);
                            return self.live_soon(st);
                        }
                        // A host's own doctor feeds its state here, never this XenoNet's.
                        if ename == "doctor" {
                            rt.doctor = core::doctor_of(&edata);
                            let page = self.for_page(st);
                            return self.emit("hosts", page);
                        }
                        let info = Self::info_of(rt);
                        if let Some(q) = merge::qualify_event(&ename, &edata, &info, &mut MergeSchemas(&self.parts.schemas)) {
                            self.emit(&ename, q);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn snapshot_part(rt: &mut Rt, key: &str, v: Value) {
        let mut snap = rt.snapshot.take().unwrap_or_else(|| json!({}));
        snap[key] = v;
        rt.snapshot = Some(snap);
    }

    fn on_probe(&self, st: &mut HubState, p: u64, name: &str, data: &Value) {
        let Some(probe) = st.probes.get_mut(&p) else { return };
        match name {
            "snapshot" => {
                probe.got = Some(core::probe_snapshot(data.get("graph").unwrap_or(&Value::Null), data.get("live").unwrap_or(&Value::Null)));
                // The clock's answer is on its way; a moment more for it.
                if !probe.link.core.info()["offsetMs"].is_null() {
                    self.finish(st, p, true);
                } else {
                    self.probe_timer(st, p, 2000, true);
                }
            }
            "offset" if probe.got.is_some() => self.finish(st, p, true),
            "state" if matches!(data.get("state").and_then(Value::as_str), Some("offline" | "error")) => self.finish(st, p, false),
            _ => {}
        }
    }

    fn probe_timer(&self, st: &mut HubState, p: u64, ms: u64, ok: bool) {
        let Some(probe) = st.probes.get_mut(&p) else { return };
        if let Some(t) = probe.timer.take() {
            t.abort();
        }
        let me = self.me.clone();
        probe.timer = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            if let Some(hub) = me.upgrade() {
                let mut st = hub.st.locked();
                hub.finish(&mut st, p, ok);
            }
        }));
    }

    fn finish(&self, st: &mut HubState, p: u64, ok: bool) {
        let Some(mut probe) = st.probes.remove(&p) else { return };
        if let Some(t) = probe.timer.take() {
            t.abort();
        }
        let info = probe.link.core.info();
        let _ = probe.link.core.stop(now_ms());
        probe.link.abandon();
        let out = json!({
            "id": probe.id, "ok": ok, "ms": xenonet_base::json::number(now_ms() - probe.started), "state": info["state"], "error": info["error"], "hello": info["hello"],
            "offsetMs": info["offsetMs"], "rttMs": info["rttMs"], "target": info["target"], "snapshot": probe.got, "dropped": info["counts"]["dropped"],
        });
        if let Some(done) = probe.done.take() {
            let _ = done.send(out);
        }
    }

    // ---------- what goes out ----------

    fn info_of(rt: &Rt) -> Value {
        core::info_of(&rt.link.core.info(), rt.cached, &rt.cached_hello, rt.skew_ms)
    }

    fn view(rt: &Rt) -> Value {
        // Only the map's nodes count toward how a host stands.
        let snap = rt.snapshot.as_ref().and_then(|s| s.get("graph")).filter(|g| !g.is_null()).map_or(Value::Null, |g| json!({ "graph": { "nodes": g.get("nodes") } }));
        core::view(&Self::info_of(rt), &rt.link.core.entry, &snap, rt.cached, rt.simulated, &rt.doctor)
    }

    fn list_of(st: &HubState) -> Vec<Value> {
        st.runtimes.iter().map(|(_, rt)| Self::view(rt)).collect()
    }

    fn for_page(&self, st: &HubState) -> Value {
        Value::Array(Self::list_of(st).iter().map(core::for_page).collect())
    }

    fn hosts_changed(&self, st: &mut HubState) {
        let page = self.for_page(st);
        self.emit("hosts", page);
        self.graph_soon(st);
        self.live_soon(st);
    }

    // At most once a second for the map, every half second for the live
    // state, the last change always going; read from the map outside the
    // lock.
    fn graph_soon(&self, st: &mut HubState) {
        if st.soon_graph.is_some() {
            return;
        }
        let wait = core::soon_wait(st.last_graph, core::GRAPH_EVERY_MS, now_ms());
        let me = self.me.clone();
        st.soon_graph = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
            let Some(hub) = me.upgrade() else { return };
            {
                let mut st = hub.st.locked();
                st.soon_graph = None;
                st.last_graph = now_ms();
            }
            if let Ok(Some(g)) = hub.parts.state.graph() {
                let merged = hub.merged_graph(g);
                hub.emit("graph", merged);
            }
        }));
    }

    fn live_soon(&self, st: &mut HubState) {
        if st.soon_live.is_some() {
            return;
        }
        let wait = core::soon_wait(st.last_live, core::LIVE_EVERY_MS, now_ms());
        let me = self.me.clone();
        st.soon_live = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
            let Some(hub) = me.upgrade() else { return };
            {
                let mut st = hub.st.locked();
                st.soon_live = None;
                st.last_live = now_ms();
            }
            if let Ok(l) = hub.parts.state.live_snapshot() {
                let merged = hub.merged_live(l);
                hub.emit("live", merged);
            }
        }));
    }

    fn merged_graph(&self, local: Value) -> Value {
        // The layout and names are the store's, read before the hub's lock.
        let layout = self.parts.store.layout().unwrap_or_else(|_| json!({}));
        let ids = self.parts.store.identities().unwrap_or_else(|_| json!({}));
        let self_id = self.parts.hosts.self_entry().get("id").cloned().unwrap_or(Value::Null);
        let mut st = self.st.locked();
        if st.runtimes.is_empty() {
            return local;
        }
        let aliases = match st.aliases.as_mut().map(aliases::AliasReader::look) {
            Some(aliases::Look::Read(file)) => {
                let text = std::fs::read_to_string(&file).unwrap_or_default();
                Some(xenonet_state::identities::parse_ssh_aliases(&text).into_iter().collect())
            }
            Some(aliases::Look::None) => Some(HashMap::new()),
            _ => None,
        };
        if let Some(a) = aliases {
            st.alias_map = a;
        }
        let alias_map = st.alias_map.clone();
        let mut ins: Vec<merge::HostIn> = Vec::new();
        let infos: Vec<(Value, Value)> = st.runtimes.iter().map(|(_, rt)| (Self::info_of(rt), rt.snapshot.clone().unwrap_or(Value::Null))).collect();
        for ((_, rt), (info, snapshot)) in st.runtimes.iter_mut().zip(infos) {
            ins.push(merge::HostIn { info, snapshot, clean: Some(&mut rt.clean) });
        }
        let names = ids.get("names").cloned().unwrap_or_else(|| json!({}));
        let hidden = ids.get("hidden").cloned().unwrap_or_else(|| json!({}));
        merge::merge_graph(&local, &mut ins, &layout, now_ms(), &self.parts.windows, &self_id, &names, &hidden, &alias_map, &mut MergeSchemas(&self.parts.schemas))
    }

    fn pairs(st: &HubState) -> Vec<(Value, Value)> {
        st.runtimes.iter().map(|(_, rt)| (Self::info_of(rt), rt.snapshot.clone().unwrap_or(Value::Null))).collect()
    }

    fn merged_live(&self, local: Value) -> Value {
        let st = self.st.locked();
        if st.runtimes.is_empty() {
            return local;
        }
        merge::merge_live_with(&local, &Self::pairs(&st), &mut MergeSchemas(&self.parts.schemas)).0
    }

    // ---------- the cache ----------

    fn save(&self, rt: &mut Rt, id: &str) {
        if let Some(t) = rt.cache_timer.take() {
            t.abort();
        }
        let key = s(rt.link.core.entry.get("key"));
        let (Some(snap), false, Some(key)) = (rt.snapshot.as_ref(), rt.simulated, key) else { return };
        rt.last_cache_at = Some(now_ms());
        let saved_at = rt.link.core.last_heard.unwrap_or_else(now_ms);
        if !cache::write_cache(
            Some(&self.parts.cache_dir),
            &key,
            &json!(id),
            &xenonet_base::json::number(saved_at),
            &rt.link.core.hello,
            snap.get("graph").filter(|g| !g.is_null()),
            snap.get("live").filter(|l| !l.is_null()),
            snap.get("engine").unwrap_or(&Value::Null),
        ) {
            return;
        }
        rt.cache_key = Some(key);
    }

    fn save_soon(&self, rt: &mut Rt, id: &str) {
        if rt.cache_timer.is_some() || rt.simulated {
            return;
        }
        let wait = core::cache_wait(rt.last_cache_at, now_ms());
        let (me, id, gen) = (self.me.clone(), id.to_string(), rt.gen);
        rt.cache_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(wait as u64)).await;
            let Some(hub) = me.upgrade() else { return };
            let mut st = hub.st.locked();
            if let Some(rt) = Self::rt_mut(&mut st, &id, gen) {
                rt.cache_timer = None;
                hub.save(rt, &id);
            }
        }));
    }

    // A file another host's link uses too stays for that one.
    fn forget_cache(&self, st: &HubState, gone: &Rt) {
        if gone.simulated {
            return;
        }
        let keys: HashSet<String> = [s(gone.link.core.entry.get("key")), gone.cache_key.clone()].into_iter().flatten().collect();
        for key in keys {
            let used =
                st.runtimes.iter().any(|(_, o)| !o.simulated && (s(o.link.core.entry.get("key")).as_deref() == Some(key.as_str()) || o.cache_key.as_deref() == Some(key.as_str())));
            if !used {
                let _ = cache::remove_cache(Some(&self.parts.cache_dir), &key);
            }
        }
    }

    // ---------- each host ----------

    fn start_one(&self, st: &mut HubState, entry: Value, simulated: bool) -> u64 {
        let id = s(entry.get("id")).unwrap_or_default();
        let cached = s(entry.get("key")).and_then(|k| cache::read_cache(Some(&self.parts.cache_dir), &k));
        st.next += 1;
        let gen = st.next;
        let mut rt = Rt {
            gen,
            link: self.new_link(entry.clone(), false),
            simulated,
            clean: merge::CleanOnce::default(),
            snapshot: None,
            cached: false,
            cached_hello: Value::Null,
            doctor: Value::Null,
            last_cache_at: None,
            cache_timer: None,
            cache_key: None,
            skew_ms: 0.0,
        };
        if let Some(c) = cached {
            rt.snapshot = Some(json!({ "graph": c.get("graph"), "live": c.get("live"), "engine": c.get("engine").cloned().unwrap_or(Value::Null) }));
            rt.cached = true;
            rt.cached_hello = c.get("hello").cloned().unwrap_or(Value::Null);
            rt.cache_key = s(entry.get("key"));
            rt.link.core.last_heard = c.get("savedAt").and_then(Value::as_f64);
        }
        st.runtimes.retain(|(k, _)| *k != id);
        st.runtimes.push((id.clone(), rt));
        let r = LinkRef::Host(id.clone(), gen);
        let fx = match Self::rt_mut(st, &id, gen) {
            Some(rt) => rt.link.core.start(now_ms(), &mut self.asks()),
            None => Vec::new(),
        };
        self.apply(st, &r, fx);
        gen
    }

    fn stop_one(&self, rt: &mut Rt, id: &str, keep: bool) {
        // Its listeners first, as removeAllListeners: what it says now
        // reaches no one.
        let _ = rt.link.core.stop(now_ms());
        rt.link.abandon();
        if keep && rt.cache_timer.is_some() {
            self.save(rt, id);
        }
        if let Some(t) = rt.cache_timer.take() {
            t.abort();
        }
    }

    fn take(&self, st: &mut HubState, id: &str) -> Option<Rt> {
        let at = st.runtimes.iter().position(|(k, _)| k == id)?;
        Some(st.runtimes.remove(at).1)
    }

    fn need<'a>(st: &'a mut HubState, id: &str) -> R<&'a mut Rt> {
        st.runtimes.iter_mut().find(|(k, _)| k == id).map(|(_, rt)| rt).ok_or_else(|| fail(format!("there's no host called {id}"), 404))
    }
}

/// The Hub trait, for the routes and the glue.
pub struct HubCore(pub Arc<HostHub>);

#[async_trait]
impl Hub for HubCore {
    fn has(&self, host: &str) -> bool {
        self.0.st.locked().runtimes.iter().any(|(k, _)| k == host)
    }

    async fn get(&self, host: &str, path: &str) -> R<HubAnswer> {
        let hub = &self.0;
        let rx = {
            let mut st = hub.st.locked();
            let rt = HostHub::need(&mut st, host)?;
            let gen = rt.gen;
            let ms = rt.link.core.get_ms;
            let (gid, fx) = rt.link.core.get(&json!(path), ms).map_err(|(m, status)| fail(m, status))?;
            let (tx, rx) = oneshot::channel();
            rt.link.gets.insert(gid, (tx, ms));
            hub.apply(&mut st, &LinkRef::Host(host.to_string(), gen), fx);
            rx
        };
        match rx.await {
            Ok(Ok(v)) => Ok(HubAnswer { status: v.get("status").and_then(Value::as_u64).map_or(502, |s| s as u16), body: v.get("body").cloned().unwrap_or(Value::Null) }),
            Ok(Err((m, status))) => Err(fail(m, status)),
            Err(_) => Err(fail(format!("{host} isn't connected"), 503)),
        }
    }

    fn list(&self) -> R<Value> {
        Ok(Value::Array(HostHub::list_of(&self.0.st.locked())))
    }

    fn add(&self, body: &Value, replace: bool) -> R<Value> {
        let hub = &self.0;
        let entry = hub.parts.hosts.add(body, replace)?;
        let id = s(entry.get("id")).unwrap_or_default();
        let mut st = hub.st.locked();
        if let Some(mut old) = hub.take(&mut st, &id) {
            hub.stop_one(&mut old, &id, false);
            hub.forget_cache(&st, &old);
        }
        let gen = hub.start_one(&mut st, entry, false);
        hub.hosts_changed(&mut st);
        HostHub::rt_mut(&mut st, &id, gen).map(|rt| HostHub::view(rt)).ok_or_else(|| fail(format!("there's no host called {id}"), 404))
    }

    fn detail(&self, id: &str) -> R<Value> {
        let hub = &self.0;
        let mut st = hub.st.locked();
        let rt = HostHub::need(&mut st, id)?;
        let mut v = HostHub::view(rt);
        let engine = rt.snapshot.as_ref().and_then(|s| s.get("engine")).filter(|e| !e.is_null());
        v["engine"] = match engine {
            Some(e) => merge::qualify_engine(e, &HostHub::info_of(rt), &mut MergeSchemas(&hub.parts.schemas)),
            None => Value::Null,
        };
        Ok(v)
    }

    fn remove(&self, id: &str, forget: bool) -> R<Value> {
        let hub = &self.0;
        let simulated = {
            let mut st = hub.st.locked();
            HostHub::need(&mut st, id)?;
            let Some(mut rt) = hub.take(&mut st, id) else { return Err(fail(format!("there's no host called {id}"), 404)) };
            hub.stop_one(&mut rt, id, false);
            hub.forget_cache(&st, &rt);
            rt.simulated
        };
        if !simulated {
            hub.parts.hosts.remove(id)?;
        }
        let forgot = if forget { hub.parts.state.forget_layout(&format!("@{id}:")).unwrap_or(0) } else { 0 };
        let mut st = hub.st.locked();
        hub.hosts_changed(&mut st);
        Ok(json!({ "id": id, "removed": true, "forgot": forgot }))
    }

    fn reconnect(&self, id: &str) -> R<Value> {
        let hub = &self.0;
        let mut st = hub.st.locked();
        let rt = HostHub::need(&mut st, id)?;
        let gen = rt.gen;
        let fx = rt.link.core.reconnect(now_ms(), &mut hub.asks());
        hub.apply(&mut st, &LinkRef::Host(id.to_string(), gen), fx);
        HostHub::need(&mut st, id).map(|rt| HostHub::view(rt))
    }

    // One try, apart from the host's own link, that says how far it got.
    async fn test(&self, id: &str) -> R<Value> {
        let hub = &self.0;
        let rx = {
            let mut st = hub.st.locked();
            let entry = HostHub::need(&mut st, id)?.link.core.entry.clone();
            st.next += 1;
            let p = st.next;
            let (tx, rx) = oneshot::channel();
            st.probes.insert(p, Probe { link: hub.new_link(entry, true), id: id.to_string(), started: now_ms(), got: None, timer: None, done: Some(tx) });
            hub.probe_timer(&mut st, p, core::TEST_MS as u64, false);
            let fx = match st.probes.get_mut(&p) {
                Some(probe) => probe.link.core.start(now_ms(), &mut hub.asks()),
                None => Vec::new(),
            };
            hub.apply(&mut st, &LinkRef::Probe(p), fx);
            rx
        };
        rx.await.map_err(|_| fail(format!("the test of {id} stopped"), 500))
    }

    fn graph(&self, g: Value) -> Value {
        self.0.merged_graph(g)
    }

    fn live(&self, s: Value) -> Value {
        self.0.merged_live(s)
    }

    fn summary(&self, s: Value) -> Value {
        let hub = &self.0;
        let st = hub.st.locked();
        if st.runtimes.is_empty() {
            return s;
        }
        merge::merge_summary(&s, &HostHub::pairs(&st), &mut MergeSchemas(&hub.parts.schemas))
    }

    // XenoNet Dev's simulations (POST /api/dev/simulate).
    fn simulate(&self, kind: &str, args: &Value) -> R<Option<Value>> {
        let hub = &self.0;
        let arg = |k: &str| args.get(k).filter(|v| !v.is_null());
        if kind == "host-add" {
            let input = json!({ "id": arg("id").or(arg("a")), "local": true, "port": arg("port").or(arg("b")) });
            let entry = match hub.parts.hosts.entry(&input) {
                Ok(e) => e,
                Err(e) => return Ok(Some(json!({ "error": e.message }))),
            };
            let id = s(entry.get("id")).unwrap_or_default();
            let mut st = hub.st.locked();
            if st.runtimes.iter().any(|(k, _)| *k == id) {
                return Ok(Some(json!({ "error": format!("there's already a host called {id}") })));
            }
            hub.start_one(&mut st, entry, true);
            hub.hosts_changed(&mut st);
            return Ok(Some(json!({ "ok": true, "kind": kind, "id": id })));
        }
        let mut st = hub.st.locked();
        let named = s(arg("a").or(arg("id")));
        let which = match &named {
            Some(n) => st.runtimes.iter().find(|(k, _)| k == n).map(|(k, rt)| (k.clone(), rt.gen)),
            None => st.runtimes.first().map(|(k, rt)| (k.clone(), rt.gen)),
        };
        let Some((id, gen)) = which else { return Ok(None) };
        let r = LinkRef::Host(id.clone(), gen);
        let Some(rt) = HostHub::rt_mut(&mut st, &id, gen) else { return Ok(None) };
        match kind {
            "host-offline" => {
                let fx = rt.link.core.pause("taken offline by a simulation", now_ms());
                hub.apply(&mut st, &r, fx);
            }
            "host-online" => {
                let fx = rt.link.core.start(now_ms(), &mut hub.asks());
                hub.apply(&mut st, &r, fx);
            }
            "host-skew" => {
                rt.skew_ms = core::skew_of(args.get("ms").unwrap_or(&Value::Null), args.get("b").unwrap_or(&Value::Null));
                hub.hosts_changed(&mut st);
            }
            _ => return Ok(None),
        }
        Ok(Some(json!({ "ok": true, "kind": kind, "a": id })))
    }

    fn start(&self) {
        let hub = &self.0;
        if let Err(e) = hub.parts.hosts.write_ssh_config() {
            hub.parts.emitter.log("warn", "hosts.ssh-config", &format!("couldn't write {}: {}", hub.parts.hosts.ssh_config_file(), e.message));
        }
        let entries = hub.parts.hosts.list();
        let mut st = hub.st.locked();
        for entry in entries {
            hub.start_one(&mut st, entry, false);
        }
        if !st.runtimes.is_empty() {
            hub.hosts_changed(&mut st);
        }
    }

    fn stop(&self) {
        let hub = &self.0;
        let mut st = hub.st.locked();
        for t in [st.soon_graph.take(), st.soon_live.take()].into_iter().flatten() {
            t.abort();
        }
        let mut all = std::mem::take(&mut st.runtimes);
        for (id, rt) in all.iter_mut() {
            hub.stop_one(rt, id, true);
        }
        for (_, mut p) in st.probes.drain() {
            p.link.abandon();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_service::cores::Pending;

    fn hub(name: &str) -> (Arc<HostHub>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let schemas = Arc::new(HostSchemas::new());
        let hosts = Arc::new(HostsStore::new(&dir.join("state").to_string_lossy(), schemas.clone()));
        let parts = HubParts {
            hosts,
            schemas,
            state: Arc::new(Pending),
            store: Arc::new(Pending),
            cache_dir: dir.join("cache").to_string_lossy().into_owned(),
            root: dir.to_string_lossy().into_owned(),
            home: "/home/nova".into(),
            demo: true,
            windows: json!({ "activeWindowMs": 600000, "recentWindowMs": 86400000, "projectActiveMs": 3600000 }),
            emitter: Emitter::new(),
            exec: "/no/such/node".into(),
            run_as_node: false,
            ssh_bin: vec!["ssh".into()],
        };
        (HostHub::new(parts), dir)
    }

    // test/hosts-hub.test.js's simulation cases: a host added by XenoNet
    // Dev, once; its program missing, so it's offline, and named in the
    // list; the map as it was with no host.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_simulated_host_whose_relay_cant_start() {
        let (h, dir) = hub("sim");
        let core = HubCore(h.clone());
        assert_eq!(core.list().unwrap(), json!([]));
        assert_eq!(core.graph(json!({ "nodes": [] })), json!({ "nodes": [] }), "no hosts, the map as it was");
        let added = core.simulate("host-add", &json!({ "kind": "host-add", "id": "demo-b", "port": 47999 })).unwrap().unwrap();
        assert_eq!(added, json!({ "ok": true, "kind": "host-add", "id": "demo-b" }));
        assert!(core.simulate("host-add", &json!({ "id": "demo-b", "port": 47999 })).unwrap().unwrap().get("error").is_some(), "once");
        assert!(core.has("demo-b"));
        let list = core.list().unwrap();
        assert_eq!(list[0]["id"], "demo-b");
        assert_eq!(list[0]["simulated"], true);
        assert!(core.get("demo-b", "/api/graph").await.is_err(), "nothing to read through yet");
        assert_eq!(core.simulate("host-skew", &json!({ "a": "demo-b", "ms": 180000 })).unwrap().unwrap()["a"], "demo-b");
        assert_eq!(core.simulate("host-offline", &json!({ "a": "nobody" })).unwrap(), None);
        let gone = core.remove("demo-b", false).unwrap();
        assert_eq!(gone["removed"], true);
        assert!(core.detail("demo-b").is_err());
        core.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
