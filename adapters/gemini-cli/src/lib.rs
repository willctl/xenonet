//! Gemini CLI's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): the reader of its chat records.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::reader::Native;
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
}
