//! This host's map for a console on another host in the network
//! (docs/design/network.md, section 7; `network/relay/v1`): the relay
//! `xenonet relay` runs for a console over ssh, run here in the service for
//! each window host that subscribes, with its frames sealed between the two
//! hosts through the gateway.
//!
//! - It serves only a window host in the roster, one subscription for each,
//!   while it's renewed (every 20 s; it lapses after 60 s without one), and
//!   ends it when the host goes offline, is removed or stops being a window
//!   host, or this host's link to the gateway goes down.
//! - The shaping is the SSH relay's (`xenonet_hosts::relay::Producer`): the
//!   home folder as ~, other hosts' parts left out, and detail unless this
//!   host shares titles only.
//! - It follows this machine's own view of its events in-process (the
//!   stream `?hosts=0` gives), and reads its own API as a local client
//!   with no key (`SelfApi`), so the allowlist and the routes' own checks
//!   both apply.
//! - Its gets are the allowlist's, as GET. A host that shares titles only
//!   answers a transcript with 403 and serves stats and the map, the live
//!   state and the engine's, with detail off. Collisions and MCP servers'
//!   details over the network come later: 403 for now.
//! - What it sends, for each subscription: at most one body every half
//!   second (longer with more subscriptions, so the link's 120 relay
//!   frames a minute go round), the map at most once a second and the live
//!   state every half second within it, other events gathered in order, a
//!   ping when it has been quiet for 15 s.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use xenonet_base::network::{Incoming, Kind, Net, Outgoing, RelaySink, To};
use xenonet_hosts::frames::{encode, MAX_DEPTH, PROTOCOL};
use xenonet_hosts::network::{self as wire, Op};
use xenonet_hosts::relay::{self as core, Producer, Shape};
use xenonet_net::sink::LogFn;
use xenonet_service::cores::SelfApi;
use xenonet_service::events::Events;

use crate::fx::{now_ms, Locked};

/// A body's frames, at most this many bytes of JSON: what's left of
/// `xenonet_net::node::RELAY_BODY` once the body is wrapped.
const BODY_BYTES: usize = (8 << 20) - (64 << 10);
/// Other events waiting for the next body, at most; past this a fresh
/// snapshot goes in their place.
const WAITING_BYTES: usize = 4 << 20;
const WAITING_FRAMES: usize = 800;

/// The relay's timings, on both sides: the design's, or shorter ones for
/// tests.
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    pub frame_ms: f64,
    pub graph_ms: f64,
    pub live_ms: f64,
    pub lapse_ms: f64,
    pub ping_ms: f64,
    /// How often subscriptions are looked over: lapsed, offline, removed.
    pub watch_ms: u64,
    pub get_ms: f64,
    /// How often a console renews its subscriptions, and looks at the
    /// roster for hosts that came or went.
    pub renew_ms: f64,
    pub roster_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            frame_ms: wire::FRAME_MS,
            graph_ms: wire::GRAPH_MS,
            live_ms: wire::LIVE_MS,
            lapse_ms: wire::LAPSE_MS,
            ping_ms: core::PING_MS,
            watch_ms: 5000,
            get_ms: core::GET_TIMEOUT_MS,
            renew_ms: wire::RENEW_MS,
            roster_ms: 3000,
        }
    }
}

/// What a relay needs of this machine.
pub struct Parts {
    pub net: Arc<dyn Net>,
    pub events: Arc<Events>,
    pub api: Arc<dyn SelfApi>,
    pub home: String,
    pub version: String,
    pub platform: String,
    pub demo: bool,
    pub aliases: core::AliasParse,
    pub log: LogFn,
    pub timing: Timing,
}

/// What a subscription's task is asked.
enum Cmd {
    Get { id: u64, path: String },
    Time { id: u64 },
    Fresh,
}

struct Sub {
    id: String,
    renewed: f64,
    tx: mpsc::UnboundedSender<Cmd>,
    task: JoinHandle<()>,
}

/// The relays for the consoles that subscribed to this host's map.
pub struct RelayServer {
    parts: Parts,
    /// One for each subscribing host, by its id.
    subs: Mutex<HashMap<String, Sub>>,
    watch: Mutex<Option<JoinHandle<()>>>,
    me: Weak<RelayServer>,
}

/// A host's hello key for its relay over the network: made from the
/// network's id and the host's, so the console knows it before the hello
/// and keeps the host's last snapshot under it.
pub fn net_key(network: &str, host: &str) -> String {
    let digest = xenonet_net::codec::sha256(format!("xenonet-net-relay\n{network}\n{host}").as_bytes());
    xenonet_base::jsstr::hex(&digest[..16])
}

impl RelayServer {
    pub fn new(parts: Parts) -> Arc<RelayServer> {
        Arc::new_cyclic(|me| RelayServer { parts, subs: Mutex::new(HashMap::new()), watch: Mutex::new(None), me: me.clone() })
    }

    fn log(&self, code: &str, words: &str) {
        (self.parts.log)(code, words);
    }

    /// Looks over the subscriptions every so often: one not renewed in
    /// time lapses, and one whose host is offline, removed or no longer a
    /// window host ends.
    pub fn start(&self) {
        let mut watch = self.watch.locked();
        if watch.is_some() {
            return;
        }
        let (me, every) = (self.me.clone(), Duration::from_millis(self.parts.timing.watch_ms));
        *watch = Some(tokio::spawn(async move {
            loop {
                tokio::time::sleep(every).await;
                let Some(s) = me.upgrade() else { return };
                s.look_over();
            }
        }));
    }

    pub fn stop(&self) {
        if let Some(t) = self.watch.locked().take() {
            t.abort();
        }
        for (_, s) in self.subs.locked().drain() {
            s.task.abort();
        }
    }

    /// The hosts subscribed now, and their subscriptions.
    pub fn subscribed(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self.subs.locked().iter().map(|(h, s)| (h.clone(), s.id.clone())).collect();
        v.sort();
        v
    }

    fn look_over(&self) {
        let now = now_ms();
        let hosts = self.parts.net.hosts();
        let ended: Vec<(String, &'static str)> = self
            .subs
            .locked()
            .iter()
            .filter_map(|(h, s)| {
                let row = hosts.iter().find(|x| x.id == *h);
                let why = match row {
                    None => "removed from the network",
                    Some(r) if !r.online => "offline",
                    Some(r) if !r.window => "no longer a window host",
                    Some(_) if now - s.renewed > self.parts.timing.lapse_ms => "lapsed",
                    Some(_) => return None,
                };
                Some((h.clone(), why))
            })
            .collect();
        for (h, why) in ended {
            if why == "no longer a window host" {
                if let Some(sub) = self.subs.locked().get(&h).map(|s| s.id.clone()) {
                    self.refuse(&h, &sub);
                }
            }
            self.end(&h, why);
        }
    }

    fn end(&self, host: &str, why: &str) {
        if let Some(s) = self.subs.locked().remove(host) {
            s.task.abort();
            self.log("net.relay", &format!("{host}'s view of this machine's map ended: {why}"));
        }
    }

    /// The bye a host that isn't a window host gets.
    fn refuse(&self, host: &str, sub: &str) {
        let bye = json!({ "t": "bye", "code": "not-window", "message": "this machine shows its map only to a window host's console" });
        let _ = self.parts.net.send(Outgoing { to: To::Host(host.into()), kind: Kind::Relay, body: wire::batch(sub, 1, &[bye]) });
    }

    fn open(&self, host: &str, sub: &str) {
        let (tx, rx) = mpsc::unbounded_channel();
        let session = Session::new(self, host, sub);
        let task = tokio::spawn(session.run(self.me.clone(), rx));
        let old = self.subs.locked().insert(host.to_string(), Sub { id: sub.to_string(), renewed: now_ms(), tx, task });
        if let Some(o) = old {
            o.task.abort();
        }
        self.log("net.relay", &format!("{host} watches this machine's map"));
    }

    fn ask(&self, host: &str, sub: &str, cmd: Option<Cmd>) {
        let mut subs = self.subs.locked();
        let Some(s) = subs.get_mut(host).filter(|s| s.id == sub) else { return };
        s.renewed = now_ms();
        if let Some(c) = cmd {
            let _ = s.tx.send(c);
        }
    }
}

impl RelaySink for RelayServer {
    fn relay(&self, incoming: Incoming) {
        if !wire::is_ask(&incoming.body) {
            return;
        }
        let ask = match wire::parse_ask(&incoming.body) {
            Ok(a) => a,
            Err(why) => {
                self.log("net.relay", &format!("an ask from {} was refused: {why}", incoming.from));
                return;
            }
        };
        let host = incoming.from.as_str();
        // Only a window host's console sees this machine's map.
        if !incoming.from_window_host {
            if ask.op == Op::Open {
                self.refuse(host, &ask.sub);
            }
            return;
        }
        match ask.op {
            Op::Open => self.open(host, &ask.sub),
            Op::Get { id, path } => self.ask(host, &ask.sub, Some(Cmd::Get { id, path })),
            Op::Time { id } => self.ask(host, &ask.sub, Some(Cmd::Time { id })),
            Op::Renew { fresh } => self.ask(host, &ask.sub, fresh.then_some(Cmd::Fresh)),
            Op::Close => {
                if self.subs.locked().get(host).is_some_and(|s| s.id == ask.sub) {
                    self.end(host, "closed");
                }
            }
        }
    }

    fn removed(&self, host: &str) {
        self.end(host, "removed from the network");
    }

    fn linked(&self, up: bool) {
        if !up {
            let hosts: Vec<String> = self.subs.locked().keys().cloned().collect();
            for h in hosts {
                self.end(&h, "this machine's link to the gateway went down");
            }
        }
    }
}

// ---------- one subscription ----------

/// A read's answer, back to its subscription's task.
enum Read {
    Get { id: u64, shape: Shape, answer: Result<(u16, Value), (String, u16)> },
    Snapshot(Result<[(u16, Value); 3], String>),
}

/// One console's view of this host's map: what it shapes and what waits
/// for its next body.
struct Session {
    host: String,
    sub: String,
    net: Arc<dyn Net>,
    api: Arc<dyn SelfApi>,
    events: Arc<Events>,
    p: Producer,
    titles_only: bool,
    hello: Value,
    timing: Timing,
    seq: u64,
    frames: Vec<Value>,
    waiting_bytes: usize,
    graph: Option<Value>,
    live: Option<Value>,
    last_graph: f64,
    last_live: f64,
    last_sent: f64,
    /// Events were dropped while too many waited: a fresh snapshot is owed.
    gap: bool,
    ready: bool,
    inflight: usize,
    log: LogFn,
}

impl Session {
    fn new(s: &RelayServer, host: &str, sub: &str) -> Session {
        let p = &s.parts;
        let titles_only = p.net.titles_only();
        let me = p.net.me();
        let id = me.as_ref().map_or(Value::Null, |m| json!(m.host));
        let key = me.as_ref().map(|m| net_key(&m.network, &m.host)).unwrap_or_default();
        let hello = json!({
            "t": "hello", "schema": "host/hello/v1", "protocol": PROTOCOL, "version": p.version,
            "host": { "id": id, "key": key }, "platform": p.platform, "demo": p.demo, "now": xenonet_base::json::number(now_ms()),
            "sinceMs": xenonet_base::json::number(core::DEFAULT_SINCE_MS), "detail": !titles_only,
        });
        Session {
            host: host.to_string(),
            sub: sub.to_string(),
            net: p.net.clone(),
            api: p.api.clone(),
            events: p.events.clone(),
            p: Producer::new(Some(&p.home), core::DEFAULT_SINCE_MS, !titles_only, p.aliases),
            titles_only,
            hello,
            timing: p.timing,
            seq: 0,
            frames: Vec::new(),
            waiting_bytes: 0,
            graph: None,
            live: None,
            last_graph: 0.0,
            last_live: 0.0,
            last_sent: 0.0,
            gap: false,
            ready: false,
            inflight: 0,
            log: p.log.clone(),
        }
    }

    async fn run(mut self, server: Weak<RelayServer>, mut cmds: mpsc::UnboundedReceiver<Cmd>) {
        let mut events = self.events.subscribe(true);
        let (reads_tx, mut reads) = mpsc::unbounded_channel();
        self.frames.push(self.hello.clone());
        self.flush(now_ms(), 1);
        self.snapshot(&reads_tx);
        let mut tick = tokio::time::interval(Duration::from_millis(self.timing.frame_ms.max(10.0) as u64));
        loop {
            tokio::select! {
                chunk = events.recv() => match chunk {
                    Some(bytes) => {
                        for (name, data) in sse(&bytes) {
                            self.event(&name, &data);
                        }
                    }
                    None => return,
                },
                cmd = cmds.recv() => match cmd {
                    Some(c) => self.cmd(c, &reads_tx),
                    None => return,
                },
                Some(r) = reads.recv() => self.read(r),
                _ = tick.tick() => {
                    let subs = server.upgrade().map_or(1, |s| s.subs.locked().len().max(1));
                    self.tick(now_ms(), subs, &reads_tx);
                }
            }
        }
    }

    fn snapshot(&self, reads: &mpsc::UnboundedSender<Read>) {
        let (api, to) = (self.api.clone(), reads.clone());
        tokio::spawn(async move {
            let (g, l, e) = tokio::join!(api.get("/api/graph?hosts=0"), api.get("/api/live?hosts=0"), api.get("/api/engine/state"));
            let read = match (g, l, e) {
                (Ok(g), Ok(l), Ok(e)) => Ok([g, l, e]),
                (Err((m, _)), ..) | (_, Err((m, _)), _) | (.., Err((m, _))) => Err(m),
            };
            let _ = to.send(Read::Snapshot(read));
        });
    }

    fn event(&mut self, name: &str, data: &Value) {
        if !self.ready {
            return;
        }
        let Some((frame, coalesce)) = self.p.event(name, data, now_ms()) else { return };
        match coalesce {
            Some("graph") => self.graph = Some(frame),
            Some("live") => self.live = Some(frame),
            _ => self.wait(frame),
        }
    }

    /// A frame for the next body; past the most that may wait, it goes,
    /// and a fresh snapshot is owed.
    fn wait(&mut self, frame: Value) {
        let n = frame.to_string().len();
        if self.waiting_bytes + n > WAITING_BYTES || self.frames.len() >= WAITING_FRAMES {
            self.gap = true;
            return;
        }
        self.waiting_bytes += n;
        self.frames.push(frame);
    }

    fn cmd(&mut self, c: Cmd, reads: &mpsc::UnboundedSender<Read>) {
        match c {
            Cmd::Time { id } => self.wait(json!({ "t": "time", "id": id, "now": xenonet_base::json::number(now_ms()) })),
            Cmd::Fresh => self.snapshot(reads),
            Cmd::Get { id, path } => {
                let (shape, read) = match self.allowed(&path) {
                    Ok(x) => x,
                    Err(why) => return self.wait(json!({ "t": "res", "id": id, "status": 403, "body": { "error": why } })),
                };
                if self.inflight >= core::MAX_GETS {
                    return self.wait(json!({ "t": "res", "id": id, "status": 429, "body": { "error": "too many reads at once" } }));
                }
                self.inflight += 1;
                let (api, to, ms) = (self.api.clone(), reads.clone(), self.timing.get_ms);
                tokio::spawn(async move {
                    let answer = match tokio::time::timeout(Duration::from_millis(ms as u64), api.get(&read)).await {
                        Ok(a) => a,
                        Err(_) => Err((format!("no answer in {} s", (ms / 1000.0).round()), 504)),
                    };
                    let _ = to.send(Read::Get { id, shape, answer });
                });
            }
        }
    }

    /// A get's path against the relay's allowlist, then what this host
    /// serves over the network: with titles only, no transcript, and
    /// collisions and MCP servers' details not yet at all. The path read,
    /// as the relay reads it.
    fn allowed(&self, path: &str) -> Result<(Shape, String), String> {
        let (shape, path) = core::allowed(Some(&json!(path)))?;
        let route = path.split('?').next().unwrap_or("");
        if route == "/api/collision" || route == "/api/mcp" {
            return Err("collisions and MCP servers' details from another machine in the network come later: open them on that machine".into());
        }
        // Read now, not as the subscription began: titles only goes at once.
        if self.net.titles_only() && route.ends_with("/transcript") {
            return Err("this machine shares titles only with the network: its transcripts stay on it".into());
        }
        Ok((shape, core::route_to_read(shape, &path)))
    }

    fn read(&mut self, r: Read) {
        match r {
            Read::Get { id, shape, answer } => {
                self.inflight = self.inflight.saturating_sub(1);
                let frame = match answer {
                    Ok((status, body)) => self.p.answer(id as i64, shape, status, &body, now_ms()),
                    Err((message, status)) => json!({ "t": "res", "id": id, "status": status, "body": { "error": message } }),
                };
                self.wait(frame);
            }
            Read::Snapshot(s) => match s.and_then(|reads| self.p.snapshot(reads, now_ms())) {
                Ok(frame) => {
                    // What came before the snapshot is in it.
                    self.frames.retain(|f| f["t"] != "event");
                    self.waiting_bytes = self.frames.iter().map(|f| f.to_string().len()).sum();
                    self.graph = None;
                    self.live = None;
                    self.frames.push(frame);
                    self.ready = true;
                    self.gap = false;
                }
                Err(why) => {
                    (self.log)("net.relay", &format!("this machine's map couldn't be read for {}: {why}", self.host));
                    if !self.ready {
                        self.frames.push(json!({ "t": "bye", "code": "xenonet-gone", "message": "this machine's XenoNet couldn't read its own map" }));
                    }
                }
            },
        }
    }

    fn tick(&mut self, now: f64, subs: usize, reads: &mpsc::UnboundedSender<Read>) {
        // Titles only, switched on or off: a fresh snapshot, shaped anew.
        let titles_only = self.net.titles_only();
        if titles_only != self.titles_only {
            self.titles_only = titles_only;
            self.p.detail = !titles_only;
            self.snapshot(reads);
        }
        if self.gap {
            self.gap = false;
            self.snapshot(reads);
        }
        if now - self.last_sent < self.timing.frame_ms * subs as f64 {
            return;
        }
        self.flush(now, subs);
    }

    /// What waits, in bodies of at most BODY_BYTES: the map and the live
    /// state when they're due, and a ping when it's been quiet.
    fn flush(&mut self, now: f64, _subs: usize) {
        if self.graph.is_some() && now - self.last_graph >= self.timing.graph_ms {
            if let Some(g) = self.graph.take() {
                self.frames.push(g);
                self.last_graph = now;
            }
        }
        if self.live.is_some() && now - self.last_live >= self.timing.live_ms {
            if let Some(l) = self.live.take() {
                self.frames.push(l);
                self.last_live = now;
            }
        }
        if self.frames.is_empty() && now - self.last_sent >= self.timing.ping_ms {
            self.frames.push(json!({ "t": "ping", "now": xenonet_base::json::number(now) }));
        }
        if self.frames.is_empty() {
            return;
        }
        let frames = std::mem::take(&mut self.frames);
        self.waiting_bytes = 0;
        let mut body: Vec<Value> = Vec::new();
        let mut bytes = 0;
        for f in frames {
            let f = self.fit(f);
            let Some(f) = f else { continue };
            let n = encode(&f).len();
            if !body.is_empty() && (bytes + n > BODY_BYTES || body.len() >= wire::MAX_FRAMES) {
                self.send(std::mem::take(&mut body));
                bytes = 0;
            }
            bytes += n;
            body.push(f);
        }
        if !body.is_empty() {
            self.send(body);
        }
        self.last_sent = now;
    }

    /// A frame that fits a body: a read's answer too big becomes a 413,
    /// and anything else too big, or nested past what a console reads, is
    /// left out.
    fn fit(&self, f: Value) -> Option<Value> {
        let n = encode(&f).len();
        if n <= BODY_BYTES && xenonet_hosts::frames::shallow(&f, MAX_DEPTH) {
            return Some(f);
        }
        if f["t"] == "res" {
            return Some(json!({ "t": "res", "id": f["id"], "status": 413, "body": { "error": "too big to send over the network" } }));
        }
        (self.log)("net.relay", &format!("left out a {} too big to send to {} ({} MB)", f["t"].as_str().unwrap_or("frame"), self.host, (n as f64 / 1e6).round()));
        None
    }

    /// One body. Its seq counts even when it doesn't go, so the console
    /// sees the hole and asks for a fresh snapshot.
    fn send(&mut self, frames: Vec<Value>) {
        self.seq += 1;
        let body = wire::batch(&self.sub, self.seq, &frames);
        if let Err(why) = self.net.send(Outgoing { to: To::Host(self.host.clone()), kind: Kind::Relay, body }) {
            (self.log)("net.relay", &format!("this machine's map didn't go to {}: {why}", self.host));
        }
    }
}

/// The events in one chunk of this machine's event stream: each block's
/// name and JSON data.
fn sse(bytes: &[u8]) -> Vec<(String, Value)> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::new();
    for block in text.split("\n\n") {
        let mut name = "message".to_string();
        let mut data: Vec<&str> = Vec::new();
        for l in block.split('\n') {
            if let Some(rest) = l.strip_prefix("event:") {
                name = rest.trim().to_string();
            } else if let Some(rest) = l.strip_prefix("data:") {
                data.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        if data.is_empty() {
            continue;
        }
        if let Ok(v) = xenonet_base::json::parse(&data.join("\n")) {
            out.push((name, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use xenonet_base::network::{Host, Me};

    const SUB: &str = "sub_0123456789abcdef0123456789abcdef";

    /// studio's network, as laptop's relay sees it: studio a window host,
    /// desk not; what laptop sends is kept.
    struct FakeNet {
        sent: Mutex<Vec<Outgoing>>,
        hosts: Mutex<Vec<Host>>,
        titles_only: Mutex<bool>,
    }

    impl Net for FakeNet {
        fn me(&self) -> Option<Me> {
            Some(Me { network: "net_0123456789abcdef0123456789abcdef".into(), name: "home".into(), host: "laptop".into() })
        }
        fn hosts(&self) -> Vec<Host> {
            self.hosts.locked().clone()
        }
        fn send(&self, out: Outgoing) -> Result<(), String> {
            self.sent.locked().push(out);
            Ok(())
        }
        fn titles_only(&self) -> bool {
            *self.titles_only.locked()
        }
    }

    /// laptop's own API: a session, its transcript and stats; what was
    /// read is kept.
    #[derive(Default)]
    struct FakeApi {
        read: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl SelfApi for FakeApi {
        async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)> {
            self.read.locked().push(path.to_string());
            let route = path.split('?').next().unwrap_or("");
            Ok(match route {
                "/api/graph" => (
                    200,
                    json!({ "nodes": [{ "id": "core", "kind": "core", "path": "/home/nova" }, { "id": "claude:abc", "kind": "session", "title": "Fix the login form", "cwd": "/home/nova/atlas", "updatedAt": now_ms() }], "links": [{ "source": "core", "target": "claude:abc", "kind": "session" }] }),
                ),
                "/api/live" => (200, json!({ "sessions": [{ "id": "claude:abc", "lastMessage": "Done in /home/nova/atlas" }] })),
                "/api/engine/state" => (200, json!({ "agents": [] })),
                "/api/session/claude:abc/transcript" => (200, json!({ "entries": [{ "role": "user", "text": "in /home/nova/atlas" }] })),
                "/api/session/claude:abc/stats" => (200, json!({ "calls": 3 })),
                _ => (404, json!({ "error": "not found" })),
            })
        }
    }

    fn server(titles_only: bool) -> (Arc<RelayServer>, Arc<FakeNet>, Arc<FakeApi>, Arc<Events>) {
        let net = Arc::new(FakeNet {
            sent: Mutex::new(Vec::new()),
            hosts: Mutex::new(vec![Host { id: "studio".into(), window: true, online: true, seen: None }, Host { id: "desk".into(), window: false, online: true, seen: None }]),
            titles_only: Mutex::new(titles_only),
        });
        let api = Arc::new(FakeApi::default());
        let events = Arc::new(Events::new());
        let timing = Timing { frame_ms: 20.0, graph_ms: 40.0, live_ms: 20.0, lapse_ms: 300.0, ping_ms: 200.0, watch_ms: 50, get_ms: 2000.0, renew_ms: 100.0, roster_ms: 100 };
        let parts = Parts {
            net: net.clone(),
            events: events.clone(),
            api: api.clone(),
            home: "/home/nova".into(),
            version: "0.0.0-test".into(),
            platform: "linux".into(),
            demo: true,
            aliases: |_| Vec::new(),
            log: Arc::new(|_: &str, _: &str| {}),
            timing,
        };
        let s = RelayServer::new(parts);
        s.start();
        (s, net, api, events)
    }

    fn ask(from: &str, window: bool, op: &Op) -> Incoming {
        Incoming { from: from.into(), kind: Kind::Relay, body: wire::ask(SUB, op), at: now_ms(), from_window_host: window }
    }

    /// Every frame sent to `host` so far, in order, with the seqs checked.
    fn frames_to(net: &FakeNet, host: &str) -> Vec<Value> {
        let sent = net.sent.locked();
        let mut seq = 0;
        let mut out = Vec::new();
        for o in sent.iter().filter(|o| o.to == To::Host(host.into())) {
            let b = wire::parse_batch(&o.body).unwrap();
            assert_eq!((b.sub.as_str(), b.seq), (SUB, seq + 1), "in order, with no hole");
            seq = b.seq;
            out.extend(b.frames);
        }
        out
    }

    async fn until(what: &str, mut ok: impl FnMut() -> bool) {
        let end = std::time::Instant::now() + Duration::from_secs(5);
        while !ok() {
            assert!(std::time::Instant::now() < end, "waited for {what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn res(frames: &[Value], id: u64) -> Option<Value> {
        frames.iter().find(|f| f["t"] == "res" && f["id"] == id).cloned()
    }

    #[tokio::test]
    async fn a_window_host_gets_a_hello_a_snapshot_its_reads_and_the_events_after() {
        let (s, net, api, events) = server(false);
        s.relay(ask("studio", true, &Op::Open));
        until("the snapshot", || frames_to(&net, "studio").iter().any(|f| f["t"] == "snapshot")).await;
        let frames = frames_to(&net, "studio");
        assert_eq!(frames[0]["t"], "hello");
        assert_eq!(frames[0]["host"], json!({ "id": "laptop", "key": net_key("net_0123456789abcdef0123456789abcdef", "laptop") }));
        assert_eq!(frames[0]["detail"], true);
        let snap = frames.iter().find(|f| f["t"] == "snapshot").unwrap();
        assert_eq!(snap["graph"]["nodes"][0]["path"], "~", "the home folder as ~");
        assert_eq!(snap["live"]["sessions"][0]["lastMessage"], "Done in ~/atlas", "detail, inside a network");
        // A transcript and stats, as GET, and only what the allowlist has.
        for (id, path) in [
            (1, "/api/session/claude:abc/transcript"),
            (2, "/api/session/claude:abc/stats"),
            (3, "/api/open"),
            (4, "/api/../etc/passwd"),
            (5, "/api/mcp?server=agent-wiki"),
            (6, "/api/collision?a=claude:abc&b=claude:def"),
        ] {
            s.relay(ask("studio", true, &Op::Get { id, path: path.into() }));
        }
        until("six answers", || (1..=6).all(|id| res(&frames_to(&net, "studio"), id).is_some())).await;
        let frames = frames_to(&net, "studio");
        assert_eq!(res(&frames, 1).unwrap()["body"]["entries"][0]["text"], "in ~/atlas");
        assert_eq!(res(&frames, 2).unwrap()["body"]["calls"], 3);
        for id in [3, 4, 5, 6] {
            assert_eq!(res(&frames, id).unwrap()["status"], 403, "{id}: {}", res(&frames, id).unwrap());
        }
        assert!(res(&frames, 5).unwrap()["body"]["error"].as_str().unwrap().contains("come later"));
        let read = api.read.locked().clone();
        assert!(
            read.iter()
                .all(|p| ["/api/graph?hosts=0", "/api/live?hosts=0", "/api/engine/state", "/api/session/claude:abc/transcript", "/api/session/claude:abc/stats"]
                    .contains(&p.as_str())),
            "{read:?}"
        );
        // A POST, by its method or as an op, never reaches the API.
        let mut post = wire::ask(SUB, &Op::Get { id: 7, path: "/api/session/claude:abc/stats".into() });
        post["method"] = json!("POST");
        s.relay(Incoming { from: "studio".into(), kind: Kind::Relay, body: post, at: now_ms(), from_window_host: true });
        s.relay(Incoming {
            from: "studio".into(),
            kind: Kind::Relay,
            body: json!({ "schema": wire::SCHEMA, "op": "post", "sub": SUB, "id": 8, "path": "/api/open" }),
            at: now_ms(),
            from_window_host: true,
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(api.read.locked().len(), read.len(), "nothing more was read");
        // The clock, and this machine's events: the map coalesced, the rest
        // in order.
        s.relay(ask("studio", true, &Op::Time { id: 9 }));
        events.send("activity", &json!({ "sessionId": "claude:abc", "kind": "tool", "detail": "ran tests in /home/nova/atlas" }));
        events.send("graph", &json!({ "nodes": [], "links": [] }));
        until("the time and the activity", || {
            let f = frames_to(&net, "studio");
            f.iter().any(|x| x["t"] == "time" && x["id"] == 9) && f.iter().any(|x| x["t"] == "event" && x["name"] == "activity")
        })
        .await;
        let act = frames_to(&net, "studio").into_iter().find(|x| x["name"] == "activity").unwrap();
        assert_eq!(act["data"]["detail"], "ran tests in ~/atlas");
        s.stop();
    }

    #[tokio::test]
    async fn titles_only_keeps_transcripts_and_detail_here() {
        let (s, net, api, _) = server(true);
        s.relay(ask("studio", true, &Op::Open));
        until("the snapshot", || frames_to(&net, "studio").iter().any(|f| f["t"] == "snapshot")).await;
        let frames = frames_to(&net, "studio");
        assert_eq!(frames[0]["detail"], false);
        let snap = frames.iter().find(|f| f["t"] == "snapshot").unwrap();
        assert_eq!(snap["live"]["sessions"][0]["lastMessage"], Value::Null, "no detail");
        s.relay(ask("studio", true, &Op::Get { id: 1, path: "/api/session/claude:abc/transcript".into() }));
        s.relay(ask("studio", true, &Op::Get { id: 2, path: "/api/session/claude:abc/stats".into() }));
        until("both answers", || res(&frames_to(&net, "studio"), 2).is_some() && res(&frames_to(&net, "studio"), 1).is_some()).await;
        let frames = frames_to(&net, "studio");
        assert_eq!(res(&frames, 1).unwrap()["status"], 403);
        assert!(res(&frames, 1).unwrap()["body"]["error"].as_str().unwrap().contains("titles only"));
        assert_eq!(res(&frames, 2).unwrap()["status"], 200, "stats still go");
        assert!(!api.read.locked().iter().any(|p| p.contains("transcript")), "never read");
        s.stop();
    }

    #[tokio::test]
    async fn only_window_hosts_and_only_while_renewed_and_online() {
        let (s, net, _, _) = server(false);
        // desk isn't a window host: refused with a bye, and nothing runs.
        s.relay(ask("desk", false, &Op::Open));
        let frames = frames_to(&net, "desk");
        assert_eq!((frames.len(), frames[0]["t"].clone(), frames[0]["code"].clone()), (1, json!("bye"), json!("not-window")));
        assert_eq!(s.subscribed(), Vec::<(String, String)>::new());
        // studio's lapses without a renewal…
        s.relay(ask("studio", true, &Op::Open));
        assert_eq!(s.subscribed(), [("studio".to_string(), SUB.to_string())]);
        until("the lapse", || s.subscribed().is_empty()).await;
        // …and lasts while it's renewed, until studio goes offline.
        s.relay(ask("studio", true, &Op::Open));
        for _ in 0..8 {
            tokio::time::sleep(Duration::from_millis(60)).await;
            s.relay(ask("studio", true, &Op::Renew { fresh: false }));
        }
        assert_eq!(s.subscribed().len(), 1, "renewed, it stays");
        net.hosts.locked()[0].online = false;
        until("offline ends it", || s.subscribed().is_empty()).await;
        net.hosts.locked()[0].online = true;
        // Removed, or no longer a window host: it ends, the latter with a bye.
        s.relay(ask("studio", true, &Op::Open));
        s.removed("studio");
        assert_eq!(s.subscribed(), Vec::<(String, String)>::new());
        s.relay(ask("studio", true, &Op::Open));
        net.hosts.locked()[0].window = false;
        until("no longer a window host", || s.subscribed().is_empty()).await;
        assert_eq!(net.sent.locked().last().unwrap().body["frames"][0]["code"], "not-window");
        // Closed by the console, and on a link that went down.
        net.hosts.locked()[0].window = true;
        s.relay(ask("studio", true, &Op::Open));
        s.relay(ask("studio", true, &Op::Close));
        assert_eq!(s.subscribed(), Vec::<(String, String)>::new());
        s.relay(ask("studio", true, &Op::Open));
        s.linked(false);
        assert_eq!(s.subscribed(), Vec::<(String, String)>::new());
        s.stop();
    }

    #[test]
    fn the_event_stream_read_as_its_events() {
        let got = sse(b"retry: 3000\n\nevent: graph\ndata: {\"n\":1}\n\n: ping\n\nevent: live\ndata: 2\n\n");
        assert_eq!(got, [("graph".to_string(), json!({ "n": 1 })), ("live".to_string(), json!(2))]);
        assert_eq!(net_key("net_x", "laptop").len(), 32);
        assert_ne!(net_key("net_x", "laptop"), net_key("net_y", "laptop"));
    }
}
