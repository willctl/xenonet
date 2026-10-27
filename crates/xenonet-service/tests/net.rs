// One network across machines, end to end (docs/design/network.md): three
// services in this process, each with its own state folder, the first a
// LAN-only gateway on 127.0.0.1, the others linking to it over HTTP as
// hosts on other machines would. Their /api/net routes are called as the
// window calls them, and a test Sink stands in for the engine.
//
// Each test takes its ports from the OS (common::free_ports), so no two
// tests share one. NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use xenonet_base::log::Log;
use xenonet_base::network::{Incoming, Kind, Net, Outgoing, Sink, To};
use xenonet_net::gwstore::Caps;
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_service::config::Config;
use xenonet_service::cores::{self, Cores};
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

/// The person, from the window and their terminal alike.
struct You {
    person: Value,
}

#[async_trait]
impl cores::Verify for You {
    async fn person(&self, _peer: Peer) -> Value {
        self.person.clone()
    }
    async fn window(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
}

#[derive(Default)]
struct TestSink {
    got: Mutex<Vec<(String, String, Value)>>,
    /// Who each body came from, and whether the link said that host is a
    /// window host.
    window: Mutex<Vec<(String, bool)>>,
    links: Mutex<Vec<bool>>,
    removed: Mutex<Vec<String>>,
    gaps: Mutex<u32>,
    /// When set, the link this host sends its presence through each time
    /// the link comes up, as the engine does, and how many it sent.
    presence: Mutex<Option<Weak<dyn Net>>>,
    presences: AtomicU64,
}

impl Sink for TestSink {
    fn receive(&self, i: Incoming) {
        self.window.lock().unwrap().push((i.from.clone(), i.from_window_host));
        self.got.lock().unwrap().push((i.from, i.kind.as_str().to_string(), i.body));
    }
    fn linked(&self, up: bool) {
        self.links.lock().unwrap().push(up);
        let net = self.presence.lock().unwrap().as_ref().and_then(Weak::upgrade);
        if let Some(net) = net.filter(|_| up) {
            let counter = self.presences.fetch_add(1, Ordering::SeqCst) + 1;
            // Off the link's task, as the engine sends it from its own.
            std::thread::spawn(move || {
                let body = json!({ "schema": "network/presence/v1", "counter": counter, "full": true, "agents": [], "removed": [], "you": false, "roster": "" });
                net.send(Outgoing { to: To::All, kind: Kind::Presence, body }).unwrap();
            });
        }
    }
    fn removed(&self, host: &str) {
        self.removed.lock().unwrap().push(host.into());
    }
    fn gap(&self) {
        *self.gaps.lock().unwrap() += 1;
    }
}

impl TestSink {
    fn texts(&self, kind: &str) -> Vec<String> {
        self.got.lock().unwrap().iter().filter(|(_, k, _)| k == kind).map(|(_, _, b)| b["body"].as_str().unwrap_or("").to_string()).collect()
    }
    fn up(&self) -> bool {
        self.links.lock().unwrap().last() == Some(&true)
    }
    /// The counters of the presence that came from `host`.
    fn presence_from(&self, host: &str) -> Vec<u64> {
        self.got.lock().unwrap().iter().filter(|(f, k, _)| f == host && k == "presence").filter_map(|(_, _, b)| b["counter"].as_u64()).collect()
    }
}

struct Host {
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
    sink: Arc<TestSink>,
    dir: PathBuf,
    shift: f64,
    caps: Caps,
}

fn timing() -> Timing {
    Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, answer_ms: 4000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 }
}

fn options(dir: &std::path::Path, shift: f64, caps: Caps) -> Options {
    let who = dir.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
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
    o.clock = Arc::new(move || now_ms() + shift);
    o.timing = timing();
    o.caps = caps;
    o
}

impl Host {
    async fn new(test: &str, name: &str, shift: f64, caps: Caps) -> Host {
        let dir = std::env::temp_dir().join(format!("xenonet-net-it-{test}-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Host::open(dir, shift, caps).await
    }

    async fn open(dir: PathBuf, shift: f64, caps: Caps) -> Host {
        let home = dir.join("home").to_string_lossy().into_owned();
        let state = dir.join("state").to_string_lossy().into_owned();
        let env = move |k: &str| match k {
            "HOME" | "USERPROFILE" => Some(home.clone()),
            "XENONET_STATE_DIR" => Some(state.clone()),
            _ => None,
        };
        let config = Config::from(&[], &env);
        let mut cores = Cores::pending(false, false, false);
        cores.verify = Arc::new(You { person: json!({ "ok": true }) });
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let svc = NetService::open(options(&dir, shift, caps)).unwrap();
        let sink = Arc::new(TestSink::default());
        svc.sinks().set(sink.clone());
        let _ = ctx.net.set(svc.clone());
        svc.start().await.unwrap();
        Host { ctx, svc, sink, dir, shift, caps }
    }

    /// The same machine started again: its files as they were.
    async fn restart(self) -> Host {
        self.svc.stop();
        let (dir, shift, caps) = (self.dir.clone(), self.shift, self.caps);
        drop(self);
        Host::open(dir, shift, caps).await
    }

    async fn call(&self, method: &str, path: &str, body: Value, window: bool) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        let headers: Vec<(&str, &str)> = if window { vec![("x-xenonet-window", key.as_str())] } else { vec![] };
        let text = body.to_string();
        let req = Req::fake(method, path, &headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, req).await.unwrap();
        (r.status, r.body_json())
    }

    async fn state(&self) -> Value {
        self.call("GET", "/api/net/state", Value::Null, true).await.1
    }

    fn send(&self, to: &str, kind: Kind, text: &str) {
        let body = json!({ "schema": format!("network/{}/v1", kind.as_str()), "body": text });
        self.svc.net().send(Outgoing { to: To::Host(to.into()), kind, body }).unwrap();
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.svc.stop();
    }
}

async fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn live(h: &Host, hosts: usize) {
    let mut ok = false;
    for _ in 0..400 {
        let s = h.state().await;
        if s["gateway"]["link"] == "live" && s["hosts"].as_array().map_or(0, Vec::len) == hosts {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "never live with {hosts} hosts: {}", h.state().await);
}

/// studio makes the network on `port`; laptop and desk join with tickets
/// from its window.
async fn network(test: &str, port: u16, caps: Caps, shifts: [f64; 3]) -> (Host, Host, Host) {
    let a = Host::new(test, "studio", shifts[0], caps).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{port}") }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["self"]["founder"], true);
    assert_eq!(v["gateway"]["kind"], "loopback");
    let mut joined = Vec::new();
    for (name, shift) in [("laptop", shifts[1]), ("desk", shifts[2])] {
        let (s, t) = a.call("POST", "/api/net/ticket", json!({ "host": name }), true).await;
        assert_eq!(s, 200, "{t}");
        let line = t["line"].as_str().unwrap().to_string();
        assert!(line.starts_with(&format!("xenonet net join 127.0.0.1:{port} ")), "{line}");
        let ticket = line.rsplit(' ').next().unwrap().to_string();
        let h = Host::new(test, name, shift, caps).await;
        let (s, v) = h.call("POST", "/api/net/join", json!({ "address": format!("127.0.0.1:{port}"), "ticket": ticket }), true).await;
        assert_eq!(s, 200, "{v}");
        assert_eq!(v["self"]["id"], name);
        joined.push(h);
    }
    let c = joined.pop().unwrap();
    let b = joined.pop().unwrap();
    for h in [&a, &b, &c] {
        live(h, 3).await;
    }
    (a, b, c)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_hosts_join_message_both_ways_and_survive_a_gateway_restart() {
    let [port] = common::free_ports();
    let (a, b, c) = network("restart", port, Caps::default(), [0.0; 3]).await;
    // Every host's roster agrees, and each sees the others online.
    let heads: Vec<Value> = [a.state().await, b.state().await, c.state().await].iter().map(|s| s["rosterHead"].clone()).collect();
    assert!(heads.iter().all(|h| *h == heads[0] && !h.is_null()), "{heads:?}");
    until("desk online at laptop", 10, || b.svc.net().hosts().iter().any(|h| h.id == "desk" && h.online)).await;
    assert!(b.sink.up() && c.sink.up());

    // Frames both ways, through a host and through the gateway's own host.
    b.send("desk", Kind::Message, "m1 from laptop");
    until("m1 at desk", 10, || c.sink.texts("message") == ["m1 from laptop"]).await;
    c.send("laptop", Kind::Receipt, "stored m1");
    until("the receipt at laptop", 10, || c.sink.texts("message").len() == 1 && b.sink.texts("receipt") == ["stored m1"]).await;
    a.send("laptop", Kind::Message, "from the gateway's host");
    b.send("studio", Kind::Message, "to the gateway's host");
    until("both", 10, || b.sink.texts("message") == ["from the gateway's host"] && a.sink.texts("message") == ["to the gateway's host"]).await;
    // The engine is told whether the sender is a window host, from the
    // roster: studio founded the network; laptop and desk were added
    // without a window.
    assert!(b.sink.window.lock().unwrap().iter().all(|(from, w)| *w == (from == "studio")), "{:?}", b.sink.window.lock().unwrap());
    assert_eq!(*a.sink.window.lock().unwrap(), [("laptop".to_string(), false)]);
    assert_eq!(*c.sink.window.lock().unwrap(), [("laptop".to_string(), false)]);
    // Presence goes to every host.
    b.svc.net().send(Outgoing { to: To::All, kind: Kind::Presence, body: json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [{ "id": "claude:1", "waitsOnYou": true }], "removed": [], "you": false, "roster": "x" }) }).unwrap();
    until("presence", 10, || a.sink.got.lock().unwrap().iter().any(|g| g.1 == "presence") && c.sink.got.lock().unwrap().iter().any(|g| g.1 == "presence")).await;
    let s = a.state().await;
    let laptop = s["hosts"].as_array().unwrap().iter().find(|h| h["id"] == "laptop").unwrap().clone();
    assert_eq!((laptop["agents"].clone(), laptop["waitsOnYou"].clone()), (json!(1), json!(1)));
    assert_eq!(s["rosterMismatch"], json!(["laptop"]), "its presence named another head");
    // laptop's own row counts the agents its own presence lists.
    let me = b.state().await["hosts"].as_array().unwrap().iter().find(|h| h["self"] == true).unwrap().clone();
    assert_eq!((me["id"].clone(), me["agents"].clone(), me["waitsOnYou"].clone()), (json!("laptop"), json!(1), json!(1)), "{me}");
    b.svc
        .net()
        .send(Outgoing {
            to: To::All,
            kind: Kind::Presence,
            body: json!({ "schema": "network/presence/v1", "counter": 2, "full": false, "agents": [{ "id": "claude:2" }], "removed": ["claude:1"], "you": false, "roster": "x" }),
        })
        .unwrap();
    let me = b.state().await["hosts"].as_array().unwrap().iter().find(|h| h["self"] == true).unwrap().clone();
    assert_eq!((me["agents"].clone(), me["waitsOnYou"].clone()), (json!(1), json!(0)), "a change, and one gone: {me}");
    let epoch = s["epoch"].clone();

    // The gateway goes, mid-stream: the hosts go offline, and what they
    // send waits in their outboxes.
    let a = {
        a.svc.stop();
        until("laptop offline", 15, || !b.sink.up()).await;
        b.send("desk", Kind::Message, "m2 while the gateway was down");
        b.send("desk", Kind::Message, "m3 while the gateway was down");
        assert_eq!(b.state().await["outbox"]["frames"], 2);
        a.restart().await
    };
    assert_eq!(a.state().await["epoch"], epoch, "the same store, the same epoch");
    until("m2 and m3 at desk", 20, || c.sink.texts("message").len() == 3).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(c.sink.texts("message"), ["m1 from laptop", "m2 while the gateway was down", "m3 while the gateway was down"], "no loss and no repeats");
    assert_eq!(*b.sink.gaps.lock().unwrap() + *c.sink.gaps.lock().unwrap(), 0, "no gap: nothing was lost");
    until("the outbox empty", 10, || b.svc.node().shared().outbox().frames == 0).await;

    // The long-poll, for proxies that hold streams back: the first batch,
    // or the hello alone after the wait.
    let gw = a.svc.node().gateway().unwrap();
    let poll = |after: u64| {
        let (req, nonce) = c.svc.node().shared().signed_request("GET", &format!("/net/v1/link?epoch={}&after={after}&wait=1", gw.epoch()), Vec::new()).unwrap();
        (req, nonce)
    };
    let (req, nonce) = poll(u64::MAX);
    let r = gw.handle(req).await;
    let lines: Vec<Value> = String::from_utf8(match r.body {
        xenonet_net::gateway::NetBody::Full(b) => b,
        xenonet_net::gateway::NetBody::Stream(_) => panic!("a long-poll answers whole"),
    })
    .unwrap()
    .lines()
    .map(|l| serde_json::from_str(l).unwrap())
    .collect();
    assert_eq!(lines.len(), 1, "the hello alone: {lines:?}");
    assert_eq!(lines[0]["kind"], "hello");
    let hello = xenonet_net::frame::Frame::parse(&lines[0]).unwrap();
    assert_eq!(hello.body_json().unwrap()["nonce"], nonce, "signed over the host's nonce");
    // A replayed request is refused.
    let (req, _) = poll(0);
    let again = req.clone();
    assert_eq!(gw.handle(req).await.status, 200);
    assert_eq!(gw.handle(again).await.status, 409);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_full_queue_gives_a_gap_and_a_removed_host_is_refused() {
    let caps = Caps { frames: 3, ..Caps::default() };
    let [port] = common::free_ports();
    let (a, b, c) = network("gap", port, caps, [0.0; 3]).await;
    // desk is away; laptop sends it five, and the gateway keeps three.
    let c = {
        c.svc.stop();
        let mut sent = 0;
        for i in 1..=5 {
            b.send("desk", Kind::Message, &format!("q{i}"));
            sent += 1;
        }
        assert_eq!(sent, 5);
        until("laptop told of the frames that went", 15, || *b.sink.gaps.lock().unwrap() >= 1).await;
        let q = a.state().await["queues"].clone();
        assert!(q.as_array().unwrap().iter().any(|x| x["host"] == "desk" && x["frames"] == 3), "{q}");
        c.restart().await
    };
    until("desk's gap and the last three", 20, || *c.sink.gaps.lock().unwrap() >= 1 && c.sink.texts("message").len() == 3).await;
    assert_eq!(c.sink.texts("message"), ["q3", "q4", "q5"]);

    // studio removes desk: the others hear, its frames are refused, and
    // it's told, signed by the gateway.
    let (s, v) = a.call("POST", "/api/net/remove", json!({ "host": "desk" }), true).await;
    assert_eq!(s, 200, "{v}");
    until("laptop hears desk went", 10, || b.sink.removed.lock().unwrap().contains(&"desk".to_string())).await;
    until("desk knows it was removed, on its open stream", 5, || c.svc.node().shared().is_removed()).await;
    let st = c.state().await;
    assert_eq!(st["gateway"]["link"], "removed");
    assert!(st["gateway"]["reason"].as_str().unwrap().contains("by studio"), "{st}");
    assert!(c.svc.net().send(Outgoing { to: To::Host("laptop".into()), kind: Kind::Message, body: json!({}) }).is_err());
    assert!(b.svc.net().send(Outgoing { to: To::Host("desk".into()), kind: Kind::Message, body: json!({}) }).is_err());
    assert_eq!(b.state().await["hosts"].as_array().unwrap().len(), 2);
    // The founder can't be removed, from anywhere.
    let (s, v) = b.call("POST", "/api/net/remove", json!({ "host": "studio" }), true).await;
    assert_eq!(s, 403, "{v}");
    // studio founded the network, so it leaves only with --force.
    let (s, v) = a.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!((s, v["code"].clone()), (409, json!("needed")), "{v}");
    assert!(v["error"].as_str().unwrap().contains("--force"), "{v}");
    // desk leaves: it forgets the network, its files with it (its store
    // closed first, as Windows deletes no file that's open).
    let (s, v) = c.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(c.state().await["network"], Value::Null);
    let files = c.dir.join("state").join("network");
    assert!(["network.json", "host.key", "net.sqlite"].iter().all(|f| !files.join(f).exists()), "{}", files.display());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_time_comes_only_signed_by_the_gateway_and_frames_only_within_the_week() {
    let [port] = common::free_ports();
    let (a, b, c) = network("signed-time", port, Caps::default(), [0.0; 3]).await;
    let laptop = b.svc.node().shared().clone();
    // A refusal's time, unsigned, or signed by another key, or over another
    // request's nonce, isn't taken.
    assert!(laptop.clock_answer(&json!({ "code": "clock", "now": now_ms() + 3600e3 }), "bm9uY2Vub25jZQ").is_err());
    let other = xenonet_net::keys::HostKey::generate().unwrap();
    let net = laptop.network().unwrap();
    let draft =
        xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind: "ping", from: &net.gateway.host, to: "laptop", at: now_ms() as i64 };
    let forged = xenonet_net::frame::gateway_frame(&draft, &other, &json!({ "now": now_ms() + 3600e3, "nonce": "bm9uY2Vub25jZQ" }));
    let v = json!({ "code": "clock", "frame": forged.line().trim_end() });
    assert!(laptop.clock_answer(&v, "bm9uY2Vub25jZQ").unwrap_err().contains("didn't check"));
    // A time more than a day off isn't signed with: the link says why.
    laptop.learn_offset(now_ms() + 2.0 * 24.0 * 3600e3);
    let st = b.state().await;
    assert!(st["gateway"]["reason"].as_str().unwrap_or("").contains("more than a day off"), "{st}");
    assert!(laptop.gateway_now() - now_ms() < 60e3, "the offset stays as it was");
    // A host frame dated ten minutes ahead, or past the week frame ids are
    // kept, is dropped; one dated now is taken.
    let desk = c.svc.node().shared().clone();
    let desk_keys = *desk.key().unwrap().public();
    let key = laptop.key().unwrap();
    let frame = |at: f64| {
        let d = xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind: "message", from: "laptop", to: "desk", at: at as i64 };
        xenonet_net::frame::host_frame(&d, &key, &desk_keys, &json!({ "schema": "network/message/v1", "body": format!("at {at}") }), None).unwrap()
    };
    let now = now_ms();
    for at in [now + 10.0 * 60e3, now - 8.0 * 24.0 * 3600e3, now] {
        desk.frame(&frame(at));
    }
    assert_eq!(c.sink.texts("message"), [format!("at {now}")]);
    // An up batch's frames are 64 KiB at most each, but relay frames.
    let d =
        xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind: "message", from: "laptop", to: "desk", at: laptop.gateway_now() as i64 };
    let big = xenonet_net::frame::host_frame(&d, &key, &desk_keys, &json!({ "body": "x".repeat(70 << 10) }), None).unwrap();
    let batch = format!("{{\"frames\":[{}],\"ack\":null}}", big.line().trim_end());
    let (req, _) = laptop.signed_request("POST", "/net/v1/frames", batch.into_bytes()).unwrap();
    let answer = a.svc.node().gateway().unwrap().handle(req).await.body_json();
    assert_eq!(answer["refused"][0]["code"], "too-big", "{answer}");
    // The gateway remembers the ids of frames it keeps, not of presence.
    let gw = a.svc.node().gateway().unwrap();
    let send = |kind: &'static str| {
        let d = xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind, from: "laptop", to: "desk", at: laptop.gateway_now() as i64 };
        let f = xenonet_net::frame::host_frame(&d, &key, &desk_keys, &json!({ "schema": format!("network/{kind}/v1") }), None).unwrap();
        let batch = format!("{{\"frames\":[{}],\"ack\":null}}", f.line().trim_end());
        (f.id.clone(), laptop.signed_request("POST", "/net/v1/frames", batch.into_bytes()).unwrap().0)
    };
    let (presence, req) = send("presence");
    gw.handle(req).await;
    let (message, req) = send("message");
    gw.handle(req).await;
    assert_eq!((gw.remembers(&presence), gw.remembers(&message)), (false, true));
    // A host holds two down streams at most: a third ends the oldest.
    let open = || async {
        let (req, _) = laptop.signed_request("GET", &format!("/net/v1/link?epoch={}&after=0", gw.epoch()), Vec::new()).unwrap();
        match gw.handle(req).await.body {
            xenonet_net::gateway::NetBody::Stream(rx) => rx,
            xenonet_net::gateway::NetBody::Full(_) => panic!("a stream"),
        }
    };
    let mut first = open().await;
    let _second = open().await;
    let _third = open().await;
    let ended = tokio::time::timeout(Duration::from_secs(5), async { while first.recv().await.is_some() {} }).await;
    assert!(ended.is_ok(), "the oldest stream ended");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clock_five_minutes_off_still_links() {
    let [port] = common::free_ports();
    let (a, b, c) = network("clock", port, Caps::default(), [0.0, 300e3, -300e3]).await;
    for h in [&b, &c] {
        let skew = h.state().await["gateway"]["skewMs"].as_f64().unwrap();
        assert!((skew.abs() - 300e3).abs() < 5e3, "{skew}");
    }
    b.send("desk", Kind::Message, "ten minutes apart");
    until("the message", 10, || c.sink.texts("message") == ["ten minutes apart"]).await;
    let at = c.sink.got.lock().unwrap()[0].2.clone();
    assert_eq!(at["body"], "ten minutes apart");
    // The doctor reads it from the files: the skew, the link, the address.
    let checks = |h: &Host| -> std::collections::HashMap<String, Value> {
        xenonet_net::doctor::checks(&h.dir.join("state").join("network"), now_ms() + h.shift).into_iter().map(|(id, _, v)| (id.to_string(), v)).collect()
    };
    let b_checks = checks(&b);
    assert_eq!(b_checks["network.clock"]["status"], "warn", "{:?}", b_checks["network.clock"]);
    assert!(b_checks["network.clock"]["detail"].as_str().unwrap().contains("5 min"), "{:?}", b_checks["network.clock"]);
    assert_eq!(b_checks["network.link"]["status"], "ok");
    assert_eq!(b_checks["network.key"]["status"], "ok");
    assert_eq!(b_checks["network.files"]["status"], "ok", "{:?}", b_checks["network.files"]);
    let a_checks = checks(&a);
    assert_eq!(a_checks["network.address"]["status"], "info");
    assert!(a_checks["network.address"]["detail"].as_str().unwrap().contains("this computer alone"));
    assert_eq!(a_checks["network.link"]["detail"], "this machine is the network's gateway");
    // What the firewall check reads: a is the gateway, on this computer alone.
    let standing = |h: &Host| xenonet_net::doctor::standing(&h.dir.join("state").join("network"));
    assert_eq!((standing(&a)["gateway"].clone(), standing(&a)["kind"].clone()), (json!(true), json!("loopback")));
    assert_eq!(standing(&b)["gateway"], json!(false));
}

async fn until_state(h: &Host, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..300 {
        let s = h.state().await;
        if ok(&s) {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let s = h.state().await;
    assert!(ok(&s), "waited for {what}: {s}");
    s
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_gateway_moves_by_hand_and_a_host_that_missed_it_follows() {
    // desk's LAN address once it takes the gateway, and one nothing
    // answers on.
    let [port, desk_port, unused] = common::free_ports();
    let (desk_lan, nobody) = (format!("127.0.0.1:{desk_port}"), format!("127.0.0.1:{unused}"));
    let (a, b, c) = network("move", port, Caps::default(), [0.0; 3]).await;
    // laptop is away while the gateway moves.
    b.svc.stop();
    // desk asks to take it over, at its own LAN address: desk isn't the
    // founder, so it waits for approval, and the founder gives it.
    let (s, v) = c.call("POST", "/api/net/gateway/take", json!({ "lan": desk_lan }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["proposed"], true, "{v}");
    let st = until_state(&a, "the proposal at studio", |s| s["proposals"].as_array().is_some_and(|p| !p.is_empty())).await;
    assert_eq!((st["proposals"][0]["op"].clone(), st["proposals"][0]["host"].clone()), (json!("gateway"), json!("desk")));
    assert_eq!(st["proposals"][0]["signers"], json!(["desk"]));
    assert_eq!(st["proposals"][0]["gateway"], desk_lan);
    // Approved by its id: an id nothing waits under signs nothing.
    let (s, v) = a.call("POST", "/api/net/approve", json!({ "id": "bm90LXRoaXMtb25l" }), true).await;
    assert_eq!(s, 404, "{v}");
    let (s, v) = a.call("POST", "/api/net/approve", json!({ "id": st["proposals"][0]["id"] }), true).await;
    assert_eq!(s, 200, "{v}");
    // desk is the gateway now, studio links to it, and frames go through it.
    until_state(&c, "desk as the gateway", |s| s["self"]["gateway"] == true && s["gateway"]["link"] == "live").await;
    until_state(&a, "studio linked to desk", |s| s["self"]["gateway"] == false && s["gateway"]["address"] == desk_lan && s["gateway"]["link"] == "live").await;
    // The gateway's host can't be removed while it holds the gateway.
    let (s, v) = a.call("POST", "/api/net/remove", json!({ "host": "desk" }), true).await;
    assert_eq!(s, 403, "{v}");
    assert!(v["error"].as_str().unwrap().contains("move the gateway first"), "{v}");
    // Sent at once, while studio still answers the hosts it had: it waits in
    // studio's outbox for the new gateway.
    a.send("desk", Kind::Message, "through the new gateway");
    until("the message at desk", 10, || c.sink.texts("message") == ["through the new gateway"]).await;
    c.send("studio", Kind::Message, "and back");
    until("the answer at studio", 10, || a.sink.texts("message") == ["and back"]).await;
    // laptop comes back to studio's address, where nothing answers now,
    // and follows the gateway by hand.
    let b = b.restart().await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_ne!(b.state().await["gateway"]["link"], "live");
    let (s, v) = b.call("POST", "/api/net/gateway/set", json!({ "address": nobody }), true).await;
    assert_ne!(s, 200, "a wrong address isn't followed: {v}");
    let (s, v) = b.call("POST", "/api/net/gateway/set", json!({ "address": desk_lan }), true).await;
    assert_eq!(s, 200, "{v}");
    let st = until_state(&b, "laptop linked to desk", |s| s["gateway"]["link"] == "live" && s["gateway"]["address"] == desk_lan).await;
    assert_eq!(st["gateway"]["keyId"], c.state().await["self"]["keyId"]);
    b.send("studio", Kind::Message, "followed");
    until("laptop's message at studio", 10, || a.sink.texts("message").contains(&"followed".to_string())).await;
    // desk is the gateway now, so it leaves only with --force, or once
    // the gateway has moved on.
    let (s, v) = c.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!(s, 409, "{v}");
    assert!(v["error"].as_str().unwrap().contains("Move the gateway first, or leave with --force"), "{v}");
}

/// One HTTP/1.1 request by hand: its status.
fn raw(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> u16 {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut text = format!("{method} {path} HTTP/1.1\r\n");
    for (k, v) in headers {
        let _ = write!(text, "{k}: {v}\r\n");
    }
    let _ = write!(text, "Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    s.write_all(text.as_bytes()).unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_lan_door_checks_its_host_and_bad_tickets_get_404() {
    let [port] = common::free_ports();
    let a = Host::new("lan", "studio", 0.0, Caps::default()).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{port}") }), true).await;
    assert_eq!(s, 200, "{v}");
    let host = format!("127.0.0.1:{port}");
    let ok = std::thread::spawn(move || {
        let h = host.as_str();
        let join =
            r#"{"sign":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","seal":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","nonce":"bm9uY2Vub25jZW5vbmNl","proof":"x","sig":"y"}"#;
        vec![
            raw(port, "GET", "/net/v1/roster", &[("Host", "evil.example")], ""),
            raw(port, "GET", "/net/v1/roster", &[("Host", h), ("Origin", "http://evil.example")], ""),
            raw(port, "POST", "/mcp", &[("Host", h)], "{}"),
            raw(port, "POST", "/a2a", &[("Host", h)], "{}"),
            raw(port, "GET", "/.well-known/agent-card.json", &[("Host", h)], ""),
            raw(port, "GET", "/net/v1/roster", &[("Host", h)], ""),
            raw(port, "POST", "/net/v1/join", &[("Host", h)], join),
        ]
    })
    .join()
    .unwrap();
    assert_eq!(ok, [421, 403, 404, 404, 404, 401, 404], "a wrong host, a web page, /mcp, /a2a and the card on a LAN, unsigned, a bad ticket");
    // A real ticket works only until 20 bad ones shut the route for an hour.
    let (_, t) = a.call("POST", "/api/net/ticket", json!({ "host": "laptop" }), true).await;
    let ticket = t["ticket"].as_str().unwrap().to_string();
    let host = format!("127.0.0.1:{port}");
    std::thread::spawn(move || {
        let join =
            r#"{"sign":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","seal":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","nonce":"bm9uY2Vub25jZW5vbmNl","proof":"x","sig":"y"}"#;
        for _ in 0..20 {
            assert_eq!(raw(port, "POST", "/net/v1/join", &[("Host", host.as_str())], join), 404);
        }
    })
    .join()
    .unwrap();
    let b = Host::new("lan", "laptop", 0.0, Caps::default()).await;
    let (s, v) = b.call("POST", "/api/net/join", json!({ "address": format!("127.0.0.1:{port}"), "ticket": ticket }), true).await;
    assert_eq!(s, 404, "the route is shut to this source: {v}");
    // Only to that source: the same ticket works from another.
    let gw = a.svc.node().gateway().unwrap();
    let code = xenonet_net::ticket::normalize(&ticket).unwrap();
    let join = |code: &str, source: &str| {
        let key = xenonet_net::keys::HostKey::generate().unwrap();
        let body = xenonet_net::gateway::join_body(&key, code, "bm9uY2Vub25jZW5vbmNlMTIz", "linux", "0", false);
        xenonet_net::gateway::NetRequest {
            method: "POST".into(),
            target: "/net/v1/join".into(),
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
            source: Some(source.into()),
        }
    };
    assert_eq!(gw.handle(join(&code, "198.51.100.7")).await.status, 200, "another source joins");
    // Five wrong tries aimed at a ticket (by its tag) retire it.
    let (_, t) = a.call("POST", "/api/net/ticket", json!({ "host": "desk" }), true).await;
    let desk = xenonet_net::ticket::normalize(t["ticket"].as_str().unwrap()).unwrap();
    let tag = xenonet_net::ticket::tag(&xenonet_net::ticket::hash(&desk));
    for i in 0..5 {
        let mut wrong = join("0000000000", &format!("203.0.113.{i}"));
        let mut body: Value = serde_json::from_slice(&wrong.body).unwrap();
        body["tag"] = json!(tag);
        wrong.body = body.to_string().into_bytes();
        assert_eq!(gw.handle(wrong).await.status, 404);
    }
    assert_eq!(gw.handle(join(&desk, "203.0.113.9")).await.status, 404, "retired");
    // 200 bad tickets in an hour, from anywhere, shut it for everyone, and
    // the state and the doctor say until when.
    assert_eq!(a.state().await["joinClosedUntil"], Value::Null);
    for i in 0..200 {
        gw.handle(join("0000000000", &format!("192.0.2.{}", i % 250))).await;
    }
    let st = a.state().await;
    assert!(st["joinClosedUntil"].as_f64().is_some_and(|t| t > now_ms()), "{st}");
    a.svc.node().shared().write_link(true);
    let doctor: std::collections::HashMap<String, Value> =
        xenonet_net::doctor::checks(&a.dir.join("state").join("network"), now_ms()).into_iter().map(|(id, _, v)| (id.to_string(), v)).collect();
    assert_eq!(doctor["network.join"]["status"], "warn", "{:?}", doctor["network.join"]);
    // The routes are the owner's: an agent's process is refused.
    let mut cores = Cores::pending(false, false, false);
    cores.verify = Arc::new(You { person: json!({ "ok": false, "reason": "it came from under a Claude Code process" }) });
    let ctx = Arc::new(Ctx::new(Config::from(&[], &|_| None), "0".into(), cores, Log::new(None, "service", None, None, "/", 1 << 20)));
    let _ = ctx.net.set(b.svc.clone());
    let r = api(&ctx, Req::fake("POST", "/api/net/create", &[], Some("{}"))).await.unwrap();
    assert_eq!((r.status, r.body_json()["code"].clone()), (403, json!("yours")));
    let r = api(&ctx, Req::fake("GET", "/api/net/state", &[], None)).await.unwrap();
    assert_eq!(r.status, 403);
    // A ticket from a terminal is macOS's alone, and the person's there.
    let r = api(&ctx, Req::fake("POST", "/api/net/ticket", &[], Some("{\"host\":\"x1\"}"))).await.unwrap();
    assert_eq!(r.status, 403);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_founder_alone_leaves_only_with_force_and_its_lan_door_closes() {
    // As `xenonet net leave` does it from the person's terminal
    // (crates/xenonet-cli/tests/net.rs, which only gets this far where
    // it isn't run from under an agent).
    let [port] = common::free_ports();
    let a = Host::new("alone", "studio", 0.0, Caps::default()).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{port}") }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["self"]["founder"].clone(), v["self"]["gateway"].clone()), (json!(true), json!(true)));
    let host = format!("127.0.0.1:{port}");
    let door = move || raw(port, "GET", "/net/v1/roster", &[("Host", host.as_str())], "");
    let unsigned = door.clone();
    assert_eq!(tokio::task::spawn_blocking(unsigned).await.unwrap(), 401, "the LAN door is open");
    // Alone, the founder's leaving ends the network: --force, or nothing.
    let (s, v) = a.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!((s, v["code"].clone()), (409, json!("needed")), "{v}");
    let why = v["error"].as_str().unwrap();
    assert!(why.contains("--force") && why.contains("the network ends"), "{why}");
    assert_eq!(a.state().await["network"]["name"], "home", "still in it");
    assert_eq!(tokio::task::spawn_blocking(door).await.unwrap(), 401, "the LAN door is still open");
    let (s, v) = a.call("POST", "/api/net/leave", json!({ "force": true }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["left"].clone(), v["removed"].clone(), v["founder"].clone()), (json!("home"), json!(false), json!(true)), "the founder never asks to be removed");
    assert_eq!(a.state().await["network"], Value::Null);
    let files = a.dir.join("state").join("network");
    assert!(["network.json", "host.key", "net.sqlite", "gateway.sqlite"].iter().all(|f| !files.join(f).exists()), "{}", files.display());
    until("the LAN door closed", 10, || std::net::TcpStream::connect(("127.0.0.1", port)).is_err()).await;
}

/// What a relay sink heard: each body's sender and whether it's a window
/// host, and the roster's news.
#[derive(Default)]
struct Relays {
    got: Mutex<Vec<(String, bool, Value)>>,
    removed: Mutex<Vec<String>>,
}

impl xenonet_base::network::RelaySink for Relays {
    fn relay(&self, i: Incoming) {
        assert_eq!(i.kind, Kind::Relay);
        self.got.lock().unwrap().push((i.from, i.from_window_host, i.body));
    }
    fn removed(&self, host: &str) {
        self.removed.lock().unwrap().push(host.into());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_maps_relay_frames_cross_in_batches_and_a_bucket_of_their_own() {
    let [port] = common::free_ports();
    let (a, b, c) = network("relay", port, Caps::default(), [0.0; 3]).await;
    let heard = Arc::new(Relays::default());
    b.svc.sinks().set_relay("hear", heard.clone());
    // desk sends laptop the map: it reaches laptop's relay sink, never its
    // engine, and so does the gateway's host's.
    let relay = |h: &Host, to: &str, n: u64| {
        h.svc.net().send(Outgoing { to: To::Host(to.into()), kind: Kind::Relay, body: json!({ "sub": "s1", "seq": n, "frames": [{ "t": "ping", "now": n }] }) })
    };
    relay(&c, "laptop", 1).unwrap();
    relay(&a, "laptop", 2).unwrap();
    until("both at laptop's relay", 10, || heard.got.lock().unwrap().len() == 2).await;
    let mut got: Vec<(String, bool, u64)> = heard.got.lock().unwrap().iter().map(|(f, w, b)| (f.clone(), *w, b["seq"].as_u64().unwrap())).collect();
    got.sort();
    assert_eq!(got, [("desk".to_string(), false, 1), ("studio".to_string(), true, 2)], "with whether the sender is a window host");
    assert!(b.sink.got.lock().unwrap().iter().all(|g| g.1 != "relay"), "never the engine's");
    // A relay body is 8 MiB at most, before it's sealed.
    let big = json!({ "sub": "s1", "seq": 3, "frames": [{ "t": "snapshot", "graph": "x".repeat(9 << 20) }] });
    assert!(c.svc.net().send(Outgoing { to: To::Host("laptop".into()), kind: Kind::Relay, body: big }).unwrap_err().contains("8 MiB"));

    // At the gateway: relay frames only in a relay batch, and nothing else
    // in one.
    let gw = a.svc.node().gateway().unwrap();
    let desk = c.svc.node().shared();
    let net = desk.network().unwrap();
    let key = desk.key().unwrap();
    let laptop_keys = desk.roster().view().member("laptop").unwrap().keys;
    let frame = |kind: &'static str| {
        let d = xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind, from: "desk", to: "laptop", at: desk.gateway_now() as i64 };
        xenonet_net::frame::host_frame(&d, &key, &laptop_keys, &json!({ "sub": "s1" }), None).unwrap()
    };
    let post = |frames: &[xenonet_net::frame::Frame], relay: bool| {
        let lines: Vec<String> = frames.iter().map(|f| f.line().trim_end().to_string()).collect();
        let batch = format!("{{\"frames\":[{}],\"ack\":null}}", lines.join(","));
        desk.signed_request("POST", if relay { "/net/v1/frames?relay=1" } else { "/net/v1/frames" }, batch.into_bytes()).unwrap().0
    };
    let answer = gw.handle(post(&[frame("relay")], false)).await.body_json();
    assert_eq!(answer["refused"][0]["code"], "bad-kind", "{answer}");
    let answer = gw.handle(post(&[frame("message")], true)).await.body_json();
    assert_eq!(answer["refused"][0]["code"], "bad-kind", "{answer}");
    // 120 relay frames a minute, in a bucket of their own: the engine's
    // frames still go once it's spent.
    let many: Vec<_> = (0..130).map(|_| frame("relay")).collect();
    let answer = gw.handle(post(&many, true)).await.body_json();
    let accepted = answer["accepted"].as_array().unwrap().len();
    let rated = answer["refused"].as_array().unwrap().iter().filter(|r| r["code"] == "rate").count();
    assert!((110..=120).contains(&accepted) && accepted + rated == 130, "{accepted} taken, {rated} held back: {answer}");
    assert!(answer["retryAfterMs"].as_f64().is_some_and(|ms| ms > 0.0), "{answer}");
    let answer = gw.handle(post(&[frame("message")], false)).await.body_json();
    assert_eq!(answer["accepted"].as_array().unwrap().len(), 1, "the engine's bucket is untouched: {answer}");

    // A host removed: every relay sink hears of it.
    let (s, v) = a.call("POST", "/api/net/remove", json!({ "host": "desk" }), true).await;
    assert_eq!(s, 200, "{v}");
    until("laptop's relay hears desk went", 10, || heard.removed.lock().unwrap().contains(&"desk".to_string())).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_and_svc_frames_never_touch_the_gateways_store() {
    // A svc frame may carry a service's own credential, sealed: the
    // gateway hands relay and svc frames to the host's link in memory, and
    // drops them when the host isn't linked, rather than writing them to
    // its store, where a stream that ended before its ack would leave them
    // for a week.
    let [port] = common::free_ports();
    let (a, b, c) = network("held", port, Caps::default(), [0.0; 3]).await;
    let heard = Arc::new(Relays::default());
    b.svc.sinks().set_relay("hear", heard.clone());
    let gw = a.svc.node().gateway().unwrap();
    let desk = c.svc.node().shared();
    let net = desk.network().unwrap();
    let key = desk.key().unwrap();
    let laptop_keys = desk.roster().view().member("laptop").unwrap().keys;
    let frame = |kind: &'static str, seq: u64| {
        let d = xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind, from: "desk", to: "laptop", at: desk.gateway_now() as i64 };
        xenonet_net::frame::host_frame(&d, &key, &laptop_keys, &json!({ "sub": "s1", "seq": seq, "frames": [] }), None).unwrap()
    };
    let post = |frames: &[xenonet_net::frame::Frame]| {
        let lines: Vec<String> = frames.iter().map(|f| f.line().trim_end().to_string()).collect();
        let batch = format!("{{\"frames\":[{}],\"ack\":null}}", lines.join(","));
        desk.signed_request("POST", "/net/v1/frames?relay=1", batch.into_bytes()).unwrap().0
    };
    let stored = || {
        let db = rusqlite::Connection::open_with_flags(a.dir.join("state").join("network").join("gateway.sqlite"), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        db.query_row("SELECT count(*) FROM queue WHERE kind IN ('relay', 'svc')", [], |r| r.get::<_, i64>(0)).unwrap()
    };
    let answer = gw.handle(post(&[frame("relay", 1), frame("svc", 0), frame("relay", 2)])).await.body_json();
    assert_eq!(answer["accepted"].as_array().unwrap().len(), 3, "{answer}");
    assert_eq!(stored(), 0, "nothing in the store");
    until("laptop's link read what was held for it", 10, || heard.got.lock().unwrap().len() == 2).await;
    assert_eq!(gw.held("laptop"), 0);

    // laptop's link goes: what comes for it is dropped, never kept.
    b.svc.stop();
    until("laptop unlinked at the gateway", 10, || !gw.linked(now_ms()).contains(&"laptop".to_string())).await;
    let answer = gw.handle(post(&[frame("svc", 0), frame("relay", 3)])).await.body_json();
    assert_eq!(answer["accepted"].as_array().unwrap().len(), 2, "taken, as live frames are: {answer}");
    assert_eq!((gw.held("laptop"), stored()), (0, 0), "and dropped");
}

/// What the firewall between laptop and the gateway does.
const PASS: u8 = 0;
/// Takes each connection and answers nothing, as a firewall that blocks
/// the gateway's process leaves it once the kernel took the connection; a
/// connection that was passing goes quiet for good.
const SILENT: u8 = 1;
/// Answers a new connection with a stream's headers, then nothing.
const HEADERS: u8 = 2;

/// A path to the gateway that a test can block. While it passes, `Host`
/// is changed to the gateway's own address, which its LAN door wants; the
/// two ports have as many digits, so nothing else moves.
struct Firewall {
    mode: Arc<AtomicU8>,
    task: tokio::task::JoinHandle<()>,
}

impl Firewall {
    async fn new(port: u16, gateway: u16) -> Firewall {
        assert_eq!(port.to_string().len(), gateway.to_string().len());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
        let mode = Arc::new(AtomicU8::new(PASS));
        let m = mode.clone();
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(Firewall::connection(client, m.clone(), port, gateway));
            }
        });
        Firewall { mode, task }
    }

    fn set(&self, mode: u8) {
        self.mode.store(mode, Ordering::SeqCst);
    }

    async fn connection(mut client: tokio::net::TcpStream, mode: Arc<AtomicU8>, port: u16, gateway: u16) {
        let mut buf = vec![0u8; 64 << 10];
        let now = mode.load(Ordering::SeqCst);
        if now != PASS {
            if now == HEADERS {
                let mut head = Vec::new();
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match client.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let _ = client.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/x-ndjson\r\ntransfer-encoding: chunked\r\n\r\n").await;
            }
            // Held open until the host gives up on it.
            while matches!(client.read(&mut buf).await, Ok(n) if n > 0) {}
            return;
        }
        let Ok(upstream) = tokio::net::TcpStream::connect(("127.0.0.1", gateway)).await else { return };
        let (mut cr, mut cw) = client.into_split();
        let (mut ur, mut uw) = upstream.into_split();
        let (from, to) = (format!("127.0.0.1:{port}").into_bytes(), format!("127.0.0.1:{gateway}").into_bytes());
        // Once either way sees the firewall shut, the connection passes
        // nothing more, either way.
        let shut = AtomicBool::new(false);
        let closed = || {
            if mode.load(Ordering::SeqCst) != PASS {
                shut.store(true, Ordering::SeqCst);
            }
            shut.load(Ordering::SeqCst)
        };
        let up = async {
            let mut buf = vec![0u8; 64 << 10];
            while let Ok(n @ 1..) = cr.read(&mut buf).await {
                if closed() {
                    continue;
                }
                let mut chunk = buf[..n].to_vec();
                let mut i = 0;
                while i + from.len() <= chunk.len() {
                    if chunk[i..i + from.len()] == from[..] {
                        chunk[i..i + from.len()].copy_from_slice(&to);
                    }
                    i += 1;
                }
                if uw.write_all(&chunk).await.is_err() {
                    break;
                }
            }
        };
        let down = async {
            let mut buf = vec![0u8; 64 << 10];
            while let Ok(n @ 1..) = ur.read(&mut buf).await {
                if !closed() && cw.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        };
        // Either side's end ends both: the gateway sees the host give up.
        tokio::select! {
            () = up => {}
            () = down => {}
        }
    }
}

impl Drop for Firewall {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_gateway_that_takes_connections_and_says_nothing_puts_the_link_offline() {
    // studio is the gateway on `lan`. desk links to it; laptop links
    // through a firewall on `path`, and sends its presence each time its
    // link comes up, as its engine would. The firewall swaps one address
    // for the other in place, so the two ports need as many digits; the
    // OS's nearly always have five.
    let [lan, path] = loop {
        let [lan, path] = common::free_ports();
        if lan.to_string().len() == path.to_string().len() {
            break [lan, path];
        }
    };
    let a = Host::new("silent", "studio", 0.0, Caps::default()).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{lan}") }), true).await;
    assert_eq!(s, 200, "{v}");
    let fw = Firewall::new(path, lan).await;
    let mut joined = Vec::new();
    for (name, port) in [("laptop", path), ("desk", lan)] {
        let (s, t) = a.call("POST", "/api/net/ticket", json!({ "host": name }), true).await;
        assert_eq!(s, 200, "{t}");
        let h = Host::new("silent", name, 0.0, Caps::default()).await;
        if name == "laptop" {
            *h.sink.presence.lock().unwrap() = Some(Arc::downgrade(&h.svc.net()));
        }
        let (s, v) = h.call("POST", "/api/net/join", json!({ "address": format!("127.0.0.1:{port}"), "ticket": t["ticket"] }), true).await;
        assert_eq!(s, 200, "{v}");
        joined.push(h);
    }
    let (c, b) = (joined.pop().unwrap(), joined.pop().unwrap());
    for h in [&a, &b, &c] {
        live(h, 3).await;
    }
    let online = |h: &Host, other: &str| h.svc.net().hosts().iter().any(|x| x.id == other && x.online);
    // Its first went to studio alone: desk joined after.
    until("laptop's presence at studio", 10, || a.sink.presence_from("laptop") == [1]).await;
    until("each side sees the other online", 10, || online(&b, "studio") && online(&b, "desk") && online(&a, "laptop") && online(&c, "laptop")).await;
    let was = b.state().await["gateway"]["since"].as_f64().unwrap();

    // The firewall blocks the gateway's process: laptop's stream goes
    // quiet, and its new requests are taken and never answered. Its link
    // is offline within the idle time, and says why once a request has
    // waited out its answer time (2.5 s and 4 s here).
    let blocked = std::time::Instant::now();
    fw.set(SILENT);
    until_state(&b, "laptop offline", |s| s["gateway"]["link"] == "offline").await;
    assert!(blocked.elapsed() < Duration::from_secs(6), "offline after {:?}", blocked.elapsed());
    let st = until_state(&b, "the reason", |s| s["gateway"]["reason"].as_str().is_some_and(|r| r.contains("didn't answer in 4 s"))).await;
    assert!(blocked.elapsed() < Duration::from_secs(12), "said why after {:?}", blocked.elapsed());
    assert_eq!(st["gateway"]["link"], "offline", "{st}");
    let since = st["gateway"]["since"].as_f64().unwrap();
    assert!(since > was, "since it went offline: {st}");
    assert!(!b.sink.up(), "the engine heard the link went down");
    // The doctor reads it from link.json.
    let doctor: std::collections::HashMap<String, Value> =
        xenonet_net::doctor::checks(&b.dir.join("state").join("network"), now_ms()).into_iter().map(|(id, _, v)| (id.to_string(), v)).collect();
    assert_eq!(doctor["network.link"]["status"], "warn", "{:?}", doctor["network.link"]);
    assert!(doctor["network.link"]["detail"].as_str().unwrap().contains("didn't answer"), "{:?}", doctor["network.link"]);
    // The gateway saw laptop give up on its stream: the others see it
    // offline, and laptop sees them offline too.
    until("laptop offline at studio and desk", 15, || !online(&a, "laptop") && !online(&c, "laptop")).await;
    assert!(!online(&b, "studio") && !online(&b, "desk"));

    // It answers a stream's headers, then nothing: the hello's time runs
    // out the same way. The tries in between keep the state and its since.
    fw.set(HEADERS);
    let st = until_state(&b, "no hello", |s| s["gateway"]["reason"].as_str().is_some_and(|r| r.contains("didn't say hello in 4 s"))).await;
    assert_eq!((st["gateway"]["link"].clone(), st["gateway"]["since"].as_f64()), (json!("offline"), Some(since)), "{st}");

    // The firewall lets it through again: laptop links up, its presence
    // arrives, and each side sees the other online.
    fw.set(PASS);
    let st = until_state(&b, "laptop live again", |s| s["gateway"]["link"] == "live").await;
    assert_eq!(st["gateway"]["reason"], Value::Null, "{st}");
    assert!(b.sink.up());
    until("laptop's second presence at studio and desk", 10, || a.sink.presence_from("laptop").contains(&2) && c.sink.presence_from("laptop").contains(&2)).await;
    until("each side sees the other online again", 10, || online(&b, "studio") && online(&b, "desk") && online(&a, "laptop") && online(&c, "laptop")).await;
}
