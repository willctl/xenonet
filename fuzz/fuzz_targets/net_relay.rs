#![no_main]

// The map between hosts (docs/design/network.md, section 7;
// network/relay/v1): what another host sends once a frame has checked and
// opened, on both sides of a subscription.
//
// - Asks, as laptop's relay takes them (`RelayServer::relay`): from
//   studio, a window host, or desk, which isn't one; each run's few asks
//   are the schema's own with the input's fields over them, or the input's
//   JSON alone.
// - A get's path, as the relay serves it over the network
//   (`net_allowed`), with titles only on or off.
// - Answers, as studio's hub takes them (`HostHub::relay`) for laptop's
//   subscription: the relay's frames (hello, snapshot, event, res, time,
//   ping, bye) with the input's over them, in bodies with any seq, from
//   laptop or from desk.
//
// Nothing may panic. Only a window host gets a subscription; nothing but
// the allowlist's reads, as GET, and never a transcript with titles only;
// and what's drawn of another host is that host's, under its own id.

use async_trait::async_trait;
use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, OnceLock};
use xenonet_base::network::{Host, Incoming, Kind, Me, Net, Outgoing, RelaySink};
use xenonet_fuzz::net::{overlay, Choices};
use xenonet_hosts::network::{self as wire, Op};
use xenonet_service::cores::{Hub, Pending, SelfApi};
use xenonet_service::events::Events;
use xenonet_service::wire::Emitter;
use xenonet_wire_app::hosts::config::HostsStore;
use xenonet_wire_app::hosts::hub::{HostHub, HubCore, HubParts};
use xenonet_wire_app::hosts::relay::{net_allowed, net_key, Parts, RelayServer, Timing};
use xenonet_wire_app::hosts::schemas::HostSchemas;
use xenonet_wire_app::hosts::transport::{new_sub, Transport};

const NETWORK: &str = "net_0123456789abcdef0123456789abcdef";
const SUBS: [&str; 2] = ["sub_0123456789abcdef0123456789abcdef", "sub_fedcba9876543210fedcba9876543210"];
const PATHS: [&str; 10] = [
    "/api/graph",
    "/api/live",
    "/api/engine/state",
    "/api/session/claude:abc/transcript?limit=5",
    "/api/session/codex:x/stats",
    "/api/mcp?server=agent-wiki",
    "/api/collision?a=claude:a&b=claude:b",
    "/api/open",
    "/api/../etc/passwd",
    "/api/session/claude:abc/transcript?cursor=a%20b",
];

/// laptop's network: studio is a window host, desk isn't.
struct FakeNet;

impl Net for FakeNet {
    fn me(&self) -> Option<Me> {
        Some(Me { network: NETWORK.into(), name: "home".into(), host: "laptop".into() })
    }
    fn hosts(&self) -> Vec<Host> {
        vec![Host { id: "studio".into(), window: true, online: true, seen: None }, Host { id: "desk".into(), window: false, online: true, seen: None }]
    }
    fn send(&self, _out: Outgoing) -> Result<(), String> {
        Ok(())
    }
}

/// laptop's own API, which keeps what was read.
#[derive(Default)]
struct Api {
    read: Mutex<Vec<String>>,
}

#[async_trait]
impl SelfApi for Api {
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)> {
        if let Ok(mut r) = self.read.lock() {
            r.push(path.to_string());
        }
        Ok((200, json!({ "nodes": [], "links": [], "sessions": [] })))
    }
}

/// studio's link to laptop: its subscription, made here.
struct Fake {
    sub: Mutex<String>,
}

impl Transport for Fake {
    fn start(&self, _host: &str) -> Result<String, String> {
        let s = new_sub()?;
        if let Ok(mut sub) = self.sub.lock() {
            sub.clone_from(&s);
        }
        Ok(s)
    }
    fn write(&self, _host: &str, _sub: &str, _text: &str) {}
    fn renew(&self, _host: &str, _sub: &str, _fresh: bool) {}
    fn end(&self, _host: &str, _sub: &str) {}
}

struct World {
    server: Arc<RelayServer>,
    api: Arc<Api>,
    hub: Arc<HostHub>,
    link: Arc<Fake>,
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime"))
}

fn world() -> &'static World {
    static W: OnceLock<World> = OnceLock::new();
    W.get_or_init(|| {
        let _in = runtime().enter();
        let api = Arc::new(Api::default());
        let server = RelayServer::new(Parts {
            net: Arc::new(FakeNet),
            events: Arc::new(Events::new()),
            api: api.clone(),
            home: "/home/nova".into(),
            version: "0.0.0".into(),
            platform: "linux".into(),
            demo: false,
            aliases: |_| Vec::new(),
            log: Arc::new(|_: &str, _: &str| {}),
            timing: Timing::default(),
        });
        let dir = std::env::temp_dir().join(format!("xenonet-fuzz-relay-{}", std::process::id()));
        let schemas = Arc::new(HostSchemas::new());
        let hub = HostHub::new(HubParts {
            hosts: Arc::new(HostsStore::new(&dir.join("state").to_string_lossy(), schemas.clone())),
            schemas,
            state: Arc::new(Pending),
            store: Arc::new(Pending),
            cache_dir: dir.join("cache").to_string_lossy().into_owned(),
            root: dir.to_string_lossy().into_owned(),
            home: "/home/nova".into(),
            demo: false,
            windows: json!({ "activeWindowMs": 600000, "recentWindowMs": 86400000, "projectActiveMs": 3600000 }),
            emitter: Emitter::new(),
            bin: "/no/such/xenonet".into(),
            ssh_bin: vec!["ssh".into()],
            ssh: false,
            relay: Timing::default(),
        });
        let link = Arc::new(Fake { sub: Mutex::new(String::new()) });
        hub.add_network("laptop", NETWORK, link.clone());
        World { server, api, hub, link }
    })
}

/// An ask, the schema's own with the input's fields over it, or the
/// input's JSON alone.
fn ask(c: &mut Choices<'_>) -> Value {
    if c.rarely(16) {
        return c.value();
    }
    let sub = *c.pick(&SUBS);
    let op = match c.below(5) {
        0 => Op::Open,
        1 => Op::Get { id: u64::from(c.u16()) + 1, path: if c.bool() { (*c.pick(&PATHS)).to_string() } else { String::from_utf8_lossy(c.chunk()).into_owned() } },
        2 => Op::Time { id: u64::from(c.u16()) + 1 },
        3 => Op::Renew { fresh: c.bool() },
        _ => Op::Close,
    };
    let b = wire::ask(sub, &op);
    if c.bool() {
        let over = c.value();
        overlay(b, over, c)
    } else {
        b
    }
}

/// One of the relay's frames, with the input's over it.
fn frame(c: &mut Choices<'_>) -> Value {
    let at = xenonet_net::node::now_ms();
    let base = match c.below(7) {
        0 => json!({ "t": "hello", "schema": "host/hello/v1", "protocol": 1, "version": "0.0.0", "host": { "id": "laptop", "key": net_key(NETWORK, "laptop") }, "platform": "linux", "demo": false, "now": at, "sinceMs": 604_800_000, "detail": true }),
        1 => json!({ "t": "snapshot", "at": at, "graph": { "nodes": [
            { "id": "core", "kind": "core", "name": "laptop", "path": "~" },
            { "id": "claude:abc", "kind": "session", "app": "claude", "rawId": "abc", "title": "Fix the form", "preview": null, "cwd": "~/atlas", "createdAt": at, "updatedAt": at, "archived": false, "model": null, "branch": null, "parentId": null, "parentKind": null, "activity": "active", "fx": null, "fy": null, "fz": null, "pinned": false, "actions": [] },
            { "id": "@desk:claude:x", "kind": "session", "app": "claude", "rawId": "x", "title": "desk's", "preview": null, "cwd": "~", "createdAt": at, "updatedAt": at, "archived": false, "model": null, "branch": null, "parentId": null, "parentKind": null, "activity": "active", "fx": null, "fy": null, "fz": null, "pinned": false, "actions": [] }
        ], "links": [{ "source": "core", "target": "claude:abc", "kind": "session" }] }, "live": { "sessions": [] }, "engine": null }),
        2 => json!({ "t": "event", "name": *c.pick(&["graph", "live", "activity", "tokens", "pulse", "agent-message", "doctor"]), "data": {} }),
        3 => json!({ "t": "res", "id": u64::from(c.u16()), "status": 200, "body": {} }),
        4 => json!({ "t": "time", "id": u64::from(c.u16()), "now": at }),
        5 => json!({ "t": "ping", "now": at }),
        _ => json!({ "t": "bye", "code": *c.pick(&["relay", "not-window", "xenonet-gone"]), "message": "x" }),
    };
    if c.bool() {
        let over = c.value();
        overlay(base, over, c)
    } else {
        base
    }
}

/// An answer for laptop's subscription, with any seq, or the input's JSON.
fn answer(c: &mut Choices<'_>, sub: &str) -> Value {
    if c.rarely(16) {
        return c.value();
    }
    let frames: Vec<Value> = (0..c.below(4)).map(|_| frame(c)).collect();
    let seq = match c.below(3) {
        0 => 1,
        1 => u64::from(c.u16()),
        _ => u64::from(c.u8()) + 1,
    };
    wire::batch(if c.rarely(8) { SUBS[0] } else { sub }, seq, &frames)
}

fuzz_target!(|data: &[u8]| {
    let mut c = Choices::new(data);
    let w = world();
    let _in = runtime().enter();

    // A get's path, as the relay serves it over the network.
    let path = String::from_utf8_lossy(c.chunk()).into_owned();
    let titles = c.bool();
    xenonet_fuzz::run(data, || {
        if let Ok((_, read)) = net_allowed(&path, titles) {
            let route = read.split('?').next().unwrap_or("");
            let ok = ["/api/graph", "/api/live", "/api/engine/state"].contains(&route)
                || route.strip_prefix("/api/session/").is_some_and(|r| r.ends_with("/stats") || (!titles && r.ends_with("/transcript")));
            assert!(ok, "{path} was served as {read}");
            assert!(!read.contains("..") && !read.contains("//") && !read.contains('%') && !read.contains('#'), "{path} was served as {read}");
        }
    });

    // Asks, at laptop's relay.
    for _ in 0..=c.below(3) {
        let from = *c.pick(&["studio", "studio", "desk"]);
        let window = from == "studio";
        let body = ask(&mut c);
        if let Ok(a) = wire::parse_ask(&body) {
            if let Op::Get { path, .. } = &a.op {
                assert!(!path.is_empty() && path.len() <= 2048);
            }
        }
        let incoming = Incoming { from: from.into(), kind: Kind::Relay, body, at: xenonet_net::node::now_ms(), from_window_host: window };
        // In the runtime, as the link that hands bodies on is, on either thread.
        xenonet_fuzz::run(data, || {
            let _in = runtime().enter();
            w.server.relay(incoming.clone());
        });
        let subs = w.server.subscribed();
        assert!(subs.iter().all(|(h, _)| h == "studio"), "only a window host subscribes: {subs:?}");
    }
    // What laptop's relay read of its own API is the allowlist's.
    if let Ok(read) = w.api.read.lock() {
        for p in read.iter() {
            let route = p.split('?').next().unwrap_or("");
            let ok = ["/api/graph", "/api/live", "/api/engine/state"].contains(&route) || (route.starts_with("/api/session/") && (route.ends_with("/stats") || route.ends_with("/transcript")));
            assert!(ok, "the relay read {p}");
        }
    }

    // Answers, at studio's hub.
    let sub = w.link.sub.lock().map(|s| s.clone()).unwrap_or_default();
    for _ in 0..=c.below(3) {
        let from = *c.pick(&["laptop", "laptop", "laptop", "desk"]);
        let body = answer(&mut c, &sub);
        let incoming = Incoming { from: from.into(), kind: Kind::Relay, body, at: xenonet_net::node::now_ms(), from_window_host: true };
        xenonet_fuzz::run(data, || {
            let _in = runtime().enter();
            w.hub.relay(incoming.clone());
        });
        if c.is_empty() {
            break;
        }
    }
    let core = HubCore(w.hub.clone());
    let g = core.graph(json!({ "nodes": [{ "id": "claude:mine", "kind": "session" }], "links": [] }));
    for n in g["nodes"].as_array().into_iter().flatten() {
        let id = n["id"].as_str().unwrap_or("");
        match n.get("host").and_then(Value::as_str) {
            None => assert_eq!(id, "claude:mine", "only this machine's own isn't another host's: {n}"),
            Some(h) => {
                assert_eq!(h, "laptop", "only laptop is a host here: {n}");
                let local = id.strip_prefix("@laptop:").unwrap_or_else(|| panic!("laptop's node not under its id: {n}"));
                assert!(!local.starts_with('@'), "a node laptop said was another host's: {n}");
            }
        }
    }
    let _ = core.live(json!({ "sessions": [] }));
});
