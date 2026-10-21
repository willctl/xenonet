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
use crate::wire::{Emitter, VendorReaders, WireEnv};
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
use xenonet_base::sync::Locked;
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
        Err(CoreError { status: Some(status), message, .. }) => Reply::error(status, message),
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
    if let Some(tx) = &ctx.music.locked().again {
        let _ = tx.send(ms);
    }
}

// What's playing goes to the page, a few seconds apart while a window is
// open; with the dip on, your volume dips while a session waits on you.
async fn poll_spotify(ctx: Arc<Ctx>, mut again: mpsc::UnboundedReceiver<u64>) {
    let mut wait = 0u64;
    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(wait)) => {}
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
                    let mut m = ctx.music.locked();
                    let changed = m.last != text;
                    m.last = text;
                    changed
                };
                if changed {
                    ctx.broadcast("spotify", &now);
                }
                if xenonet_base::text::truthy(st.get("duck")) {
                    let waiting = ctx.cores.state.someone_waits(now_ms()).unwrap_or(false);
                    let held = ctx.music.locked().held.clone();
                    let plan = s.duck_plan(waiting, &now, st.get("device").unwrap_or(&Value::Null), &held)?;
                    ctx.music.locked().held = plan.get("held").cloned().unwrap_or(Value::Null);
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
                let quiet = ctx.music.locked().quiet_until;
                if t > quiet {
                    ctx.log("warn", &format!("spotify.{}", e.code.as_deref().unwrap_or("error")), &e.message, None, None);
                    ctx.music.locked().quiet_until = t + 10.0 * 60e3;
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

/// A line of the service's own on stdout: where it listens, its mode, and
/// the endpoint's and the gateway's news, as server/index.js logs them.
#[expect(clippy::print_stdout, reason = "the service's ready line and its notes are its output on stdout, as the JS's console.log")]
fn say(line: &str) {
    println!("[xenonet] {line}");
}

pub fn run(args: &[String], built: &str) -> i32 {
    run_with(args, built, Box::new(|config, _env| Cores::pending(config.actors, config.assistant, config.hosts)))
}

/// What makes the service's cores: given the settings (the demo's world
/// changes its folders) and what the drivers need.
pub type Builder = Box<dyn FnOnce(&mut Config, &WireEnv) -> Cores + Send>;

/// `xenonet serve` with the cores `build` makes (the binary's drivers).
pub fn run_with(args: &[String], built: &str, build: Builder) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("[xenonet] {e}");
            return 1;
        }
    };
    let args = args.to_vec();
    let built = built.to_string();
    let code = rt.block_on(async move {
        let mut config = Config::from_env(&args);
        let env = WireEnv { version: config.version(&built), emitter: Emitter::new(), drift: Arc::new(Mutex::new(DriftLog::default())), readers: VendorReaders::default() };
        // The drivers run on the runtime: some start tasks of their own.
        let cores = build(&mut config, &env);
        serve_with(&mut config, cores, &built, env).await
    });
    // A runtime dropped waits for its blocking tasks, and some wait long
    // (an inbox's or a Stop hook's wait): stopped, the service goes now.
    rt.shutdown_timeout(Duration::from_secs(2));
    code
}

/// The service with every core pending.
pub async fn main(args: Vec<String>, built: String) -> i32 {
    let mut config = Config::from_env(&args);
    let cores = Cores::pending(config.actors, config.assistant, config.hosts);
    serve_with(
        &mut config,
        cores,
        &built,
        WireEnv { version: String::new(), emitter: Emitter::new(), drift: Arc::new(Mutex::new(DriftLog::default())), readers: VendorReaders::default() },
    )
    .await
}

pub async fn serve_with(config: &mut Config, cores: Cores, built: &str, env: WireEnv) -> i32 {
    let emitter = env.emitter.clone();
    // Demo mode swaps every source for a made-up workspace in a temp
    // folder, which the drivers made first when they could.
    if config.demo && config.world.is_none() {
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
    // What the lenient readers find in other apps' data, kept in the cache
    // folder for npm run contracts and the doctor: the log the drivers
    // were given, opened there unless one of them did.
    let drift = env.drift.clone();
    {
        let mut d = drift.locked();
        if d.view().0.is_none() {
            d.open(&Path::new(&config.cache_dir).join(DRIFT_FILE).to_string_lossy());
        }
    }
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
    ctx.music.get_mut().unwrap_or_else(std::sync::PoisonError::into_inner).again = Some(again_tx);
    ctx.drift = drift;
    let ctx = Arc::new(ctx);
    emitter.bind(&ctx);
    // The actor gateway (gateway/), for actors outside this machine: off
    // until you turn it on, on a port of its own beside this one.
    if let (Some(actors), Some(registration)) = (ctx.cores.actors.clone(), ctx.cores.registration.clone()) {
        let (state, targets) = (ctx.cores.state.clone(), actors.clone());
        let c = ctx.clone();
        let parts = crate::gateway::control::ControlParts {
            cores: crate::gateway::actor_gateway::GatewayCores {
                engine: ctx.cores.engine.clone(),
                registration,
                actors,
                reports: ctx.cores.reports.clone(),
                runs: ctx.cores.runs.clone(),
            },
            store: ctx.cores.engine_store.clone(),
            // A run an actor reports from elsewhere: a session on the map,
            // with a beam from its actor.
            on_run: Arc::new(move |actor: &str, id: &str| {
                let (state, targets, actor, id) = (state.clone(), targets.clone(), actor.to_string(), id.to_string());
                tokio::spawn(async move {
                    if state.refresh_after_change().await.is_ok() {
                        let _ = targets.target(&actor, &id, "works-on");
                    }
                });
            }),
            port: config.gateway_port,
            service_port: config.port,
            version: version.clone(),
            dir: (!config.demo).then(|| config.state_dir.clone()),
            home: config.home.clone(),
            probe_local: config.gateway_probe_local,
            log: Arc::new(move |code: &str, detail: &str| c.log(if code == "gateway.error" { "warn" } else { "info" }, code, detail, None, None)),
            tunnel_host: Arc::new(crate::gateway::tunnel::SystemHost),
            network: Arc::new(crate::gateway::reach::SystemNetwork),
            now: Arc::new(now_ms),
        };
        let _ = ctx.gateway.set(crate::gateway::control::GatewayControl::new(parts));
    }
    // A2A agents, which reach the sessions as actors through the gateway
    // (a2a/): off until you turn it on in the Actors panel.
    if let Some(actors) = ctx.cores.actors.clone() {
        let state_dir = config.state_dir.clone();
        let parts = crate::a2a::A2aParts {
            cores: crate::a2a::A2aCores { engine: ctx.cores.engine.clone(), actors: actors.clone() },
            file: (!config.demo).then(|| Path::new(&config.state_dir).join("a2a.sqlite")),
            version: version.clone(),
            home: config.home.clone(),
            name: Arc::new(move || crate::a2a::network_name(&state_dir)),
            now: Arc::new(now_ms),
            limits: crate::a2a::Limits::default(),
        };
        match crate::a2a::A2a::new(parts) {
            Ok(a) => {
                // A removed actor's tasks go with its record.
                let weak = Arc::downgrade(&a);
                actors.before_remove(Arc::new(move |slug: &str| {
                    if let Some(a) = weak.upgrade() {
                        a.forget(&format!("actor:{slug}"));
                    }
                }));
                // The gateway's front door answers /a2a and the card with it.
                if let Some(g) = ctx.gateway.get() {
                    g.gateway().set_a2a(a.clone());
                }
                let _ = ctx.a2a.set(a);
            }
            Err(e) => ctx.log("warn", "a2a.error", &e, None, None),
        }
    }
    // The network across machines (docs/design/network.md): this machine's
    // node, whose gateway routes the actor gateway's front door serves
    // while this host is the network's gateway.
    {
        let c = ctx.clone();
        let log: xenonet_net::sink::LogFn = Arc::new(move |code: &str, words: &str| c.log("info", code, words, None, None));
        match crate::gateway::network::NetService::open(crate::gateway::network::NetService::options(&config.state_dir, &config.home, &version, log)) {
            Ok(svc) => {
                if let Some(g) = ctx.gateway.get() {
                    g.gateway().set_net(svc.node().clone());
                }
                // The engine takes what arrives, and sends through it.
                if let Some(plug) = &ctx.cores.on_net {
                    plug(&svc);
                }
                let _ = ctx.net.set(svc);
            }
            Err(e) => ctx.log(
                "warn",
                "net.open",
                &format!("the network's files couldn't be read: {}", e.message),
                None,
                Some("xenonet doctor says more; xenonet net leave forgets the network"),
            ),
        }
    }

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
    let port = listener.local_addr().map_or(config.port, |a| a.port());
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();

    if config.standalone {
        // No app started this service, so the Desktop that connects is the
        // one the window's requests come from (verify.desktop).
        let c = ctx.clone();
        let person = ctx.clone();
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
            on_person: Arc::new(move |peer| {
                let c = person.clone();
                Box::pin(async move { c.cores.verify.person(peer).await })
            }),
            terminals: ctx.cores.terminals.clone(),
        };
        match component::start(config.descriptor_file(), hooks).await {
            Ok(c) => *ctx.components.locked() = Some(c),
            Err(e) => {
                eprintln!("[xenonet] {e}");
                return 1;
            }
        }
    }

    say(&format!("http://{}:{port}{}", config.host, if config.dev { "  (dev)" } else { "" }));
    say(&format!("projects: {}", config.projects_dir));
    if config.demo {
        say("demo mode: made-up projects and sessions, nothing real is read");
    }
    if let Some(up) = &ctx.upstream {
        say(&format!("the page reads {}, and changes nothing there", up.host));
    } else if std::env::var("XENONET_UPSTREAM").is_ok_and(|u| !u.is_empty()) {
        say("XENONET_UPSTREAM is for XenoNet Dev (--dev) only; this XenoNet reads its own");
    }

    // Remote actors' private endpoint, listening while a grant could be
    // used, and the connections' clock.
    if let (Some(connections), Some(service)) = (ctx.cores.connections.clone(), ctx.cores.assistant.clone()) {
        let e = crate::assistant_endpoint::Endpoint::new(connections.clone(), service, ctx.cores.audit.clone(), config.endpoint_file(), Arc::new(say));
        *ctx.endpoint.locked() = Some(Arc::new(e.clone()));
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
    if let Some(g) = ctx.gateway.get().cloned() {
        tokio::spawn(async move {
            g.init().await;
            if g.gateway().listening() {
                say(&format!("the actor gateway listens on 127.0.0.1:{}", g.gateway().port()));
            }
        });
    }
    if let Some(t) = ctx.cores.tunnels.clone() {
        tokio::spawn(async move {
            let _ = t.autostart().await;
        });
    }
    if let Some(n) = ctx.net.get().cloned() {
        let c = ctx.clone();
        tokio::spawn(async move {
            if let Err(e) = n.start().await {
                c.log("warn", "net.start", &format!("the network's link didn't start: {}", e.message), None, None);
            }
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
        () = serving => {}
        _ = stop_rx.recv() => {}
        () = signal() => {}
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
    c.terminals.kill_all();
    if let Some(t) = &c.tunnels {
        t.stop_all();
    }
    if let Some(w) = &c.wakes {
        w.stop();
    }
    if let Some(g) = ctx.gateway.get() {
        g.tunnel().stop();
        g.gateway().stop();
    }
    if let Some(n) = ctx.net.get() {
        n.stop();
    }
    if let Some(comp) = ctx.components.locked().take() {
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
    ctx.drift.locked().flush();
    let _ = json!(null);
}
