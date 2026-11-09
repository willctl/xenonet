//! Every adapter this XenoNet is built with, in the registry's order, and
//! putting them in place: the binary's main calls `install` before
//! anything asks the registry.

use std::sync::Arc;
use xenonet_adapter::Platform;

/// The adapters, made.
pub fn all() -> Vec<Arc<dyn Platform>> {
    vec![
        xenonet_claude_code::Adapter::platform(),
        xenonet_codex::Adapter::platform(),
        xenonet_grok_build::Adapter::platform(),
        xenonet_opencode::Adapter::platform(),
        xenonet_cursor::Adapter::platform(),
        xenonet_gemini_cli::Adapter::platform(),
        xenonet_copilot::Adapter::platform(),
        xenonet_hermes::Adapter::platform(),
        xenonet_grok_bot::Adapter::platform(),
        xenonet_muse::Adapter::platform(),
    ]
}

/// Puts every adapter in place (`xenonet_adapter::install`), once.
pub fn install() {
    xenonet_adapter::install(all());
}
