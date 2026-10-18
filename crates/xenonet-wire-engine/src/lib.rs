//! The binary's drivers for the engine, delivery and actors (core/engine,
//! core/delivery, core/actors, core/procs.js): their collaborators, timers,
//! processes and waits, natively, where the twins' JS answers the same
//! Rust cores during the port.

use xenonet_service::config::Config;
use xenonet_service::cores::Cores;
use xenonet_service::wire::WireEnv;

/// Puts this area's cores in place of the pending ones.
pub fn wire(_cores: &mut Cores, _config: &mut Config, _env: &WireEnv) {}
