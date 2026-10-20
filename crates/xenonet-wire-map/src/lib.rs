//! The binary's drivers for the map (core/state.js and what it reads):
//! the demo's made-up world, the readers, the live tailer, search and the
//! state's own timers and events, natively, where the twins' JS drives
//! the same Rust cores during the port.

// A core's failure is the service's CoreError, as its traits return it.
#![expect(clippy::result_large_err, reason = "the cores' traits return the service's CoreError")]

pub mod blocks;
pub mod claude;
pub mod codex;
pub mod config;
pub mod demo;
pub mod gatherer;
pub mod git;
pub mod identities;
pub mod live;
pub mod narrate;
pub mod observed;
pub mod projects;
pub mod schemas;
pub mod sdk;
pub mod search;
pub mod state;
pub mod stats;
pub mod store;
pub mod util;
pub mod vendor;

use std::sync::Arc;
use util::lock;
use xenonet_service::config::Config;
use xenonet_service::cores::{Cores, Runs};
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones: the demo's world
/// first, which moves the folders everything else reads.
pub fn wire(cores: &mut Cores, config: &mut Config, env: &WireEnv) {
    util::set_log(&env.emitter);
    let home = config.home.clone();
    let demo = config.demo.then(|| Arc::new(demo::DemoWorld::new(config.port)));
    let world = match &demo {
        Some(d) => match d.create() {
            Ok(w) => {
                config.use_world(&w);
                Some(w)
            }
            Err(e) => {
                util::log("error", "demo.world", &format!("the demo's world couldn't be made: {}", e.message));
                config.demo_folders();
                None
            }
        },
        None => None,
    };
    // The service's one drift log, where the readers record what they
    // find, in the cache folder the demo may have moved.
    {
        let mut d = lock(&env.drift);
        if d.view().0.is_none() {
            d.open(&std::path::Path::new(&config.cache_dir).join(xenonet_schema::drift::DRIFT_FILE).to_string_lossy());
        }
    }
    let cfg = config::MapConfig::new(config.demo, config.dev, config::process_env(), &home, world.as_ref());
    let dir = vendor::schemas_dir(&config.root);
    let vendor = vendor::Vendor::new(dir.clone(), env.drift.clone());
    let store = store::StateStore::new(&cfg.state_dir, &cfg.projects_dir);
    let codex_sessions = std::path::Path::new(&cfg.codex_home).join("sessions").to_string_lossy().into_owned();
    let live = live::Live::new(&cfg.claude_projects_dir, &codex_sessions, cfg.claude_desktop_dir.as_deref(), cfg.claude_usage, &cfg.home_dir, vendor.clone());
    let codex = codex::CodexClient::new(cfg.codex_binary(), env.version.clone(), vendor.clone());
    let readers = match &demo {
        Some(d) => state::Readers::Demo(d.clone()),
        None => state::Readers::Real { claude: claude::Claude::new(&cfg.claude_projects_dir, cfg.claude_desktop_dir.as_deref(), vendor.clone()) },
    };
    let map = state::MapState::new(cfg, store.clone(), vendor.clone(), env.emitter.clone(), readers, live, codex, &env.version);
    cores.state = map.clone();
    cores.store = Arc::new(store::StoreCore(store));
    cores.schemas = Arc::new(schemas::SchemaCore::new(dir));
    cores.drift = Arc::new(schemas::Drift { vendor });
    if let Some(d) = demo {
        cores.demo = Arc::new(demo::DemoCore(d));
    }
    // Runs actors report, when there are actors.
    if cores.runs.is_some() {
        cores.runs = Some(Arc::new(state::RunsCore(map)) as Arc<dyn Runs>);
    }
}
