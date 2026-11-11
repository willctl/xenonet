//! Cursor's adapter (docs/design/direction.md, "Adapters and surfaces"):
//! the reader of the runs its XenoNet hook reports, whose file the
//! watcher watches.
use std::path::PathBuf;
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::reader::{ListerEnv, Native};
use xenonet_adapter::watch::{self, Cursor, SessionEvent, WatchSpec};
use xenonet_adapter::Platform;

pub mod reader;

/// The adapter.
pub struct Adapter(&'static App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        Arc::new(Adapter(app("cursor").expect("the registry has cursor")))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        self.0
    }

    fn native(&self) -> Option<Box<dyn Native>> {
        Some(Box::new(reader::Row))
    }

    /// The file XenoNet's Cursor hook appends each event to.
    fn watch(&self, env: &ListerEnv<'_>) -> Vec<WatchSpec> {
        WatchSpec::file("events", reader::Row.at(&env.paths()), &[]).into_iter().collect()
    }

    fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
        watch::whole_source(paths, cursor)
    }
}
