//! What the apps' plugins tell XenoNet as it happens (OpenCode's), kept in
//! memory per session by each app's adapter (`Native::plugin_events`). A
//! session's events go to its app's keeper, found by the app its id begins
//! with.

use serde_json::Value;
use xenonet_adapter::reader::{PluginEvents, PluginState};

/// The keepers of the apps whose plugins report, in the registry's order.
pub struct Plugins {
    keepers: Vec<(&'static str, Box<dyn PluginEvents>)>,
}

impl Default for Plugins {
    fn default() -> Self {
        Self::new()
    }
}

impl Plugins {
    /// A keeper for each app in place whose plugin reports.
    pub fn new() -> Self {
        let keepers = xenonet_adapter::platforms().native().into_iter().filter_map(|(app, row)| Some((app.id.as_str(), row.plugin_events()?))).collect();
        Plugins { keepers }
    }

    fn of(&mut self, id: &str) -> Option<&mut Box<dyn PluginEvents>> {
        let app = id.split_once(':')?.0;
        self.keepers.iter_mut().find(|(a, _)| *a == app).map(|(_, k)| k)
    }

    /// Takes a request's events for session `id`, run in `cwd`. Returns how
    /// many it used: none for an app without a plugin that reports.
    pub fn ingest(&mut self, id: &str, events: &Value, cwd: Option<&str>, now: f64) -> usize {
        self.of(id).map_or(0, |k| k.ingest(id, events, cwd, now))
    }

    /// The session as its plugin last said it.
    pub fn state(&mut self, id: &str, now: f64) -> Option<PluginState> {
        self.of(id)?.state(id, now)
    }

    /// The session as the engine reads a live one.
    pub fn live(&mut self, id: &str, now: f64) -> Option<Value> {
        self.of(id)?.live(id, now)
    }

    /// The live view's session snapshots of every keeper, in order.
    pub fn snapshot(&mut self, now: f64, has: &dyn Fn(&str) -> bool) -> Vec<Value> {
        self.keepers.iter_mut().flat_map(|(_, k)| k.snapshot(now, has)).collect()
    }
}
