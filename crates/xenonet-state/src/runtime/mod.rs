//! The map's runtime: `MapState` (`state`), which refreshes the map from
//! the vendor files and answers the routes, with the live tailer that
//! drives it (`live`), the drivers of the crate's `stats`, `search`,
//! `projects` and `identities` (each in the module of the same name here),
//! the gatherer, git and the narration, the demo world's driver (`demo`),
//! the vendor readers (`vendor`, `observed`), what XenoNet saves for you
//! (`saved`, over the Store core the service gives it), and the schemas'
//! drift report. The service calls `demo_world`, then `wire`.

// A core's failure is the api's CoreError, as its traits return it.
#![expect(clippy::result_large_err, reason = "the cores' traits return the api's CoreError")]

pub mod blocks;
pub mod config;
pub mod demo;
pub mod gatherer;
pub mod git;
pub mod identities;
pub mod live;
pub mod narrate;
pub mod observed;
pub mod projects;
pub mod saved;
pub mod schemas;
pub mod search;
pub mod state;
pub mod stats;
pub mod util;
pub mod vendor;

use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use util::lock;
use xenonet_api::cores::{Cores, Runs};
use xenonet_api::wire::WireEnv;

/// What the map's driver runs with, from the service's settings, once the
/// demo's world has moved their folders.
pub struct Settings {
    pub demo: bool,
    pub dev: bool,
    /// Your own home, as it was before the demo's world.
    pub home: String,
    pub cache_dir: String,
    /// Where XenoNet's files are: schemas/ among them.
    pub root: PathBuf,
    /// The demo's world, when it was made.
    pub world: Option<Value>,
}

/// The demo's made-up world, in the cores as their Demo, when the demo is
/// on: the service makes its world first, which moves every folder the
/// drivers take, then gives it to `wire`.
pub fn demo_world(cores: &mut Cores, on: bool, port: u16) -> Option<Arc<demo::DemoWorld>> {
    let d = on.then(|| Arc::new(demo::DemoWorld::new(port)))?;
    cores.demo = Arc::new(demo::DemoCore(d.clone()));
    Some(d)
}

/// Puts this area's cores in place of the pending ones, over the demo's
/// world when there is one (`demo`), and over what XenoNet saves for you
/// (`cores.store`, which the service puts in place first).
pub fn wire(cores: &mut Cores, settings: &Settings, demo: Option<Arc<demo::DemoWorld>>, env: &WireEnv) {
    util::set_log(&env.emitter);
    // The service's one drift log, where the readers record what they
    // find, in the cache folder the demo may have moved.
    {
        let mut d = lock(&env.drift);
        if d.view().0.is_none() {
            d.open(&std::path::Path::new(&settings.cache_dir).join(xenonet_schema::drift::DRIFT_FILE).to_string_lossy());
        }
    }
    let cfg = config::MapConfig::new(settings.demo, settings.dev, config::process_env(), &settings.home, settings.world.as_ref());
    let dir = vendor::schemas_dir(&settings.root);
    // The service's one set of vendor readers, which the actors' observers
    // check their records through too.
    let readers = env.readers.get_or_make(|| xenonet_schema::vendor::Readers::new(dir.clone(), env.drift.clone()));
    let vendor = vendor::Vendor::with_readers(readers);
    let store = saved::Saved::new(cores.store.clone());
    // What the demo's made-up person would have saved: the names Nova
    // gave, and the accounts XenoNet saw.
    if let Some(remembered) = demo.as_ref().and_then(|d| d.remembered()) {
        saved::seed(&store, &remembered);
    }
    // The folder each app writes its transcripts in now, which the tailer
    // follows.
    let roots: Vec<(String, String)> =
        xenonet_adapter::platforms().iter().filter_map(|p| Some((state::transcript_dirs(&cfg, p.id()).into_iter().next()?, p.id().to_string()))).collect();
    let live = live::Live::with_clock(&roots, cfg.claude_desktop_dir.as_deref(), cfg.claude_usage, &cfg.home_dir, vendor.clone(), cfg.clock.clone());
    live.context_server(cfg.wiki_server.as_str());
    let readers = match &demo {
        Some(d) => state::Readers::Demo(d.clone()),
        None => state::Readers::Real,
    };
    let map = state::MapState::new(cfg, store.clone(), vendor.clone(), env, readers, live);
    cores.state = map.clone();
    cores.schemas = Arc::new(schemas::SchemaCore);
    cores.drift = Arc::new(schemas::Drift { vendor });
    // Runs actors report, when there are actors.
    if cores.runs.is_some() {
        cores.runs = Some(Arc::new(state::RunsCore(map)) as Arc<dyn Runs>);
    }
}
