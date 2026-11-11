//! The engine running on this machine: the runtime its cores run in
//! (`runtime`), the drivers for the engine, delivery and the network
//! across machines, starts, asks and runs, the process checks and the
//! store's sides, the actors' sides (registration, wakes, reports, tokens,
//! spend, observers), the person's intents (`intents`), the local door's
//! decisions (`door`), and the timers,
//! processes and waits they need. The
//! adapters are the ones in place (`xenonet_adapter::platforms()`), which
//! the binary installs at start.

// The service's traits answer with its CoreError, and a command that
// failed with what it wrote (exec::Failed): both are passed on whole.
#![expect(clippy::result_large_err, reason = "the service's traits return its CoreError, and a failed command keeps its output")]

pub mod acp;
pub mod actors;
pub mod asks;
pub mod checkouts;
pub mod delivery;
pub mod door;
pub mod engine;
pub mod exec;
pub mod fs;
pub mod grants;
pub mod ground;
pub mod intents;
pub mod map;
pub mod network;
pub mod observers;
pub mod procs;
pub mod registration;
pub mod remote;
pub mod reports;
pub mod runs;
pub mod runtime;
pub mod saved;
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
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_store::engine::{EngineStore, Shared};
use xenonet_store::Db;

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
/// (`network::plug`), or None when the store couldn't be opened.
/// Connecting the agents (`cores.connect`) is the composition's to set.
pub fn wire(cores: &mut Cores, setup: Setup, env: &WireEnv) -> Option<Arc<Runtime>> {
    let w = runtime(cores, setup, env)?;
    cores.engine_store = w.tables.clone();
    cores.verify = w.checks.clone();
    cores.engine = Arc::new(engine::EngineDriver(w.clone()));
    cores.channels = Arc::new(delivery::ChannelsDriver(w.clone()));
    cores.policy = Arc::new(delivery::PolicyDriver(w.clone()));
    cores.router = Arc::new(delivery::RouterDriver(w.clone()));
    cores.reports = Arc::new(reports::ReportsDriver(w.clone()));
    cores.spend = Arc::new(spend::SpendDriver::new(w.clone()));
    cores.grants = Arc::new(grants::GrantsDriver(w.clone()));
    cores.intents = Arc::new(intents::IntentsDriver(w.clone()));
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

/// The one store, `<state>/xenonet.sqlite`, which the service opens once
/// and gives every part that keeps something; None, with why in the log,
/// when it can't be opened, as when the folder holds data from an earlier
/// build (the doctor says to set the machine up again).
pub fn open_store(state_dir: &str, env: &WireEnv) -> Option<Arc<Db>> {
    match Db::open(Path::new(state_dir)) {
        Ok(db) => {
            // The ids it mints, the same each run for the golden files,
            // which name a seed.
            if let Ok(seed) = std::env::var("XENONET_IDS_SEED") {
                db.use_ids(xenonet_model::ids::Ids::seeded(&seed));
            }
            Some(Arc::new(db))
        }
        Err(e) => {
            log_later(env, "error", "engine.store", format!("the store can't be opened: {}", e.message));
            None
        }
    }
}

/// The store's random ids from `seed` from now on: the same ids each run,
/// for a made-up world.
pub fn seed_ids(db: &Db, seed: &str) {
    db.use_ids(xenonet_model::ids::Ids::seeded(seed));
}

/// The runtime, made on the map the cores already have; None without the
/// store (`Setup::db`), and the engine's routes stay pending.
pub fn runtime(cores: &Cores, setup: Setup, env: &WireEnv) -> Option<Arc<Runtime>> {
    let now = xenonet_text::time::now_ms();
    let mut life = Lifecycle::new(now);
    let opened = EngineStore::new(setup.db.clone()?, now);
    let store = match opened {
        Ok(s) => Shared::new(s),
        Err(e) => {
            log_later(env, "error", "engine.store", format!("the engine's tables can't be read: {e}"));
            return None;
        }
    };
    life.count("opens", 1.0);
    life.ok(now);
    // The store, listed with the map's blocks in /api/health and the
    // doctor.
    let _ = cores.state.add_block(Arc::new(move || store::block_health("engine", "store", &life)), None);
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
        let names = actors::DemoNames::new(&setup.projects_dir, &setup.made_up, setup.db.clone());
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
    let policy_store = store.clone();
    let runs_db = setup.db.clone();
    let w = Arc::new(Runtime {
        setup,
        env: env.clone(),
        rt: tokio::runtime::Handle::current(),
        tables: Arc::new(store::Store(store.clone())),
        store,
        map,
        sys,
        checks,
        engine: xenonet_engine::engine::Engine::default(),
        mail: engine::Mail::default(),
        repos: engine::Repos::default(),
        policy: delivery::PolicySide::new(Some(policy_store)),
        router: delivery::RouterSide::new(demo, listen_file, state_dir, runs_db),
        channels: delivery::ChannelSide::default(),
        actors,
        registration,
        wakes,
        observers: OnceLock::new(),
        me: OnceLock::new(),
        net: network::NetState::default(),
        starts: starts::StartSide::default(),
        asks: asks::AskSide::default(),
        intents: intents::IntentSide::default(),
        acp: OnceLock::new(),
    });
    let _ = w.me.set(Arc::downgrade(&w));
    if let Some(db) = &w.setup.db {
        checkouts::own(db, Arc::downgrade(&w));
    }
    if w.actors.is_some() && !demo {
        observers::make(&w);
    }
    Some(w)
}
