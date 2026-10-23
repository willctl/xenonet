//! The console's side of hosts (core/hosts/hub.js and link.js;
//! crates/xenonet-hosts/src/hub.rs, link.rs, merge.rs, cache.rs): a link to
//! each host, each host's last snapshot kept in the cache folder, and the
//! merged view the service sends in place of this machine's own. How a
//! host stands, what the page hears of it, the links' states, the merging
//! and the cache are the Rust's. The programs (`ssh <host> xenonet relay`,
//! or another XenoNet's relay on this machine), their pipes, the network's
//! subscriptions, the timers and the reads' waits are here.
//!
//! The hosts come from two places: hosts.json, over SSH, with
//! XENONET_HOSTS=1; and the network's roster, always (docs/design/
//! network.md, section 7). A network host's link is a subscription to its
//! map through the gateway (transport.rs), opened only while a console is
//! open here and renewed while it stays open. A host in both is shown once,
//! from hosts.json. This host's own map, for the consoles on other hosts
//! that subscribe to it, is relay.rs's.
//!
//! Everything sits behind one lock, as the JS ran on one thread: a link's
//! effects, the hub's handlers of its events, and the merges. The map and
//! the live state are read from the other cores outside it, so a core that
//! tells the hub of a change while it holds its own lock can't wait on the
//! hub while the hub waits on it.

use super::config::HostsStore;
use super::relay::{self as serve, net_key, RelayServer};
use super::schemas::{HostSchemas, MergeSchemas};
use super::transport::{LoopNet, LoopTransport, NetTransport, Transport};
use crate::fx::{env_map, now_ms, Locked};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use xenonet_base::network::{Incoming, Net, RelaySink};
use xenonet_hosts::network::{self as wire, Order};
use xenonet_hosts::{aliases, cache, frames, hub as core, link, merge};
use xenonet_service::cores::hosts::HubAnswer;
use xenonet_service::cores::{CoreError, Hub, HubNet, State, Store, R};
use xenonet_service::glue::From;
use xenonet_service::wire::Emitter;

/// What the page sees on a network host drawn from its relay's snapshot,
/// as `network`; one drawn from its presence has `true` there.
const NET_DRAWN: &str = "relay";

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
    /// The xenonet binary a local link runs `relay` with.
    pub bin: String,
    pub ssh_bin: Vec<String>,
    /// Hosts over SSH, from hosts.json (XENONET_HOSTS=1).
    pub ssh: bool,
    /// The relay's timings, for the consoles that subscribe to this host.
    pub relay: serve::Timing,
}

/// The network, once the service has it.
struct NetSide {
    net: Arc<dyn Net>,
    transport: Arc<dyn Transport>,
    server: Arc<RelayServer>,
    plug: HubNet,
}

/// A subscription a link's run holds: which link and run, the host it's
/// to, the last seq taken, and how it goes.
struct SubRef {
    link: LinkRef,
    run: u64,
    host: String,
    last: u64,
    transport: Arc<dyn Transport>,
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

/// One link: the Rust's core, and its programs or subscriptions, timers
/// and reads.
struct LinkRt {
    core: link::Link,
    children: HashMap<u64, Child>,
    lines: HashMap<u64, frames::LineSplitter>,
    timers: HashMap<String, JoinHandle<()>>,
    gets: HashMap<u64, (oneshot::Sender<GetReply>, f64)>,
    unstarted: HashSet<u64>,
    /// A network host's way to it, and its runs' subscriptions.
    transport: Option<Arc<dyn Transport>>,
    subs: HashMap<u64, String>,
}

impl LinkRt {
    /// Its timers, programs and subscriptions end (a probe has none of the
    /// last, and no `subs`).
    fn abandon(&mut self, subs: Option<&Mutex<HashMap<String, SubRef>>>) {
        for (_, t) in self.timers.drain() {
            t.abort();
        }
        for (_, mut c) in self.children.drain() {
            if let Some(k) = c.kill.take() {
                let _ = k.send(());
            }
        }
        for (_, sub) in self.subs.drain() {
            if let Some(subs) = subs {
                Self::close(subs, &sub);
            }
        }
    }

    /// Ends a subscription: the host hears, and its answers go nowhere.
    fn close(subs: &Mutex<HashMap<String, SubRef>>, sub: &str) {
        if let Some(s) = subs.locked().remove(sub) {
            s.transport.end(&s.host, sub);
        }
    }
}

struct Rt {
    gen: u64,
    link: LinkRt,
    simulated: bool,
    /// A host of this machine's network, from its roster.
    network: bool,
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
    /// Whether a console is open here: network hosts' links run only then.
    console: bool,
}

pub struct HostHub {
    parts: HubParts,
    st: Mutex<HubState>,
    me: Weak<HostHub>,
    net: Mutex<Option<NetSide>>,
    /// Every network link's subscription, by its id. Taken after `st`,
    /// never before it.
    subs: Mutex<HashMap<String, SubRef>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
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

/// Why a network host's link waits.
fn paused(id: &str, console: bool, windowed: bool) -> String {
    if !windowed {
        "this machine isn't a window host, and the network's maps are shown only on one".into()
    } else if !console {
        "no console is open here".into()
    } else {
        format!("{id} is offline in the network")
    }
}

impl HostHub {
    pub fn new(parts: HubParts) -> Arc<Self> {
        let home = parts.home.clone();
        Arc::new_cyclic(|me| HostHub {
            parts,
            st: Mutex::new(HubState { aliases: Some(aliases::AliasReader::new((!home.is_empty()).then_some(home.as_str()))), ..Default::default() }),
            me: me.clone(),
            net: Mutex::new(None),
            subs: Mutex::new(HashMap::new()),
            tasks: Mutex::new(Vec::new()),
        })
    }

    fn asks(&self) -> Asks<'_> {
        Asks { schemas: &self.parts.schemas, home: &self.parts.home }
    }

    fn emit(&self, name: &str, data: Value) {
        self.parts.emitter.emit(From::Hub, name, data);
    }

    fn options(&self) -> link::Options {
        link::Options { ssh_config: Some(self.parts.hosts.ssh_config_file()), ssh_bin: self.parts.ssh_bin.clone(), bin: self.parts.bin.clone(), entry: None, env: env_map() }
    }

    fn new_link(&self, entry: Value, once: bool) -> LinkRt {
        // A network host is the person's own machine, real whatever this
        // XenoNet shows; a made-up one is made up here too.
        let demo_only = self.parts.demo && entry.get("transport").and_then(Value::as_str) != Some("network");
        let core = link::Link::new(entry, self.options(), now_ms(), demo_only, once, link::HELLO_MS, link::STALE_MS, (link::BACKOFF_MIN, link::BACKOFF_MAX), link::GET_MS);
        LinkRt {
            core,
            children: HashMap::new(),
            lines: HashMap::new(),
            timers: HashMap::new(),
            gets: HashMap::new(),
            unstarted: HashSet::new(),
            transport: None,
            subs: HashMap::new(),
        }
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
            link::Effect::Start { run, start: link::Start::Process((file, args, env)) } => self.spawn(l, r, run, &file, &args, &env),
            link::Effect::Start { run, start: link::Start::Network { host } } => self.subscribe(l, r, run, &host),
            link::Effect::Write { run, text } => {
                if let Some(c) = l.children.get(&run) {
                    let _ = c.stdin.send(text);
                } else if let (Some(sub), Some(t)) = (l.subs.get(&run), &l.transport) {
                    t.write(&s(l.core.entry.get("id")).unwrap_or_default(), sub, &text);
                }
                Vec::new()
            }
            link::Effect::Kill { run } => {
                if let Some(k) = l.children.get_mut(&run).and_then(|c| c.kill.take()) {
                    let _ = k.send(());
                }
                if let Some(sub) = l.subs.remove(&run) {
                    LinkRt::close(&self.subs, &sub);
                }
                Vec::new()
            }
            link::Effect::Fail { run } => match l.children.get_mut(&run).and_then(|c| c.kill.take()) {
                Some(k) => {
                    let _ = k.send(());
                    Vec::new()
                }
                None => {
                    if let Some(sub) = l.subs.remove(&run) {
                        LinkRt::close(&self.subs, &sub);
                    }
                    self.end(l, run, None, None, None, Some("closed"))
                }
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

    fn end(&self, l: &mut LinkRt, run: u64, exit_code: Option<f64>, signal: Option<String>, spawn_error: Option<&str>, network: Option<&str>) -> Vec<link::Effect> {
        let e = link::Ended { exit_code, signal, stderr: "", bye: None, spawn_error, network };
        l.core.end(run, &e, now_ms(), &mut self.asks())
    }

    /// A network host's run: a subscription to its map, through the link's
    /// transport; what comes back reaches `relay`.
    fn subscribe(&self, l: &mut LinkRt, r: &LinkRef, run: u64, host: &str) -> Vec<link::Effect> {
        let Some(t) = l.transport.clone() else {
            l.unstarted.insert(run);
            return self.end(l, run, None, None, Some("this machine isn't in a network"), None);
        };
        match t.start(host) {
            Ok(sub) => {
                self.subs.locked().insert(sub.clone(), SubRef { link: r.clone(), run, host: host.to_string(), last: 0, transport: t });
                l.subs.insert(run, sub);
                Vec::new()
            }
            Err(why) => {
                l.unstarted.insert(run);
                self.end(l, run, None, None, Some(&why), None)
            }
        }
    }

    /// Ends a network link's run for a reason the network gave (offline,
    /// removed, lapsed, link): the link backs off and subscribes again.
    fn end_network(&self, st: &mut HubState, id: &str, why: &str) {
        let Some((_, rt)) = st.runtimes.iter_mut().find(|(k, rt)| k == id && rt.network) else { return };
        let gen = rt.gen;
        let runs: Vec<(u64, String)> = rt.link.subs.drain().collect();
        let mut fx = Vec::new();
        for (run, sub) in runs {
            LinkRt::close(&self.subs, &sub);
            fx.extend(self.end(&mut rt.link, run, None, None, None, Some(why)));
        }
        self.apply(st, &LinkRef::Host(id.to_string(), gen), fx);
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
                return self.end(l, run, None, None, Some(&code), None);
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
            let fx = hub.end(l, run, code, signal, None, None);
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
                        // A network host is the roster's, never hosts.json's.
                        if rt.simulated || rt.network {
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
                            let page = Self::for_page(st);
                            return self.emit("hosts", page);
                        }
                        let info = Self::info_of(rt);
                        if let Some(q) = merge::qualify_event(&ename, &edata, &info, &mut MergeSchemas(&self.parts.schemas)) {
                            self.emit(&ename, q);
                        } else if let Some(note) = merge::event_note(&ename, &edata, &info) {
                            self.parts.emitter.log("info", "hosts.unknown-app", &note);
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
                    Self::finish(st, p, true);
                } else {
                    self.probe_timer(st, p, 2000, true);
                }
            }
            "offset" if probe.got.is_some() => Self::finish(st, p, true),
            "state" if matches!(data.get("state").and_then(Value::as_str), Some("offline" | "error")) => Self::finish(st, p, false),
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
                HostHub::finish(&mut st, p, ok);
            }
        }));
    }

    fn finish(st: &mut HubState, p: u64, ok: bool) {
        let Some(mut probe) = st.probes.remove(&p) else { return };
        if let Some(t) = probe.timer.take() {
            t.abort();
        }
        let info = probe.link.core.info();
        let _ = probe.link.core.stop(now_ms());
        probe.link.abandon(None);
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
        let mut v = core::view(&Self::info_of(rt), &rt.link.core.entry, &snap, rt.cached, rt.simulated, &rt.doctor);
        if rt.network {
            v["network"] = json!(NET_DRAWN);
        }
        v
    }

    /// Whether the hub draws a host: a network host only once it has a
    /// snapshot, live or kept; until then the page draws it from presence.
    fn drawn(rt: &Rt) -> bool {
        !rt.network || rt.snapshot.is_some()
    }

    /// The hosts over SSH, for /api/hosts and the doctor: the network's
    /// have the network's own.
    fn list_of(st: &HubState) -> Vec<Value> {
        st.runtimes.iter().filter(|(_, rt)| !rt.network).map(|(_, rt)| Self::view(rt)).collect()
    }

    fn for_page(st: &HubState) -> Value {
        Value::Array(st.runtimes.iter().filter(|(_, rt)| Self::drawn(rt)).map(|(_, rt)| core::for_page(&Self::view(rt))).collect())
    }

    fn hosts_changed(&self, st: &mut HubState) {
        let page = Self::for_page(st);
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
        if !self.st.locked().runtimes.iter().any(|(_, rt)| Self::drawn(rt)) {
            return local;
        }
        // The layout and names are the store's, read outside the hub's lock.
        let layout = self.parts.store.layout().unwrap_or_else(|_| json!({}));
        let ids = self.parts.store.identities().unwrap_or_else(|_| json!({}));
        let self_id = self.parts.hosts.self_entry().get("id").cloned().unwrap_or(Value::Null);
        let mut st = self.st.locked();
        if !st.runtimes.iter().any(|(_, rt)| Self::drawn(rt)) {
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
        let network: HashSet<String> = st.runtimes.iter().filter(|(_, rt)| rt.network && Self::drawn(rt)).map(|(k, _)| k.clone()).collect();
        let mut ins: Vec<merge::HostIn<'_>> = Vec::new();
        let infos: Vec<(Value, Value)> =
            st.runtimes.iter().filter(|(_, rt)| Self::drawn(rt)).map(|(_, rt)| (Self::info_of(rt), rt.snapshot.clone().unwrap_or(Value::Null))).collect();
        for ((_, rt), (info, snapshot)) in st.runtimes.iter_mut().filter(|(_, rt)| Self::drawn(rt)).zip(infos) {
            ins.push(merge::HostIn { info, snapshot, clean: Some(&mut rt.clean) });
        }
        let names = ids.get("names").cloned().unwrap_or_else(|| json!({}));
        let hidden = ids.get("hidden").cloned().unwrap_or_else(|| json!({}));
        let mut g = merge::merge_graph(&local, &mut ins, &layout, now_ms(), &self.parts.windows, &self_id, &names, &hidden, &alias_map, &mut MergeSchemas(&self.parts.schemas));
        // What a network host's relay sent says so, for the page: a machine
        // in your network, whose sessions take your messages.
        if !network.is_empty() {
            for key in ["nodes", "hosts"] {
                for n in g.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten() {
                    let host = n.get("host").or_else(|| n.get("id")).and_then(Value::as_str).unwrap_or("");
                    if network.contains(host) {
                        n["network"] = json!(NET_DRAWN);
                    }
                }
            }
        }
        g
    }

    fn pairs(st: &HubState) -> Vec<(Value, Value)> {
        st.runtimes.iter().filter(|(_, rt)| Self::drawn(rt)).map(|(_, rt)| (Self::info_of(rt), rt.snapshot.clone().unwrap_or(Value::Null))).collect()
    }

    fn merged_live(&self, local: Value) -> Value {
        let st = self.st.locked();
        if !st.runtimes.iter().any(|(_, rt)| Self::drawn(rt)) {
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
        self.add_one(st, entry, simulated, None, true)
    }

    /// A host's runtime: its cached snapshot, if any, and its link, started
    /// now when `run`. A network host's link goes by `transport`.
    fn add_one(&self, st: &mut HubState, entry: Value, simulated: bool, transport: Option<Arc<dyn Transport>>, run: bool) -> u64 {
        let id = s(entry.get("id")).unwrap_or_default();
        let cached = s(entry.get("key")).and_then(|k| cache::read_cache(Some(&self.parts.cache_dir), &k));
        st.next += 1;
        let gen = st.next;
        let mut link = self.new_link(entry.clone(), false);
        let network = transport.is_some();
        link.transport = transport;
        let mut rt = Rt {
            gen,
            link,
            simulated,
            network,
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
        if run {
            self.run_one(st, &id, gen);
        }
        gen
    }

    fn run_one(&self, st: &mut HubState, id: &str, gen: u64) {
        let r = LinkRef::Host(id.to_string(), gen);
        let fx = match Self::rt_mut(st, id, gen) {
            Some(rt) => rt.link.core.start(now_ms(), &mut self.asks()),
            None => Vec::new(),
        };
        self.apply(st, &r, fx);
    }

    /// A network host's link stops until `run_one`, its subscription ended:
    /// it keeps its snapshot, drawn dimmed.
    fn pause_one(&self, st: &mut HubState, id: &str, gen: u64, why: &str) {
        let fx = match Self::rt_mut(st, id, gen) {
            Some(rt) => rt.link.core.pause(why, now_ms()),
            None => return,
        };
        self.apply(st, &LinkRef::Host(id.to_string(), gen), fx);
    }

    // ---------- the network ----------

    /// The roster's hosts as runtimes: one for each host that isn't over
    /// SSH already, started while a console is open; and the ones that
    /// left the roster gone, with their kept snapshots. A host offline in
    /// presence has its subscription ended, and tries again as it comes
    /// back.
    fn sync_roster(&self) {
        let Some((net, transport)) = self.net.locked().as_ref().map(|n| (n.net.clone(), n.transport.clone())) else { return };
        let me = net.me();
        let windowed = net.is_window_host();
        let hosts = if me.is_some() { net.hosts() } else { Vec::new() };
        let mut st = self.st.locked();
        let console = st.console && windowed;
        let mut changed = false;
        // Gone from the roster (or this machine left its network).
        let gone: Vec<String> = st.runtimes.iter().filter(|(k, rt)| rt.network && !rt.simulated && !hosts.iter().any(|h| h.id == **k)).map(|(k, _)| k.clone()).collect();
        for id in gone {
            if let Some(mut rt) = Self::take(&mut st, &id) {
                self.stop_one(&mut rt, &id, false);
                self.forget_cache(&st, &rt);
                changed = true;
            }
        }
        for h in &hosts {
            let have = st.runtimes.iter().find(|(k, _)| *k == h.id).map(|(_, rt)| (rt.network, rt.simulated, rt.gen, rt.link.core.stopped));
            match have {
                // Over SSH already, or made up by XenoNet Dev: shown once.
                Some((false, ..) | (true, true, ..)) => {}
                None => {
                    let key = me.as_ref().map(|m| net_key(&m.network, &h.id));
                    let entry = json!({ "id": h.id, "transport": "network", "key": key });
                    let gen = self.add_one(&mut st, entry, false, Some(transport.clone()), console && h.online);
                    if !(console && h.online) {
                        self.pause_one(&mut st, &h.id, gen, &paused(&h.id, console, windowed));
                    }
                    changed = true;
                }
                Some((true, false, gen, stopped)) => {
                    if !(console && h.online) {
                        if !stopped {
                            self.pause_one(&mut st, &h.id, gen, &paused(&h.id, console, windowed));
                        }
                    } else if stopped {
                        self.run_one(&mut st, &h.id, gen);
                    }
                }
            }
        }
        if changed {
            self.hosts_changed(&mut st);
        }
    }

    /// Every subscription renewed, and one gone quiet for longer than a
    /// host keeps it ended, so its link subscribes again.
    fn renew_all(&self) {
        let now = now_ms();
        let quiet: Vec<String> = {
            let st = self.st.locked();
            st.runtimes
                .iter()
                .filter(|(_, rt)| {
                    rt.network && !rt.link.subs.is_empty() && rt.link.core.state != "connecting" && rt.link.core.last_heard.is_none_or(|h| now - h > self.parts.relay.lapse_ms)
                })
                .map(|(k, _)| k.clone())
                .collect()
        };
        if !quiet.is_empty() {
            let mut st = self.st.locked();
            for id in quiet {
                self.end_network(&mut st, &id, "lapsed");
            }
        }
        let subs: Vec<(String, String, Arc<dyn Transport>)> = self.subs.locked().iter().map(|(sub, s)| (sub.clone(), s.host.clone(), s.transport.clone())).collect();
        for (sub, host, t) in subs {
            t.renew(&host, &sub, false);
        }
    }

    /// A body back from a host: its frames to the subscription's link, as
    /// lines from ssh would go, in order. A hole asks for a fresh snapshot;
    /// a bye ends the run.
    fn relay_in(&self, from: &str, body: &Value) {
        let Ok(b) = wire::parse_batch(body) else { return };
        let mut st = self.st.locked();
        let (r, run, fresh, transport) = {
            let mut subs = self.subs.locked();
            let Some(sref) = subs.get_mut(&b.sub).filter(|s| s.host == from) else { return };
            let fresh = match wire::order(sref.last, b.seq) {
                Order::Old => return,
                Order::Next => false,
                Order::Hole => true,
            };
            sref.last = b.seq;
            (sref.link.clone(), sref.run, fresh, sref.transport.clone())
        };
        if fresh {
            transport.renew(from, &b.sub, true);
        }
        let mut bye = false;
        for f in b.frames {
            let now = now_ms();
            let Some(l) = Self::link_mut(&mut st, &r) else { return };
            bye |= f.get("t").and_then(Value::as_str) == Some("bye");
            let line = frames::encode(&f);
            let fx = l.core.line(run, line.trim_end(), now, &mut self.asks());
            self.apply(&mut st, &r, fx);
        }
        if bye {
            let Some(l) = Self::link_mut(&mut st, &r) else { return };
            if let Some(sub) = l.subs.remove(&run) {
                self.subs.locked().remove(&sub);
            }
            let fx = self.end(l, run, None, None, None, None);
            self.apply(&mut st, &r, fx);
        }
    }

    /// Whether a console is open here, from Desktop's window and the pages
    /// reading the event stream: shown, or a page besides a hidden
    /// window's own, or with no window, any page.
    pub fn console_open(window: Option<&str>, pages: usize) -> bool {
        match window {
            Some("shown") => true,
            Some("hidden") => pages > 1,
            _ => pages > 0,
        }
    }

    /// The network, once the service has it: the hub hears the relay's
    /// answers, the relay serves this host's map to the window hosts that
    /// subscribe, and the roster is looked at every few seconds.
    pub fn plug_net(&self, plug: HubNet) {
        let log = {
            let e = self.parts.emitter.clone();
            Arc::new(move |code: &str, words: &str| e.log("info", code, words)) as xenonet_net::sink::LogFn
        };
        let server = RelayServer::new(serve::Parts {
            net: plug.net.clone(),
            events: plug.events.clone(),
            api: plug.api.clone(),
            home: plug.home.clone(),
            version: plug.version.clone(),
            platform: plug.platform.clone(),
            demo: plug.demo,
            aliases: xenonet_state::identities::parse_ssh_aliases,
            log,
            timing: self.parts.relay,
        });
        server.start();
        plug.sinks.set_relay("serve", server.clone());
        if let Some(me) = self.me.upgrade() {
            plug.sinks.set_relay("hear", me);
        }
        let transport: Arc<dyn Transport> = Arc::new(NetTransport { net: plug.net.clone() });
        let old = self.net.locked().replace(NetSide { net: plug.net.clone(), transport, server, plug });
        if let Some(o) = old {
            o.server.stop();
        }
        let mut tasks = self.tasks.locked();
        if tasks.is_empty() {
            let (roster_ms, renew_ms) = (self.parts.relay.roster_ms, self.parts.relay.renew_ms as u64);
            let me = self.me.clone();
            tasks.push(tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_millis(roster_ms));
                loop {
                    every.tick().await;
                    let Some(hub) = me.upgrade() else { return };
                    hub.sync_roster();
                }
            }));
            let me = self.me.clone();
            tasks.push(tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_millis(renew_ms));
                every.tick().await;
                loop {
                    every.tick().await;
                    let Some(hub) = me.upgrade() else { return };
                    hub.renew_all();
                }
            }));
        }
    }

    /// The relay serving this host's map, for the tests.
    pub fn relay_server(&self) -> Option<Arc<RelayServer>> {
        self.net.locked().as_ref().map(|n| n.server.clone())
    }

    /// A network host whose link goes by `transport`, started now: for the
    /// tests and the fuzz target, which carry its frames themselves.
    pub fn add_network(&self, id: &str, network: &str, transport: Arc<dyn Transport>) {
        let mut st = self.st.locked();
        let entry = json!({ "id": id, "transport": "network", "key": net_key(network, id) });
        self.add_one(&mut st, entry, true, Some(transport), true);
    }

    /// Stops renewing its subscriptions and looking at the roster, as a
    /// console that vanished without a word would: for the tests of a
    /// subscription that lapses.
    pub fn stop_renewing(&self) {
        for t in self.tasks.locked().drain(..) {
            t.abort();
        }
    }

    /// XenoNet Dev's net-relay: a made-up network host, `id`, drawn from a
    /// relay of this machine's own map served as if it were that host's,
    /// over a network inside this process. `off` takes it away.
    fn simulate_net(&self, id: Option<&str>, off: bool) -> Value {
        let id = id.filter(|i| xenonet_base::ids::is_host_id(i)).unwrap_or("laptop").to_string();
        let mut st = self.st.locked();
        let had = st.runtimes.iter().position(|(k, rt)| *k == id && rt.network && rt.simulated);
        if let Some(at) = had {
            let (_, mut rt) = st.runtimes.remove(at);
            self.stop_one(&mut rt, &id, false);
        }
        if off {
            self.hosts_changed(&mut st);
            return json!({ "ok": true, "kind": "net-relay", "id": id, "off": true });
        }
        let Some(side) = self.net.locked().as_ref().map(|n| (n.plug.events.clone(), n.plug.api.clone(), n.plug.home.clone(), n.plug.version.clone(), n.plug.platform.clone()))
        else {
            return json!({ "error": "the network isn't there in this XenoNet" });
        };
        let (events, api, home, version, platform) = side;
        let Some(me) = self.me.upgrade() else { return json!({ "error": "the hub stopped" }) };
        let hub: Weak<dyn RelaySink> = Arc::downgrade(&(me as Arc<dyn RelaySink>));
        let console = "studio".to_string();
        let net: Arc<dyn Net> = Arc::new(LoopNet { host: id.clone(), console: console.clone(), hub });
        let server = RelayServer::new(serve::Parts {
            net,
            events,
            api,
            home,
            version,
            platform,
            demo: true,
            aliases: xenonet_state::identities::parse_ssh_aliases,
            log: Arc::new(|_: &str, _: &str| {}),
            timing: self.parts.relay,
        });
        let transport: Arc<dyn Transport> = Arc::new(LoopTransport { relay: server, console });
        let entry = json!({ "id": id, "transport": "network", "key": net_key("net_00000000000000000000000000000000", &id) });
        self.add_one(&mut st, entry, true, Some(transport), true);
        self.hosts_changed(&mut st);
        json!({ "ok": true, "kind": "net-relay", "id": id })
    }
}

impl RelaySink for HostHub {
    fn relay(&self, incoming: Incoming) {
        if wire::is_ask(&incoming.body) {
            return;
        }
        self.relay_in(&incoming.from, &incoming.body);
    }

    fn removed(&self, host: &str) {
        let mut st = self.st.locked();
        if st.runtimes.iter().any(|(k, rt)| k == host && rt.network && !rt.simulated) {
            self.end_network(&mut st, host, "removed");
        }
        drop(st);
        self.sync_roster();
    }

    fn linked(&self, up: bool) {
        if up {
            return;
        }
        let mut st = self.st.locked();
        let ids: Vec<String> = st.runtimes.iter().filter(|(_, rt)| rt.network && !rt.simulated && !rt.link.subs.is_empty()).map(|(k, _)| k.clone()).collect();
        for id in ids {
            self.end_network(&mut st, &id, "link");
        }
    }

    /// Frames for this host were lost: a fresh snapshot of every map.
    fn gap(&self) {
        let subs: Vec<(String, String, Arc<dyn Transport>)> = self.subs.locked().iter().map(|(sub, s)| (sub.clone(), s.host.clone(), s.transport.clone())).collect();
        for (sub, host, t) in subs {
            t.renew(&host, &sub, true);
        }
    }
}

impl HostHub {
    fn stop_one(&self, rt: &mut Rt, id: &str, keep: bool) {
        // Its listeners first, as removeAllListeners: what it says now
        // reaches no one.
        let _ = rt.link.core.stop(now_ms());
        rt.link.abandon(Some(&self.subs));
        if keep && rt.cache_timer.is_some() {
            self.save(rt, id);
        }
        if let Some(t) = rt.cache_timer.take() {
            t.abort();
        }
    }

    fn take(st: &mut HubState, id: &str) -> Option<Rt> {
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
            // Over the network, a collision's diffs and an MCP server's
            // details come later.
            let route = path.split('?').next().unwrap_or("");
            if rt.network && (route == "/api/collision" || route == "/api/mcp") {
                return Err(fail(format!("collisions and MCP servers' details from another machine in the network come later: open them on {host}"), 403));
            }
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
        if let Some(mut old) = HostHub::take(&mut st, &id) {
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
            let Some(mut rt) = HostHub::take(&mut st, id) else { return Err(fail(format!("there's no host called {id}"), 404)) };
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
        if kind == "net-relay" {
            return Ok(Some(hub.simulate_net(arg("id").or(arg("a")).and_then(Value::as_str), arg("off") == Some(&json!(true)))));
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

    fn ssh(&self) -> bool {
        self.0.parts.ssh
    }

    fn plug_net(&self, plug: HubNet) {
        self.0.plug_net(plug);
    }

    fn console(&self, window: Option<&str>, pages: usize) {
        let hub = &self.0;
        let open = HostHub::console_open(window, pages);
        {
            let mut st = hub.st.locked();
            if st.console == open {
                return;
            }
            st.console = open;
        }
        hub.sync_roster();
    }

    fn start(&self) {
        let hub = &self.0;
        if !hub.parts.ssh {
            return;
        }
        if let Err(e) = hub.parts.hosts.write_ssh_config() {
            hub.parts.emitter.log("warn", "hosts.ssh-config", &format!("couldn't write {}: {}", hub.parts.hosts.ssh_config_file(), e.message));
        }
        let entries = hub.parts.hosts.list();
        let mut st = hub.st.locked();
        for entry in entries {
            // A network host shown already gives way to its hosts.json entry.
            if let Some(id) = s(entry.get("id")) {
                if let Some(mut old) = HostHub::take(&mut st, &id) {
                    hub.stop_one(&mut old, &id, false);
                }
            }
            hub.start_one(&mut st, entry, false);
        }
        if !st.runtimes.is_empty() {
            hub.hosts_changed(&mut st);
        }
    }

    fn stop(&self) {
        let hub = &self.0;
        for t in hub.tasks.locked().drain(..) {
            t.abort();
        }
        if let Some(n) = hub.net.locked().as_ref() {
            n.server.stop();
        }
        let mut st = hub.st.locked();
        for t in [st.soon_graph.take(), st.soon_live.take()].into_iter().flatten() {
            t.abort();
        }
        let mut all = std::mem::take(&mut st.runtimes);
        for (id, rt) in &mut all {
            hub.stop_one(rt, id, true);
        }
        for (_, mut p) in st.probes.drain() {
            p.link.abandon(None);
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
            bin: "/no/such/xenonet".into(),
            ssh_bin: vec!["ssh".into()],
            ssh: true,
            relay: serve::Timing::default(),
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

    const NETWORK: &str = "net_0123456789abcdef0123456789abcdef";

    /// A transport that keeps what the hub asked: (op, host, sub, text).
    #[derive(Default)]
    struct Fake {
        asked: Mutex<Vec<(String, String, String, String)>>,
    }

    impl Transport for Fake {
        fn start(&self, host: &str) -> Result<String, String> {
            let sub = super::super::transport::new_sub()?;
            self.asked.locked().push(("open".into(), host.into(), sub.clone(), String::new()));
            Ok(sub)
        }
        fn write(&self, host: &str, sub: &str, text: &str) {
            self.asked.locked().push(("write".into(), host.into(), sub.into(), text.into()));
        }
        fn renew(&self, host: &str, sub: &str, fresh: bool) {
            self.asked.locked().push((if fresh { "fresh" } else { "renew" }.into(), host.into(), sub.into(), String::new()));
        }
        fn end(&self, host: &str, sub: &str) {
            self.asked.locked().push(("close".into(), host.into(), sub.into(), String::new()));
        }
    }

    impl Fake {
        fn ops(&self) -> Vec<String> {
            self.asked.locked().iter().map(|a| a.0.clone()).collect()
        }
        fn sub(&self) -> String {
            self.asked.locked().iter().rev().find(|a| a.0 == "open").map(|a| a.2.clone()).unwrap()
        }
    }

    fn hello(host: &str) -> Value {
        json!({ "t": "hello", "schema": "host/hello/v1", "protocol": 1, "version": "0.0.0-test", "host": { "id": host, "key": net_key(NETWORK, host) }, "platform": "linux", "demo": false, "now": now_ms(), "sinceMs": 604_800_000, "detail": true })
    }

    fn snapshot() -> Value {
        json!({ "t": "snapshot", "at": now_ms(), "graph": { "nodes": [
            { "id": "core", "kind": "core", "name": "laptop", "path": "~" },
            { "id": "claude:abcd-0001", "kind": "session", "app": "claude", "rawId": "abcd-0001", "title": "Fix the login form", "preview": null, "cwd": "~/atlas", "createdAt": now_ms(), "updatedAt": now_ms(), "archived": false, "model": null, "branch": null, "parentId": null, "parentKind": null, "activity": "active", "fx": null, "fy": null, "fz": null, "pinned": false, "actions": [] }
        ], "links": [{ "source": "core", "target": "claude:abcd-0001", "kind": "session" }] }, "live": { "sessions": [] }, "engine": null })
    }

    fn from(host: &str, sub: &str, seq: u64, frames: &[Value]) -> Incoming {
        Incoming { from: host.into(), kind: xenonet_base::network::Kind::Relay, body: wire::batch(sub, seq, frames), at: now_ms(), from_window_host: true }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_network_host_over_a_fake_transport() {
        let (h, dir) = hub("net");
        let core = HubCore(h.clone());
        let fake = Arc::new(Fake::default());
        {
            let mut st = h.st.locked();
            h.add_one(&mut st, json!({ "id": "laptop", "transport": "network", "key": net_key(NETWORK, "laptop") }), false, Some(fake.clone()), true);
        }
        assert_eq!(fake.ops(), ["open"]);
        let sub = fake.sub();
        assert!(core.has("laptop"));
        assert_eq!(core.list().unwrap(), json!([]), "the network's hosts aren't the doctor's or /api/hosts'");
        assert_eq!(core.graph(json!({ "nodes": [] })), json!({ "nodes": [] }), "no snapshot yet: the page draws it from presence");
        // Its hello and snapshot: drawn from them, as a network host.
        h.relay(from("laptop", &sub, 1, &[hello("laptop"), snapshot()]));
        assert_eq!(h.st.locked().runtimes[0].1.link.core.state, "live");
        let g = core.graph(json!({ "nodes": [], "links": [] }));
        let session = g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == "@laptop:claude:abcd-0001").cloned().unwrap_or_else(|| panic!("{g}"));
        assert_eq!((session["host"].clone(), session["network"].clone()), (json!("laptop"), json!("relay")), "{session}");
        assert_eq!(g["hosts"][0]["network"], "relay");
        // Another host's frames under laptop's subscription go nowhere.
        h.relay(from("desk", &sub, 2, &[json!({ "t": "bye", "code": "relay" })]));
        assert_eq!(h.st.locked().runtimes[0].1.link.core.state, "live");
        // A read goes as the link writes it, and its answer comes back.
        let read = tokio::spawn({
            let core = HubCore(h.clone());
            async move { core.get("laptop", "/api/session/claude:abcd-0001/transcript").await }
        });
        let wrote = loop {
            if let Some(w) = fake.asked.locked().iter().find(|a| a.0 == "write" && a.3.contains("transcript")).cloned() {
                break w;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        let id = serde_json::from_str::<Value>(&wrote.3).unwrap()["id"].as_u64().unwrap();
        h.relay(from("laptop", &sub, 2, &[json!({ "t": "res", "id": id, "status": 200, "body": { "entries": [] } })]));
        let answer = read.await.unwrap().unwrap();
        assert_eq!((answer.status, answer.body), (200, json!({ "entries": [] })));
        // Collisions and MCP servers' details over the network come later.
        for path in ["/api/mcp?server=agent-wiki", "/api/collision?a=claude:a&b=claude:b"] {
            assert_eq!(core.get("laptop", path).await.err().unwrap().status, Some(403), "{path}");
        }
        // A hole asks for a fresh snapshot; an old body is dropped.
        h.relay(from("laptop", &sub, 4, &[json!({ "t": "ping", "now": now_ms() })]));
        assert_eq!(fake.ops().last().unwrap(), "fresh");
        let before = fake.ops().len();
        h.relay(from("laptop", &sub, 3, &[json!({ "t": "bye", "code": "relay" })]));
        assert_eq!((fake.ops().len(), h.st.locked().runtimes[0].1.link.core.state), (before, "live"), "an old body is dropped");
        // Renewed while it runs; a gap asks every map afresh.
        h.renew_all();
        h.gap();
        assert_eq!(fake.ops()[fake.ops().len() - 2..], ["renew", "fresh"]);
        // A bye ends the run, with why, and it tries again.
        h.relay(from("laptop", &sub, 5, &[json!({ "t": "bye", "code": "not-window", "message": "x" })]));
        let info = h.st.locked().runtimes[0].1.link.core.info();
        assert_eq!((info["state"].clone(), info["error"]["code"].clone()), (json!("error"), json!("not-window")), "{info}");
        // Removed from the roster: the run ends as removed, and its kept
        // snapshot still draws it, dimmed, until the roster goes over.
        core.reconnect("laptop").unwrap();
        assert_ne!(fake.sub(), sub, "a new subscription");
        h.relay(from("laptop", &fake.sub(), 1, &[hello("laptop"), snapshot()]));
        h.removed("laptop");
        let info = h.st.locked().runtimes[0].1.link.core.info();
        assert_eq!(info["error"]["code"], "removed", "{info}");
        assert_eq!(fake.ops().last().unwrap(), "close");
        let g = core.graph(json!({ "nodes": [], "links": [] }));
        assert!(g["nodes"].as_array().unwrap().iter().any(|n| n["id"] == "@laptop:claude:abcd-0001" && n["stale"] == true), "{g}");
        core.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// studio's network, as its hub sees it: laptop online, a window host;
    /// what it sends is kept.
    struct Roster {
        hosts: Mutex<Vec<xenonet_base::network::Host>>,
        sent: Mutex<Vec<xenonet_base::network::Outgoing>>,
    }

    impl Net for Roster {
        fn me(&self) -> Option<xenonet_base::network::Me> {
            Some(xenonet_base::network::Me { network: NETWORK.into(), name: "home".into(), host: "studio".into() })
        }
        fn hosts(&self) -> Vec<xenonet_base::network::Host> {
            self.hosts.locked().clone()
        }
        fn send(&self, out: xenonet_base::network::Outgoing) -> Result<(), String> {
            self.sent.locked().push(out);
            Ok(())
        }
        fn is_window_host(&self) -> bool {
            true
        }
    }

    struct NoApi;

    #[async_trait]
    impl xenonet_service::cores::SelfApi for NoApi {
        async fn get(&self, _path: &str) -> Result<(u16, Value), (String, u16)> {
            Err(("nothing here".into(), 502))
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_rosters_hosts_subscribe_only_while_a_console_is_open() {
        let (h, dir) = hub("roster");
        let core = HubCore(h.clone());
        let net =
            Arc::new(Roster { hosts: Mutex::new(vec![xenonet_base::network::Host { id: "laptop".into(), window: true, online: true, seen: None }]), sent: Mutex::new(Vec::new()) });
        let sinks = Arc::new(xenonet_net::sink::SinkSlot::new(Arc::new(xenonet_net::sink::LogSink::new(Arc::new(|_: &str, _: &str| {})))));
        core.plug_net(HubNet {
            net: net.clone(),
            sinks,
            events: Arc::new(xenonet_service::events::Events::new()),
            api: Arc::new(NoApi),
            home: "/home/nova".into(),
            version: "0.0.0-test".into(),
            platform: "linux".into(),
            demo: false,
        });
        let opens = || net.sent.locked().iter().filter(|o| o.body["op"] == "open").count();
        h.sync_roster();
        assert!(core.has("laptop"), "in the roster, so on the hub");
        assert_eq!(opens(), 0, "no console: no subscription");
        assert!(!HostHub::console_open(Some("hidden"), 1), "a hidden window's own page isn't a console");
        assert!(HostHub::console_open(None, 1) && HostHub::console_open(Some("hidden"), 2), "a browser page is");
        core.console(Some("shown"), 1);
        assert_eq!(opens(), 1, "the console opened: laptop's map");
        // laptop goes offline in presence: its subscription ends, and comes
        // back with it.
        net.hosts.locked()[0].online = false;
        h.sync_roster();
        assert_eq!(net.sent.locked().last().unwrap().body["op"], "close");
        net.hosts.locked()[0].online = true;
        h.sync_roster();
        assert_eq!(opens(), 2);
        core.console(Some("closed"), 0);
        assert_eq!(net.sent.locked().last().unwrap().body["op"], "close", "the console closed: so did the subscription");
        // Gone from the roster: gone from the hub.
        net.hosts.locked().clear();
        h.sync_roster();
        assert!(!core.has("laptop"));
        core.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
