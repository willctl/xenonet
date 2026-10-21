#![no_main]

// The engine's side of what other hosts send (docs/design/network.md,
// sections 5, 6, 8 and 10): presence, claims, messages, receipts,
// handoffs and the person's actions, as the network passes them on once a
// frame checked and opened, through `network::receive` on a real engine
// with its own state folder. Each run takes a few bodies, each one of the
// schemas' own with the input's fields over it, or the input's JSON
// alone, and then reads them back as the engine does: other hosts' agents
// for peers and the console, their claims on this host's files, and the
// handoffs open to a session.
//
// Nothing may panic. What another host says lands as that host's: its
// agents and messages are qualified with its id, and a message from you
// is taken only from a window host's console, checked strongly.

use async_trait::async_trait;
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_base::network::{Host, Incoming, Kind, Me, Net, Outgoing};
use xenonet_engine::sql::Arg;
use xenonet_fuzz::net::{body_for, overlay, Choices};
use xenonet_service::config::Config;
use xenonet_service::cores::{Cores, State, R};
use xenonet_service::wire::{Emitter, VendorReaders, WireEnv};
use xenonet_wire_engine::engine::EngineCalls;
use xenonet_wire_engine::network;
use xenonet_wire_engine::world::World;

const A: &str = "claude:aaaaaaaa-0001";
const B: &str = "codex:cccccccc-0001";
const HOSTS: [(&str, bool); 3] = [("laptop", true), ("desk", true), ("vm1", false)];
/// How many runs one engine takes before a new one, so its files don't grow.
const RUNS: u64 = 400;

/// studio's network: laptop, desk and vm1 online; what it sends is kept.
#[derive(Default)]
struct FakeNet {
    sent: Mutex<Vec<Outgoing>>,
}

impl Net for FakeNet {
    fn me(&self) -> Option<Me> {
        Some(Me { network: "net_0123456789abcdef0123456789abcdef".into(), name: "home".into(), host: "studio".into() })
    }
    fn hosts(&self) -> Vec<Host> {
        HOSTS.iter().map(|(h, window)| Host { id: (*h).into(), window: *window, online: true, seen: Some(1_792_512_000_000.0) }).collect()
    }
    fn send(&self, out: Outgoing) -> Result<(), String> {
        if let Ok(mut s) = self.sent.lock() {
            s.push(out);
        }
        Ok(())
    }
}

/// Two made-up sessions on this host, in folders that aren't git's.
struct Map {
    cwd: String,
}

#[async_trait]
impl State for Map {
    fn graph(&self) -> R<Option<Value>> {
        Ok(Some(json!({})))
    }
    fn graph_version(&self) -> u64 {
        1
    }
    fn nodes(&self) -> R<Vec<Value>> {
        Ok([(A, "Add partial refunds"), (B, "Ledger fixtures")]
            .iter()
            .map(|(id, title)| json!({ "id": id, "kind": "session", "title": title, "cwd": self.cwd, "updatedAt": 1_792_512_000_000.0 }))
            .collect())
    }
    fn node(&self, id: &str) -> R<Option<Value>> {
        Ok(self.nodes()?.into_iter().find(|n| n["id"] == id))
    }
    fn home_of(&self, _id: &str) -> R<Value> {
        Ok(Value::Null)
    }
    fn live_ids(&self) -> R<Vec<(String, Value)>> {
        Ok([A, B].iter().map(|id| ((*id).to_string(), json!(xenonet_wire_engine::js::now_ms()))).collect())
    }
    fn live_info(&self, id: &str, _now: f64) -> R<Option<Value>> {
        Ok([A, B].contains(&id).then(|| json!({ "lastEventAt": xenonet_wire_engine::js::now_ms(), "endedAt": null, "status": "tool", "needsYou": null, "edits": [] })))
    }
    fn observed_live(&self, _id: &str) -> R<bool> {
        Ok(false)
    }
    fn identities(&self) -> R<Vec<Value>> {
        Ok(Vec::new())
    }
}

struct Engine {
    dir: PathBuf,
    w: Arc<World>,
    net: Arc<FakeNet>,
    runs: u64,
    /// The last message id the checks read.
    last: f64,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime"))
}

fn engine() -> Engine {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!("xenonet-fuzz-engine-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home/atlas")).expect("the engine's folder");
    let home = dir.join("home").to_string_lossy().into_owned();
    let state = dir.join("state").to_string_lossy().into_owned();
    let vars = [("HOME", home.as_str()), ("XENONET_STATE_DIR", state.as_str()), ("XENONET_CACHE_DIR", state.as_str()), ("XENONET_ACTORS", "off"), ("XENONET_PORT", "47996")];
    let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_string());
    let mut config = Config::from(&[], &get);
    let mut cores = Cores::pending(false, false, false);
    cores.state = Arc::new(Map { cwd: dir.join("home/atlas").to_string_lossy().into_owned() });
    let env = WireEnv { version: "0.0.0".into(), emitter: Emitter::new(), drift: Arc::default(), readers: VendorReaders::default() };
    let _in = runtime().enter();
    let w = xenonet_wire_engine::world(&cores, &mut config, &env).expect("the engine's parts");
    let net = Arc::new(FakeNet::default());
    network::set_net(&w, net.clone());
    Engine { dir, w, net, runs: 0, last: 0.0 }
}

static ENGINE: Mutex<Option<Engine>> = Mutex::new(None);

const KINDS: [Kind; 6] = [Kind::Presence, Kind::Claims, Kind::Message, Kind::Receipt, Kind::Handoff, Kind::Operator];

/// A body of `kind`, as studio's engine would get it from `from`.
fn body(kind: Kind, c: &mut Choices<'_>) -> Value {
    if c.rarely(24) {
        return c.value();
    }
    let mut b = body_for(kind.as_str(), c);
    match kind {
        Kind::Message => {
            b["target"] = json!(*c.pick(&[A, B, "claude:zzzzzzzz-0009", "actor:you", "actor:wren", "@laptop:claude:aaaaaaaa-0001", "hello"]));
            b["sender"] = json!(*c.pick(&["claude:bbbbbbbb-0001", "actor:you", "actor:wren", "@studio:claude:aaaaaaaa-0001", "codex:dddddddd-0002"]));
            if c.bool() {
                b["replyTo"] = json!(format!("msg_{:08x}", c.u16()));
            }
        }
        Kind::Handoff => {
            b["to"] = json!({ "agent": *c.pick(&[Value::Null, json!(A), json!("@studio:claude:aaaaaaaa-0001"), json!("@laptop:claude:x")]), "repoKey": "github.com/acme/atlas", "host": *c.pick(&[Value::Null, json!("studio"), json!("laptop")]) });
            b["by"] = json!(*c.pick(&["@laptop:claude:bbbbbbbb-0001", "claude:bbbbbbbb-0001", "@studio:claude:aaaaaaaa-0001", "@desk:codex:x"]));
        }
        Kind::Presence => {
            let agents: Vec<Value> = (0..c.below(4)).map(|_| json!({ "id": *c.pick(&["claude:bbbbbbbb-0001", "actor:wren", "@studio:claude:aaaaaaaa-0001", "actor:you", "codex:x", "claude:"]), "title": String::from_utf8_lossy(c.chunk()), "repoKey": "github.com/acme/atlas", "cwd": "~/atlas", "state": "working" })).collect();
            b["agents"] = Value::Array(agents);
            b["counter"] = json!(xenonet_wire_engine::js::now_ms() + f64::from(c.u16()));
        }
        _ => {}
    }
    let over = c.value();
    overlay(b, over, c)
}

/// Every message stored after message `last`: a host's own are qualified
/// with its id, and one as you came from a window host's console,
/// strongly.
fn landed_as_theirs(w: &World, last: &mut f64, from: &str, window: bool, b: &Value) {
    while let Ok(Some(m)) = w.store.with(|s| s.message(Arg::Num(*last + 1.0))) {
        *last += 1.0;
        let sender = m.text("sender").unwrap_or("").to_string();
        if sender == "actor:you" {
            let strong = b["auth"]["strength"] == "strong";
            assert!(window && strong && b["senderKind"] == "you", "a message as you was taken from {from} (window {window}, {b})");
        } else {
            assert!(sender.starts_with(&format!("@{from}:")), "a message from {from} landed as {sender}'s");
        }
    }
}

/// Other hosts' agents, as peers and the console see them: each its own
/// host's, under that host's id, never you or this host's.
fn views_are_theirs(w: &World) {
    for detail in [false, true] {
        for v in network::remote_views(w, detail) {
            let id = v["id"].as_str().unwrap_or("");
            let host = v["host"].as_str().unwrap_or("");
            assert!(HOSTS.iter().any(|(h, _)| *h == host), "a view on a host that isn't in the network: {v}");
            let local = id.strip_prefix(&format!("@{host}:")).unwrap_or_else(|| panic!("a view not under its host: {v}"));
            assert!(!local.starts_with('@') && local != "actor:you", "another host listed {local} as its own");
            assert!(v["title"].as_str().is_none_or(|t| t.chars().count() <= 90), "a title past its limit: {v}");
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let mut slot = ENGINE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.as_ref().is_none_or(|e| e.runs >= RUNS) {
        *slot = None;
        *slot = Some(engine());
    }
    let Some(e) = slot.as_mut() else { return };
    e.runs += 1;
    let w = &e.w;
    for _ in 0..=c.below(3) {
        let kind = *c.pick(&KINDS);
        let from = *c.pick(&["laptop", "laptop", "desk", "vm1"]);
        let window = HOSTS.iter().find(|(h, _)| *h == from).is_some_and(|(_, win)| *win) || c.rarely(16);
        let b = body(kind, &mut c);
        let incoming = Incoming { from: from.into(), kind, body: b.clone(), at: xenonet_wire_engine::js::now_ms(), from_window_host: window };
        xenonet_fuzz::run(data, || network::receive(w, incoming.clone()));
        landed_as_theirs(w, &mut e.last, from, window, &b);
        if c.is_empty() {
            break;
        }
    }
    views_are_theirs(w);
    // What the engine reads back of it: a view by id, claims on this host's
    // files, and the handoffs open to a session.
    let mut calls = EngineCalls::new(w, None);
    let me = Arg::text(A);
    let file = Arg::text(format!("{}/home/atlas/src/a.ts", e.dir.to_string_lossy()));
    let _ = network::call(w, &mut calls, "view", &[Arg::text(format!("@laptop:{}", String::from_utf8_lossy(c.chunk())))]);
    let _ = network::call(w, &mut calls, "claimHits", &[me.clone(), Arg::List(vec![file])]);
    if let Ok(view) = calls.view(&me) {
        let _ = network::call(w, &mut calls, "handoffs", &[view]);
    }
    if let Ok(mut s) = e.net.sent.lock() {
        s.clear();
    };
});
