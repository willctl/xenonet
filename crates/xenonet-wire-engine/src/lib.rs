//! The binary's drivers for the engine, delivery and actors: the runtime
//! their cores run in (`runtime`), the hosts each core asks of it, and the
//! timers, processes and waits they need.

// The service's traits answer with its CoreError, and a command that
// failed with what it wrote (exec::Failed): both are passed on whole.
#![expect(clippy::result_large_err, reason = "the service's traits return its CoreError, and a failed command keeps its output")]

pub mod actors;
pub mod asks;
pub mod connect;
pub mod delivery;
pub mod engine;
pub mod exec;
pub mod fs;
pub mod map;
pub mod network;
pub mod observers;
pub mod procs;
pub mod registration;
pub mod reports;
pub mod runs;
pub mod runtime;
pub mod spend;
pub mod starts;
pub mod store;
pub mod tokens;
pub mod verify;
pub mod wakes;

use runtime::{Runtime, Setup};
use std::path::Path;
use std::sync::{Arc, OnceLock};
use xenonet_api::cores::Cores;
use xenonet_api::wire::WireEnv;
use xenonet_engine::store::{EngineStore, Shared};

/// A line for the service's log once it's up: what went wrong while the
/// cores were made, before there was a log to write it in.
fn log_later(env: &WireEnv, level: &'static str, code: &'static str, message: String) {
    let env = env.clone();
    tokio::spawn(async move {
        for _ in 0..600 {
            if env.emitter.sink().is_some() {
                env.emitter.log(level, code, &message);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });
}

/// Puts this area's cores in place of the pending ones: the engine's
/// store, the process checks, the engine and delivery, and with actors on,
/// the actors, their registration, wakes, reports and observers (never in
/// the demo). The runtime it made, for the network's link to plug into
/// (`network::plug`), or None when engine.sqlite couldn't be opened.
pub fn wire(cores: &mut Cores, setup: Setup, env: &WireEnv) -> Option<Arc<Runtime>> {
    let w = runtime(cores, setup, env)?;
    cores.engine_store = Arc::new(store::Store(w.store.clone()));
    cores.verify = w.checks.clone();
    cores.engine = Arc::new(engine::EngineDriver(w.clone()));
    cores.connect = Arc::new(connect::ConnectDriver::new(&w.setup));
    cores.channels = Arc::new(delivery::ChannelsDriver(w.clone()));
    cores.policy = Arc::new(delivery::PolicyDriver(w.clone()));
    cores.router = Arc::new(delivery::RouterDriver(w.clone()));
    cores.reports = Arc::new(reports::ReportsDriver(w.clone()));
    cores.spend = Arc::new(spend::SpendDriver::new(w.clone()));
    if w.actors.is_some() {
        cores.actors = Some(Arc::new(actors::ActorsDriver(w.clone())));
        cores.registration = Some(Arc::new(registration::RegistrationDriver(w.clone())));
        cores.wakes = Some(Arc::new(wakes::WakesDriver(w.clone())));
    } else {
        cores.actors = None;
        cores.registration = None;
        cores.wakes = None;
    }
    Some(w)
}

/// The runtime, made on the map the cores already have; None when
/// engine.sqlite can't be opened, and the engine's routes stay pending.
pub fn runtime(cores: &Cores, setup: Setup, env: &WireEnv) -> Option<Arc<Runtime>> {
    // The adapters the engine's checks and delivery ask for, in place
    // before anything asks: the binary installs them at start, and a test
    // that makes a runtime here gets them too.
    xenonet_platforms::install();
    let file = Path::new(&setup.state_dir).join("engine.sqlite");
    if let Err(e) = std::fs::create_dir_all(&setup.state_dir) {
        log_later(env, "error", "engine.store", format!("the state folder can't be made: {e}"));
    }
    let store = match EngineStore::open(&file.to_string_lossy(), xenonet_text::time::now_ms()) {
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
    let map = map::Map::new(cores.state.clone());
    let sys = procs::System::new();
    let checks = Arc::new(verify::Checks::new(store.clone(), sys.clone()));
    // The settings a Claude Code session starts with to listen after its
    // turns, which a resumed turn gets too.
    let listen_file = if setup.demo {
        None
    } else {
        delivery::listen_settings(&setup.state_dir, setup.port).map_err(|e| log_later(env, "warn", "delivery.listen", format!("listen settings: {e}"))).ok()
    };
    let actors = if setup.actors {
        let names = actors::DemoNames::new(&setup.projects_dir, &setup.made_up);
        match actors::ActorSide::new(store.clone(), setup.demo, &map, names) {
            Ok(a) => Some(a),
            Err(e) => {
                log_later(env, "error", "actors.start", format!("the actors couldn't be read: {e}"));
                None
            }
        }
    } else {
        None
    };
    let registration = actors.as_ref().map(|_| registration::RegSide::new(store.clone(), &setup.home));
    let wakes = actors.as_ref().map(|_| wakes::WakeSide::new(store.clone(), setup.demo, &env.version));
    let (demo, state_dir) = (setup.demo, setup.state_dir.clone());
    let state_dir = Path::new(&state_dir);
    let w = Arc::new(Runtime {
        setup,
        env: env.clone(),
        rt: tokio::runtime::Handle::current(),
        store,
        map,
        sys,
        checks,
        engine: xenonet_engine::engine::Engine::default(),
        mail: engine::Mail::default(),
        repos: engine::Repos::default(),
        policy: delivery::PolicySide::new(
            Some(state_dir.join("delivery.json").to_string_lossy().into_owned()),
            Some(state_dir.join("delivery-audit.jsonl").to_string_lossy().into_owned()),
        ),
        router: delivery::RouterSide::new(demo, listen_file, state_dir),
        channels: delivery::ChannelSide::default(),
        actors,
        registration,
        wakes,
        observers: OnceLock::new(),
        me: OnceLock::new(),
        net: network::NetState::default(),
        starts: starts::StartSide::default(),
        asks: asks::AskSide::default(),
    });
    let _ = w.me.set(Arc::downgrade(&w));
    if w.actors.is_some() && !demo {
        observers::make(&w);
    }
    Some(w)
}
