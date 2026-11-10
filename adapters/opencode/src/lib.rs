//! OpenCode's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, its sessions reached through its own
//! server, and the reader of its database.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::delivery::Ways;
use xenonet_adapter::processes::Rules;
use xenonet_adapter::reader::Native;
use xenonet_adapter::Platform;

pub mod reader;

// OpenCode: its one binary, however it was installed. Every OpenCode reads
// the same database and can open any session.
fn program(_: &str, base: &str) -> bool {
    base == "opencode" || base == "opencode.exe"
}

/// The adapter.
pub struct Adapter(&'static App);

impl Adapter {
    /// The adapter, as the registry holds one.
    pub fn platform() -> Arc<dyn Platform> {
        Arc::new(Adapter(app("opencode").expect("the registry has opencode")))
    }
}

impl Platform for Adapter {
    fn app(&self) -> &App {
        self.0
    }

    fn processes(&self) -> Option<Rules> {
        Some(Rules { program, any_process: true, ..Rules::NONE })
    }

    fn delivery(&self) -> Option<Ways> {
        Some(Ways { server: true, server_alias: Some("alias opencode='opencode --hostname 127.0.0.1'"), ..Ways::default() })
    }

    fn native(&self) -> Option<Box<dyn Native>> {
        Some(Box::new(reader::Row))
    }
}
