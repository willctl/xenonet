//! The binary's drivers for the engine, delivery and actors (core/engine,
//! core/delivery, core/actors, core/procs.js): their collaborators, timers,
//! processes and waits, natively, where the twins' JS answers the same
//! Rust cores during the port.

// The cores' collaborators answer with xenonet-engine's Thrown, which is
// passed on as it is: what a JS call threw, kept whole.
#![expect(clippy::result_large_err, reason = "the cores' Calls return xenonet-engine's Thrown, passed on whole")]

pub mod exec;
pub mod fsx;
pub mod js;
pub mod procs;
pub mod store;
pub mod verify;

use std::path::Path;
use std::sync::Arc;
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

/// Puts this area's cores in place of the pending ones: the engine's
/// store and the process checks.
pub fn wire(cores: &mut Cores, config: &mut Config, env: &WireEnv) {
    let file = Path::new(&config.state_dir).join("engine.sqlite");
    if let Err(e) = std::fs::create_dir_all(&config.state_dir) {
        log_later(env, "error", "engine.store", format!("the state folder can't be made: {e}"));
    }
    let store = match EngineStore::open(&file.to_string_lossy(), js::now_ms()) {
        Ok(s) => Shared::new(s),
        Err(e) => {
            log_later(env, "error", "engine.store", format!("engine.sqlite can't be opened: {e}"));
            return;
        }
    };
    let held = store.clone();
    let _ = cores.state.add_block(Arc::new(move || held.with(|s| store::block_health("engine", "store", &s.block.life))), None);
    cores.verify = Arc::new(verify::Checks::new(store.clone(), procs::System::new()));
    cores.engine_store = Arc::new(store::Store(store));
}
