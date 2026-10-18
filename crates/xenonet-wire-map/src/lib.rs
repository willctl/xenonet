//! The binary's drivers for the map (core/state.js and what it reads):
//! the demo's made-up world, the readers, the live tailer, search and the
//! state's own timers and events, natively, where the twins' JS drives
//! the same Rust cores during the port.

// A core's failure is the service's CoreError, as its traits return it.
#![expect(clippy::result_large_err, reason = "the cores' traits return the service's CoreError")]

pub mod blocks;
pub mod config;
pub mod demo;
pub mod gatherer;
pub mod git;
pub mod live;
pub mod projects;
pub mod schemas;
pub mod store;
pub mod util;
pub mod vendor;

use crate::util::lock;
use std::sync::Arc;
use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones.
pub fn wire(cores: &mut Cores, config: &mut Config, env: &WireEnv) {
    // The service's one drift log, where the readers record what they find.
    {
        let mut d = lock(&env.drift);
        if d.view().0.is_none() {
            d.open(&std::path::Path::new(&config.cache_dir).join(xenonet_schema::drift::DRIFT_FILE).to_string_lossy());
        }
    }
    let dir = vendor::schemas_dir(&config.root);
    let vendor = vendor::Vendor::new(dir.clone(), env.drift.clone());
    let store = store::StateStore::new(&config.state_dir, &config.projects_dir);
    cores.store = Arc::new(store::StoreCore(store));
    cores.schemas = Arc::new(schemas::SchemaCore::new(dir));
    cores.drift = Arc::new(schemas::Drift { vendor });
}
