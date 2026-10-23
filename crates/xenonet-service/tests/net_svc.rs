// Services through the network (docs/design/network.md, "Services through
// the network"): three services in this process, each with its own state
// folder. hub founds the network and is its LAN-only gateway on
// 127.0.0.1:48131; studio and laptop join. studio publishes a made-up HTTP
// service, and laptop's programs reach it at /api/svc/studio/<name>/… on
// laptop's own API: an allowlisted path passes and one outside it is
// refused; the service is sent laptop's id in Forwarded, never a
// caller's own, and the caller's bearer untouched; the size caps hold; a
// host outside --hosts is refused; and with studio offline, laptop answers
// 503 at once.
//
// With Xenolith built (xenolith/rust/target/release/xenolith), a real
// curator on studio and a follower on laptop sync a page through
// /api/svc/studio/xenolith. Without it, that test says so and passes.
//
// NET_TEST_LOG=1 prints what each host logs.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

use async_trait::async_trait;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use serde_json::{json, Value};
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use xenonet_base::log::Log;
use xenonet_net::gwstore::Caps;
use xenonet_net::node::{now_ms, Options, Timing};
use xenonet_service::config::Config;
use xenonet_service::cores::{self, Cores};
use xenonet_service::ctx::Ctx;
use xenonet_service::gateway::actor_gateway::GatewayCores;
use xenonet_service::gateway::control::{ControlParts, GatewayControl};
use xenonet_service::gateway::network::NetService;
use xenonet_service::gateway::reach::SystemNetwork;
use xenonet_service::gateway::tunnel::SystemHost;
use xenonet_service::http::{Peer, Req};
use xenonet_service::routes::api;

const LAN: u16 = 48131;
const XENOLITH_LAN: u16 = 48134;
/// laptop's own API, listening for Xenolith's follower in the second test.
const XENOLITH_API: u16 = 48135;
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

fn timing() -> Timing {
    Timing { ping_ms: 400, stream_ms: 12_000, idle_ms: 2500, poll_ms: 3000, backoff_min_ms: 150.0, backoff_max_ms: 800.0, handover_ms: 1000 }
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
    // A caller waits 6 s here, not 90, so the timeout shows quickly.
    o.svc_ms = 6000;
    o
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
        let mut cores = Cores::pending(false, false, false);
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
        let r = api(&self.ctx, req).await.unwrap();
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
        let r = api(&self.ctx, req).await.unwrap();
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
    let (s, t) = maker.window("POST", "/api/net/ticket", json!({ "host": h.name, "window": false })).await;
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
                            "/sync/v1/big" => (200, vec![b'b'; (4 << 20) + 1]),
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
    let (s, v) = gw.window("POST", "/api/net/create", json!({ "name": "home", "host": "hub", "lan": format!("127.0.0.1:{LAN}") })).await;
    assert_eq!(s, 200, "{v}");
    let a = Host::open("studio", root.join("studio"), 47996).await;
    join(&a, &gw).await;
    let b = Host::open("laptop", root.join("laptop"), 47997).await;
    join(&b, &gw).await;
    for h in [&gw, &a, &b] {
        live(h, 3).await;
    }

    let seen = Arc::new(Mutex::new(Vec::new()));
    let port = made_up_service(seen.clone()).await;
    let target = format!("http://127.0.0.1:{port}");

    // Publishing is the owner's: refused from under an agent.
    a.peers.agent.store(true, Ordering::SeqCst);
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "files", "target": target, "paths": ["/sync/v1/"] })).await;
    assert_eq!((s, v["code"].clone()), (403, json!("yours")), "{v}");
    a.peers.agent.store(false, Ordering::SeqCst);
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
    // as the frame's signature proved it, and nothing the caller said
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
    // The gateway carried it and never had the bearer in the clear.
    let gw_db = root.join("hub").join("state").join("network");
    for f in ["gateway.sqlite", "gateway.sqlite-wal"] {
        let bytes = std::fs::read(gw_db.join(f)).unwrap_or_default();
        assert!(!bytes.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()), "{f} holds the bearer");
    }

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

    // The size caps: a body over 1 MiB, an answer over 4 MiB.
    let (s, h, _) = b.call("POST", "/api/svc/studio/files/sync/v1/echo", Some(&vec![b'x'; (1 << 20) + 1]), &[]).await;
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

    // Taken off the network: not published any more.
    let (s, v) = a.window("POST", "/api/net/svc/unpublish", json!({ "name": "private" })).await;
    assert_eq!(s, 200, "{v}");
    let (s, h, _) = gw.call("GET", "/api/svc/studio/private/sync/v1/x", None, &[]).await;
    assert_eq!((s, header(&h, "x-xenonet-refused")), (404, Some("not-published")));

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

// ---------- Xenolith's sync, when it's built ----------

fn xenolith_bin() -> Option<PathBuf> {
    let exe = if cfg!(windows) { "xenolith.exe" } else { "xenolith" };
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../xenolith/rust/target/release").join(exe);
    p.exists().then_some(p)
}

/// One machine's Xenolith: its folder and its config folder.
struct Xenolith {
    bin: PathBuf,
    wiki: PathBuf,
    home: PathBuf,
}

impl Xenolith {
    fn new(bin: &Path, dir: &Path) -> Xenolith {
        let x = Xenolith { bin: bin.to_path_buf(), wiki: dir.join("wiki"), home: dir.join("xhome") };
        std::fs::create_dir_all(&x.home).unwrap();
        x
    }

    fn command(&self, args: &[&str]) -> std::process::Command {
        let mut c = std::process::Command::new(&self.bin);
        c.args(args).env("XENOLITH_DIR", &self.wiki).env("XENOLITH_HOME", &self.home).env_remove("XENOLITH_TOKEN").env_remove("XENOLITH_WRITE_MODE");
        c
    }

    fn ok(&self, args: &[&str], input: Option<&str>) -> String {
        use std::io::Write as _;
        let mut c = self.command(args);
        c.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let mut child = c.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        if let Some(i) = input {
            stdin.write_all(i.as_bytes()).unwrap();
        }
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "xenolith {}: {}{}", args.join(" "), String::from_utf8_lossy(&out.stderr), String::from_utf8_lossy(&out.stdout));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn atlas(&self) -> PathBuf {
        self.wiki.join("spaces").join("project").join("atlas")
    }
}

/// laptop's own API on a port of its own, as `xenonet serve` listens, for
/// Xenolith's follower to call.
async fn listen(ctx: Arc<Ctx>, port: u16) -> tokio::task::JoinHandle<()> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((stream, remote)) = listener.accept().await else { return };
            let local = stream.local_addr().unwrap();
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let answer = hyper::service::service_fn(move |req| {
                    let ctx = ctx.clone();
                    async move { Ok::<_, Infallible>(xenonet_service::serve::to_response(xenonet_service::serve::handle(&ctx, req, Peer { remote, local }).await)) }
                });
                let _ = hyper::server::conn::http1::Builder::new().serve_connection(hyper_util::rt::TokioIo::new(stream), answer).await;
            });
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_xenolith_follower_syncs_with_its_curator_through_the_network() {
    let Some(bin) = xenolith_bin() else {
        eprintln!("skipped: Xenolith isn't built (cargo build --release in xenolith/rust)");
        return;
    };
    let root = std::env::temp_dir().join(format!("xenonet-net-svc-xenolith-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    // studio curates project:atlas and serves Xenolith on a port of its own.
    let curator = Xenolith::new(&bin, &root.join("studio"));
    curator.ok(&["serve", "--init"], None);
    curator.ok(&["spaces", "add", "project:atlas"], None);
    let page = "---\ntitle: Atlas database\ntype: project\n---\n\n# Atlas database\n\nPostgres 16 on the studio box.\n";
    std::fs::write(curator.atlas().join("pages").join("db.md"), page).unwrap();
    curator.ok(&["grant", "machine:laptop", "ctx:write:project:atlas"], None);
    let token: Value = serde_json::from_str(&curator.ok(&["token", "create", "machine:laptop", "--json"], None)).unwrap();
    let token = token["token"].as_str().unwrap().to_string();
    let mut server =
        curator.command(&["serve", "--http", "--port", "0", "--parent-stdin"]).stdin(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    // Its log is read to the end, so it never writes to a closed pipe.
    let (tx, rx) = std::sync::mpsc::channel();
    let stderr = server.stderr.take().unwrap();
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(port) = line.split("listening on http://127.0.0.1:").nth(1).and_then(|r| r.split('/').next()).and_then(|p| p.parse::<u16>().ok()) {
                let _ = tx.send(port);
            }
        }
    });
    let port = rx.recv_timeout(Duration::from_secs(30)).expect("Xenolith said where it listens");

    // studio founds the network and publishes Xenolith's sync routes;
    // laptop joins, with its own API listening.
    let a = Host::open("studio", root.join("studio"), 47998).await;
    let (s, v) = a.window("POST", "/api/net/create", json!({ "name": "home", "host": "studio", "lan": format!("127.0.0.1:{XENOLITH_LAN}") })).await;
    assert_eq!(s, 200, "{v}");
    let b = Host::open("laptop", root.join("laptop"), XENOLITH_API).await;
    join(&b, &a).await;
    live(&a, 2).await;
    live(&b, 2).await;
    let door = listen(b.ctx.clone(), XENOLITH_API).await;
    let (s, v) = a.window("POST", "/api/net/svc/publish", json!({ "name": "xenolith", "target": format!("http://127.0.0.1:{port}"), "paths": ["/sync/v1/"] })).await;
    assert_eq!(s, 200, "{v}");

    // laptop follows it at the address its XenoNet gives, as Xenolith's
    // README says, and pulls the page.
    let follower = Xenolith::new(&bin, &root.join("laptop"));
    follower.ok(&["serve", "--init"], None);
    follower.ok(&["sync", "name", "laptop"], None);
    let peer = format!("http://127.0.0.1:{XENOLITH_API}/api/svc/studio/xenolith");
    follower.ok(&["sync", "peer", "add", "studio", &peer], Some(&token));
    follower.ok(&["sync", "follow", "project:atlas", "studio"], None);
    let follower_bin = follower.bin.clone();
    let (wiki, home) = (follower.wiki.clone(), follower.home.clone());
    // The sync blocks on HTTP to this process's own API: off the runtime.
    let synced = tokio::task::spawn_blocking(move || Xenolith { bin: follower_bin, wiki, home }.ok(&["sync", "--json"], None)).await.unwrap();
    let r: Value = serde_json::from_str(&synced).unwrap();
    assert_eq!((r["spaces"][0]["space"].clone(), r["spaces"][0]["pulled"].clone()), (json!("project:atlas"), json!(1)), "{r}");
    assert_eq!(std::fs::read_to_string(follower.atlas().join("pages").join("db.md")).unwrap(), page, "the page came through the network");

    door.abort();
    let _ = server.kill();
    let _ = server.wait();
    drop((a, b));
    let _ = std::fs::remove_dir_all(root);
}
