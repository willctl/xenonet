// Other machines on the map (docs/design/network.md, section 7): three
// services in this process, each with its own state folder and its
// made-up sessions, on link/2 over loopback. studio founds the network;
// laptop joins as a window host and desk as a server, with invites. The
// page draws another machine's sessions as cards from its presence, so
// studio's state counts laptop's sessions from what laptop's presence
// says. A remote session's transcript and stats open through the window,
// on svc/2's window-read service, read on laptop as any local program
// reads its API: an agent on studio is refused, desk (no window host) is
// refused at laptop, and a machine that shares titles only keeps its
// transcripts.
//
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use xenonet_api::cores::{self, Cores, SelfApi, State, R};
use xenonet_model::ops::{Kind, Outgoing, To};
use xenonet_net::gwstore::Caps;
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_os::log::Log;
use xenonet_service::config::Config;
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

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

/// The service's own API, in-process: what a window read from another
/// machine reads, kept, each as any local program would ask it.
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
        // The service reading itself, with its own key, as HttpApi does.
        let own = ctx.self_key.reveal().to_string();
        let r = api(&ctx, Req::fake("GET", path, &[("x-xenonet-self", own.as_str())], None)).await.map_err(|e| (e.message, 502))?;
        Ok((r.status, r.body_json()))
    }
}

fn options(dir: &Path, who: &str) -> Options {
    let who = who.to_string();
    let mut o = Options::new(
        dir.join("state").join("network"),
        common::kept(&dir.join("state")),
        "0.0.0-test",
        "linux",
        Arc::new(move |code: &str, words: &str| {
            if std::env::var("NET_TEST_LOG").is_ok() {
                eprintln!("[{who}] {code}: {words}");
            }
        }),
    );
    o.timing = Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, answer_ms: 4000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 };
    // Fake keys: a test never touches the machine's own, or prompts.
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    o.caps = Caps::default();
    // link/2's and join/2's endpoints on loopback alone.
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o.link = true;
    o
}

struct Host {
    name: &'static str,
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
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
        let mut cores = Cores::pending(false, false);
        xenonet_service::serve::wire_root(&mut cores);
        cores.state = Arc::new(Map { home, sessions });
        let peers = Arc::new(Peers::default());
        cores.verify = peers.clone();
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let svc = NetService::open(options(&dir, name)).unwrap();
        let _ = ctx.net.set(svc.clone());
        let own = Arc::new(OwnApi::default());
        *own.ctx.lock().unwrap() = Arc::downgrade(&ctx);
        // As serve.rs plugs it: window reads from other machines read this
        // service's own API.
        svc.nodes().common().set_reads(Arc::new(xenonet_service::serve::SelfReads(own.clone())));
        svc.start().await.unwrap();
        Host { name, ctx, svc, own, peers }
    }

    async fn call(&self, method: &str, path: &str, body: Value, headers: &[(&str, &str)]) -> (u16, Value) {
        let text = body.to_string();
        let req = Req::fake(method, path, headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
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
}

impl Drop for Host {
    fn drop(&mut self) {
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
    let (s, t) = maker.window("POST", "/api/net/invite", json!({ "host": h.name, "window": window })).await;
    assert_eq!(s, 200, "{t}");
    let at = t["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
    let (s, v) = h.window("POST", "/api/net/join", json!({ "invite": t["invite"], "code": t["code"], "at": at })).await;
    assert_eq!(s, 200, "{} joins: {v}", h.name);
}

/// The row of `of` in `h`'s state.
async fn row(h: &Host, of: &str) -> Value {
    h.state().await["hosts"].as_array().unwrap().iter().find(|x| x["id"] == of).cloned().unwrap_or(Value::Null)
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn other_machines_show_from_presence_and_their_transcripts_open_through_the_window() {
    let root = std::env::temp_dir().join(format!("xenonet-net-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let a = Host::open("studio", root.join("studio"), vec![(S1, "Add partial refunds")]).await;
    let (s, v) = a.window("POST", "/api/net/create", json!({ "name": "home", "host": "studio" })).await;
    assert_eq!(s, 200, "{v}");
    let b = Host::open("laptop", root.join("laptop"), vec![(L1, "Fix the login form")]).await;
    join(&b, &a, true).await;
    let c = Host::open("desk", root.join("desk"), vec![]).await;
    join(&c, &a, false).await;
    for h in [&a, &b, &c] {
        until(&format!("{} with every member online", h.name), 20, || async {
            let s = h.state().await;
            s["hosts"].as_array().map_or(0, Vec::len) == 3 && s["hosts"].as_array().unwrap().iter().all(|x| x["online"] == true)
        })
        .await;
    }

    // laptop's presence says what sessions it has: studio and desk count
    // them on its row, which is what the page draws its cards from.
    let presence = json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [{ "id": L1, "title": "Fix the login form", "waitsOnYou": true }], "removed": [], "you": false, "roster": "" });
    b.svc.net().send(Outgoing::new(To::All, Kind::Presence, presence)).unwrap();
    for h in [&a, &c] {
        until(&format!("laptop's session at {}", h.name), 10, || async {
            let r = row(h, "laptop").await;
            r["agents"] == 1 && r["waitsOnYou"] == 1
        })
        .await;
    }

    // The transcript and stats, through the window, on svc/2: laptop reads
    // its own API as a local client does.
    let remote = format!("@laptop:{L1}");
    let path = format!("/api/session/{}/transcript", enc(&remote));
    let (s, t) = a.window("GET", &path, Value::Null).await;
    assert_eq!(s, 200, "{t}");
    assert_eq!(t["host"], "laptop", "{t}");
    assert!(t["entries"][0]["text"].as_str().unwrap().starts_with("fix the form in "), "{t}");
    let (s, st) = a.window("GET", &format!("/api/session/{}/stats", enc(&remote)), Value::Null).await;
    assert_eq!((s, st["calls"].clone()), (200, json!(3)), "{st}");
    let read = b.own.read.lock().unwrap().clone();
    assert!(read.iter().any(|p| p.starts_with(&format!("/api/session/{L1}/transcript"))), "laptop read it as a local client: {read:?}");
    // From an agent on studio, never: without the window's key, or with an
    // actor's token beside it.
    a.peers.agent.store(true, Ordering::SeqCst);
    let key = a.ctx.window_key.reveal().to_string();
    for headers in [vec![("x-xenonet-agent", S1)], vec![], vec![("x-xenonet-window", key.as_str()), ("authorization", "Bearer xnt_wren")]] {
        let (s, v) = a.call("GET", &path, Value::Null, &headers).await;
        assert_eq!(s, 403, "{headers:?}: {v}");
    }
    a.peers.agent.store(false, Ordering::SeqCst);
    // desk is no window host: laptop refuses its window.
    let (s, v) = c.window("GET", &path, Value::Null).await;
    assert_eq!((s, v["code"].clone()), (403, json!("not-window")), "{v}");

    // laptop shares titles only: its transcripts stay on it, its stats go.
    let (s, v) = b.window("PUT", "/api/net/settings", json!({ "titlesOnly": true })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = a.window("GET", &path, Value::Null).await;
    assert_eq!(s, 403, "{v}");
    let (s, _) = a.window("GET", &format!("/api/session/{}/stats", enc(&remote)), Value::Null).await;
    assert_eq!(s, 200);
    drop((a, b, c));
    let _ = std::fs::remove_dir_all(root);
}
