// One network across machines, end to end (docs/design/network.md): three
// services in this process, each with its own state folder and its own
// iroh endpoint on loopback, joined with invites over join/2 and talking
// over link/2 alone, as machines on a LAN would. Their /api/net routes are
// called as the window calls them, and a test Sink stands in for the
// engine.
//
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use xenonet_api::cores::{self, Cores};
use xenonet_model::ops::{Incoming, Kind, Net, Outgoing, Sink, To};
use xenonet_net::gwstore::Caps;
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_os::log::Log;
use xenonet_service::config::Config;
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
                net.send(Outgoing { network: None, authority: None, to: To::All, kind: Kind::Presence, body }).unwrap();
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
        common::kept(&dir.join("state")),
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
    // Fake keys: a test never touches the machine's own, or prompts.
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    o.caps = caps;
    // join/2's endpoints on loopback alone, and link/2's.
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o.link = true;
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
        let mut cores = Cores::pending(false, false);
        xenonet_service::serve::wire_root(&mut cores);
        cores.verify = Arc::new(You { person: json!({ "ok": true }) });
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let svc = NetService::open(options(&dir, shift, caps)).unwrap();
        let sink = Arc::new(TestSink::default());
        svc.sinks().set(sink.clone());
        let _ = ctx.net.set(svc.clone());
        svc.start().await.unwrap();
        Host { ctx, svc, sink, dir, shift, caps }
    }

    /// What this machine keeps of its network, as the doctor reads it.
    fn kept(&self) -> xenonet_net::files::Dir {
        xenonet_net::files::Dir::new(self.dir.join("state").join("network"), common::kept(&self.dir.join("state")))
    }

    /// The roster's entries this machine keeps, as JSON.
    fn roster(&self) -> Vec<Value> {
        let kept = self.kept();
        let net = kept.read_network().unwrap().expect("in a network");
        kept.persist.roster(&net.id).unwrap().iter().map(xenonet_net::roster::Entry::to_json).collect()
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
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
        (r.status, r.body_json())
    }

    async fn state(&self) -> Value {
        self.call("GET", "/api/net/state", Value::Null, true).await.1
    }

    fn send(&self, to: &str, kind: Kind, text: &str) {
        let body = json!({ "schema": format!("network/{}/v1", kind.as_str()), "body": text });
        self.svc.net().send(Outgoing { network: None, authority: None, to: To::Host(to.into()), kind, body }).unwrap();
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

/// Waits for `h` to have `hosts` members, every one online.
async fn online(h: &Host, hosts: usize) {
    let mut ok = false;
    for _ in 0..600 {
        let s = h.state().await;
        if s["hosts"].as_array().map_or(0, Vec::len) == hosts && s["hosts"].as_array().unwrap().iter().all(|h| h["online"] == true) {
            ok = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ok, "never {hosts} hosts all online: {}", h.state().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_hosts_message_both_ways_and_go_on_with_one_away() {
    let (a, b, c) = network("restart").await;
    // Every host's roster agrees, and each sees the others online.
    let heads: Vec<Value> = [a.state().await, b.state().await, c.state().await].iter().map(|s| s["rosterHead"].clone()).collect();
    assert!(heads.iter().all(|h| *h == heads[0] && !h.is_null()), "{heads:?}");
    until("desk online at laptop", 10, || b.svc.net().hosts().iter().any(|h| h.id == "desk" && h.online)).await;
    assert!(b.sink.up() && c.sink.up());

    // Messages both ways, between members and with the founder.
    b.send("desk", Kind::Message, "m1 from laptop");
    until("m1 at desk", 10, || c.sink.texts("message") == ["m1 from laptop"]).await;
    c.send("laptop", Kind::Receipt, "stored m1");
    until("the receipt at laptop", 10, || c.sink.texts("message").len() == 1 && b.sink.texts("receipt") == ["stored m1"]).await;
    a.send("laptop", Kind::Message, "from the founder");
    b.send("studio", Kind::Message, "to the founder");
    until("both", 10, || b.sink.texts("message") == ["from the founder"] && a.sink.texts("message") == ["to the founder"]).await;
    // The engine is told whether the sender is a window host, from the
    // chain: studio founded the network and laptop is one; desk isn't.
    assert!(c.sink.window.lock().unwrap().iter().all(|(from, w)| *w == (from == "studio" || from == "laptop")), "{:?}", c.sink.window.lock().unwrap());
    assert_eq!(*a.sink.window.lock().unwrap(), [("laptop".to_string(), true)]);
    // Presence goes to every host.
    b.svc.net().send(Outgoing { network: None, authority: None, to: To::All, kind: Kind::Presence, body: json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [{ "id": "claude:1", "waitsOnYou": true }], "removed": [], "you": false, "roster": "x" }) }).unwrap();
    until("presence", 10, || {
        a.sink.got.lock().unwrap().iter().any(|g| g.1 == "presence" && g.0 == "laptop") && c.sink.got.lock().unwrap().iter().any(|g| g.1 == "presence" && g.0 == "laptop")
    })
    .await;
    let s = a.state().await;
    let laptop = host_in(&s, "laptop");
    assert_eq!((laptop["agents"].clone(), laptop["waitsOnYou"].clone()), (json!(1), json!(1)));
    assert_eq!(s["rosterMismatch"], json!(["laptop"]), "its presence named another head");
    // laptop's own row counts the agents its own presence lists.
    let me = b.state().await["hosts"].as_array().unwrap().iter().find(|h| h["self"] == true).unwrap().clone();
    assert_eq!((me["id"].clone(), me["agents"].clone(), me["waitsOnYou"].clone()), (json!("laptop"), json!(1), json!(1)), "{me}");
    b.svc
        .net()
        .send(Outgoing {
            network: None,
            authority: None,
            to: To::All,
            kind: Kind::Presence,
            body: json!({ "schema": "network/presence/v1", "counter": 2, "full": false, "agents": [{ "id": "claude:2" }], "removed": ["claude:1"], "you": false, "roster": "x" }),
        })
        .unwrap();
    let me = b.state().await["hosts"].as_array().unwrap().iter().find(|h| h["self"] == true).unwrap().clone();
    assert_eq!((me["agents"].clone(), me["waitsOnYou"].clone()), (json!(1), json!(0)), "a change, and one gone: {me}");

    // The founder goes: laptop and desk go on between themselves, since
    // nothing passes through it. Its presence ends 30 s after its
    // connection went, and it's offline at once.
    let a = {
        a.svc.stop();
        until("studio offline at laptop", 15, || b.svc.net().hosts().iter().any(|h| h.id == "studio" && !h.online)).await;
        b.send("desk", Kind::Message, "m2 with studio away");
        until("m2 at desk", 10, || c.sink.texts("message").len() == 2).await;
        b.send("studio", Kind::Message, "m3 for studio, which is away");
        a.restart().await
    };
    // Back: the message that waited for it comes, once.
    until("m3 at studio", 20, || a.sink.texts("message").contains(&"m3 for studio, which is away".to_string())).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(c.sink.texts("message"), ["m1 from laptop", "m2 with studio away"], "no loss and no repeats");
    assert_eq!(a.sink.texts("message").iter().filter(|t| t.starts_with("m3")).count(), 1);
    assert_eq!(*b.sink.gaps.lock().unwrap() + *c.sink.gaps.lock().unwrap(), 0, "no gap: nothing was lost");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_founder_alone_leaves_only_with_force() {
    // As `xenonet net leave` does it from the person's terminal
    // (crates/xenonet-cli/tests/net.rs, which only gets this far where
    // it isn't run from under an agent).
    let a = Host::new("alone", "studio", 0.0, Caps::default()).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio" }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["self"]["founder"], true);
    // Alone, the founder's leaving ends the network: --force, or nothing.
    let (s, v) = a.call("POST", "/api/net/leave", json!({}), true).await;
    assert_eq!((s, v["code"].clone()), (409, json!("needed")), "{v}");
    let why = v["error"].as_str().unwrap();
    assert!(why.contains("--force") && why.contains("the network ends"), "{why}");
    assert_eq!(a.state().await["network"]["name"], "home", "still in it");
    let (s, v) = a.call("POST", "/api/net/leave", json!({ "force": true }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["left"].clone(), v["removed"].clone(), v["founder"].clone()), (json!("home"), json!(false), json!(true)), "the founder never asks to be removed");
    assert_eq!(a.state().await["network"], Value::Null);
    let files = a.dir.join("state").join("network");
    assert!(!files.join("host.key").exists(), "{}", files.display());
    assert!(a.kept().read_network().unwrap().is_none(), "the network is forgotten");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_goes_names_its_network_and_presence_shows_only_what_is_shared_into_it() {
    let (a, b, _c) = network("sharing").await;
    let id = b.state().await["network"]["id"].as_str().unwrap().to_string();
    let mine = b.svc.net().networks();
    assert_eq!((mine.len(), mine[0].network.as_str(), mine[0].host.as_str()), (1, id.as_str(), "laptop"));
    // A body for a network this machine isn't in is refused, never sent in
    // the one it is in.
    let other = Outgoing::new(To::Host("studio".into()), Kind::Message, json!({ "schema": "network/message/v1", "body": "x" })).within("net_ffffffffffffffffffffffffffffffff");
    assert!(b.svc.net().send(other).unwrap_err().contains("isn't in net_ffff"));
    let agents = json!([{ "id": "claude:web", "projectKey": "prj_web" }, { "id": "claude:api", "projectKey": "prj_api" }, { "id": "claude:loose" }]);
    let presence = |counter: u64| json!({ "schema": "network/presence/v1", "counter": counter, "full": true, "agents": agents, "removed": [], "you": false, "roster": "" });
    let heard = || a.sink.got.lock().unwrap().iter().rev().find(|g| g.0 == "laptop" && g.1 == "presence").map(|g| g.2["agents"].clone());
    // The first network shares everything, as the one network did.
    b.svc.net().send(Outgoing::new(To::All, Kind::Presence, presence(1)).within(id.clone())).unwrap();
    until("the first presence", 10, || heard().is_some_and(|a| a.as_array().map(Vec::len) == Some(3))).await;
    // Shared down to one project, at the window: laptop's presence goes
    // again whole, carrying its sessions alone.
    let (s, v) = b.call("PUT", "/api/net/sharing", json!({ "sharing": ["prj_web"] }), true).await;
    assert_eq!(s, 200, "{v}");
    until("the presence shared down", 10, || heard() == Some(json!([{ "id": "claude:web", "projectKey": "prj_web" }]))).await;
}

/// Each entry of a member's feed as it arrived, with what its act rests on.
#[derive(Default)]
struct HeldFeeds(Mutex<Vec<(String, Value, Value)>>);

impl xenonet_net::sink::Feeds for HeldFeeds {
    fn entry(&self, a: xenonet_net::sink::Arrived) {
        self.0.lock().unwrap().push((a.host.clone(), a.entry.body.clone(), a.authority.clone()));
    }
    fn unfreeze(&self, _author: &str, _network: &str) -> Result<usize, String> {
        Ok(0)
    }
}

impl HeldFeeds {
    fn texts(&self) -> Vec<String> {
        self.0.lock().unwrap().iter().filter_map(|(_, b, _)| b["body"].as_str().map(str::to_string)).collect()
    }
}

/// The host `of` in a state's hosts.
fn host_in(s: &Value, of: &str) -> Value {
    s["hosts"].as_array().unwrap().iter().find(|h| h["id"] == of).cloned().unwrap_or(Value::Null)
}

/// studio makes a network; laptop (a window host) and desk join with
/// invites from its window, found at the address the invite's door gives,
/// and each reaches the others on link/2.
async fn network(test: &str) -> (Host, Host, Host) {
    let a = Host::new(test, "studio", 0.0, Caps::default()).await;
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio" }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["gateway"]["link"], "live", "{v}");
    let mut joined = Vec::new();
    for (name, window) in [("laptop", true), ("desk", false)] {
        let (s, inv) = a.call("POST", "/api/net/invite", json!({ "host": name, "window": window }), true).await;
        assert_eq!(s, 200, "{inv}");
        let at = inv["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
        let h = Host::new(test, name, 0.0, Caps::default()).await;
        let (s, v) = h.call("POST", "/api/net/join", json!({ "invite": inv["invite"], "code": inv["code"], "at": at }), true).await;
        assert_eq!(s, 200, "{v}");
        joined.push(h);
    }
    let c = joined.pop().unwrap();
    let b = joined.pop().unwrap();
    for h in [&a, &b, &c] {
        let mut ok = false;
        for _ in 0..600 {
            let s = h.state().await;
            let hosts = s["hosts"].as_array().map_or(0, Vec::len);
            if hosts == 3 && s["hosts"].as_array().unwrap().iter().all(|h| h["online"] == true) {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ok, "never all three online: {}", h.state().await);
    }
    (a, b, c)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_network_runs_on_link2_alone_its_chain_messages_and_acts_included() {
    let (a, b, c) = network("link2-alone").await;
    // The chain reached every member, though desk joined after laptop and
    // only studio signed it in; desk found laptop by the address record
    // studio passed on.
    let len = a.roster().len();
    assert_eq!(len, 3);
    until("the chain everywhere", 10, || b.roster().len() == len && c.roster().len() == len).await;
    let s = c.state().await;
    assert_eq!(s["gateway"]["address"], "", "{s}");
    assert_eq!(host_in(&s, "laptop")["path"], "direct", "{s}");

    // A message, and one whose act rests on an authority: each reaches
    // desk once, with what it rests on, over link/2 alone.
    let feeds = Arc::new(HeldFeeds::default());
    c.svc.sinks().set_feeds(feeds.clone());
    b.send("desk", Kind::Message, "plain");
    let authority = json!({ "presence": { "what": "start", "sig": "c2ln" } });
    let body = json!({ "schema": "network/message/v1", "body": "with authority" });
    b.svc.net().send(Outgoing { network: None, authority: Some(authority.clone()), to: To::Host("desk".into()), kind: Kind::Message, body }).unwrap();
    until("both at desk", 20, || feeds.texts().len() == 2).await;
    let got = feeds.0.lock().unwrap().clone();
    assert_eq!((got[0].0.as_str(), got[0].2.clone()), ("laptop", json!({})));
    assert_eq!(got[1].2, authority, "the act's authority came with it");

    // desk goes away: a message for it waits at laptop, and goes when desk
    // is back.
    let (dir, shift, caps) = (c.dir.clone(), c.shift, c.caps);
    c.svc.stop();
    drop(c);
    let mut away = false;
    for _ in 0..800 {
        if host_in(&b.state().await, "desk")["online"] == false {
            away = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(away, "laptop never saw desk go: {}", b.state().await);
    b.send("desk", Kind::Message, "while away");
    let c = Host::open(dir, shift, caps).await;
    let feeds = Arc::new(HeldFeeds::default());
    c.svc.sinks().set_feeds(feeds.clone());
    until("the waiting message at desk", 30, || feeds.texts().contains(&"while away".to_string())).await;

    // A widening laptop signs alone waits for a second admin: studio sees
    // it, approves it, and the entry reaches every member.
    let (s, v) = b.call("PUT", "/api/net/settings", json!({ "start": "network" }), true).await;
    assert_eq!((s, v["proposed"].clone()), (200, json!(true)), "{v}");
    let mut id = Value::Null;
    for _ in 0..200 {
        let pending = host_in(&a.state().await, "laptop")["reach"]["pending"].clone();
        if let Some(p) = pending.as_array().and_then(|p| p.first()) {
            id = p["id"].clone();
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(id.is_string(), "studio never saw laptop's proposal: {}", a.state().await);
    let (s, v) = a.call("POST", "/api/net/approve", json!({ "id": id }), true).await;
    assert_eq!(s, 200, "{v}");
    for h in [&a, &b, &c] {
        until("the approved entry everywhere", 10, || h.roster().len() == len + 1).await;
    }

    // studio removes desk: desk hears it, and laptop's chain has it.
    let (s, v) = a.call("POST", "/api/net/remove", json!({ "host": "desk" }), true).await;
    assert_eq!(s, 200, "{v}");
    until("laptop holds the removal", 10, || b.roster().len() == len + 2).await;
    let mut removed = false;
    for _ in 0..200 {
        if c.state().await["removed"].is_object() {
            removed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(removed, "desk never heard it was removed: {}", c.state().await);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admits_count_only_presence_for_them_from_a_live_device_at_a_fresh_head() {
    use xenonet_log::person;
    use xenonet_net::roster::{HostRef, Op, ADMIT};
    let (a, b, c) = network("presence").await;
    let n = a.svc.node().shared().clone();
    let id = n.identity().clone();
    // One person on all three: studio founded it at its window and paired
    // laptop's and desk's devices as they joined.
    let person = id.person_id().unwrap();
    for h in [&b, &c] {
        let st = h.state().await;
        assert_eq!((st["person"]["id"].as_str(), st["person"]["acts"].clone(), st["person"]["owner"].clone()), (Some(person.as_str()), json!(true), json!(true)), "{st}");
    }
    let key = |seed: u8| xenonet_id::sign::Signer::public(&xenonet_id::sign::SeedSigner::from_seed(&[seed; 32], xenonet_id::sign::KeyHome::File).unwrap());
    let spare = || HostRef { id: "spare".into(), keys: *xenonet_net::keys::HostKey::generate().unwrap().public(), window: false, device: None, machine: key(9) };
    // Entries are signed by the machine signing key, never host.key.
    let machine = id.machine_key().unwrap();
    let now = now_ms() as i64;
    let admit = |e: &xenonet_net::roster::Entry| id.prove(true, ADMIT, &e.digest(), "", now).unwrap();
    // With presence for it: taken. Without, or with a block lifted off
    // another entry: refused, as presence.
    let mut e = n.draft(Op::Add, spare(), None);
    let block = admit(&e);
    e.sign_with("studio", machine.as_ref(), Some(block.clone())).unwrap();
    n.roster().would_take(&e).unwrap();
    let mut bare = n.draft(Op::Add, spare(), None);
    bare.sign_by("studio", machine.as_ref()).unwrap();
    assert_eq!(n.roster().would_take(&bare).unwrap_err().code, "presence");
    let mut lifted = n.draft(Op::Add, spare(), None);
    lifted.sign_with("studio", machine.as_ref(), Some(block)).unwrap();
    assert_eq!(n.roster().would_take(&lifted).unwrap_err().code, "presence", "a block lifted off another entry");
    // Asking presence away from the window asks nobody.
    assert_eq!(id.prove(false, ADMIT, &e.digest(), "", now).unwrap_err().code, "window");

    // The person pairs another device: their chain's new head reaches
    // laptop and desk, pushed; an entry made under the old head is stale
    // to a live receiver, though the chain itself would take it.
    let old = n.draft(Op::Add, spare(), None);
    let tablet = xenonet_id::sign::SeedSigner::from_seed(&[11; 32], xenonet_id::sign::KeyHome::File).unwrap();
    id.pair(true, person::NewDevice { key: xenonet_id::sign::Signer::public(&tablet), kind: person::DeviceKind::Weaker, label: "tablet".into() }, now).unwrap();
    let head = id.head();
    for h in [&b, &c] {
        until("the new head pushed", 10, || h.svc.node().shared().identity().head() == head).await;
    }
    assert_eq!(n.fresh_person_head(&old).unwrap_err().code, "stale-person-head");
    n.fresh_person_head(&n.draft(Op::Add, spare(), None)).unwrap();

    // studio's own device is revoked: its presence counts for nothing made
    // under that head, and it acts as the person no more.
    let device = id.device_key().unwrap().unwrap();
    let dev = device.signer.public().id();
    let mut revoke = id.chain().next(person::Body { op: person::Op::Revoke(person::Revocation::Device(dev.clone())), at: now });
    let block = id.prove(true, person::REVOKE, &revoke.digest(), "", now).unwrap();
    revoke.sign(&dev, device.signer.as_ref(), Some(block)).unwrap();
    id.offer(&person, &[revoke.to_json()]).unwrap();
    assert!(!id.can_act());
    let mut e = n.draft(Op::Add, spare(), None);
    let block = xenonet_id::presence::prove(device.signer.as_ref(), device.check.as_deref(), ADMIT, &e.digest(), "", now).unwrap();
    e.sign_with("studio", machine.as_ref(), Some(block)).unwrap();
    assert_eq!(n.roster().would_take(&e).unwrap_err().code, "presence", "a revoked device's presence");
    let (s, v) = a.call("POST", "/api/net/invite", json!({ "host": "spare" }), true).await;
    assert_eq!((s, v["code"].clone()), (409, json!("no-person")), "{v}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_machine_joins_with_an_invite_over_join2() {
    let a = Host::new("invite", "studio", 0.0, Caps::default()).await;
    // Without the window's key: no network is made, since setting up the
    // person asks them.
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio" }), false).await;
    assert_eq!((s, v["code"].clone()), (403, json!("window")), "{v}");
    let (s, v) = a.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio" }), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["person"]["owner"].clone(), v["person"]["acts"].clone()), (json!(true), json!(true)), "{v}");
    // An invite only from the window, since the join asks the person.
    let (s, v) = a.call("POST", "/api/net/invite", json!({ "host": "laptop" }), false).await;
    assert_eq!(s, 403, "{v}");
    let (s, inv) = a.call("POST", "/api/net/invite", json!({ "host": "laptop" }), true).await;
    assert_eq!(s, 200, "{inv}");
    let invite = inv["invite"].as_str().unwrap().to_string();
    let code = inv["code"].as_str().unwrap().to_string();
    assert!(invite.starts_with("xni2_") && !inv["line"].as_str().unwrap().contains(&code), "the code goes apart: {inv}");
    // The invite names no address: the person says where studio is.
    let at = inv["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).unwrap_or_else(|| panic!("{inv}")).to_string();
    // desk's invite keeps studio's door open after laptop's is used.
    let (s, desk) = a.call("POST", "/api/net/invite", json!({ "host": "desk" }), true).await;
    assert_eq!(s, 200, "{desk}");
    let b = Host::new("invite", "laptop", 0.0, Caps::default()).await;
    let join = |invite: &Value, code: &str| json!({ "invite": invite, "code": code, "at": at });
    let (s, v) = b.call("POST", "/api/net/join", json!({ "invite": invite, "code": code }), true).await;
    assert_eq!((s, v["code"].clone()), (409, json!("no-path")), "{v}");
    let (s, v) = b.call("POST", "/api/net/join", join(&inv["invite"], "0000-0000-00"), true).await;
    assert_eq!((s, v["code"].clone()), (403, json!("wrong-code")), "{v}");
    assert_eq!(a.state().await["invites"]["open"][0]["wrong"], 1);
    let (s, v) = b.call("POST", "/api/net/join", join(&inv["invite"], &code), true).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["network"]["name"].clone(), v["self"]["id"].clone()), (json!("home"), json!("laptop")), "{v}");
    // laptop reaches studio on link/2, by the address record studio sent,
    // and its device is the person's, which the chain's add names.
    online(&b, 2).await;
    let st = b.state().await;
    assert_eq!((st["person"]["id"].clone(), st["person"]["acts"].clone()), (a.state().await["person"]["id"].clone(), json!(true)));
    let add = b.roster().last().unwrap().clone();
    assert_eq!((add["op"].clone(), add["host"]["device"].clone()), (json!("add"), st["person"]["device"].clone()), "{add}");
    assert!(add["signers"][0]["presence"]["what"] == "admit", "{add}");
    assert_eq!(a.state().await["invites"]["open"].as_array().unwrap().len(), 1, "laptop's is used");
    // It worked once: another machine with the same invite and code is
    // told no such ticket is open.
    let c = Host::new("invite", "desk", 0.0, Caps::default()).await;
    let (s, v) = c.call("POST", "/api/net/join", join(&inv["invite"], &code), true).await;
    assert_eq!((s, v["code"].clone()), (404, json!("no-ticket")), "{v}");
    // Five wrong codes retire desk's invite, and studio's console says so.
    // (A third keeps the door open, so the last try below is answered.)
    let (s, _) = a.call("POST", "/api/net/invite", json!({ "host": "spare" }), true).await;
    assert_eq!(s, 200);
    for i in 1..=5 {
        let (s, v) = c.call("POST", "/api/net/join", join(&desk["invite"], "0000-0000-00"), true).await;
        assert_eq!(s, if i < 5 { 403 } else { 410 }, "try {i}: {v}");
    }
    let (s, v) = c.call("POST", "/api/net/join", join(&desk["invite"], desk["code"].as_str().unwrap()), true).await;
    assert_eq!((s, v["code"].clone()), (404, json!("no-ticket")), "the right code after it's retired: {v}");
    let burned = a.state().await["invites"]["burned"].clone();
    assert_eq!((burned[0]["host"].clone(), burned[0]["retired"].clone()), (json!("desk"), json!(true)), "{burned}");
}
