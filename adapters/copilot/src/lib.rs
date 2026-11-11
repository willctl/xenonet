//! Copilot CLI's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): the reader of its session logs, which the watcher
//! watches.
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
        Arc::new(Adapter(app("copilot").expect("the registry has copilot")))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        self.0
    }

    fn native(&self) -> Option<Box<dyn Native>> {
        Some(Box::new(reader::Row))
    }

    /// Each session's event log: `<id>/events.jsonl`, or `<id>.jsonl`
    /// before 1.0.11.
    fn watch(&self, env: &ListerEnv<'_>) -> Vec<WatchSpec> {
        vec![WatchSpec::tree("sessions", reader::Row.at(&env.paths()), &["*/events.jsonl", "*.jsonl"])]
    }

    fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        for p in paths {
            let Some(rel) = cursor.spec_of(p).and_then(|s| s.relative(p)) else { continue };
            let id = rel.strip_suffix("/events.jsonl").or_else(|| rel.strip_suffix(".jsonl")).map(str::to_string);
            watch::changed(p, id, &mut out);
        }
        out
    }
}
