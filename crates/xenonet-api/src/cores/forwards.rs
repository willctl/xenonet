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
    /// What `GET /api/net/forwards` answers: the forwardable ALPNs, and
    /// each actor forwarding one with its connections.
    fn list(&self) -> R<Value> {
        pending(FORWARDS)
    }
    /// The forwards themselves, for the node and the local socket; None in
    /// a XenoNet that forwards nothing.
    fn net(&self) -> Option<Arc<xenonet_net::forward::Forwards>> {
        None
    }
}

impl Forwards for Pending {}
