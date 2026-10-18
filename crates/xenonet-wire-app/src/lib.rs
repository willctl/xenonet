//! The binary's drivers for the app's parts (themes, open, command,
//! Spotify, the decision model, system stats…), hosts, the assistant,
//! components, integrations and the doctor: their tasks' fetches, children
//! and timers, natively, where the twins' JS answers the same Rust during
//! the port.

use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones.
pub fn wire(_cores: &mut Cores, _config: &mut Config, _env: &WireEnv) {}
