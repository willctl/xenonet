//! The running XenoNet a command talks to (core/client.js's find and need),
//! picked by the command's flags: --port, --dev, --demo, or XENONET_PORT.

use crate::args::Flags;
use crate::out::Fail;
use std::time::Duration;
use xenonet_base::client::{self, Server};
use xenonet_base::json::number_string;

/// The first XenoNet the flags name that answers, or None.
pub fn find(flags: &Flags) -> Option<Server> {
    let pick = flags.pick();
    client::find(&client::candidates(&pick), Duration::from_millis(client::health_ms(&pick)))
}

/// The XenoNet the flags name, or why there's none: an Error, as JS's
/// need() throws it.
pub fn need(flags: &Flags) -> Result<Server, Fail> {
    client::need(&flags.pick()).map_err(Fail::error)
}

/// The ports the flags name, as candidates() gives them.
pub fn candidates(flags: &Flags) -> Vec<f64> {
    client::candidates(&flags.pick())
}

/// "127.0.0.1:47840 or 127.0.0.1:47842".
pub fn where_(flags: &Flags) -> String {
    candidates(flags).iter().map(|p| format!("127.0.0.1:{}", number_string(*p))).collect::<Vec<_>>().join(" or ")
}

/// A header, for an actor's calls.
pub fn bearer(token: &str) -> Vec<(String, String)> {
    vec![("authorization".into(), format!("Bearer {token}"))]
}
