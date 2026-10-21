// One network across machines, end to end (docs/design/network.md): three
// services in this process, each with its own state folder, the first a
// LAN-only gateway on 127.0.0.1, the others linking to it over HTTP as
// hosts on other machines would. Their /api/net routes are called as the
// window calls them, and a test Sink stands in for the engine.
//
// Each test has its own ports among 48113-48119 (48112 is one nothing
// answers on), never 47840-47842. NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::{Read, Write as _};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_base::log::Log;
use xenonet_base::network::{Incoming, Kind, Outgoing, Sink, To};
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
}

impl Sink for TestSink {
    fn receive(&self, i: Incoming) {
        self.window.lock().unwrap().push((i.from.clone(), i.from_window_host));
        self.got.lock().unwrap().push((i.from, i.kind.as_str().to_string(), i.body));
    }
    fn linked(&self, up: bool) {
        self.links.lock().unwrap().push(up);
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
    Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 }
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
    let (a, b, c) = network("restart", 48113, Caps::default(), [0.0; 3]).await;
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
    let (a, b, c) = network("gap", 48115, caps, [0.0; 3]).await;
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
    // desk leaves: it forgets the network.
    let (s, v) = c.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(c.state().await["network"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_time_comes_only_signed_by_the_gateway_and_frames_only_within_the_week() {
    let (_a, b, c) = network("signed-time", 48114, Caps::default(), [0.0; 3]).await;
    let laptop = b.svc.node().shared().clone();
    // A refusal's time, unsigned, or signed by another key, or over another
    // request's nonce, isn't taken.
    assert!(laptop.clock_answer(&json!({ "code": "clock", "now": now_ms() + 3600e3 }), "bm9uY2Vub25jZQ").is_err());
    let other = xenonet_net::keys::HostKey::generate().unwrap();
    let net = laptop.network().unwrap();
    let draft = xenonet_net::frame::Draft { network: &net.id, id: xenonet_net::codec::frame_id().unwrap(), kind: "ping", from: &net.gateway.host, to: "laptop", at: now_ms() as i64 };
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_clock_five_minutes_off_still_links() {
    let (a, b, c) = network("clock", 48117, Caps::default(), [0.0, 300e3, -300e3]).await;
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
    let (a, b, c) = network("move", 48116, Caps::default(), [0.0; 3]).await;
    // laptop is away while the gateway moves.
    b.svc.stop();
    // desk asks to take it over, at its own LAN address: desk isn't the
    // founder, so it waits for approval, and the founder gives it.
    let (s, v) = c.call("POST", "/api/net/gateway/take", json!({ "lan": "127.0.0.1:48118" }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["proposed"], true, "{v}");
    let st = until_state(&a, "the proposal at studio", |s| s["proposals"].as_array().is_some_and(|p| !p.is_empty())).await;
    assert_eq!((st["proposals"][0]["op"].clone(), st["proposals"][0]["host"].clone()), (json!("gateway"), json!("desk")));
    assert_eq!(st["proposals"][0]["signers"], json!(["desk"]));
    assert_eq!(st["proposals"][0]["gateway"], "127.0.0.1:48118");
    // Approved by its id: an id nothing waits under signs nothing.
    let (s, v) = a.call("POST", "/api/net/approve", json!({ "id": "bm90LXRoaXMtb25l" }), true).await;
    assert_eq!(s, 404, "{v}");
    let (s, v) = a.call("POST", "/api/net/approve", json!({ "id": st["proposals"][0]["id"] }), true).await;
    assert_eq!(s, 200, "{v}");
    // desk is the gateway now, studio links to it, and frames go through it.
    until_state(&c, "desk as the gateway", |s| s["self"]["gateway"] == true && s["gateway"]["link"] == "live").await;
    until_state(&a, "studio linked to desk", |s| s["self"]["gateway"] == false && s["gateway"]["address"] == "127.0.0.1:48118" && s["gateway"]["link"] == "live").await;
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
    let (s, v) = b.call("POST", "/api/net/gateway/set", json!({ "address": "127.0.0.1:48112" }), true).await;
    assert_ne!(s, 200, "a wrong address isn't followed: {v}");
    let (s, v) = b.call("POST", "/api/net/gateway/set", json!({ "address": "127.0.0.1:48118" }), true).await;
    assert_eq!(s, 200, "{v}");
    let st = until_state(&b, "laptop linked to desk", |s| s["gateway"]["link"] == "live" && s["gateway"]["address"] == "127.0.0.1:48118").await;
    assert_eq!(st["gateway"]["keyId"], c.state().await["self"]["keyId"]);
    b.send("studio", Kind::Message, "followed");
    until("laptop's message at studio", 10, || a.sink.texts("message").contains(&"followed".to_string())).await;
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
    let port = 48119;
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
    assert_eq!(s, 404, "the route is shut: {v}");
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
