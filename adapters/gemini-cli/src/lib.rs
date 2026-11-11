//! Gemini CLI's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): the reader of its chat records, which the watcher
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
        Arc::new(Adapter(app("gemini-cli").expect("the registry has gemini-cli")))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        self.0
    }

    fn native(&self) -> Option<Box<dyn Native>> {
        Some(Box::new(reader::Row))
    }

    /// Each project's chats: a session's (`chats/session-*`) and its
    /// subagents' (`chats/<parent>/*`). The session's id is inside its
    /// file, so a change says only that the records changed.
    fn watch(&self, env: &ListerEnv<'_>) -> Vec<WatchSpec> {
        let root = reader::Row.at(&env.paths());
        vec![WatchSpec::tree("chats", root, &["*/chats/session-*.json", "*/chats/session-*.jsonl", "*/chats/*/*.json", "*/chats/*/*.jsonl"])]
    }

    fn changed(&self, paths: &[PathBuf], cursor: &mut Cursor) -> Vec<SessionEvent> {
        watch::whole_source(paths, cursor)
    }
}
