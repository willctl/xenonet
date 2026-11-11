//! One network across machines (docs/design/network.md, sections 1–4 and
//! 9): a gateway and the hosts that joined it, each host dialling out to
//! the gateway over plain HTTP(S).
//!
//! - `keys`: a host's two keys, Ed25519 to sign and X25519 to be sealed
//!   to, in one owner-only file, and its key id (from `xenonet-id`).
//! - `frame`: a frame on the wire (`network/frame/v1`), its head, its
//!   signature and the sealing of its body.
//! - `envelope`: the envelope (`xenonet-net/2`) that carries a feed entry,
//!   sealed for one recipient, or a live presence delta: a head for
//!   routing only, checked against the signed entry inside.
//! - `roster`: the signed chain of hosts, and who may sign what.
//! - `ticket`: the one-time code that lets one machine join.
//! - `request`: how a host signs its requests to the gateway.
//! - `gwstore`: the gateway's queues.
//! - `gateway`: the gateway's routes, `/net/v1/…`, over any transport.
//! - `link`: a host's link to its gateway, with backoff, the clock
//!   offset, epochs and the long-poll fallback.
//! - `node`: this machine in its network, behind
//!   `xenonet_model::ops::Net`, calling a `Sink`: its own feed in the
//!   network as its author (xenonet-log's `feed`), whose entries carry
//!   messages, receipts and handoffs, and the other members' entries,
//!   checked against their keys in the roster, to the engine's `Feeds`.
//! - `identity`: the person on this machine: their chain, device key,
//!   sealed root, the people's chains it holds, the presence it took.
//! - `ground`: the facts this machine decides incoming acts on
//!   (`xenonet_authz::Ground`), and the `Governance` the engine reads them
//!   through.
//! - `svc`: services a host publishes to the network, and requests to
//!   other hosts' services, in `svc` frames.
//! - `doctor`: what the doctor says about the network, from its files.
//!
//! The transport that replaces the gateway (phase 5), beside it until the
//! node moves over:
//!
//! - `dial`: the `Dial` a transport implements, and the `Mesh` every
//!   connection goes through (one per peer and protocol, crossing dials,
//!   the networks a connection carries).
//! - `admission`: who may hold a connection, on which protocol.
//! - `endpoint`: the `Dial` over iroh, keyed by host.key: relays the
//!   person names or none, the memory lookup and mDNS behind a setting,
//!   and a door per protocol.
//! - `join`: joining over `xenonet/join/2`: the invite, the code apart
//!   from it, proofs both ways, the inviter's tickets, and the joiner's
//!   check of the chain from its genesis. `ticket`'s gateway proofs stay
//!   beside it for the gateway's join route until the gateway goes.

pub mod admission;
pub mod codec;
pub mod dial;
pub mod doctor;
pub mod endpoint;
pub mod envelope;
pub mod files;
pub mod frame;
pub mod gateway;
pub mod ground;
pub mod gwstore;
pub mod identity;
pub mod join;
pub mod keys;
pub mod link;
pub mod node;
pub mod persist;
pub mod request;
pub mod roots;
pub mod roster;
#[cfg(test)]
pub mod sim;
pub mod sink;
pub mod svc;
pub mod ticket;

/// The feeds (xenonet-log's), as the network carries them, for the engine
/// that applies their entries.
pub use xenonet_log::feed;

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

impl From<xenonet_id::IdError> for NetError {
    fn from(e: xenonet_id::IdError) -> Self {
        match e.code {
            "refused" => NetError::new(403, "refused", e.message),
            _ => NetError::internal(e.message),
        }
    }
}

impl From<xenonet_authz::AuthzError> for NetError {
    fn from(e: xenonet_authz::AuthzError) -> Self {
        NetError::internal(e.message)
    }
}

// A roster entry's refusal keeps its status and code: `roster-gap`,
// `needs-approval` and the rest are the routes' answers.
impl From<xenonet_log::chain::ChainError> for NetError {
    fn from(e: xenonet_log::chain::ChainError) -> Self {
        NetError::new(e.status, e.code, e.message)
    }
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::internal(format!("the network's files: {e}"))
    }
}
