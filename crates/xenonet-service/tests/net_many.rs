// Several networks on one machine, end to end (phase 5, commit 13): studio
// founds N1 and desk founds N2, and laptop joins both with invites, so
// laptop is the one machine in two networks. Each service runs in this
// process with its own state folder, on link/2 over loopback, and a test
// Sink stands in for each engine, keeping the network each body came in.
//
// The gate: nothing of N1 alone reaches a machine in N2 alone, presence
// included, and laptop removed from N1 while in N2 keeps N2 flowing.
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_api::cores::{self, Cores};
use xenonet_model::ops::{Incoming, Kind, Outgoing, Sink, To};
use xenonet_net::node::{now_ms, Options};
use xenonet_os::log::Log;
use xenonet_service::config::Config;
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::network::NetService;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

struct You;

#[async_trait]
impl cores::Verify for You {
    async fn person(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
    async fn window(&self, _peer: Peer) -> Value {
        json!({ "ok": true })
    }
}

/// What reached the engine: the network, the sender, the kind, the body.
#[derive(Default)]
struct Heard(Mutex<Vec<(String, String, String, Value)>>);

impl Sink for Heard {
    fn receive(&self, i: Incoming) {
        self.0.lock().unwrap().push((i.network, i.from, i.kind.as_str().to_string(), i.body));
    }
    fn linked(&self, _up: bool) {}
    fn removed(&self, _host: &str) {}
    fn gap(&self) {}
}

impl Heard {
    fn all(&self) -> Vec<(String, String, String, Value)> {
        self.0.lock().unwrap().clone()
    }
    fn texts(&self, kind: &str) -> Vec<String> {
        self.all().into_iter().filter(|h| h.2 == kind).map(|h| h.3["body"].as_str().unwrap_or("").to_string()).collect()
    }
    /// The agents each presence from `host` listed.
    fn agents_from(&self, host: &str) -> Vec<Vec<String>> {
        self.all()
            .into_iter()
            .filter(|h| h.1 == host && h.2 == "presence")
            .map(|h| h.3["agents"].as_array().map(|a| a.iter().filter_map(|x| x["id"].as_str().map(str::to_string)).collect()).unwrap_or_default())
            .collect()
    }
}

struct Host {
    /// Added to the clock this machine's node reads, in ms.
    offset: Arc<std::sync::atomic::AtomicI64>,
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
    heard: Arc<Heard>,
    dir: PathBuf,
}

fn options(dir: &std::path::Path, offset: &Arc<std::sync::atomic::AtomicI64>, compact_after: u64) -> Options {
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
    let offset = offset.clone();
    o.clock = Arc::new(move || now_ms() + offset.load(std::sync::atomic::Ordering::SeqCst) as f64);
    o.compact_after = compact_after;
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o
}

impl Host {
    async fn new(test: &str, name: &str) -> Host {
        Host::compacting(test, name, xenonet_net::compaction::AFTER).await
    }

    /// One that compacts its feed after `compact_after` kept entries.
    async fn compacting(test: &str, name: &str, compact_after: u64) -> Host {
        let dir = std::env::temp_dir().join(format!("xenonet-net-many-{test}-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
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
        cores.verify = Arc::new(You);
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let offset = Arc::new(std::sync::atomic::AtomicI64::new(0));
        let svc = NetService::open(options(&dir, &offset, compact_after)).unwrap();
        let heard = Arc::new(Heard::default());
        svc.sinks().set(heard.clone());
        let _ = ctx.net.set(svc.clone());
        svc.start().await.unwrap();
        Host { offset, ctx, svc, heard, dir }
    }

    async fn call(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        self.call_as(true, method, path, body).await
    }

    /// From the window (`window`), or from the person's own terminal.
    async fn call_as(&self, window: bool, method: &str, path: &str, body: Value) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        let headers: Vec<(&str, &str)> = if window { vec![("x-xenonet-window", key.as_str())] } else { Vec::new() };
        let text = body.to_string();
        let req = Req::fake(method, path, &headers, if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
        (r.status, r.body_json())
    }

    async fn state(&self) -> Value {
        self.call("GET", "/api/net/state", Value::Null).await.1
    }

    fn send(&self, network: Option<&str>, to: To, kind: Kind, body: Value) -> Result<(), String> {
        self.svc.net().send(Outgoing { network: network.map(str::to_string), authority: None, to, kind, body })
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.svc.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn until(what: &str, secs: u64, mut ok: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while !ok() {
        assert!(std::time::Instant::now() < end, "waited {secs} s for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `founder` makes a network named `name`; `joiner` joins it as `host`
/// with an invite from the founder's window. The network's id.
async fn network(founder: &Host, joiner: &Host, name: &str, me: &str, host: &str) -> String {
    let (s, v) = founder.call("POST", "/api/net/create", json!({ "name": name, "host": me })).await;
    assert_eq!(s, 200, "{v}");
    let id = v["networks"].as_array().unwrap().iter().find(|n| n["name"] == name).unwrap()["id"].as_str().unwrap().to_string();
    let (s, t) = founder.call("POST", "/api/net/invite", json!({ "host": host, "network": name })).await;
    assert_eq!(s, 200, "{t}");
    let at = t["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
    let (s, v) = joiner.call("POST", "/api/net/join", json!({ "invite": t["invite"], "code": t["code"], "at": at })).await;
    assert_eq!(s, 200, "{v}");
    id
}

fn presence(counter: u64) -> Value {
    json!({
        "schema": "network/presence/v1",
        "counter": counter,
        "full": true,
        // Each a change, so each goes: presence on link/2 goes only when
        // something changed.
        "agents": [{ "id": "claude:web", "projectKey": "prj_web", "status": format!("v{counter}") }, { "id": "claude:api", "projectKey": "prj_api", "status": format!("v{counter}") }],
        "removed": [],
        "you": false,
        "roster": "",
    })
}

fn message(text: &str) -> Value {
    json!({ "schema": "network/message/v1", "body": text })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_machine_in_two_networks_keeps_them_apart_and_one_goes_on_without_the_other() {
    let studio = Host::new("apart", "studio").await;
    let desk = Host::new("apart", "desk").await;
    let laptop = Host::new("apart", "laptop").await;
    let n1 = network(&studio, &laptop, "home", "studio", "laptop").await;
    // laptop goes by another host id in N2: each network has its own.
    let n2 = network(&desk, &laptop, "work", "desk", "lap").await;

    // laptop lists both, each with its own host id, members and gateway.
    let st = laptop.state().await;
    let nets = st["networks"].as_array().unwrap();
    assert_eq!(nets.len(), 2, "{st}");
    assert_eq!((nets[0]["id"].as_str(), nets[0]["host"].as_str()), (Some(n1.as_str()), Some("laptop")));
    assert_eq!((nets[1]["id"].as_str(), nets[1]["host"].as_str()), (Some(n2.as_str()), Some("lap")));
    assert_eq!(nets[0]["sharing"], "all", "the first network sees everything");
    assert_eq!(nets[1]["sharing"], json!([]), "a later one starts with nothing");
    let members = |n: &Value| n["hosts"].as_array().unwrap().iter().map(|h| h["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(members(&nets[1]), ["lap", "desk"]);
    // What acts on one network says which, in two.
    let (s, v) = laptop.call("POST", "/api/net/leave", json!({})).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("which-network")), "{v}");
    let (s, v) = laptop.call("POST", "/api/net/invite", json!({ "host": "vm1", "network": "nowhere" })).await;
    assert_eq!((s, v["code"].as_str()), (404, Some("no-network")), "{v}");

    for h in [&studio, &desk, &laptop] {
        let body = json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [], "removed": [], "you": false, "roster": "" });
        h.send(None, To::All, Kind::Presence, body).unwrap();
    }
    // One link/2 endpoint for both: studio found in N1, desk in N2, each
    // by a direct path.
    let path = |s: &Value, net: usize, of: &str| s["networks"][net]["hosts"].as_array().and_then(|h| h.iter().find(|x| x["id"] == of).map(|x| x["path"].clone()));
    let mut st = Value::Null;
    for _ in 0..300 {
        st = laptop.state().await;
        if path(&st, 0, "studio") == Some(json!("direct")) && path(&st, 1, "desk") == Some(json!("direct")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!((path(&st, 0, "studio"), path(&st, 1, "desk")), (Some(json!("direct")), Some(json!("direct"))), "{st}");
    assert_eq!(st["networks"][1]["inContact"], true, "{st}");
    // laptop's presence: each network sees what is shared into it.
    laptop.send(None, To::All, Kind::Presence, presence(2)).unwrap();
    until("laptop's presence at studio", 15, || studio.heard.agents_from("laptop").iter().any(|a| a.len() == 2)).await;
    until("laptop's presence at desk", 15, || desk.heard.agents_from("lap").iter().any(Vec::is_empty)).await;
    // Shared down: N2 sees prj_api's session from now, never prj_web's.
    let (s, v) = laptop.call("PUT", "/api/net/sharing", json!({ "network": "work", "sharing": ["prj_api"] })).await;
    assert_eq!(s, 200, "{v}");
    laptop.send(None, To::All, Kind::Presence, presence(3)).unwrap();
    until("prj_api's session at desk", 15, || desk.heard.agents_from("lap").iter().any(|a| a == &["claude:api"])).await;

    // A message for studio goes in N1; a handoff to all goes in the first
    // network; a claim in prj_web goes where prj_web is shared: none of it
    // reaches desk.
    laptop.send(None, To::Host("studio".into()), Kind::Message, message("for studio")).unwrap();
    laptop.send(None, To::All, Kind::Claims, json!({ "schema": "network/claim/v1", "op": "claim", "project": "prj_web", "path": "src/", "agent": "claude:web" })).unwrap();
    laptop.send(None, To::All, Kind::Claims, json!({ "schema": "network/claim/v1", "op": "claim", "project": "prj_api", "path": "lib/", "agent": "claude:api" })).unwrap();
    laptop.send(None, To::Host("desk".into()), Kind::Message, message("for desk")).unwrap();
    until("the message at studio", 15, || studio.heard.texts("message") == ["for studio"]).await;
    until("the message at desk", 15, || desk.heard.texts("message") == ["for desk"]).await;
    until("prj_api's claim at desk", 15, || desk.heard.all().iter().any(|h| h.2 == "claims" && h.3["project"] == "prj_api")).await;
    until("prj_web's claim at studio", 15, || studio.heard.all().iter().any(|h| h.2 == "claims" && h.3["project"] == "prj_web")).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    // desk heard nothing in N1, nothing of prj_web, and from laptop only
    // as lap, in N2.
    for (network, from, kind, body) in desk.heard.all() {
        assert_eq!(network, n2, "desk heard {kind} from {from} in another network: {body}");
        assert!(!body.to_string().contains("prj_web") && !body.to_string().contains("claude:web"), "desk heard N1's: {body}");
        assert_ne!(from, "laptop", "laptop is lap in N2");
    }
    for (network, _, kind, body) in studio.heard.all() {
        assert_eq!(network, n1, "studio heard {kind} in another network: {body}");
    }
    // A service published into N1 is N1's: N2's list doesn't hold it, so
    // nothing answers N2's members for it.
    let (s, v) = laptop.call("POST", "/api/net/svc/publish", json!({ "network": "home", "name": "files", "target": "http://127.0.0.1:9", "paths": ["/f/"] })).await;
    assert_eq!(s, 200, "{v}");
    let (_, home) = laptop.call("GET", "/api/net/svc?network=home", Value::Null).await;
    let (_, work) = laptop.call("GET", &format!("/api/net/svc?network={n2}"), Value::Null).await;
    assert_eq!((home["services"].as_array().map(Vec::len), home["host"].as_str()), (Some(1), Some("laptop")), "{home}");
    assert_eq!((work["services"].as_array().map(Vec::len), work["host"].as_str()), (Some(0), Some("lap")), "{work}");
    // A message for a network named that this machine isn't in is refused.
    assert!(laptop.send(Some("net_ffffffffffffffffffffffffffffffff"), To::Host("desk".into()), Kind::Message, message("x")).unwrap_err().contains("isn't in"));

    // studio takes laptop out of N1: N2 goes on.
    let (s, v) = studio.call("POST", "/api/net/remove", json!({ "host": "laptop" })).await;
    assert_eq!(s, 200, "{v}");
    until("laptop removed from N1", 15, || {
        let s = futures_lite(laptop.state());
        s["networks"][0]["link"]["state"] == "removed"
    })
    .await;
    laptop.send(None, To::Host("desk".into()), Kind::Message, message("after N1")).unwrap();
    until("N2's message after N1 went", 15, || desk.heard.texts("message") == ["for desk", "after N1"]).await;
    // N1's node refuses to send now; the engine's "to all" goes to N2.
    assert!(laptop.send(Some(&n1), To::Host("studio".into()), Kind::Message, message("x")).is_err());
    let before = studio.heard.all().len();
    laptop.send(None, To::All, Kind::Presence, presence(9)).unwrap();
    until("presence at desk after N1 went", 15, || desk.heard.all().iter().any(|h| h.2 == "presence" && h.3["counter"] == 9)).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(studio.heard.all().len(), before, "studio hears nothing of laptop once it's out");

    // Leaving N1 leaves N2 as it was.
    let (s, v) = laptop.call("POST", "/api/net/leave", json!({ "network": n1, "force": true })).await;
    assert_eq!(s, 200, "{v}");
    let st = laptop.state().await;
    assert_eq!(st["networks"].as_array().map(Vec::len), Some(1), "{st}");
    assert_eq!(st["network"]["id"], json!(n2));
    laptop.send(None, To::Host("desk".into()), Kind::Message, message("after leaving")).unwrap();
    until("N2's message after leaving N1", 15, || desk.heard.texts("message").len() == 3).await;
}

// The state, read synchronously inside `until`'s closure.
fn futures_lite(f: impl std::future::Future<Output = Value>) -> Value {
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_self_hosted_relay_goes_into_the_chain_and_one_of_this_machines_own_into_its_endpoint() {
    let studio = Host::new("relay", "studio").await;
    // The network is made with the person's own relay.
    let (s, v) = studio.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "relay": "https://relay.example.net" })).await;
    assert_eq!(s, 200, "{v}");
    assert!(v.get("relayRefused").is_none(), "{v}");
    assert_eq!(v["networks"][0]["relays"], json!(["https://relay.example.net"]));
    assert_eq!(v["relaysHere"]["every"], json!(["https://relay.example.net"]), "one network: its relays are the machine's");
    // Another, later, and n0's public ones by name.
    let (s, v) = studio.call("POST", "/api/net/relays", json!({ "url": "https://relay2.example.net/" })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["networks"][0]["relays"], json!(["https://relay.example.net", "https://relay2.example.net"]));
    let (s, v) = studio.call("POST", "/api/net/relays", json!({ "url": "n0" })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["networks"][0]["relays"].as_array().map(Vec::len), Some(6));
    // Not a URL: refused.
    let (s, v) = studio.call("POST", "/api/net/relays", json!({ "url": "relay.example.net" })).await;
    assert_eq!(s, 400, "{v}");
    // Taken out again, which narrows.
    let (s, v) = studio.call("POST", "/api/net/relays", json!({ "url": "n0", "add": false })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["networks"][0]["relays"].as_array().map(Vec::len), Some(2));
    // One of this machine's own, for its dials: in no chain.
    let (s, v) = studio.call("POST", "/api/net/relays", json!({ "url": "https://mine.example.net", "mine": true })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["relaysHere"]["own"], json!(["https://mine.example.net"]));
    assert_eq!(v["networks"][0]["relays"].as_array().map(Vec::len), Some(2), "a machine's own relay is in no chain");
    // Kept: a second start reads it again.
    let kept = common::kept(&studio.dir.join("state"));
    assert_eq!(kept.own_relays().unwrap(), ["https://mine.example.net"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mailbox_is_named_at_the_window_and_taken_off_by_a_window_host_or_itself() {
    let studio = Host::new("mailbox", "studio").await;
    let laptop = Host::new("mailbox", "laptop").await;
    network(&studio, &laptop, "home", "studio", "laptop").await;
    let boxes = |v: &Value| v["networks"][0]["mailboxes"].clone();
    // Naming one asks you: refused from a terminal, done at the window.
    let (s, v) = studio.call_as(false, "POST", "/api/net/mailboxes", json!({ "host": "laptop" })).await;
    assert_eq!((s, v["code"].as_str()), (403, Some("window")), "{v}");
    let (s, v) = studio.call("POST", "/api/net/mailboxes", json!({ "host": "laptop" })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(boxes(&v), json!(["laptop"]));
    let (s, v) = studio.call("POST", "/api/net/mailboxes", json!({ "host": "studio" })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(boxes(&v), json!(["laptop", "studio"]));
    let (s, v) = studio.call("POST", "/api/net/mailboxes", json!({ "host": "nowhere" })).await;
    assert_eq!((s, v["code"].as_str()), (404, Some("no-host")), "{v}");
    until("laptop holds both mailboxes", 15, || boxes(&futures_lite(laptop.state())) == json!(["laptop", "studio"])).await;
    // laptop isn't a window host: it can't take studio off.
    let (s, v) = laptop.call_as(false, "POST", "/api/net/mailboxes", json!({ "host": "studio", "add": false })).await;
    assert_eq!((s, v["code"].as_str()), (403, Some("not-window")), "{v}");
    // It steps down itself, from its terminal: that narrows.
    let (s, v) = laptop.call_as(false, "POST", "/api/net/mailboxes", json!({ "host": "laptop", "add": false })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(boxes(&v), json!(["studio"]));
    until("studio heard laptop step down", 15, || boxes(&futures_lite(studio.state())) == json!(["studio"])).await;
    // A window host takes one off from its terminal.
    let (s, v) = studio.call_as(false, "POST", "/api/net/mailboxes", json!({ "host": "studio", "add": false })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(boxes(&v), json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rekeyed_machine_stays_in_its_network_and_its_old_key_is_refused() {
    use xenonet_net::admission::Admission;
    use xenonet_net::dial::{Alpn, Close, Mesh};
    use xenonet_net::endpoint::{peer_id, IrohDial, LanLookup, Relays, Settings};
    use xenonet_net::keys::HostKey;
    let studio = Host::new("rekey", "studio").await;
    let laptop = Host::new("rekey", "laptop").await;
    let id = network(&studio, &laptop, "home", "studio", "laptop").await;
    let row = |st: &Value, of: &str| st["hosts"].as_array().and_then(|h| h.iter().find(|x| x["id"] == of).cloned()).unwrap_or(Value::Null);
    until("studio reaches laptop", 15, || row(&futures_lite(studio.state()), "laptop")["path"] == "direct").await;
    let key_file = laptop.dir.join("state").join("network").join("host.key");
    let old = HostKey::load(&key_file).unwrap().unwrap();
    let old_id = old.key_id();
    assert_eq!(row(&studio.state().await, "laptop")["keyId"], json!(old_id));

    // A new key asks you: refused from a terminal, given at the window.
    let (s, v) = laptop.call_as(false, "POST", "/api/net/rekey", json!({})).await;
    assert_eq!(s, 403, "{v}");
    let (s, v) = laptop.call("POST", "/api/net/rekey", json!({})).await;
    assert_eq!(s, 200, "{v}");
    let new_id = v["self"]["keyId"].as_str().unwrap().to_string();
    assert_ne!(new_id, old_id);
    assert_eq!(HostKey::load(&key_file).unwrap().unwrap().key_id(), new_id, "the file holds the new key");
    // studio takes the rekey, keeps laptop as a member, and reaches it on
    // the new key; a message goes as before.
    until("studio holds laptop's new key and reaches it", 20, || {
        let r = row(&futures_lite(studio.state()), "laptop");
        r["keyId"] == json!(new_id) && r["path"] == "direct"
    })
    .await;
    laptop.send(None, To::Host("studio".into()), Kind::Message, message("after the rekey")).unwrap();
    until("the message on the new key", 15, || studio.heard.texts("message").contains(&"after the rekey".to_string())).await;
    assert_eq!(studio.state().await["hosts"].as_array().map(Vec::len), Some(2), "still two members");

    // The old key, copied before: studio refuses it.
    let n = studio.svc.node().shared().clone();
    let t = n.link2().unwrap();
    let at = peer_id(n.key().unwrap().public());
    let admission = Arc::new(Admission::new());
    let settings = Settings { relays: Relays::None, lan: LanLookup::Off, bind: Some("127.0.0.1:0".parse().unwrap()), roots: Vec::new() };
    let dial = Arc::new(IrohDial::bind(&old, &settings, admission.clone()).await.unwrap());
    let mesh = Mesh::new(dial.clone(), admission);
    mesh.set_members(&id, [at]);
    dial.addresses().put(at, &t.dial.direct_addrs(), None, Duration::from_secs(60)).unwrap();
    match tokio::time::timeout(Duration::from_secs(10), mesh.dial(at, Alpn::Link)).await.expect("the dial ends") {
        Err(_) => {}
        Ok(l) => assert_eq!(tokio::time::timeout(Duration::from_secs(10), l.closed()).await.expect("closed"), Some(Close::Refused)),
    }
    mesh.close().await;
}

fn claim(op: &str, path: &str) -> Value {
    json!({ "schema": "network/claim/v1", "op": op, "project": "prj_web", "path": path, "agent": "claude:web" })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_machine_a_week_behind_catches_up_from_a_compaction_made_on_its_own() {
    // studio compacts its feed after four kept entries since its last
    // compact entry.
    let studio = Host::compacting("compact", "studio", 4).await;
    let laptop = Host::new("compact", "laptop").await;
    network(&studio, &laptop, "home", "studio", "laptop").await;
    let row = |st: &Value, of: &str| st["hosts"].as_array().and_then(|h| h.iter().find(|x| x["id"] == of).cloned()).unwrap_or(Value::Null);
    until("studio reaches laptop", 15, || row(&futures_lite(studio.state()), "laptop")["path"] == "direct").await;
    laptop.svc.stop();
    until("laptop away", 15, || row(&futures_lite(studio.state()), "laptop")["online"] == false).await;

    // While laptop is away: a claim that stays, one made and let go, and a
    // report signed eight days ago, which its week's retention drops.
    let s = studio.svc.node().shared().clone();
    studio.send(None, To::All, Kind::Claims, claim("claim", "src/")).unwrap();
    studio.send(None, To::All, Kind::Claims, claim("claim", "lib/")).unwrap();
    let kept = common::kept(&studio.dir.join("state"));
    let net = s.network().unwrap().id;
    let out = || kept.feed_out_after(&net, xenonet_net::feed::Pos::ORIGIN, 100).unwrap();
    let let_go = out().last().cloned().unwrap();
    assert_eq!(let_go.seq, 2);
    studio.offset.store(-8 * 24 * 3_600_000, std::sync::atomic::Ordering::SeqCst);
    s.write_entry(xenonet_net::feed::Kind::Report, json!({ "schema": "network/report/v1", "text": "eight days ago" })).unwrap();
    studio.offset.store(0, std::sync::atomic::Ordering::SeqCst);
    // The fourth: the compaction comes on its own, keeping the first claim.
    studio.send(None, To::All, Kind::Claims, claim("release", "lib/")).unwrap();
    let c = out().into_iter().find(|e| e.kind == xenonet_net::feed::Kind::Compact).expect("compacted");
    assert_eq!(c.kind, xenonet_net::feed::Kind::Compact, "a compact entry after the fourth");
    assert_eq!(c.compact().unwrap().live, [1], "only the claim that stays");

    // laptop held the let-go claim behind a gap before it went away, as if
    // it had come alone: a compaction that drops it, it never applies.
    let n = laptop.svc.node().shared().clone();
    let machine = n.identity().machine_key().unwrap().public().id();
    let env = xenonet_net::envelope::Envelope::seal_act(&let_go, &machine, n.key().unwrap().public(), None, None).unwrap();
    n.envelope_from("studio", env);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let claims = || laptop.heard.all().into_iter().filter(|h| h.2 == "claims" && h.1 == "studio").map(|h| h.3["path"].as_str().unwrap_or("").to_string()).collect::<Vec<_>>();
    assert!(claims().is_empty(), "held behind its gap");

    // Back: the author answers with its compaction, and laptop takes the
    // live claim and nothing it dropped.
    laptop.svc.start().await.unwrap();
    until("laptop took the live claim", 20, || claims() == ["src/"]).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    assert_eq!(claims(), ["src/"], "neither the let-go claim, its release nor the old report");
    assert!(!laptop.heard.all().iter().any(|h| h.2 == "report"), "the report aged out");
    // Caught up: the next entry applies at once.
    studio.send(None, To::All, Kind::Claims, claim("claim", "docs/")).unwrap();
    until("the next claim", 15, || claims() == ["src/", "docs/"]).await;
}

// mDNS on the LAN is asked at the network's setup and kept as the
// machine's setting. The test answers off: on, the endpoint would bind a
// multicast socket, which the endpoint's own tests cover by the lookups it
// gets (nodes.rs, endpoint_tests.rs) without binding one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_mdns_question_is_answered_at_setup_and_kept_as_the_machines() {
    let studio = Host::new("mdns", "studio").await;
    assert_eq!(studio.state().await["mdns"], "unasked", "not asked before a network");
    let (s, v) = studio.call("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "mdns": false })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["mdns"], "off");
    let kept = common::kept(&studio.dir.join("state"));
    assert_eq!(kept.lan().unwrap(), Some(false));
    assert_eq!(studio.svc.nodes().lan(), xenonet_net::endpoint::LanLookup::Off);
    // A second network leaves the answer as it was when it gives none.
    let (s, v) = studio.call("POST", "/api/net/create", json!({ "name": "work", "host": "studio" })).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["mdns"], "off");
    // It's this machine's own, so a terminal sets it too.
    let (s, v) = studio.call_as(false, "PUT", "/api/net/settings", json!({ "mdns": false })).await;
    assert_eq!((s, v["mdns"].clone()), (200, json!("off")), "{v}");
}
