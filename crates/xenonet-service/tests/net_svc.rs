// Services through the network (docs/design/network.md, "Services through
// the network"): three services in this process, each with its own state
// folder and its own iroh endpoint on loopback. hub founds the network,
// which runs on link/2 alone; studio and laptop join with invites from its
// window. Requests go on svc/2 between the two machines, never through
// hub. studio publishes a made-up HTTP service, and
// laptop's programs reach it at /api/svc/studio/<name>/… on laptop's own
// API: an allowlisted path passes and one outside it is refused; the
// service is sent laptop's id in Forwarded, as the connection
// authenticated it, never a caller's own, and the caller's bearer
// untouched; the size caps hold; a host outside --hosts is refused; and
// with studio offline, laptop answers 503 at once.
//
// The window's reads of another machine's transcript go on svc/2 too:
// hub's window (a window host's, the person's device on it certifying its
// window key) reads studio's; an agent's read is refused; laptop, no
// window host, is refused at studio; and once studio shares titles only,
// its transcripts answer 403 while its stats still go.
//
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_api::cores::{self, Cores};
use xenonet_net::dial::Fut;
use xenonet_net::node::{now_ms, Options};
use xenonet_os::log::Log;
use xenonet_service::config::Config;
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::actor_gateway::GatewayCores;
use xenonet_service::gateway::control::{ControlParts, GatewayControl};
use xenonet_service::gateway::network::NetService;
use xenonet_service::gateway::reach::SystemNetwork;
use xenonet_service::gateway::tunnel::SystemHost;

/// studio's session, by the agt_ its store names it by.
const ABC: &str = "agt_studiosabcsessionaaaaaaaaaaaaaaa";
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

const TOKEN: &str = "Bearer svc-token-0123456789";

/// The person at the window and their terminal, until `agent` says the
/// peer is a session's process, as process checks would find one.
#[derive(Default)]
struct Peers {
    agent: AtomicBool,
}

#[async_trait]
impl cores::Verify for Peers {
    async fn check(&self, _peer: Peer, _id: &str, _hook: bool) -> Value {
        json!({ "ok": true })
    }
    async fn person(&self, _peer: Peer) -> Value {
        if self.agent.load(Ordering::SeqCst) {
            json!({ "ok": false, "reason": "this came from under a Claude Code process" })
        } else {
            json!({ "ok": true })
        }
    }
    async fn window(&self, peer: Peer) -> Value {
        self.person(peer).await
    }
}

/// An actor gateway on `port`, as one set up with a port of its own runs:
/// not the default for XenoNet's `service_port`. Nothing listens.
fn actor_gateway(port: u16, service_port: u16) -> Arc<GatewayControl> {
    let p = Arc::new(cores::Pending);
    GatewayControl::new(ControlParts {
        cores: GatewayCores { engine: p.clone(), registration: p.clone(), actors: p.clone(), reports: p.clone(), runs: None },
        store: p,
        on_run: Arc::new(|_, _| {}),
        port: Some(port),
        service_port,
        version: "0.0.0-test".into(),
        dir: None,
        home: "/home/nova".into(),
        probe_local: true,
        log: Arc::new(|_, _| {}),
        tunnel_host: Arc::new(SystemHost),
        network: Arc::new(SystemNetwork),
        now: Arc::new(now_ms),
    })
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
    // Fake keys: a test never touches the machine's own, or prompts.
    o.vault = Some(Arc::new(xenonet_keys::Fake { allow: true }));
    // A caller waits 6 s here, not 90, so the timeout shows quickly.
    o.svc_ms = 6000;
    // svc/2 runs on each machine's endpoint, on loopback, with no mDNS.
    o.bind = Some("127.0.0.1:0".parse().unwrap());
    o.lan = xenonet_net::endpoint::LanLookup::Off;
    o
}

/// studio's own API as window reads read it: a made-up transcript, and
/// each path read.
#[derive(Default)]
struct Reads(Mutex<Vec<String>>);

impl xenonet_net::svc2::LocalReads for Reads {
    fn get(&self, path: &str) -> Fut<'_, Result<(u16, Value), (String, u16)>> {
        self.0.lock().unwrap().push(path.to_string());
        let body = if path.contains("/transcript") {
            json!({ "id": "claude:abc", "entries": [{ "role": "user", "text": "hello" }], "cursor": null })
        } else {
            json!({ "id": "claude:abc", "turns": 3 })
        };
        Box::pin(async move { Ok((200, body)) })
    }
}

struct Host {
    name: &'static str,
    ctx: Arc<Ctx>,
    svc: Arc<NetService>,
    peers: Arc<Peers>,
}

impl Host {
    async fn open(name: &'static str, dir: PathBuf, port: u16) -> Host {
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let home = dir.join("home").to_string_lossy().into_owned();
        let state = dir.join("state").to_string_lossy().into_owned();
        let port = port.to_string();
        let vars = [("HOME", home.as_str()), ("XENONET_STATE_DIR", state.as_str()), ("XENONET_CACHE_DIR", state.as_str()), ("XENONET_PORT", port.as_str())];
        let get = |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string());
        let config = Config::from(&[], &get);
        let mut cores = Cores::pending(false, false);
        xenonet_service::serve::wire_root(&mut cores);
        let peers = Arc::new(Peers::default());
        cores.verify = peers.clone();
        let ctx = Arc::new(Ctx::new(config, "0.0.0-test".into(), cores, Log::new(None, "service", None, None, "/home/nova", 1 << 20)));
        let svc = NetService::open(options(&dir, name)).unwrap();
        let _ = ctx.net.set(svc.clone());
        svc.start().await.unwrap();
        Host { name, ctx, svc, peers }
    }

    /// Any program on this machine: no window key, nothing but its own
    /// headers.
    async fn call(&self, method: &str, path: &str, body: Option<&[u8]>, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mut req = Req::fake(method, path, headers, None);
        if let Some(b) = body {
            req.body = xenonet_service::http::Body::Bytes(b.to_vec());
        }
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
        let bytes = match r.body {
            xenonet_service::http::ReplyBody::Full(b) => b.to_vec(),
            xenonet_service::http::ReplyBody::Stream(_) => Vec::new(),
        };
        (r.status, r.headers, bytes)
    }

    /// The window's own call, with its key.
    async fn window(&self, method: &str, path: &str, body: Value) -> (u16, Value) {
        let key = self.ctx.window_key.reveal().to_string();
        let text = body.to_string();
        let req = Req::fake(method, path, &[("x-xenonet-window", key.as_str())], if method == "GET" { None } else { Some(text.as_str()) });
        let r = api(&self.ctx, common::local(&self.ctx, req)).await.unwrap();
        (r.status, r.body_json())
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

async fn join(h: &Host, maker: &Host) {
    let (s, t) = maker.window("POST", "/api/net/invite", json!({ "host": h.name, "window": false })).await;
    assert_eq!(s, 200, "{t}");
    let at = t["at"].as_array().unwrap().iter().filter_map(Value::as_str).find(|a| a.starts_with("127.0.0.1:")).expect("the door on loopback").to_string();
    let (s, v) = h.window("POST", "/api/net/join", json!({ "invite": t["invite"], "code": t["code"], "at": at })).await;
    assert_eq!(s, 200, "{} joins: {v}", h.name);
}

async fn live(h: &Host, hosts: usize) {
    until(&format!("{} live with {hosts} hosts", h.name), 20, || async {
        let s = h.state().await;
        s["link"]["state"] == "live" && s["hosts"].as_array().map_or(0, Vec::len) == hosts && s["hosts"].as_array().unwrap().iter().all(|x| x["online"] == true)
    })
    .await;
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

fn json_of(b: &[u8]) -> Value {
    serde_json::from_slice(b).unwrap_or(Value::Null)
}

/// What the made-up service was sent.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A made-up HTTP service on 127.0.0.1: it keeps what it's sent and
/// answers by path. Its port.
async fn made_up_service(seen: Arc<Mutex<Vec<Seen>>>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else { return };
            let seen = seen.clone();
            tokio::spawn(async move {
                let answer = hyper::service::service_fn(move |req: hyper::Request<hyper::body::Incoming>| {
                    let seen = seen.clone();
                    async move {
                        let method = req.method().to_string();
                        let target = req.uri().path_and_query().map(ToString::to_string).unwrap_or_default();
                        let headers: Vec<(String, String)> = req.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
                        let body = req.into_body().collect().await.map(|b| b.to_bytes().to_vec()).unwrap_or_default();
                        seen.lock().unwrap().push(Seen { method: method.clone(), target: target.clone(), headers, body: body.clone() });
                        let path = target.split('?').next().unwrap_or("").to_string();
                        let (status, out): (u16, Vec<u8>) = match path.as_str() {
                            "/sync/v1/echo" => (201, json!({ "method": method, "target": target, "body": String::from_utf8_lossy(&body) }).to_string().into_bytes()),
                            "/sync/v1/large" => (200, vec![b'l'; 3 << 20]),
                            "/sync/v1/big" => (200, vec![b'b'; (8 << 20) + 1]),
                            "/sync/v1/slow" => {
                                tokio::time::sleep(Duration::from_secs(10)).await;
                                (200, b"late".to_vec())
                            }
                            _ => (200, b"anything".to_vec()),
                        };
                        let res = hyper::Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .header("set-cookie", "session=s3cret")
                            .header("keep-alive", "timeout=5")
                            .header("x-kept", "yes")
                            .body(Full::new(Bytes::from(out)))
                            .unwrap();
                        Ok::<_, Infallible>(res)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), answer).await;
            });
        }
    });
    port
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_program_on_one_host_reaches_a_service_another_host_published() {
    let root = std::env::temp_dir().join(format!("xenonet-net-svc-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let gw = Host::open("hub", root.join("hub"), 47995).await;
    let (s, v) = gw.window("POST", "/api/net/create", json!({ "name": "home", "host": "hub" })).await;
    assert_eq!(s, 200, "{v}");
    let a = Host::open("studio", root.join("studio"), 47996).await;
    join(&a, &gw).await;
    let b = Host::open("laptop", root.join("laptop"), 47997).await;
    join(&b, &gw).await;
    for h in [&gw, &a, &b] {
        live(h, 3).await;
    }
    // Each found the others by the address records hub passed on.
    until("link/2 between all three", 20, || async { [&gw, &a, &b].iter().all(|h| h.svc.node().shared().link2().is_some_and(|t| t.links.peers().len() == 2)) }).await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = made_up_service(seen.clone()).await;
    let target = format!("http://127.0.0.1:{port}");

    // Publishing is the owner's: refused from under an agent.
    a.peers.agent.store(true, Ordering::SeqCst);
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "files", "target": target, "paths": ["/sync/v1/"] })).await;
    assert_eq!((s, v["code"].clone()), (403, json!("yours")), "{v}");
    a.peers.agent.store(false, Ordering::SeqCst);
    // It widens what the network reaches, so it asks the person: the
    // window's, never the terminal's, where nothing asks.
    let terminal = json!({ "name": "files", "target": target, "paths": ["/sync/v1/"] }).to_string();
    let (s, _, out) = a.call("POST", "/api/net/svc/publish", Some(terminal.as_bytes()), &[]).await;
    assert_eq!(s, 403, "{}", String::from_utf8_lossy(&out));
    // Never XenoNet's own API or actor gateway, nor a name, nor anything
    // but loopback. studio's actor gateway runs on a port it saved, not
    // the default.
    let _ = a.ctx.gateway.set(actor_gateway(47999, 47996));
    for (t, why) in [
        ("http://127.0.0.1:47996", "XenoNet's own"),
        ("http://127.0.0.1:47999", "XenoNet's own"),
        ("http://localhost:4790", "looks no names up"),
        ("http://192.168.1.5:4790", "loopback"),
        ("https://127.0.0.1:4790", "plain http"),
    ] {
        let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "files", "target": t, "paths": ["/sync/v1/"] })).await;
        assert_eq!(s, 400, "{t}: {v}");
        assert!(v["error"].as_str().unwrap().contains(why), "{t}: {v}");
    }
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "files", "target": target, "paths": ["/sync/v1/"] })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "private", "target": target, "paths": ["/sync/v1/"], "hosts": ["hub"] })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "other", "target": target, "paths": ["/sync/v1/"], "hosts": ["nowhere"] })).await;
    assert_eq!(s, 400, "a host that isn't in the network: {v}");
    let (s, v) = a.window("GET", "/api/net/svc", Value::Null).await;
    assert_eq!(s, 200);
    assert_eq!((v["host"].clone(), v["services"].as_array().unwrap().len()), (json!("studio"), 2), "{v}");
    assert_eq!(v["services"][0], json!({ "name": "files", "target": target, "paths": ["/sync/v1/"], "hosts": [], "at": v["services"][0]["at"] }));

    // An allowlisted path passes, with the caller's bearer untouched and
    // its query as it came. The service is told the call came from laptop,
    // as the connection authenticated it, and nothing the caller said
    // about forwarding crosses.
    let (s, h, body) = b
        .call(
            "POST",
            "/api/svc/studio/files/sync/v1/echo?x=1&y=%2F",
            Some(b"{\"note\":\"hi\"}"),
            &[
                ("authorization", TOKEN),
                ("content-type", "application/json"),
                ("forwarded", "for=evil;by=evil"),
                ("x-forwarded-for", "10.0.0.9"),
                ("x-forwarded-host", "evil.example"),
                ("via", "1.1 evil"),
                ("cookie", "a=b"),
            ],
        )
        .await;
    assert_eq!(s, 201, "{}", String::from_utf8_lossy(&body));
    let echo = json_of(&body);
    assert_eq!((echo["method"].clone(), echo["target"].clone(), echo["body"].clone()), (json!("POST"), json!("/sync/v1/echo?x=1&y=%2F"), json!("{\"note\":\"hi\"}")));
    let got = seen.lock().unwrap().last().cloned().unwrap();
    assert_eq!((got.method.as_str(), got.target.as_str(), got.body.as_slice()), ("POST", "/sync/v1/echo?x=1&y=%2F", b"{\"note\":\"hi\"}".as_slice()));
    assert_eq!(header(&got.headers, "authorization"), Some(TOKEN), "the bearer arrives untouched");
    assert_eq!(header(&got.headers, "forwarded"), Some("by=xenonet;for=laptop"));
    assert_eq!(got.headers.iter().filter(|(k, _)| k == "forwarded").count(), 1, "the caller's own Forwarded is dropped");
    assert_eq!(header(&got.headers, "host"), Some(format!("127.0.0.1:{port}").as_str()));
    for gone in ["x-forwarded-for", "x-forwarded-host", "via", "cookie"] {
        assert_eq!(header(&got.headers, gone), None, "{gone}: {:?}", got.headers);
    }
    // And its answer comes back without its cookie or hop-by-hop headers.
    assert_eq!((header(&h, "x-kept"), header(&h, "set-cookie"), header(&h, "keep-alive")), (Some("yes"), None, None), "{h:?}");

    // A large answer crosses whole.
    let (s, _, body) = b.call("GET", "/api/svc/studio/files/sync/v1/large", None, &[]).await;
    assert_eq!((s, body.len()), (200, 3 << 20));

    // Paths outside the allowlist, or that could be read as another path,
    // are refused at studio, and the service never sees them.
    let before = seen.lock().unwrap().len();
    for (path, status, code) in [
        ("/api/svc/studio/files/admin", 403, "path-refused"),
        ("/api/svc/studio/files/sync/v1/%2e%2e/%2e%2e/admin", 403, "path-refused"),
        ("/api/svc/studio/files/sync/v1%2F..%2Fadmin", 400, "bad-path"),
        ("/api/svc/studio/files/sync/v1/x;y", 400, "bad-path"),
        ("/api/svc/studio/files/sync/v1/a%2F%2Fb", 400, "bad-path"),
        ("/api/svc/studio/nothing/sync/v1/x", 404, "not-published"),
        ("/api/svc/nowhere/files/sync/v1/x", 404, "no-host"),
        ("/api/svc/laptop/files/sync/v1/x", 400, "this-host"),
    ] {
        let (s, h, body) = b.call("GET", path, None, &[]).await;
        assert_eq!((s, header(&h, "x-xenonet-refused")), (status, Some(code)), "{path}: {}", String::from_utf8_lossy(&body));
    }
    let (s, h, _) = b.call("TRACE", "/api/svc/studio/files/sync/v1/x", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (405, Some("method")));
    assert_eq!(seen.lock().unwrap().len(), before, "none reached the service");

    // A web page can't use it.
    let (s, _, _) = b.call("GET", "/api/svc/studio/files/sync/v1/x", None, &[("origin", "http://evil.example")]).await;
    assert_eq!(s, 403);
    let (s, _, _) = b.call("GET", "/api/svc/studio/files/sync/v1/x", None, &[("sec-fetch-site", "cross-site")]).await;
    assert_eq!(s, 403);

    // The size caps: a body over 8 MiB, an answer over 8 MiB.
    let (s, h, _) = b.call("POST", "/api/svc/studio/files/sync/v1/echo", Some(&vec![b'x'; (8 << 20) + 1]), &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (413, Some("too-big")));
    let (s, _, body) = b.call("POST", "/api/svc/studio/files/sync/v1/echo", Some(&vec![b'x'; 1 << 20]), &[]).await;
    assert_eq!((s, json_of(&body)["body"].as_str().map(str::len)), (201, Some(1 << 20)), "1 MiB goes");
    let (s, h, _) = b.call("GET", "/api/svc/studio/files/sync/v1/big", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (413, Some("answer-too-big")));

    // A service narrowed to hub: laptop is refused, hub isn't.
    let (s, h, _) = b.call("GET", "/api/svc/studio/private/sync/v1/x", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (403, Some("not-allowed")));
    let (s, _, body) = gw.call("GET", "/api/svc/studio/private/sync/v1/x", None, &[]).await;
    assert_eq!((s, body.as_slice()), (200, b"anything".as_slice()));
    assert_eq!(header(&seen.lock().unwrap().last().unwrap().headers, "forwarded"), Some("by=xenonet;for=hub"));

    // The program that asked goes away before the answer: laptop stops
    // waiting on it then, rather than holding a place until its timeout.
    let gone = tokio::time::timeout(Duration::from_millis(500), b.call("GET", "/api/svc/studio/files/sync/v1/slow", None, &[])).await;
    assert!(gone.is_err(), "still waiting when its program went");
    assert_eq!(b.svc.node().shared().services().waiting(), 0);

    // No answer in time: 504.
    let (s, h, _) = b.call("GET", "/api/svc/studio/files/sync/v1/slow", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (504, Some("timeout")));

    // Taken off the network, from the terminal (it only narrows, so it
    // asks nothing): not published any more.
    let (s, _, out) = a.call("POST", "/api/net/svc/unpublish", Some(br#"{"name":"private"}"#), &[]).await;
    assert_eq!(s, 200, "{}", String::from_utf8_lossy(&out));
    let (s, h, _) = gw.call("GET", "/api/svc/studio/private/sync/v1/x", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (404, Some("not-published")));

    // The window's reads of studio's transcripts, on svc/2, by the agt_
    // studio's presence names its session by.
    let reads = Arc::new(Reads::default());
    a.svc.nodes().common().set_reads(reads.clone());
    let presence = json!({ "schema": "network/presence/v1", "counter": 1, "full": true, "agents": [{ "id": ABC, "app": "claude", "title": "abc", "waitsOnYou": false }], "removed": [], "you": false, "roster": "" });
    a.svc.net().send(xenonet_model::ops::Outgoing::new(xenonet_model::ops::To::All, xenonet_model::ops::Kind::Presence, presence)).unwrap();
    until("studio's agent at the gateway's host and laptop", 10, || async {
        gw.svc.nodes().host_of_agent(ABC).as_deref() == Some("studio") && b.svc.nodes().host_of_agent(ABC).as_deref() == Some("studio")
    })
    .await;
    let transcript = &format!("/api/session/{ABC}/transcript?limit=120");
    let (s, v) = gw.window("GET", transcript, Value::Null).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!((v["host"].clone(), v["id"].clone(), v["entries"][0]["text"].clone()), (json!("studio"), json!(ABC), json!("hello")));
    assert_eq!(reads.0.lock().unwrap().as_slice(), [format!("/api/session/{ABC}/transcript?limit=120")], "studio maps its agt_ to its session itself");
    // Another page of it, under the same window key.
    let (s, _) = gw.window("GET", &format!("/api/session/{ABC}/stats"), Value::Null).await;
    assert_eq!(s, 200);
    // An agent's read of another machine's transcript: refused here, with
    // or without the window's key.
    gw.peers.agent.store(true, Ordering::SeqCst);
    let (s, v) = gw.window("GET", transcript, Value::Null).await;
    assert_eq!(s, 403, "{v}");
    gw.peers.agent.store(false, Ordering::SeqCst);
    let (s, _, _) = gw.call("GET", transcript, None, &[]).await;
    assert_eq!(s, 403);
    // laptop is no window host: studio refuses its window.
    let (s, v) = b.window("GET", &format!("/api/session/{ABC}/transcript"), Value::Null).await;
    assert_eq!((s, v["code"].clone()), (403, json!("not-window")), "{v}");
    // studio shares titles only: its transcripts stay on it, its stats go.
    let (s, v) = a.window("PUT", "/api/net/settings", json!({ "titlesOnly": true })).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = gw.window("GET", transcript, Value::Null).await;
    assert_eq!((s, v["code"].clone()), (403, json!("titles-only")), "{v}");
    let (s, _) = gw.window("GET", &format!("/api/session/{ABC}/stats"), Value::Null).await;
    assert_eq!(s, 200);
    assert_eq!(reads.0.lock().unwrap().len(), 3, "none of the refused reads reached studio's API");

    // studio goes offline: laptop says so at once, without waiting.
    a.svc.stop();
    until("laptop to see studio offline", 15, || async { b.state().await["hosts"].as_array().unwrap().iter().any(|x| x["id"] == "studio" && x["online"] == false) }).await;
    let started = std::time::Instant::now();
    let (s, h, _) = b.call("GET", "/api/svc/studio/files/sync/v1/echo", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (503, Some("offline")));
    assert!(started.elapsed() < Duration::from_secs(2), "at once");

    drop((gw, a, b));
    let _ = std::fs::remove_dir_all(root);
}
