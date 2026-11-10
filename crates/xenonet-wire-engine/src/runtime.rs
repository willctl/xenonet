//! The runtime the engine's cores run in: the engine's store, the map's
//! readers, the process table and its checks, the engine and the waits on
//! it, delivery (its policy, router and channels) and, with actors on, the
//! actors, their registrations, wakes and observers. Each core's host is
//! made of the others here; the runtime also keeps the timers and tasks
//! they start, and the few facts about this machine they ask for
//! (`platform`, `alive`, local times).

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
use xenonet_api::cores::State;
use xenonet_api::wire::WireEnv;
use xenonet_api::From;
use xenonet_engine::store::Shared;

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
}

pub struct Runtime {
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
    /// The runtime itself, for the timers and tasks it starts.
    pub me: OnceLock<std::sync::Weak<Runtime>>,
    /// What this machine last told the network of itself (its presence
    /// and claims), and the timer that tells it again.
    pub net: crate::network::NetState,
    /// Sessions started here, and the start requests sent from here.
    pub starts: crate::starts::StartSide,
    /// Permission prompts of the runs XenoNet started: held here, and
    /// from other machines.
    pub asks: crate::asks::AskSide,
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

/// This system as Node names it (darwin, linux, win32), as the adapters'
/// process rules and the credential stores take it.
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
