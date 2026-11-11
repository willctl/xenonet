#![no_main]

// The engine's side of what other hosts send (docs/design/network.md,
// sections 5, 6, 8 and 10): presence, claims, messages, receipts,
// handoffs and the person's actions, as the network passes them on once a
// frame checked and opened, through `network::receive` on a real engine
// with its own state folder. Each run takes a few bodies, each one of the
// schemas' own with the input's fields over it, or the input's JSON
// alone, and then reads them back as the engine does, through its driver
// as the routes call it: other hosts' agents for peers and the console, a
// view of one by id, their claims on a file a session here is about to
// edit (its PreToolUse hook), and the handoffs open to a session (its
// SessionStart briefing).
//
// Nothing may panic. What another host says lands as that host's: its
// agents and messages are qualified with its id, and a message from you
// is taken only with the person's device signature, which no input here
// can make: the engine decides each act on its own copy of the network's
// chain and the person's (the fixed network's), never on what the body
// says of itself.

use async_trait::async_trait;
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_fuzz::net::{body_for, overlay, Choices};
use xenonet_model::ops::{Host, Incoming, Kind, Me, Net, Outgoing};
use xenonet_runtime::engine::EngineDriver;
use xenonet_runtime::network;
use xenonet_runtime::runtime::{Runtime, Setup};
use xenonet_service::config::Config;
use xenonet_service::cores::{Cores, Engine as _, State, R};
use xenonet_service::wire::WireEnv;
use xenonet_text::time::now_ms;

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
        Ok([A, B].iter().map(|id| ((*id).to_string(), json!(now_ms()))).collect())
    }
    fn live_info(&self, id: &str, _now: f64) -> R<Option<Value>> {
        Ok([A, B].contains(&id).then(|| json!({ "lastEventAt": now_ms(), "endedAt": null, "status": "tool", "needsYou": null, "edits": [] })))
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
    w: Arc<Runtime>,
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
    let config = Config::from(&[], &get);
    let mut cores = Cores::pending(false, false, false);
    cores.state = Arc::new(Map { cwd: dir.join("home/atlas").to_string_lossy().into_owned() });
    let env = WireEnv::new("0.0.0");
    // As the binary's serve closure gives the engine its settings.
    let setup = Setup {
        demo: config.demo,
        actors: config.actors,
        port: config.port,
        home: config.home.clone(),
        state_dir: config.state_dir.clone(),
        projects_dir: config.projects_dir.clone(),
        version: env.version.clone(),
        root: config.root.clone(),
        runs_bin: config.runs_bin.clone(),
        permission_hold_ms: config.permission_hold_ms,
        launcher_exec: config.launcher_exec.clone(),
        made_up: xenonet_runtime::actors::MadeUp::none(),
    };
    xenonet_fuzz::adapters();
    let _in = runtime().enter();
    let w = xenonet_runtime::runtime(&cores, setup, &env).expect("the engine's parts");
    let net = Arc::new(FakeNet::default());
    network::set_net(&w, net.clone());
    // The fixed network's chain and its owner's, as studio holds them.
    let identity_dir = dir.join("state").join("identity");
    std::fs::create_dir_all(&identity_dir).expect("the identity's folder");
    std::fs::write(identity_dir.join("person.json"), format!("{}\n", xenonet_fuzz::net::person().to_json())).expect("person.json");
    let identity = xenonet_net::identity::Identity::open(identity_dir, None).expect("the identity");
    xenonet_runtime::ground::set_governance(&w, Some(Arc::new(Gov(identity))));
    Engine { dir, w, net, runs: 0, last: 0.0 }
}

/// studio's governance: the fixed network's chain, its owner's.
struct Gov(Arc<xenonet_net::identity::Identity>);

impl xenonet_net::ground::Governance for Gov {
    fn facts(&self) -> xenonet_net::ground::Facts {
        xenonet_net::ground::Facts {
            machine: xenonet_authz::MachineId::new("studio").expect("an id"),
            view: Some(xenonet_fuzz::net::roster().view()),
            identity: self.0.clone(),
            synced_at: Some(now_ms() as i64),
            now: now_ms() as i64,
        }
    }
    fn identity(&self) -> Arc<xenonet_net::identity::Identity> {
        self.0.clone()
    }
    fn want_person(&self, _host: &str, _person: &str) {}
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
            b["counter"] = json!(now_ms() + f64::from(c.u16()));
        }
        _ => {}
    }
    let over = c.value();
    overlay(b, over, c)
}

/// Every message stored after message `last`: a host's own are qualified
/// with its id, and none is as you, since no input carries the person's
/// device signature.
fn landed_as_theirs(w: &Runtime, last: &mut f64, from: &str, window: bool, b: &Value) {
    while let Ok(Some(m)) = w.store.with(|s| s.message(*last + 1.0)) {
        *last += 1.0;
        let sender = m.text("sender").unwrap_or("").to_string();
        if sender == "actor:you" {
            panic!("a message as you was taken from {from}, with no device signature (window {window}, {b})");
        } else {
            assert!(sender.starts_with(&format!("@{from}:")), "a message from {from} landed as {sender}'s");
        }
    }
}

/// Other hosts' agents, as peers and the console see them: each its own
/// host's, under that host's id, never you or this host's.
fn views_are_theirs(w: &Runtime) {
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
        let incoming = Incoming { from: from.into(), kind, body: b.clone(), at: now_ms(), from_window_host: window };
        xenonet_fuzz::run(data, || network::receive(w, incoming.clone()));
        landed_as_theirs(w, &mut e.last, from, window, &b);
        if c.is_empty() {
            break;
        }
    }
    views_are_theirs(w);
    // What the engine reads back of it: a view by id, claims on this host's
    // files, and the handoffs open to a session.
    let driver = EngineDriver(w.clone());
    let (_, session) = A.split_once(':').unwrap_or_default();
    let cwd = e.dir.join("home/atlas").to_string_lossy().into_owned();
    let file = format!("{cwd}/src/a.ts");
    let edit = json!({ "session_id": session, "hook_event_name": "PreToolUse", "tool_name": "Edit", "tool_input": { "file_path": file }, "cwd": cwd });
    let start = json!({ "session_id": session, "hook_event_name": "SessionStart", "source": "startup", "cwd": cwd });
    let remote = format!("@laptop:{}", String::from_utf8_lossy(c.chunk()));
    runtime().block_on(async {
        let _ = driver.view(&remote).await;
        let _ = driver.hook("claude", Some("PreToolUse"), &edit).await;
        let _ = driver.hook("claude", Some("SessionStart"), &start).await;
    });
    if let Ok(mut s) = e.net.sent.lock() {
        s.clear();
    };
});
