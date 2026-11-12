//! The network across machines (docs/design/network.md): this machine in
//! every network it's in, machine to machine over iroh, with no gateway.
//!
//! - `keys`: a host's two keys, Ed25519 to sign and X25519 to be sealed
//!   to, in one owner-only file, and its key id (from `xenonet-id`).
//! - `seal`: sealing for one machine, binding a head.
//! - `envelope`: the envelope (`xenonet-net/2`) that carries a feed entry,
//!   sealed for one recipient with what its act rests on beside it, or a
//!   live presence delta: a head for routing only, checked against the
//!   signed entry inside.
//! - `roster`: the network's chain of members, and who may sign what.
//! - `ticket`: the one-time code an invite's join proves.
//! - `nodes`: this machine in every network it's in, behind
//!   `xenonet_model::ops::Net`: a node per network, what names a network
//!   going to that one's node, link/2 and the relays every network lists
//!   shared between them.
//! - `node`: this machine in one network, behind
//!   `xenonet_model::ops::Net`, calling a `Sink`: its own feed in the
//!   network as its author (xenonet-log's `feed`), whose entries carry
//!   messages, receipts, handoffs and the person's acts, the other
//!   members' entries, checked against their keys in the chain, to the
//!   engine's `Feeds`, and the chain itself, machine to machine.
//! - `identity`: the person on this machine: their chain, device key,
//!   sealed root, the people's chains it holds, the presence it took.
//! - `ground`: the facts this machine decides incoming acts on
//!   (`xenonet_authz::Ground`), and the `Governance` the engine reads them
//!   through.
//! - `svc`: services a host publishes to the network, and its checks of
//!   requests to them.
//! - `doctor`: what the doctor says about the network, from its files.
//! - `dial`: the `Dial` a transport implements, and the `Mesh` every
//!   connection goes through (one per peer and protocol, crossing dials,
//!   the networks a connection carries).
//! - `admission`: who may hold a connection, on which protocol.
//! - `endpoint`: the `Dial` over iroh, keyed by host.key: relays the
//!   person names or none, the memory lookup and mDNS behind a setting,
//!   and a door per protocol.
//! - `share`: what each network is shown of this machine: the sessions
//!   in projects shared into it, all of them in the first network.
//! - `link2`: `xenonet/link/2`, members' feeds, messages, presence and the
//!   network's chain between machines on one connection per peer: a hello
//!   with each shared network's heads, Want/Ack, kept, loose and live
//!   streams, every record checked against the networks the connection
//!   carries. The same engine runs another program's protocol shaped like
//!   it (`Protocol`, over a `Mesh` with that program's `admission::Doors`):
//!   Xenolith's sync/2, whose groups are spaces.
//! - `holder`: filling a feed's gap from any member that holds it, one
//!   holder at a time; serving the entries this machine applied, to
//!   current members only.
//! - `mailbox`: keeping sealed envelopes for away members, Held receipts
//!   signed by the mailbox, and the hand-off when they connect.
//! - `presence`: presence as a snapshot on connect and deltas on change,
//!   valid while the connection is up.
//! - `join`: joining over `xenonet/join/2`: the invite, the code apart
//!   from it, proofs both ways, the inviter's tickets, and the joiner's
//!   check of the chain from its genesis; another program's join is a
//!   `JoinTarget` of its own.
//! - `svc2`: `xenonet/svc/2`, one bi stream per request: services members
//!   publish, and the window's reads of another machine's transcripts
//!   under a per-launch window key its device certified.
//! - `forward`: actors' own protocols, forwarded between members.

pub mod admission;
pub mod codec;
pub mod compaction;
pub mod dial;
pub mod doctor;
pub mod endpoint;
pub mod envelope;
pub mod files;
pub mod forward;
pub mod ground;
pub mod holder;
pub mod identity;
pub mod join;
pub mod keys;
pub mod link2;
pub mod load;
pub mod mailbox;
pub mod node;
pub mod nodes;
pub mod persist;
pub mod presence;
pub mod roots;
pub mod roster;
pub mod seal;
pub mod share;
#[cfg(test)]
pub mod sim;
pub mod sink;
pub mod space_sign;
pub mod svc;
pub mod svc2;
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
