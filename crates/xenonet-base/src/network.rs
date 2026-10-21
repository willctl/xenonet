//! The seam between the engine and the network (docs/design/network.md,
//! section 6). The engine sends and receives bodies through it: presence,
//! messages, receipts, claims, handoffs and the person's actions, as the
//! JSON that `schemas/network/<kind>/v1.json` describes. Frames, keys,
//! sealing, the roster and the gateway live behind it, in xenonet-net, and
//! the engine knows nothing of them.
//!
//! When the service isn't in a network nothing is behind the seam, so
//! every local path, and the JS twins, are unchanged.

use serde_json::Value;

/// What a body is, and so which frame kind carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Presence,
    Claims,
    Message,
    Receipt,
    Handoff,
    Operator,
}

impl Kind {
    pub const ALL: [Self; 6] = [Self::Presence, Self::Claims, Self::Message, Self::Receipt, Self::Handoff, Self::Operator];

    /// The frame kind on the wire, and the schema's folder.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Presence => "presence",
            Self::Claims => "claims",
            Self::Message => "message",
            Self::Receipt => "receipt",
            Self::Handoff => "handoff",
            Self::Operator => "operator",
        }
    }

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == kind)
    }

    /// Whether the gateway keeps it for a host that's away. Presence and
    /// claims aren't kept: a full set goes when the host is back.
    pub fn queued(self) -> bool {
        !matches!(self, Self::Presence | Self::Claims)
    }
}

/// Who a body is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum To {
    /// One host, by its id.
    Host(String),
    /// Every other host in the network.
    All,
}

/// A body the engine asks the network to carry.
#[derive(Debug, Clone)]
pub struct Outgoing {
    pub to: To,
    pub kind: Kind,
    pub body: Value,
}

/// How the person's action was checked on the host it came from
/// (`network/operator/v1`). Only a strong one is acted on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checked {
    Window,
    Person,
}

/// A body another host sent, after the network checked its signature
/// against the roster and opened it.
#[derive(Debug, Clone)]
pub struct Incoming {
    /// The host that sent it, which is in the roster.
    pub from: String,
    pub kind: Kind,
    pub body: Value,
    /// When the sender made it, by the sender's clock shifted to ours.
    pub at: f64,
    /// Whether the sending host is a window host, which may act for the
    /// person (section 3).
    pub from_window_host: bool,
}

/// This machine in its network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    pub network: String,
    pub name: String,
    pub host: String,
}

/// Another host in the roster, as the link last saw it.
#[derive(Debug, Clone, PartialEq)]
pub struct Host {
    pub id: String,
    pub window: bool,
    pub online: bool,
    /// When it was last heard from, in epoch ms.
    pub seen: Option<f64>,
}

/// The network as the engine sees it. The service puts xenonet-net's link
/// behind it; with no network, `me` is `None` and `send` refuses.
pub trait Net: Send + Sync {
    /// This machine's host id and network, when it's in one.
    fn me(&self) -> Option<Me>;
    /// The other hosts in the roster.
    fn hosts(&self) -> Vec<Host>;
    /// Queues a body. It refuses only when this machine isn't in a network
    /// or the host isn't in the roster; delivery is the link's to retry.
    fn send(&self, out: Outgoing) -> Result<(), String>;
}

/// What the network calls in the engine.
pub trait Sink: Send + Sync {
    /// A body from another host.
    fn receive(&self, incoming: Incoming);
    /// The link to the gateway came up (`up`), or went down. On the way up
    /// the engine sends its full presence and claims.
    fn linked(&self, up: bool);
    /// A host was removed from the roster: its agents, claims and waiting
    /// handoffs go.
    fn removed(&self, host: &str);
    /// The gateway lost frames for this host (a `gap`): the engine asks the
    /// others for a full presence and sends again what has no receipt.
    fn gap(&self);
}

/// No network: what the engine gets when the service isn't in one, and in
/// the twins.
pub struct NoNet;

impl Net for NoNet {
    fn me(&self) -> Option<Me> {
        None
    }
    fn hosts(&self) -> Vec<Host> {
        Vec::new()
    }
    fn send(&self, _out: Outgoing) -> Result<(), String> {
        Err("this machine isn't in a XenoNet network".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_presence_is_not_queued() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert_eq!(Kind::parse("hello"), None);
        assert!(!Kind::Presence.queued());
        assert!(!Kind::Claims.queued());
        assert!(Kind::Message.queued());
        assert!(Kind::Receipt.queued());
    }

    #[test]
    fn no_net_refuses() {
        let net = NoNet;
        assert!(net.me().is_none());
        assert_eq!(net.hosts().len(), 0);
        let out = Outgoing { to: To::All, kind: Kind::Presence, body: Value::Null };
        assert!(net.send(out).is_err());
    }
}
