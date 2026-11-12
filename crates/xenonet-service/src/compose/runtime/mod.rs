//! The engine running on this machine: the runtime its cores run in
//! (`Runtime`), the drivers for the engine, delivery and the network
//! across machines, starts, asks and runs, the process checks and the
//! store's sides, the actors' sides (registration, wakes, reports, tokens,
//! spend, observers), the person's intents (`intents`), and the timers,
//! processes and waits they need. The adapters are the ones in place
//! (`xenonet_adapter::platforms()`), which the binary installs at start.
//!
//! It is the composition root's because it joins the engine, the store,
//! the network, authz's decisions, the actors and the api's cores, which
//! no library crate holds together. Where text from another machine or an
//! actor comes in (`network`'s message_in and handoff_in, `asks::ask_in`,
//! `starts::start_in`) it becomes xenonet-authz's `Information`, which
//! converts into no request, token or authorization.

// The service's traits answer with its CoreError, and a command that
// failed with what it wrote (exec::Failed): both are passed on whole.
#![expect(clippy::result_large_err, reason = "the service's traits return its CoreError, and a failed command keeps its output")]

pub mod acp;
pub mod actors;
pub mod asks;
pub mod checkouts;
pub mod delivery;
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
pub mod reports;
pub mod runs;
pub mod scrub;
pub mod spend;
pub mod starts;
pub mod store;
pub mod tokens;
pub mod verify;
pub mod wakes;

use actors::ActorSide;
use delivery::{ChannelSide, PolicySide, RouterSide};
use engine::{Mail, Repos};
use map::Map;
use procs::System;
use registration::RegSide;
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use verify::Checks;
use wakes::WakeSide;
use xenonet_api::cores::Cores;
use xenonet_api::cores::State;
use xenonet_api::wire::WireEnv;
use xenonet_api::From;
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
    let map = Map::new(cores.state.clone());
    let sys = System::new();
    let checks = Arc::new(Checks::new(store.clone(), sys.clone()));
    // The settings a Claude Code session starts with to listen after its
    // turns, which a resumed turn gets too.
    let listen_file = if setup.demo {
        None
    } else {
        delivery::listen_settings(&setup.state_dir, setup.port).map_err(|e| log_later(env, "warn", "delivery.listen", format!("listen settings: {e}"))).ok()
    };
    let actors = if setup.actors {
        let names = actors::DemoNames::new(&setup.projects_dir, &setup.made_up, setup.db.clone());
        match ActorSide::new(store.clone(), setup.demo, &map, names) {
            Ok(a) => Some(a),
            Err(e) => {
                log_later(env, "error", "actors.start", format!("the actors couldn't be read: {e}"));
                None
            }
        }
    } else {
        None
    };
    let registration = actors.as_ref().map(|_| RegSide::new(store.clone(), &setup.home));
    let wakes = actors.as_ref().map(|_| WakeSide::new(store.clone(), setup.demo, &env.version));
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
        mail: Mail::default(),
        repos: Repos::default(),
        policy: PolicySide::new(Some(policy_store)),
        router: RouterSide::new(demo, listen_file, state_dir, runs_db),
        channels: ChannelSide::default(),
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

// The runtime the engine's cores run in: the engine's store, the map's
// readers, the process table and its checks, the engine and the waits on
// it, delivery (its policy, router and channels) and, with actors on, the
// actors, their registrations, wakes and observers. Each core's host is
// made of the others here; the runtime also keeps the timers and tasks
// they start, and the few facts about this machine they ask for
// (`platform`, `alive`, local times).

/// What the area runs as: the service's settings it reads, once the demo's
/// world has moved their folders.
#[derive(Clone, Debug)]
pub struct Setup {
    pub demo: bool,
    /// Actors on (XENONET_ACTORS isn't off).
    pub actors: bool,
    pub port: u16,
    pub home: String,
    pub state_dir: String,
    pub projects_dir: String,
    pub version: String,
    /// Where XenoNet's files are: schemas/ among them.
    pub root: std::path::PathBuf,
    /// The program every run starts in place of the app's CLI
    /// (XENONET_RUNS_BIN): the tests' fake.
    pub runs_bin: Option<String>,
    /// How long a run's permission prompt waits for you (asks.rs).
    pub permission_hold_ms: Option<f64>,
    /// XENONET_LAUNCHER_EXEC: installed XenoNet's launcher started it.
    pub launcher_exec: Option<String>,
    /// The demo's world as the actors' requests name it.
    pub made_up: actors::MadeUp,
    /// The one store (`open_store`); without it the engine isn't
    /// made.
    pub db: Option<Arc<Db>>,
}

pub struct Runtime {
    pub setup: Setup,
    pub env: WireEnv,
    pub rt: tokio::runtime::Handle,
    pub store: Shared,
    /// The same tables, as the engine keeps what it knows in them.
    pub tables: Arc<store::Store>,
    pub map: Map,
    pub sys: Arc<System>,
    pub checks: Arc<Checks>,
    pub engine: xenonet_engine::engine::Engine,
    pub mail: Mail,
    pub repos: Repos,
    pub policy: PolicySide,
    pub router: RouterSide,
    pub channels: ChannelSide,
    pub actors: Option<ActorSide>,
    pub registration: Option<RegSide>,
    pub wakes: Option<WakeSide>,
    /// The observers of actor products, outside the demo.
    pub observers: OnceLock<observers::Observers>,
    /// The runtime itself, for the timers and tasks it starts.
    pub me: OnceLock<std::sync::Weak<Runtime>>,
    /// What this machine last told the network of itself (its presence
    /// and claims), and the timer that tells it again.
    pub net: network::NetState,
    /// Sessions started here, and the start requests sent from here.
    pub starts: starts::StartSide,
    /// Permission prompts of the runs XenoNet started: held here, and
    /// from other machines.
    pub asks: asks::AskSide,
    /// The person's intents being started here now.
    pub intents: intents::IntentSide,
    /// The ACP side, which the composition root puts in place: the apps
    /// started over ACP, and their sessions held open.
    pub acp: OnceLock<Arc<dyn acp::Acp>>,
}

impl Runtime {
    pub fn state(&self) -> &Arc<dyn State> {
        &self.map.state
    }

    /// The runtime, held, for a task that outlives the call.
    pub fn arc(&self) -> Option<Arc<Runtime>> {
        self.me.get().and_then(std::sync::Weak::upgrade)
    }

    /// An event for the service's handlers (the page, the hooks).
    pub fn emit(&self, from: From, name: &str, data: Value) {
        self.env.emitter.emit(from, name, data);
    }

    pub fn log(&self, level: &str, code: &str, message: &str) {
        self.env.emitter.log(level, code, message);
    }

    /// Runs blocking work on tokio's blocking threads, with the runtime.
    pub fn spawn_blocking(&self, f: impl FnOnce(&Runtime) + Send + 'static) {
        if let Some(w) = self.arc() {
            self.rt.spawn_blocking(move || f(&w));
        }
    }

    /// Runs `f` after `ms`, on a blocking thread, unless the runtime went.
    pub fn after(&self, ms: f64, f: impl FnOnce(&Runtime) + Send + 'static) -> tokio::task::JoinHandle<()> {
        let weak = self.me.get().cloned().unwrap_or_default();
        let ms = if ms.is_finite() && ms > 0.0 { ms } else { 0.0 };
        self.rt.spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs_f64(ms / 1000.0)).await;
            if let Some(w) = weak.upgrade() {
                let _ = tokio::task::spawn_blocking(move || f(&w)).await;
            }
        })
    }

    /// Runs `f` every `ms` on a blocking thread, until the handle is aborted.
    pub fn every(&self, ms: f64, f: impl Fn(&Runtime) + Send + Sync + 'static) -> tokio::task::JoinHandle<()> {
        let weak = self.me.get().cloned().unwrap_or_default();
        let f = Arc::new(f);
        self.rt.spawn(async move {
            let period = std::time::Duration::from_secs_f64(ms.max(1.0) / 1000.0);
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                let Some(w) = weak.upgrade() else { return };
                let f = f.clone();
                let _ = tokio::task::spawn_blocking(move || f(&w)).await;
            }
        })
    }

    /// Waits on async work from a blocking thread.
    pub fn block_on<F: std::future::Future>(&self, f: F) -> F::Output {
        self.rt.block_on(f)
    }
}

/// This system's name (darwin, linux, win32), as the adapters' process
/// rules and the credential stores take it.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Whether a process still runs, a process of another user's included.
pub fn alive(pid: f64) -> bool {
    pid.is_finite() && pid.fract() == 0.0 && pid > 0.0 && pid <= f64::from(i32::MAX) && xenonet_os::process::running(pid as u32, true)
}

/// A time in epoch ms as the console's notices write it in an English
/// locale, in local time: 9:04 AM.
pub fn time_of(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let local = ms + xenonet_text::utf16::local_offset_ms(ms);
    let minutes = (local / 60e3).floor() as i64;
    let (h, m) = (minutes.div_euclid(60).rem_euclid(24), minutes.rem_euclid(60));
    let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    format!("{h12}:{m:02} {half}")
}

/// The start of a time's local day, in epoch ms.
pub fn day_start(ms: f64) -> f64 {
    let off = xenonet_text::utf16::local_offset_ms(ms);
    let start = ((ms + off) / 86400e3).floor() * 86400e3;
    // Midnight's own offset, which differs on a day the clocks change.
    start - xenonet_text::utf16::local_offset_ms(start - off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_is_alive_and_nonsense_is_not() {
        assert!(alive(f64::from(std::process::id())));
        assert!(!alive(-1.0));
        assert!(!alive(f64::NAN));
    }
}
