//! The actors' forwards (`xenonet_net::forward`): an actor whose grant
//! names `forward(<alpn>)` has XenoNet carry its connections on that ALPN
//! to and from the members of this machine's networks. The composition
//! root makes them, registers the forwardable ALPNs, and hands the node
//! and the local socket the same `Forwards`.

use super::{pending, Pending, R};
use serde_json::Value;
use std::sync::Arc;

const FORWARDS: &str = "the forwards";

pub trait Forwards: Send + Sync {
    /// What `GET /api/net/forwards` answers: the forwardable ALPNs, each
    /// actor forwarding one with its connections, the asks for a grant
    /// waiting for the person, and the grants they approved.
    fn list(&self) -> R<Value> {
        pending(FORWARDS)
    }
    /// The person approves the ask for a grant on `alpn` for the context
    /// key `key`, reaching `networks` (ids of networks this machine is
    /// in): from the window, with their presence. What they approved.
    fn approve(&self, _alpn: &str, _key: &str, _networks: &[String], _window: bool) -> R<Value> {
        pending(FORWARDS)
    }
    /// The person turns down the ask for `key` on `alpn`.
    fn deny(&self, _alpn: &str, _key: &str) -> R<Value> {
        pending(FORWARDS)
    }
    /// Takes back the grant of the actor forwarding `alpn`: it ends now.
    fn revoke(&self, _alpn: &str) -> R<Value> {
        pending(FORWARDS)
    }
    /// The forwards themselves, for the node and the local socket; None in
    /// a XenoNet that forwards nothing.
    fn net(&self) -> Option<Arc<xenonet_net::forward::Forwards>> {
        None
    }
}

impl Forwards for Pending {}
