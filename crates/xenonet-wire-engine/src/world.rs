//! Everything this area's cores share, as server/index.js's top level
//! made them: the engine's store, the map's readers, the process table and
//! its checks, the engine and the waits on it, delivery (its policy, router
//! and channels) and, with actors on, the actors, their registrations,
//! wakes and observers. Each core's collaborators are the others here,
//! called by the paths and methods the JS called them by.

use crate::actors::ActorSide;
use crate::delivery::{ChannelSide, PolicySide, RouterSide};
use crate::engine::{Mail, Repos};
use crate::map::Map;
use crate::procs::System;
use crate::registration::RegSide;
use crate::verify::Checks;
use crate::wakes::WakeSide;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use xenonet_engine::store::Shared;
use xenonet_service::cores::State;
use xenonet_service::glue::From;
use xenonet_service::wire::WireEnv;

/// What the area runs as.
#[derive(Clone, Debug)]
pub struct Setup {
    pub demo: bool,
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
}

pub struct World {
    pub setup: Setup,
    pub env: WireEnv,
    pub rt: tokio::runtime::Handle,
    pub store: Shared,
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
    pub observers: OnceLock<crate::observers::Observers>,
    /// The world itself, for the timers and tasks it starts.
    pub me: OnceLock<std::sync::Weak<World>>,
    /// What this machine last told the network of itself (its presence
    /// and claims), and the timer that tells it again.
    pub net: crate::network::NetState,
    /// Sessions started here, and the start requests sent from here.
    pub starts: crate::starts::StartSide,
}

impl World {
    pub fn state(&self) -> &Arc<dyn State> {
        &self.map.state
    }

    /// The world, held, for a task that outlives the call.
    pub fn arc(&self) -> Option<Arc<World>> {
        self.me.get().and_then(std::sync::Weak::upgrade)
    }

    /// An event for server/index.js's handlers.
    pub fn emit(&self, from: From, name: &str, data: Value) {
        self.env.emitter.emit(from, name, data);
    }

    pub fn log(&self, level: &str, code: &str, message: &str) {
        self.env.emitter.log(level, code, message);
    }

    /// Runs blocking work on the runtime's blocking threads, with the world.
    pub fn spawn_blocking(&self, f: impl FnOnce(&World) + Send + 'static) {
        if let Some(w) = self.arc() {
            self.rt.spawn_blocking(move || f(&w));
        }
    }

    /// Runs `f` after `ms`, on a blocking thread, unless the world went.
    pub fn after(&self, ms: f64, f: impl FnOnce(&World) + Send + 'static) -> tokio::task::JoinHandle<()> {
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
    pub fn every(&self, ms: f64, f: impl Fn(&World) + Send + Sync + 'static) -> tokio::task::JoinHandle<()> {
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
