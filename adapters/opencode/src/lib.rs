//! OpenCode's adapter (docs/design/direction.md, "Adapters and
//! surfaces"): its processes, and its sessions reached through its own
//! server.
use std::sync::Arc;
use xenonet_adapter::apps::{app, App};
use xenonet_adapter::delivery::Ways;
use xenonet_adapter::processes::Rules;
use xenonet_adapter::Platform;

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
        Some(Ways { server: true, ..Ways::default() })
    }
}
