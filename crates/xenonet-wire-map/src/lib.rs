//! The binary's drivers for the map (core/state.js and what it reads):
//! the demo's made-up world, the readers, the live tailer, search and the
//! state's own timers and events, natively, where the twins' JS drives
//! the same Rust cores during the port.

use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones.
pub fn wire(_cores: &mut Cores, _config: &mut Config, _env: &WireEnv) {}
