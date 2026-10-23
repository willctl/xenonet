//! One network across machines (docs/design/network.md, sections 1–4 and
//! 9): a gateway and the hosts that joined it, each host dialling out to
//! the gateway over plain HTTP(S).
//!
//! - `keys`: a host's two keys, Ed25519 to sign and X25519 to be sealed
//!   to, in one owner-only file, and its key id.
//! - `frame`: a frame on the wire (`network/frame/v1`), its head, its
//!   signature and the sealing of its body.
//! - `roster`: the signed chain of hosts, and who may sign what.
//! - `ticket`: the one-time code that lets one machine join.
//! - `request`: how a host signs its requests to the gateway.
//! - `gwstore` and `hoststore`: the gateway's queues and a host's outbox.
//! - `gateway`: the gateway's routes, `/net/v1/…`, over any transport.
//! - `link`: a host's link to its gateway, with backoff, the clock
//!   offset, epochs and the long-poll fallback.
//! - `node`: this machine in its network, behind
//!   `xenonet_base::network::Net`, calling a `Sink`.
//! - `svc`: services a host publishes to the network, and requests to
//!   other hosts' services, in `svc` frames.
//! - `doctor`: what the doctor says about the network, from its files.

pub mod codec;
pub mod doctor;
pub mod files;
pub mod frame;
pub mod gateway;
pub mod gwstore;
pub mod hoststore;
pub mod keys;
pub mod link;
pub mod node;
pub mod request;
pub mod roster;
pub mod sink;
pub mod svc;
pub mod ticket;

use std::fmt;

/// Why something in the network was refused or failed: an HTTP status for
/// the routes, a short code, and words for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl NetError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> NetError {
        NetError { status, code, message: message.into() }
    }

    /// A request that isn't right.
    pub fn bad(message: impl Into<String>) -> NetError {
        NetError::new(400, "bad-request", message)
    }

    /// Something on this machine failed: a file, the database, randomness.
    pub fn internal(message: impl Into<String>) -> NetError {
        NetError::new(500, "internal", message)
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for NetError {}

pub type R<T> = Result<T, NetError>;

impl From<rusqlite::Error> for NetError {
    fn from(e: rusqlite::Error) -> Self {
        NetError::internal(format!("the network's database: {e}"))
    }
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::internal(format!("the network's files: {e}"))
    }
}
