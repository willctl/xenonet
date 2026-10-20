//! The binary's drivers for the engine, delivery and actors (core/engine,
//! core/delivery, core/actors, core/procs.js): their collaborators, timers,
//! processes and waits, natively, where the twins' JS answers the same
//! Rust cores during the port.

// The cores' collaborators answer with xenonet-engine's Thrown, which is
// passed on as it is: what a JS call threw, kept whole.
#![expect(clippy::result_large_err, reason = "the cores' Calls return xenonet-engine's Thrown, passed on whole")]

pub mod actors;
pub mod connect;
pub mod delivery;
pub mod engine;
pub mod exec;
pub mod fsx;
pub mod js;
pub mod map;
pub mod observers;
pub mod procs;
pub mod registration;
pub mod reports;
pub mod spend;
pub mod store;
pub mod tokens;
pub mod verify;
pub mod wakes;
pub mod world;

use std::path::Path;
use std::sync::{Arc, OnceLock};
use world::{Setup, World};
use xenonet_engine::store::{EngineStore, Shared};
use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// A line for the service's log once it's up: what went wrong while the
/// cores were made, before there was a log to write it in.
fn log_later(env: &WireEnv, level: &'static str, code: &'static str, message: String) {
    let env = env.clone();
    tokio::spawn(async move {
        for _ in 0..600 {
            if env.emitter.ctx().is_some() {
                env.emitter.log(level, code, &message);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

/// The demo's made-up folders, never your own: its world, made now if the
/// map's driver didn't make it, or the folder it would be in.
fn demo_folders(cores: &Cores, config: &mut Config) {
    if !config.demo || config.world.is_some() {
        return;
    }
    match cores.demo.create_world() {
        Ok(world) => config.use_world(&world),
        Err(_) => config.demo_folders(),
    }
}

/// Puts this area's cores in place of the pending ones, as server/index.js
/// makes them: the engine's store, the process checks, the engine and
/// delivery, and with actors on, the actors, their registration, wakes,
/// reports and observers (never in the demo).
pub fn wire(cores: &mut Cores, config: &mut Config, env: &WireEnv) {
    let Some(w) = world(cores, config, env) else { return };
    cores.engine_store = Arc::new(store::Store(w.store.clone()));
    cores.verify = w.checks.clone();
    cores.engine = Arc::new(engine::EngineDriver(w.clone()));
    cores.connect = Arc::new(connect::ConnectDriver::new(config));
    cores.channels = Arc::new(delivery::ChannelsDriver(w.clone()));
    cores.policy = Arc::new(delivery::PolicyDriver(w.clone()));
    cores.router = Arc::new(delivery::RouterDriver(w.clone()));
    cores.reports = Arc::new(reports::ReportsDriver(w.clone()));
    cores.spend = Arc::new(spend::SpendDriver(w.clone()));
    if w.actors.is_some() {
        cores.actors = Some(Arc::new(actors::ActorsDriver(w.clone())));
        cores.registration = Some(Arc::new(registration::RegistrationDriver(w.clone())));
        cores.wakes = Some(Arc::new(wakes::WakesDriver(w.clone())));
    } else {
        cores.actors = None;
        cores.registration = None;
        cores.wakes = None;
    }
}

/// The area's parts, made as server/index.js's top level makes them, on
/// the map the cores already have; None when engine.sqlite can't be
/// opened, and the engine's routes stay pending.
pub fn world(cores: &Cores, config: &mut Config, env: &WireEnv) -> Option<Arc<World>> {
    demo_folders(cores, config);
    let file = Path::new(&config.state_dir).join("engine.sqlite");
    if let Err(e) = std::fs::create_dir_all(&config.state_dir) {
        log_later(env, "error", "engine.store", format!("the state folder can't be made: {e}"));
    }
    let store = match EngineStore::open(&file.to_string_lossy(), js::now_ms()) {
        Ok(s) => Shared::new(s),
        Err(e) => {
            log_later(env, "error", "engine.store", format!("engine.sqlite can't be opened: {e}"));
            return None;
        }
    };
    // The engine's store, listed with the map's blocks in /api/health and
    // the doctor.
    let held = store.clone();
    let _ = cores.state.add_block(Arc::new(move || held.with(|s| store::block_health("engine", "store", &s.block.life))), None);
    let setup = Setup {
        demo: config.demo,
        port: config.port,
        home: config.home.clone(),
        state_dir: config.state_dir.clone(),
        projects_dir: config.projects_dir.clone(),
        version: env.version.clone(),
        root: config.root.clone(),
    };
    let map = map::Map::new(cores.state.clone());
    let sys = procs::System::new();
    let checks = Arc::new(verify::Checks::new(store.clone(), sys.clone()));
    // The settings a Claude Code session starts with to listen after its
    // turns, which a resumed turn gets too.
    let listen_file = if config.demo {
        None
    } else {
        delivery::listen_settings(&config.state_dir, config.port).map_err(|e| log_later(env, "warn", "delivery.listen", format!("listen settings: {e}"))).ok()
    };
    let actors = if config.actors {
        let names = actors::DemoNames::new(&config.projects_dir);
        match actors::ActorSide::new(store.clone(), config.demo, &map, names) {
            Ok(a) => Some(a),
            Err(e) => {
                log_later(env, "error", "actors.start", format!("the actors couldn't be read: {e}"));
                None
            }
        }
    } else {
        None
    };
    let registration = actors.as_ref().map(|_| registration::RegSide::new(store.clone(), &config.home));
    let wakes = actors.as_ref().map(|_| wakes::WakeSide::new(store.clone(), config.demo, &env.version));
    let state_dir = Path::new(&config.state_dir);
    let w = Arc::new(World {
        setup,
        env: env.clone(),
        rt: tokio::runtime::Handle::current(),
        store: store.clone(),
        map,
        sys,
        checks: checks.clone(),
        engine: xenonet_engine::engine::Engine::default(),
        mail: engine::Mail::default(),
        repos: engine::Repos::default(),
        policy: delivery::PolicySide::new(
            Some(state_dir.join("delivery.json").to_string_lossy().into_owned()),
            Some(state_dir.join("delivery-audit.jsonl").to_string_lossy().into_owned()),
        ),
        router: delivery::RouterSide::new(config.demo, listen_file),
        channels: delivery::ChannelSide::default(),
        actors,
        registration,
        wakes,
        observers: OnceLock::new(),
        me: OnceLock::new(),
    });
    let _ = w.me.set(Arc::downgrade(&w));
    if w.actors.is_some() && !config.demo {
        observers::make(&w);
    }
    Some(w)
}
