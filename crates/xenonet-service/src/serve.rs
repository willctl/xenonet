//! `xenonet serve`: what server/index.js's top level does. It makes the
//! cores (each one not in Rust yet stands in as pending), listens on
//! 127.0.0.1 with the Host and Origin checks, starts the component service
//! when it runs standalone, and the background work, and stops on SIGINT,
//! SIGTERM or the component service's stop.

use crate::a2a::A2a;
use crate::borrowed::now_ms;
use crate::component::{self, Components, Hooks};
use crate::config::Config;
use crate::cores::Cores;
use crate::ctx::Ctx;
use crate::gateway::control::GatewayControl;
use crate::gateway::network::NetService;
use crate::http::Peer;
use crate::wire::WireEnv;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use xenonet_http::server::{handle, poll_spotify, to_response};
use xenonet_os::log::{log_dir, Log, MAX_BYTES};
use xenonet_os::sync::Locked;
use xenonet_schema::drift::DRIFT_FILE;

/// The service's own parts, as it made them: what it starts and stops
/// itself, beside the routes' view of them (`Ctx`'s parts).
#[derive(Default)]
struct Parts {
    gateway: Option<Arc<GatewayControl>>,
    net: Option<Arc<NetService>>,
    components: Option<Arc<Components>>,
}

/// What the composition root puts in every service's cores, whatever made
/// the rest: the local door's decisions (xenonet-authz, through the
/// runtime) and the app registry as the routes ask it (the adapters).
pub fn wire_root(cores: &mut Cores) {
    cores.door = xenonet_runtime::door::local();
    cores.apps = xenonet_platforms::registry();
}

/// Who holds a port, when it's another XenoNet.
fn holder(port: u16) -> String {
    match xenonet_api::http::request(port, "GET", "/api/health", &[], None, Some(Duration::from_millis(1500))) {
        Ok(a) if a.status < 300 => {
            let h = a.body.ok().and_then(|b| xenonet_text::json::parse(&String::from_utf8_lossy(&b)).ok()).unwrap_or(Value::Null);
            let name = h.get("name").and_then(Value::as_str).unwrap_or("another XenoNet");
            let instance = h.get("instance").and_then(Value::as_str).filter(|i| !i.is_empty()).map(|i| format!(" ({i})")).unwrap_or_default();
            let pid = h.get("pid").filter(|p| xenonet_text::text::truthy(Some(p))).map(|p| format!(", pid {}", xenonet_text::text::display(p))).unwrap_or_default();
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
    run_with(args, built, Box::new(|config, _env| Cores::pending(config.actors, config.assistant)))
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
        let env = WireEnv::new(&config.version(&built));
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
    let cores = Cores::pending(config.actors, config.assistant);
    serve_with(&mut config, cores, &built, WireEnv::default()).await
}

pub async fn serve_with(config: &mut Config, mut cores: Cores, built: &str, env: WireEnv) -> i32 {
    wire_root(&mut cores);
    let mut parts = Parts::default();
    let emitter = env.emitter.clone();
    // Demo mode swaps every source for a made-up workspace in a temp
    // folder, which the drivers made first when they could.
    config.take_world(&*cores.demo);
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
    // The map's project picks ask the decider the app's driver made.
    let _ = env.decider.set(ctx.cores.decider.clone());
    emitter.bind(&ctx);
    // The actor gateway (gateway/), for actors outside this machine: off
    // until you turn it on, on a port of its own beside this one.
    if let (Some(actors), Some(registration)) = (ctx.cores.actors.clone(), ctx.cores.registration.clone()) {
        let (state, targets) = (ctx.cores.state.clone(), actors.clone());
        let c = ctx.clone();
        let gateway_parts = crate::gateway::control::ControlParts {
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
        let g = GatewayControl::new(gateway_parts);
        // The doctor reads its status.
        let _ = env.gateway.set(g.clone());
        let _ = ctx.gateway.set(g.clone());
        parts.gateway = Some(g);
    }
    // What this machine keeps of its network and person, in the store the
    // engine's runtime opened: none when it couldn't be, and then no
    // network either.
    let kept = kept_network(&ctx.cores);
    // A2A agents, which reach the sessions as actors through the gateway
    // (a2a/): off until you turn it on in the Actors panel.
    if let Some(actors) = ctx.cores.actors.clone() {
        let named = kept.clone();
        let a2a_parts = crate::a2a::A2aParts {
            cores: crate::a2a::A2aCores { engine: ctx.cores.engine.clone(), actors: actors.clone() },
            file: (!config.demo).then(|| Path::new(&config.state_dir).join("a2a.sqlite")),
            version: version.clone(),
            home: config.home.clone(),
            name: Arc::new(move || named.as_deref().and_then(crate::a2a::network_name)),
            now: Arc::new(now_ms),
            limits: crate::a2a::Limits::default(),
        };
        match A2a::new(a2a_parts) {
            Ok(a) => {
                // A removed actor's tasks go with its record.
                let weak = Arc::downgrade(&a);
                actors.before_remove(Arc::new(move |slug: &str| {
                    if let Some(a) = weak.upgrade() {
                        a.forget(&format!("actor:{slug}"));
                    }
                }));
                // The gateway's front door answers /a2a and the card with it.
                if let Some(g) = &parts.gateway {
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
    if let Some(kept) = kept {
        let c = ctx.clone();
        let log: xenonet_net::sink::LogFn = Arc::new(move |code: &str, words: &str| c.log("info", code, words, None, None));
        let mut opts = NetService::options(&config.state_dir, kept, &config.home, &version, log);
        opts.vault = Some(crate::compose::keys::vault(config.instance.as_deref(), config.demo));
        match NetService::open(opts) {
            Ok(svc) => {
                if let Some(g) = &parts.gateway {
                    g.gateway().set_net(svc.node().clone());
                }
                // The engine takes what arrives, and sends through it.
                if let Some(plug) = &ctx.cores.on_net {
                    plug(&*svc);
                }
                let _ = ctx.net.set(svc.clone());
                parts.net = Some(svc);
            }
            Err(e) => {
                ctx.log("warn", "net.open", &format!("the network couldn't be read: {}", e.message), None, Some("xenonet doctor says more; xenonet net leave forgets the network"));
            }
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
    // The API's credential, for the programs of this account: new at each
    // start, readable by this account alone, and never logged.
    let cred_file = xenonet_api::http::credential_file(Path::new(&config.runtime_dir), port);
    if let Err(e) = ctx.api_key.write(&cred_file) {
        eprintln!("[xenonet] the API's credential couldn't be written: {e}");
        ctx.log(
            "error",
            "service.credential",
            &format!("the API's credential couldn't be written to the runtime folder: {e}"),
            None,
            Some("XenoNet's window still works; the xenonet command, hooks and MCP servers can't reach the API until it's written"),
        );
    }
    let (stop_tx, mut stop_rx) = mpsc::unbounded_channel::<()>();
    // Desktop's window, as it last said: shown, hidden or closed.
    let window: Arc<Mutex<Option<String>>> = Arc::default();
    plug_hub(&ctx, parts.net.as_ref(), port, &window);

    if config.standalone {
        // No app started this service, so the Desktop that connects is the
        // one the window's requests come from (verify.desktop).
        let c = ctx.clone();
        let person = ctx.clone();
        let tray = ctx.clone();
        let you = ctx.clone();
        let stop = stop_tx.clone();
        let shown = window.clone();
        let hooks = Hooks {
            version: version.clone(),
            window_key: ctx.window_key.clone(),
            window_state: Arc::new(move |v: &Value| {
                if let Some(a) = &you.cores.actors {
                    a.you_window(v);
                }
                *shown.locked() = v.as_str().map(str::to_string);
                if let Some(h) = &you.cores.hub {
                    h.console(v.as_str(), you.events.pages());
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
            on_tray: Arc::new(move |peer| {
                let c = tray.clone();
                Box::pin(async move {
                    let seen = c.cores.verify.tray(peer, &c.config.home).await;
                    if !crate::cores::ok(&seen) {
                        c.log("warn", "service.tray", &format!("a program asked to be XenoNet's tray and was refused: {}", crate::cores::reason(&seen)), None, None);
                    }
                    seen
                })
            }),
            tray_key: ctx.tray_key.clone(),
            terminals: ctx.cores.terminals.clone(),
        };
        match component::start(config.descriptor_file(), hooks).await {
            Ok(c) => {
                *ctx.components.locked() = Some(c.clone());
                parts.components = Some(c);
            }
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
    if let Some(g) = parts.gateway.clone() {
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
    if let Some(n) = parts.net.clone() {
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
                let peer = Peer::new(remote, local);
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
    shutdown(&ctx, &parts);
    // The credential goes with this service, unless another start wrote
    // its own over it.
    if std::fs::read_to_string(&cred_file).is_ok_and(|c| ctx.api_key.matches(Some(c.trim()))) {
        let _ = std::fs::remove_file(&cred_file);
    }
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

/// The hub and the network, once both are there and the service listens:
/// the hub hears the map's relay frames, and serves this host's own map to
/// the window hosts that subscribe, reading this service's API on `port`
/// with the service's own key (the network decided what they may read). Every two seconds it's told
/// whether a console is open here, from Desktop's window as it last said
/// and the pages reading the event stream.
fn plug_hub(ctx: &Arc<Ctx>, net: Option<&Arc<NetService>>, port: u16, window: &Arc<Mutex<Option<String>>>) {
    let Some(hub) = ctx.cores.hub.clone() else { return };
    if let Some(svc) = net {
        hub.plug_net(crate::cores::HubNet {
            net: svc.net(),
            sinks: svc.sinks().clone(),
            events: ctx.events.clone(),
            api: Arc::new(crate::cores::HttpApi { port, timeout: Duration::from_secs(20), max_bytes: 16 << 20, own: Some(ctx.self_key.reveal().to_string()) }),
            home: ctx.config.home.clone(),
            version: ctx.version.clone(),
            platform: crate::borrowed::platform().to_string(),
            demo: ctx.config.demo,
        });
    }
    let (c, w) = (Arc::downgrade(ctx), window.clone());
    tokio::spawn(async move {
        let mut every = tokio::time::interval(Duration::from_secs(2));
        loop {
            every.tick().await;
            let Some(ctx) = c.upgrade() else { return };
            let shown = w.locked().clone();
            if let Some(h) = &ctx.cores.hub {
                h.console(shown.as_deref(), ctx.events.pages());
            }
        }
    });
}

fn shutdown(ctx: &Arc<Ctx>, parts: &Parts) {
    let c = &ctx.cores;
    c.system.stop();
    c.terminals.kill_all();
    if let Some(t) = &c.tunnels {
        t.stop_all();
    }
    if let Some(w) = &c.wakes {
        w.stop();
    }
    if let Some(g) = &parts.gateway {
        g.tunnel().stop();
        g.gateway().stop();
    }
    if let Some(n) = &parts.net {
        n.stop();
    }
    ctx.components.locked().take();
    if let Some(comp) = &parts.components {
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

/// Where the network and the person are kept: the store the engine's
/// runtime opened (`xenonet_runtime::persist`), or None when the engine is
/// pending.
pub fn kept_network(cores: &Cores) -> Option<Arc<dyn xenonet_net::persist::Persist>> {
    let native = cores.engine_store.native()?;
    let tables = native.downcast_ref::<xenonet_store::engine::Shared>()?;
    Some(Arc::new(xenonet_runtime::persist::StorePersist(tables.0.db().clone())))
}
