// Other machines' maps through the network (docs/design/network.md,
// section 7): three services in this process, each with its own state
// folder, its made-up sessions, and the binary's hub and relay as `xenonet
// serve` plugs them in. studio founds the network and is its LAN-only
// gateway on 127.0.0.1:48122; laptop joins as a window host and desk as a
// server. studio's console sees laptop's map under @laptop:, and opens a
// transcript of laptop's through the window; an agent on studio can't, a
// host that shares titles only keeps its transcripts, a gap brings a
// fresh snapshot, and laptop's relay stops once studio's subscription
// lapses.
//
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use xenonet_base::log::Log;
use xenonet_base::network::Sink as _;
use xenonet_net::gwstore::Caps;
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_service::config::Config;
use xenonet_service::cores::{self, Cores, HubNet, SelfApi, State, R};
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;
use xenonet_service::wire::Emitter;
use xenonet_wire_app::hosts::config::HostsStore;
use xenonet_wire_app::hosts::hub::{HostHub, HubCore, HubParts};
use xenonet_wire_app::hosts::relay::Timing as RelayTiming;
use xenonet_wire_app::hosts::schemas::HostSchemas;

const LAN: u16 = 48122;
const L1: &str = "claude:bbbbbbbb-0001";
const S1: &str = "claude:aaaaaaaa-0001";

/// The person at the window and their terminal, until `agent` says the
/// peer is a session's process: an agent then, as process checks find one.
#[derive(Default)]
struct Peers {
    agent: AtomicBool,
}

impl Peers {
    fn seen(&self) -> Value {
        if self.agent.load(Ordering::SeqCst) {
            json!({ "ok": false, "reason": "this came from under a Claude Code process" })
        } else {
            json!({ "ok": true })
        }
    }
}

#[async_trait]
impl cores::Verify for Peers {
    async fn check(&self, _peer: Peer, _id: &str, _hook: bool) -> Value {
        json!({ "ok": true })
    }
    async fn person(&self, _peer: Peer) -> Value {
        self.seen()
    }
    async fn window(&self, _peer: Peer) -> Value {
        self.seen()
    }
}

/// A machine's made-up map: its sessions, each with a transcript.
struct Map {
    home: String,
    sessions: Vec<(&'static str, &'static str)>,
}

impl Map {
    fn session(&self, id: &str, title: &str) -> Value {
        let at = now_ms();
        let raw = id.split_once(':').unwrap().1;
        json!({
            "id": id, "kind": "session", "app": "claude", "rawId": raw, "title": title, "preview": null,
            "cwd": format!("{}/atlas", self.home), "createdAt": at - 3600e3, "updatedAt": at, "archived": false,
            "model": null, "branch": "main", "parentId": null, "parentKind": null,
            "activity": "active", "fx": null, "fy": null, "fz": null, "pinned": false, "actions": [],
        })
    }
}

#[async_trait]
impl State for Map {
    fn graph(&self) -> R<Option<Value>> {
        let mut nodes = vec![json!({ "id": "core", "kind": "core", "name": "here", "path": self.home })];
        let mut links = Vec::new();
        for (id, title) in &self.sessions {
            nodes.push(self.session(id, title));
            links.push(json!({ "source": "core", "target": id, "kind": "session" }));
        }
        Ok(Some(json!({ "nodes": nodes, "links": links, "relations": [], "identities": [], "stats": { "sessions": self.sessions.len() } })))
    }
    fn graph_version(&self) -> u64 {
        1
    }
    fn live_snapshot(&self) -> R<Value> {
        Ok(json!({ "sessions": [], "collisions": [], "collab": [] }))
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok(self.sessions.iter().map(|(id, title)| self.session(id, title)).collect())
    }
    fn node(&self, id: &str) -> R<Option<Value>> {
        Ok(self.nodes()?.into_iter().find(|n| n["id"] == id))
    }
    fn home_of(&self, _id: &str) -> R<Value> {
        Ok(Value::Null)
    }
    fn identities(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
    fn add_block(&self, _health: Arc<dyn Fn() -> Value + Send + Sync>, _source: Option<String>) -> R<()> {
        Ok(())
    }
    async fn session_stats(&self, _id: &str) -> R<Option<Value>> {
        Ok(Some(json!({ "calls": 3, "turns": 2 })))
    }
    async fn read_transcript(&self, node: &Value, _cursor: Option<&str>, _limit: f64) -> R<Value> {
        Ok(json!({ "entries": [{ "role": "user", "text": format!("fix the form in {}", node["cwd"].as_str().unwrap_or("")) }], "cursor": null }))
    }
}

/// The service's own API, in-process: what the relay reads, kept, each as
/// any local program would ask it, with no key.
#[derive(Default)]
struct OwnApi {
    ctx: Mutex<Weak<Ctx>>,
    read: Mutex<Vec<String>>,
}

#[async_trait]
impl SelfApi for OwnApi {
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)> {
        self.read.lock().unwrap().push(path.to_string());
        let ctx = self.ctx.lock().unwrap().upgrade().ok_or(("gone".to_string(), 502))?;
        let r = api(&ctx, Req::fake("GET", path, &[], None)).await.map_err(|e| (e.message, 502))?;
        Ok((r.status, r.body_json()))
    }
}

fn timing() -> Timing {
    Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 }
}

fn relay_timing() -> RelayTiming {
    RelayTiming { frame_ms: 50.0, graph_ms: 100.0, live_ms: 50.0, lapse_ms: 3000.0, ping_ms: 1000.0, watch_ms: 200, get_ms: 5000.0, renew_ms: 500.0, roster_ms: 300 }
}

fn options(dir: &Path, who: &str) -> Options {
    let who = who.to_string();
    let mut o = Options::new(
        dir.join("state").join("network"),
        "0.0.0-test",
        "linux",
        Arc::new(move |code: &str, words: &str| {
            if std::env::var("NET_TEST_LOG").is_ok() {
                eprintln!("[{who}] {code}: {words}");
            }
        }),
    );
    o.timing = timing();
    o.caps = Caps::default();
    o
}

struct Host {
    name: &'static str,
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
    hub: Arc<HostHub>,
    own: Arc<OwnApi>,
    peers: Arc<Peers>,
}

impl Host {
    async fn open(name: &'static str, dir: PathBuf, sessions: Vec<(&'static str, &'static str)>) -> Host {
        let home = dir.join("home").to_string_lossy().into_owned();
        let state = dir.join("state").to_string_lossy().into_owned();
        let vars = [("HOME", home.as_str()), ("XENONET_STATE_DIR", state.as_str()), ("XENONET_CACHE_DIR", state.as_str()), ("XENONET_PORT", "47995")];
        let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let config = Config::from(&[], &get);
        let mut cores = Cores::pending(false, false, false);
        let map = Arc::new(Map { home: home.clone(), sessions });
        cores.state = map.clone();
        let peers = Arc::new(Peers::default());
        cores.verify = peers.clone();
        let schemas = Arc::new(HostSchemas::new());
        let hosts = Arc::new(HostsStore::new(&state, schemas.clone()));
        let emitter = Emitter::new();
        let hub = HostHub::new(HubParts {
            hosts,
            schemas,
            state: map,
            store: Arc::new(cores::Pending),
            cache_dir: state.clone(),
            root: dir.to_string_lossy().into_owned(),
            home: home.clone(),
            demo: false,
            windows: json!({ "activeWindowMs": 600000, "recentWindowMs": 86400000, "projectActiveMs": 3600000 }),
            emitter: emitter.clone(),
            bin: "/no/such/xenonet".into(),
            ssh_bin: vec!["ssh".into()],
            ssh: false,
            relay: relay_timing(),
        });
        cores.hub = Some(Arc::new(HubCore(hub.clone())));
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let svc = NetService::open(options(&dir, name)).unwrap();
        let _ = ctx.net.set(svc.clone());
        let own = Arc::new(OwnApi::default());
        *own.ctx.lock().unwrap() = Arc::downgrade(&ctx);
        // As serve.rs plugs it, with the API in-process.
        ctx.cores.hub.as_ref().unwrap().plug_net(HubNet {
            net: svc.net(),
            sinks: svc.sinks().clone(),
            events: ctx.events.clone(),
            api: own.clone(),
            home,
            version: "0.0.0-test".into(),
            platform: "linux".into(),
            demo: false,
        });
        svc.start().await.unwrap();
        Host { name, ctx, svc, hub, own, peers }
    }

    async fn call(&self, method: &str, path: &str, body: Value, headers: &[(&str, &str)]) -> (u16, Value) {
        let text = body.to_string();
        let req = Req::fake(method, path, headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, req).await.unwrap();
        (r.status, r.body_json())
    }

    /// The window's own call, with its key.
    async fn window(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        self.call(method, path, body, &[("x-xenonet-window", key.as_str())]).await
    }

    async fn state(&self) -> Value {
        self.window("GET", "/api/net/state", Value::Null).await.1
    }

    /// The map the console draws: this machine's, with the others' in it.
    async fn graph(&self) -> Value {
        self.call("GET", "/api/graph", Value::Null, &[]).await.1
    }

    fn snapshots_read(&self) -> usize {
        self.own.read.lock().unwrap().iter().filter(|p| *p == "/api/graph?hosts=0").count()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Some(h) = &self.ctx.cores.hub {
            h.stop();
        }
        self.svc.stop();
    }
}

async fn until<F: std::future::Future<Output = bool>>(what: &str, secs: u64, mut ok: impl FnMut() -> F) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !ok().await {
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn join(h: &Host, maker: &Host, window: bool) {
    let (s, t) = maker.window("POST", "/api/net/ticket", json!({ "host": h.name, "window": window })).await;
    assert_eq!(s, 200, "{t}");
    let line = t["line"].as_str().unwrap().to_string();
    let mut parts = line.rsplitn(3, ' ');
    let (code, address) = (parts.next().unwrap().to_string(), parts.next().unwrap().to_string());
    let (s, v) = h.window("POST", "/api/net/join", json!({ "address": address, "ticket": code })).await;
    assert_eq!(s, 200, "{} joins: {v}", h.name);
}

async fn live(h: &Host, hosts: usize) {
    until(&format!("{} live with {hosts} hosts", h.name), 20, || async {
        let s = h.state().await;
        s["gateway"]["link"] == "live" && s["hosts"].as_array().map_or(0, Vec::len) == hosts && s["hosts"].as_array().unwrap().iter().all(|x| x["online"] == true)
    })
    .await;
}

fn node(g: &Value, id: &str) -> Option<Value> {
    g["nodes"].as_array().into_iter().flatten().find(|n| n["id"] == id).cloned()
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_window_host_sees_another_hosts_map_and_opens_its_transcripts_from_the_window() {
    let root = std::env::temp_dir().join(format!("xenonet-net-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let a = Host::open("studio", root.join("studio"), vec![(S1, "Add partial refunds")]).await;
    let (s, v) = a.window("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{LAN}") })).await;
    assert_eq!(s, 200, "{v}");
    let b = Host::open("laptop", root.join("laptop"), vec![(L1, "Fix the login form")]).await;
    join(&b, &a, true).await;
    let c = Host::open("desk", root.join("desk"), vec![]).await;
    join(&c, &a, false).await;
    for h in [&a, &b, &c] {
        live(h, 3).await;
    }

    // No console open at studio: nothing subscribes, and laptop's map isn't
    // drawn; its sessions are presence's to draw, on the page.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(b.hub.relay_server().unwrap().subscribed(), Vec::<(String, String)>::new());
    assert!(node(&a.graph().await, &format!("@laptop:{L1}")).is_none());

    // studio's console opens: laptop's snapshot is drawn under @laptop:, as
    // a network host's, with the home folder as ~.
    a.ctx.cores.hub.as_ref().unwrap().console(Some("shown"), 1);
    let remote = format!("@laptop:{L1}");
    until("laptop's session on studio's map", 20, || async { node(&a.graph().await, &remote).is_some() }).await;
    let g = a.graph().await;
    let n = node(&g, &remote).unwrap();
    assert_eq!((n["host"].clone(), n["network"].clone(), n["title"].clone()), (json!("laptop"), json!("relay"), json!("Fix the login form")), "{n}");
    assert_eq!(n["cwd"], "~/atlas", "the home folder as ~: {n}");
    assert!(node(&g, S1).is_some(), "studio's own, beside it");
    assert_eq!(b.hub.relay_server().unwrap().subscribed().iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), ["studio"], "one subscription, studio's");
    // desk isn't a window host: it subscribes to no one.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(a.hub.relay_server().unwrap().subscribed().iter().all(|(h, _)| h != "desk"), "{:?}", a.hub.relay_server().unwrap().subscribed());

    // The transcript and stats, through the window.
    let path = format!("/api/session/{}/transcript", enc(&remote));
    let (s, t) = a.window("GET", &path, Value::Null).await;
    assert_eq!(s, 200, "{t}");
    assert_eq!((t["host"].clone(), t["id"].clone()), (json!("laptop"), Value::Null));
    assert_eq!(t["entries"][0]["text"], "fix the form in ~/atlas", "{t}");
    let (s, st) = a.window("GET", &format!("/api/session/{}/stats", enc(&remote)), Value::Null).await;
    assert_eq!((s, st["calls"].clone()), (200, json!(3)), "{st}");
    let read = b.own.read.lock().unwrap().clone();
    assert!(read.contains(&format!("/api/session/{L1}/transcript")), "laptop read it as a local client: {read:?}");
    // From an agent on studio, never: without the window's key, or with an
    // actor's token beside it.
    a.peers.agent.store(true, Ordering::SeqCst);
    let key = a.ctx.window_key.reveal().to_string();
    for headers in [vec![("x-xenonet-agent", S1)], vec![], vec![("x-xenonet-window", key.as_str()), ("authorization", "Bearer xnt_wren")]] {
        let (s, v) = a.call("GET", &path, Value::Null, &headers).await;
        assert_eq!((s, v["code"].clone()), (403, json!("window")), "{headers:?}: {v}");
    }
    a.peers.agent.store(false, Ordering::SeqCst);
    // Collisions and MCP servers' details over the network come later.
    let (s, v) = a.window("GET", &format!("/api/collision?a={}&b={}", enc(&remote), enc(&format!("@laptop:{L1}"))), Value::Null).await;
    assert_eq!(s, 403, "{v}");
    assert!(v["error"].as_str().unwrap().contains("come later"), "{v}");

    // A gap at studio: every map it watches sends a fresh snapshot.
    let before = b.snapshots_read();
    a.svc.sinks().gap();
    until("a fresh snapshot from laptop", 10, || async { b.snapshots_read() > before }).await;

    // laptop shares titles only: its transcripts stay on it, its stats go.
    let (s, v) = b.window("PUT", "/api/net/settings", json!({ "titlesOnly": true })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = a.window("GET", &path, Value::Null).await;
    assert_eq!(s, 403, "{v}");
    assert!(v["error"].as_str().unwrap().contains("titles only"), "{v}");
    let (s, _) = a.window("GET", &format!("/api/session/{}/stats", enc(&remote)), Value::Null).await;
    assert_eq!(s, 200);
    b.window("PUT", "/api/net/settings", json!({ "titlesOnly": false })).await;

    // studio's console stops renewing without a word: laptop's relay lets
    // the subscription lapse, and stops.
    a.hub.stop_renewing();
    until("laptop's relay stopped", 15, || async { b.hub.relay_server().unwrap().subscribed().is_empty() }).await;
    // The map stays on studio's, from what laptop sent last.
    assert!(node(&a.graph().await, &remote).is_some());
    drop((a, b, c));
    let _ = std::fs::remove_dir_all(root);
}
