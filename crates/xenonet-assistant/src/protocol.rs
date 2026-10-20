//! core/assistant/protocol.js: the wire between a remote actor's adapter
//! and the service's private endpoint, newline-delimited JSON over
//! 127.0.0.1, found through a file in the runtime folder.

use xenonet_app::nodepath::NATIVE;
use xenonet_components::runtime::{port_of, runtime_dir, Env, Machine};

pub const MAX_LINE: usize = 320 << 10;
pub const HELLO_KEYS: [&str; 4] = ["api", "grant_id", "role", "token"];

/// The endpoint's file for the XenoNet on XENONET_PORT (47840 by default).
pub fn endpoint_file(env: &Env, m: &Machine) -> String {
    NATIVE.join(&[&runtime_dir(env, m), &format!("assistant-{}.json", port_of(env))])
}
