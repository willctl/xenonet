//! The seam between the engine and the network (docs/design/network.md,
//! section 6). The engine sends and receives bodies through it: presence,
//! messages, receipts, claims, handoffs and the person's actions, as the
//! JSON that `schemas/network/<kind>/v1.json` describes. Frames, keys,
//! sealing, the roster and the gateway live behind it, in xenonet-net, and
//! the engine knows nothing of them.
//!
//! The map's relay bodies (section 7) cross the same seam but never reach
//! the engine: they go to a `RelaySink`, the service's relay for a console
//! on another host, and the hub that draws other hosts' maps here. Nor do
//! services' requests and answers (`svc`), which xenonet-net forwards to
//! and from a host's published services itself.
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
    /// The map, between a console and the host it shows: never the engine's.
    Relay,
    /// A request to a service another host published, or its answer:
    /// never the engine's either.
    Svc,
}

impl Kind {
    pub const ALL: [Self; 8] = [Self::Presence, Self::Claims, Self::Message, Self::Receipt, Self::Handoff, Self::Operator, Self::Relay, Self::Svc];

    /// The frame kind on the wire, and the schema's folder.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Presence => "presence",
            Self::Claims => "claims",
            Self::Message => "message",
            Self::Receipt => "receipt",
            Self::Handoff => "handoff",
            Self::Operator => "operator",
            Self::Relay => "relay",
            Self::Svc => "svc",
        }
    }

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == kind)
    }

    /// Whether the gateway keeps it for a host that's away. Presence and
    /// claims aren't kept: a full set goes when the host is back. Nor is
    /// the map: a console asks for a fresh snapshot. Nor is a service's
    /// request, which may carry the service's own credential and is
    /// answered now or not at all.
    pub fn queued(self) -> bool {
        !matches!(self, Self::Presence | Self::Claims | Self::Relay | Self::Svc)
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

/// Who may start sessions on a machine (`start`) and wake its sessions
/// for a message (`wake`), as its network.json keeps them: what
/// `delivery::reach` decides by. `pending` lists the changes to `network`
/// another window host asked for, which wait for the founder or a second
/// window host (`network/presence/v1`'s reach.pending).
#[derive(Debug, Clone, PartialEq)]
pub struct Reach {
    pub start: String,
    pub wake: String,
    pub pending: Vec<Value>,
}

/// `start`: nobody, you (from a window host's console), or the network
/// (you, and the sessions and actors on window hosts).
pub const STARTS: [&str; 3] = ["off", "you", "network"];
/// `wake`: your consent per session (`xenonet live allow`), or any sender
/// in the network, within the budgets.
pub const WAKES: [&str; 2] = ["consent", "network"];

impl Default for Reach {
    fn default() -> Self {
        Reach { start: "you".into(), wake: "consent".into(), pending: Vec::new() }
    }
}

impl Reach {
    /// As presence carries it.
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "start": self.start, "wake": self.wake, "pending": self.pending })
    }
}

/// What this build's XenoNet takes from other machines beyond messages,
/// which presence lists (`can`), so a sender knows before it asks:
/// `start` (a start request) and `answer` (an answer to a run's
/// permission prompt).
pub const CAN: [&str; 2] = ["start", "answer"];

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
    /// The hash of this machine's roster head, which presence carries so
    /// two hosts that see different rosters notice (section 3).
    fn roster_head(&self) -> Option<String> {
        None
    }
    /// Whether this host shares titles only (Settings → Network): its
    /// presence then carries no question's text, last message or stuck
    /// reason (section 7).
    fn titles_only(&self) -> bool {
        false
    }
    /// Whether this host sends its load (CPU, GPU, memory and what the
    /// parts are) to the window hosts' consoles that watch its map
    /// (Settings → Network; section 7). Read at each send.
    fn share_load(&self) -> bool {
        true
    }
    /// Whether this host is a window host in the roster: only one is shown
    /// other hosts' maps (section 7).
    fn is_window_host(&self) -> bool {
        false
    }
    /// Who may start and wake sessions here (Settings → Network).
    fn reach(&self) -> Reach {
        Reach::default()
    }
    /// Whether another host's presence said it takes `what` (`CAN`): a
    /// host that runs an older XenoNet says nothing.
    fn host_can(&self, _host: &str, _what: &str) -> bool {
        false
    }
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

/// Where the map's relay bodies go (section 7): the service's relay, for a
/// console on another host that subscribed to this host's map, and the
/// hub, for the maps this host's console subscribed to. Each takes the
/// bodies meant for it and leaves the rest. They hear of the link and the
/// roster as the engine does, so a subscription ends with its host.
pub trait RelaySink: Send + Sync {
    /// A relay body from another host.
    fn relay(&self, incoming: Incoming);
    /// A host was removed from the roster.
    fn removed(&self, _host: &str) {}
    /// The link to the gateway came up, or went down.
    fn linked(&self, _up: bool) {}
    /// The gateway lost frames for this host.
    fn gap(&self) {}
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
    fn roster_head(&self) -> Option<String> {
        None
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
        assert!(!Kind::Relay.queued(), "the map isn't kept for a host that's away");
        assert!(!Kind::Svc.queued(), "nor is a service's request, which may carry its credential");
        assert!(!Kind::Claims.queued());
        assert!(Kind::Message.queued());
        assert!(Kind::Receipt.queued());
    }

    #[test]
    fn no_net_refuses() {
        let net = NoNet;
        assert!(net.me().is_none());
        assert_eq!(net.hosts().len(), 0);
        assert_eq!(net.roster_head(), None);
        let out = Outgoing { to: To::All, kind: Kind::Presence, body: Value::Null };
        assert!(net.send(out).is_err());
    }
}
