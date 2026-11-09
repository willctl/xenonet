//! Every adapter this XenoNet is built with, in the registry's order, and
//! putting them in place: the binary's main calls `install` before
//! anything asks the registry.

use std::sync::Arc;
use xenonet_adapter::Platform;

/// The adapters, made.
pub fn all() -> Vec<Arc<dyn Platform>> {
    vec![xenonet_claude_code::Adapter::platform(), xenonet_codex::Adapter::platform(), xenonet_grok_build::Adapter::platform(), xenonet_opencode::Adapter::platform()]
}

/// Puts every adapter in place (`xenonet_adapter::install`), once.
pub fn install() {
    xenonet_adapter::install(all());
}
