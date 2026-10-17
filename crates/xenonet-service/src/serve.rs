//! `xenonet serve`: what server/index.js's top level does. It makes the
//! cores (each one not in Rust yet stands in as pending), listens on
//! 127.0.0.1 with the Host and Origin checks, starts the component service
//! when it runs standalone, and the background work, and stops on SIGINT,
//! SIGTERM or the component service's stop.

use crate::buckets::now_ms;
use crate::component::{self, Hooks};
use crate::config::Config;
use crate::cores::{CoreError, Cores};
use crate::ctx::Ctx;
use crate::http::{node_headers, parse_target, Body, Peer, Reply, ReplyBody, Req};
use crate::routes;
use bytes::Bytes;
use hyper::body::{Frame, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use xenonet_base::log::{log_dir, ErrInfo, Log, MAX_BYTES};
use xenonet_schema::drift::{DriftLog, DRIFT_FILE};

pub use crate::glue::notify_doctor;

/// What an answer's body is to hyper: all of it, or a stream.
pub enum OutBody {
    Full(Option<Bytes>),
    Stream(mpsc::UnboundedReceiver<Bytes>),
}

impl hyper::body::Body for OutBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            OutBody::Full(b) => Poll::Ready(b.take().filter(|b| !b.is_empty()).map(|b| Ok(Frame::data(b)))),
            OutBody::Stream(rx) => rx.poll_recv(cx).map(|b| b.map(|b| Ok(Frame::data(b)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self, OutBody::Full(None))
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        match self {
            OutBody::Full(Some(b)) => hyper::body::SizeHint::with_exact(b.len() as u64),
            OutBody::Full(None) => hyper::body::SizeHint::with_exact(0),
            OutBody::Stream(_) => hyper::body::SizeHint::default(),
        }
    }
}

pub fn to_response(reply: Reply) -> hyper::Response<OutBody> {
    let mut b = hyper::Response::builder().status(reply.status);
    for (k, v) in &reply.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let body = match reply.body {
        ReplyBody::Full(bytes) => OutBody::Full(Some(bytes)),
        ReplyBody::Stream(rx) => OutBody::Stream(rx),
    };
    b.body(body).unwrap_or_else(|_| hyper::Response::new(OutBody::Full(None)))
}

/// One request, as server.on("request") handles it: transcripts are
/// private, so DNS-rebinding hosts and cross-site posts are refused before
/// anything else.
pub async fn handle(ctx: &Arc<Ctx>, req: hyper::Request<Incoming>, peer: Peer) -> Reply {
    let headers = node_headers(req.headers());
    let get = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let host = get("host").unwrap_or_default();
    let port = ctx.config.port;
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Reply::error(421, "bad host");
    }
    let method = req.method().as_str().to_string();
    if method != "GET" {
        if let Some(origin) = get("origin") {
            if origin != format!("http://127.0.0.1:{port}") && origin != format!("http://localhost:{port}") {
                return Reply::error(403, "bad origin");
            }
        }
    }
    let target = req.uri().to_string();
    let (path, query) = parse_target(&target);
    if let Some(up) = ctx.upstream.as_ref().filter(|u| u.handles(&path)) {
        return up.pass(&method, &target, &headers).await;
    }
    let incoming = req.into_body();
    let request = Req { method: method.clone(), path: path.clone(), query, headers, peer, body: Body::Incoming(incoming) };
    if !path.starts_with("/api/") {
        return crate::static_files::serve(&ctx.config.root.join("dist"), &path);
    }
    let answer = routes::api(ctx, request).await;
    answered(ctx, &method, &path, answer)
}

/// What the service answers for a route's result: what a core threw goes
/// out with its status, a core not in Rust yet says so, and anything else
/// is logged and an internal error.
pub fn answered(ctx: &Ctx, method: &str, path: &str, answer: Result<Reply, CoreError>) -> Reply {
    match answer {
        Ok(r) => r,
        Err(e) if e.is_pending() => Reply::coded(501, "pending", e.message),
        Err(e) if e.status.is_some() => Reply::error(e.status.unwrap(), e.message),
        Err(e) => {
            eprintln!("{}", e.message);
            let short = shorten(path);
            ctx.log("error", "service.route", &format!("{method} {short}: {}", e.message), Some(&ErrInfo { message: Some(e.message.clone()), stack: None, code: None }), None);
            Reply::error(500, "internal error")
        }
    }
}

/// A path for the log, with long segments (ids, tokens) cut: [^/]{24,} → …
fn shorten(path: &str) -> String {
    path.split('/').map(|s| if s.chars().count() >= 24 { "…" } else { s }).collect::<Vec<_>>().join("/")
}

/// Pokes Spotify's poll to look again after `ms`.
pub fn poll_spotify_soon(ctx: &Ctx, ms: u64) {
    if let Some(tx) = &ctx.music.lock().unwrap().again {
        let _ = tx.send(ms);
    }
}

// What's playing goes to the page, a few seconds apart while a window is
// open; with the dip on, your volume dips while a session waits on you.
async fn poll_spotify(ctx: Arc<Ctx>, mut again: mpsc::UnboundedReceiver<u64>) {
    let mut wait = 0u64;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
            got = again.recv() => match got {
                Some(ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                None => return,
            },
        }
        let s = &ctx.cores.spotify;
        wait = if ctx.events.count() > 0 { 5000 } else { 30000 };
        let looked: Result<(), CoreError> = async {
            let st = s.status().await?;
            if xenonet_base::text::truthy(st.get("connected")) {
                let now = s.player().await?;
                let text = xenonet_base::json::stringify(&now, 0);
                let changed = {
                    let mut m = ctx.music.lock().unwrap();
                    let changed = m.last != text;
                    m.last = text;
                    changed
                };
                if changed {
                    ctx.broadcast("spotify", &now);
                }
                if xenonet_base::text::truthy(st.get("duck")) {
                    let waiting = ctx.cores.state.someone_waits(now_ms()).unwrap_or(false);
                    let held = ctx.music.lock().unwrap().held.clone();
                    let plan = s.duck_plan(waiting, &now, st.get("device").unwrap_or(&Value::Null), &held)?;
                    ctx.music.lock().unwrap().held = plan.get("held").cloned().unwrap_or(Value::Null);
                    if plan["op"] == "volume" {
                        s.command("volume", &plan["volume"]).await?;
                    }
                }
            }
            Ok(())
        }
        .await;
        match looked {
            Ok(()) => {}
            // Spotify isn't in the Rust service yet: no poll, and nothing to log.
            Err(e) if e.is_pending() => return,
            Err(e) => {
                // Spotify's asked XenoNet to wait, needs Premium, or isn't
                // reachable: say so once in a while, and look again later.
                let t = now_ms();
                let quiet = ctx.music.lock().unwrap().quiet_until;
                if t > quiet {
                    ctx.log("warn", &format!("spotify.{}", e.code.as_deref().unwrap_or("error")), &e.message, None, None);
                    ctx.music.lock().unwrap().quiet_until = t + 10.0 * 60e3;
                }
                wait = 60000;
            }
        }
    }
}

/// Who holds a port, when it's another XenoNet.
fn holder(port: u16) -> String {
    match xenonet_base::http::request(port, "GET", "/api/health", &[], None, Some(Duration::from_millis(1500))) {
        Ok(a) if a.status < 300 => {
            let h = a.body.ok().and_then(|b| xenonet_base::json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or(Value::Null);
            let name = h.get("name").and_then(Value::as_str).unwrap_or("another XenoNet");
            let instance = h.get("instance").and_then(Value::as_str).filter(|i| !i.is_empty()).map(|i| format!(" ({i})")).unwrap_or_default();
            let pid = h.get("pid").filter(|p| xenonet_base::text::truthy(Some(p))).map(|p| format!(", pid {}", xenonet_base::text::js_string(p))).unwrap_or_default();
            format!(" by {name}{instance}{pid}")
        }
        _ => " by a program that isn't XenoNet".into(),
    }
}

pub fn run(args: &[String], built: &str) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[xenonet] {e}");
            return 1;
        }
    };
    rt.block_on(main(args.to_vec(), built.to_string()))
}

/// The service with these cores; serve.rs's own run makes them all pending.
pub async fn main(args: Vec<String>, built: String) -> i32 {
    let mut config = Config::from_env(&args);
    let cores = Cores::pending(config.actors, config.assistant, config.hosts);
    serve_with(&mut config, cores, &built).await
}

pub async fn serve_with(config: &mut Config, cores: Cores, built: &str) -> i32 {
    // Demo mode swaps every source for a made-up workspace in a temp folder.
    if config.demo {
        match cores.demo.create_world() {
            Ok(world) => config.use_world(&world),
            Err(_) => config.demo_folders(),
        }
    }
    let version = config.version(built);
    // The service's error log, in the state folder; the demo's errors stay
    // in its console.
    let dir = log_dir(&config.state_dir);
    let log = Log::new(if config.demo { None } else { Some(&dir) }, "service", config.instance.as_deref(), Some(&version), &config.home, MAX_BYTES);
    let mut drift = DriftLog::default();
    drift.open(&Path::new(&config.cache_dir).join(DRIFT_FILE).to_string_lossy());
    // XenoNet Dev reading another XenoNet, read-only: development only.
    let upstream = match crate::upstream::upstream_of(config.upstream.as_deref()) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[xenonet] {e}");
            return 1;
        }
    };
    let mut ctx = Ctx::new(config.clone(), version.clone(), cores, log);
    ctx.upstream = upstream;
    let (again_tx, again_rx) = mpsc::unbounded_channel();
    ctx.music.get_mut().unwrap().again = Some(again_tx);
    ctx.drift = Arc::new(Mutex::new(drift));
    let ctx = Arc::new(ctx);

    let listener = match TcpListener::bind((config.host.as_str(), config.port)).await {
        Ok(l) => l,
        Err(e) => {
            let in_use = e.kind() == std::io::ErrorKind::AddrInUse;
            let words = if in_use { format!("port {} is already in use{}", config.port, holder(config.port)) } else { e.to_string() };
            eprintln!("[xenonet] {words}");
            ctx.log(
                "error",
                if in_use { "service.port-in-use" } else { "service.listen" },
                &words,
                None,
                in_use.then_some("Quit the other XenoNet, or start this one with another XENONET_PORT."),
            );
            return 1;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(config.port);
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();

    if config.standalone {
        // No app started this service, so the Desktop that connects is the
        // one the window's requests come from (verify.desktop).
        let c = ctx.clone();
        let you = ctx.clone();
        let stop = stop_tx.clone();
        let hooks = Hooks {
            version: version.clone(),
            window_key: ctx.window_key.clone(),
            window_state: Arc::new(move |v: &Value| {
                if let Some(a) = &you.cores.actors {
                    a.you_window(v);
                }
            }),
            shutdown: Arc::new(move || {
                let _ = stop.send(());
            }),
            on_desktop: Arc::new(move |peer| {
                let c = c.clone();
                Box::pin(async move {
                    let seen = c.cores.verify.desktop(peer).await;
                    if peer.is_some() && !crate::cores::ok(&seen) {
                        eprintln!("[xenonet] the window's own routes stay closed: {}", crate::cores::reason(&seen));
                    }
                    seen
                })
            }),
            terminals: ctx.cores.terminals.clone(),
        };
        match component::start(config.descriptor_file(), hooks).await {
            Ok(c) => *ctx.components.lock().unwrap() = Some(c),
            Err(e) => {
                eprintln!("[xenonet] {e}");
                return 1;
            }
        }
    }

    println!("[xenonet] http://{}:{port}{}", config.host, if config.dev { "  (dev)" } else { "" });
    println!("[xenonet] projects: {}", config.projects_dir);
    if config.demo {
        println!("[xenonet] demo mode: made-up projects and sessions, nothing real is read");
    }
    if let Some(up) = &ctx.upstream {
        println!("[xenonet] the page reads {}, and changes nothing there", up.host);
    } else if std::env::var("XENONET_UPSTREAM").is_ok_and(|u| !u.is_empty()) {
        println!("[xenonet] XENONET_UPSTREAM is for XenoNet Dev (--dev) only; this XenoNet reads its own");
    }

    // Remote actors' private endpoint, listening while a grant could be
    // used, and the connections' clock.
    if let (Some(connections), Some(service)) = (ctx.cores.connections.clone(), ctx.cores.assistant.clone()) {
        let e = crate::assistant_endpoint::Endpoint::new(
            connections.clone(),
            service,
            ctx.cores.audit.clone(),
            config.endpoint_file(),
            Arc::new(|line: &str| println!("[xenonet] {line}")),
        );
        *ctx.endpoint.lock().unwrap() = Some(Arc::new(e.clone()));
        tokio::spawn(async move { e.sync().await });
        tokio::spawn(async move {
            let mut every = tokio::time::interval(Duration::from_secs(60));
            loop {
                every.tick().await;
                connections.tick();
            }
        });
    }
    // The background work, as the JS service starts it once it listens.
    ctx.cores.doctor.start();
    if let Some(a) = &ctx.cores.actors {
        a.start();
    }
    if let Some(h) = &ctx.cores.hub {
        h.start();
    }
    if let Some(t) = ctx.cores.tunnels.clone() {
        tokio::spawn(async move {
            let _ = t.autostart().await;
        });
    }
    tokio::spawn(poll_spotify(ctx.clone(), again_rx));
    {
        let c = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = c.cores.state.start().await {
                if !e.is_pending() {
                    eprintln!("[xenonet] start failed: {}", e.message);
                    c.log("error", "service.start", &format!("reading sessions failed to start: {}", e.message), None, None);
                }
            }
        });
    }

    let serving = {
        let ctx = ctx.clone();
        async move {
            loop {
                let Ok((stream, remote)) = listener.accept().await else { continue };
                let local = stream.local_addr().unwrap_or(SocketAddr::from(([127, 0, 0, 1], port)));
                let peer = Peer { remote, local };
                let ctx = ctx.clone();
                tokio::spawn(async move {
                    let service = service_fn(move |req| {
                        let ctx = ctx.clone();
                        async move { Ok::<_, Infallible>(to_response(handle(&ctx, req, peer).await)) }
                    });
                    let _ = http1::Builder::new().keep_alive(true).serve_connection(TokioIo::new(stream), service).await;
                });
            }
        }
    };

    tokio::select! {
        _ = serving => {}
        _ = stop_rx.recv() => {}
        _ = signal() => {}
    }
    shutdown(&ctx);
    0
}

async fn signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return std::future::pending().await };
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn shutdown(ctx: &Arc<Ctx>) {
    let c = &ctx.cores;
    c.system.stop();
    if let Some(t) = &c.tunnels {
        t.stop_all();
    }
    if let Some(w) = &c.wakes {
        w.stop();
    }
    if let Some(comp) = ctx.components.lock().unwrap().take() {
        comp.close();
    }
    if let Some(h) = &c.hub {
        h.stop();
    }
    if let Some(a) = &c.actors {
        a.stop();
    }
    c.state.stop();
    c.doctor.stop();
    ctx.drift.lock().unwrap().flush();
    let _ = json!(null);
}
