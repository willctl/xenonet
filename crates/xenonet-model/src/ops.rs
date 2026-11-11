//! The coordination operations: what one XenoNet tells another, and what
//! an agent or actor asks of the engine (docs/design/direction.md, "Domain
//! model"). An operation's body is one of the network documents
//! (`crate::network`); its kind says which, and whether a gateway keeps it
//! for a host that's away.
//!
//! It's also the seam between the engine and the network
//! (docs/design/network.md, section 6): the bodies the engine sends and
//! receives (`Outgoing`, `Incoming`), this machine and the other hosts as
//! the link sees them (`Me`, `Host`, `Reach`), and the traits each side
//! implements (`Net`, the network as the engine sees it, which xenonet-net
//! puts its link behind; `Sink`, what the network calls in the engine; and
//! `RelaySink`, where the map's relay bodies go). Frames, keys, sealing,
//! the roster and the gateway live behind it, in xenonet-net. With no
//! network, `NoNet` is behind it and every local path is unchanged.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a body is, and so which frame kind carries it: a message, a
/// receipt and a handoff are entries in this machine's feed
/// (xenonet-log's `feed`), which `message` frames carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Which agents are on a host and what each is doing.
    Presence,
    /// The files a host's agents have claimed.
    Claims,
    /// A message from an agent to one on another host.
    Message,
    /// What became of a message on the host it went to.
    Receipt,
    /// Work handed from one agent to another.
    Handoff,
    /// The person's action on another host: an answer, a start, a setting.
    Operator,
    /// The map, between a console and the host it shows: never the engine's.
    Relay,
    /// A request to a service another host published, or its answer:
    /// never the engine's either.
    Svc,
    /// An intent the person made here, or their cancel of one.
    Intent,
    /// The maker's machine giving an intent to the machine it names.
    Assign,
    /// How an intent worked here stands: started, a note, done.
    Report,
}

impl Kind {
    pub const ALL: [Self; 11] =
        [Self::Presence, Self::Claims, Self::Message, Self::Receipt, Self::Handoff, Self::Operator, Self::Relay, Self::Svc, Self::Intent, Self::Assign, Self::Report];

    /// Its name, and the schema's folder; the frame kind on the wire for
    /// what isn't a feed entry.
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
            Self::Intent => "intent",
            Self::Assign => "assign",
            Self::Report => "report",
        }
    }

    pub fn parse(kind: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == kind)
    }

    /// Whether a gateway keeps it for a host that's away. Presence isn't
    /// kept: a full set goes when the host is back. Nor is the map: a
    /// console asks for a fresh snapshot. Nor is a service's request,
    /// which may carry the service's own credential and is answered now or
    /// not at all.
    pub fn queued(self) -> bool {
        !matches!(self, Self::Presence | Self::Relay | Self::Svc)
    }

    /// Whether it travels as an entry of this machine's feed: a message
    /// and a receipt loose, each taken once by its id, and a claim, a
    /// handoff and an intent's entries kept, in the feed's gapless
    /// sequence to every member.
    pub fn fed(self) -> bool {
        matches!(self, Self::Message | Self::Receipt | Self::Claims | Self::Handoff | Self::Intent | Self::Assign | Self::Report)
    }

    /// The schema of a body of this kind: a message's is the body of its
    /// feed entry, and the person's answer is at v2, which carries the
    /// act's authority. An intent's entries are all `network/intent/v1`,
    /// told apart by their op.
    pub fn schema(self) -> String {
        match self {
            Self::Message => "network/send/v1".into(),
            Self::Operator => "network/operator/v2".into(),
            Self::Intent | Self::Assign | Self::Report => "network/intent/v1".into(),
            _ => format!("network/{}/v1", self.as_str()),
        }
    }
}

/// Who a body is for. A host is the text the sender gave: the network
/// checks it against the roster, which is what says which hosts there are.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum To {
    /// One host, by its id.
    Host(String),
    /// Every other host in the network.
    All,
}

/// A body the engine asks the network to carry.
#[derive(Debug, Clone)]
pub struct Outgoing {
    /// The network it goes in, by id: one of `Net::networks`. None is the
    /// first of them, the one network the gateway's path carries. A network
    /// this machine isn't in is refused, never another taken in its place.
    pub network: Option<String>,
    pub to: To,
    pub kind: Kind,
    pub body: Value,
    /// For a message: what the act rests on (`network/message/v3`'s
    /// `authority`), which goes beside its feed entry, for its one
    /// recipient.
    pub authority: Option<Value>,
}

impl Outgoing {
    /// A body that rests on nothing but this machine's word, in its first
    /// network.
    pub fn new(to: To, kind: Kind, body: Value) -> Outgoing {
        Outgoing { network: None, to, kind, body, authority: None }
    }

    /// The same body in `network`.
    #[must_use]
    pub fn within(mut self, network: impl Into<String>) -> Outgoing {
        self.network = Some(network.into());
        self
    }
}

/// A body another host sent, after the network checked its signature
/// against the roster and opened it.
#[derive(Debug, Clone)]
pub struct Incoming {
    /// The network it came in, by id: presence and the rest are each
    /// network's own, even from a machine in several with this one.
    pub network: String,
    /// The host that sent it, which is in the roster.
    pub from: String,
    pub kind: Kind,
    pub body: Value,
    /// When the sender made it, by the sender's clock shifted to ours.
    pub at: f64,
    /// For the map's relay: whether this machine decided the sender may
    /// read what it relays (an admin's read). The engine decides every
    /// other kind of act itself, on its own copy of the chains.
    pub from_window_host: bool,
}

/// This machine in one of its networks: the network, and the host id it
/// has there, which may differ from one network to the next.
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
/// for a message (`wake`), as the network's chain records them (each
/// member's settings entry). `pending` lists the settings entries for it
/// another admin signed, which wait for the founder or a second admin
/// (`network/presence/v1`'s reach.pending).
#[derive(Debug, Clone, PartialEq)]
pub struct Reach {
    pub start: String,
    pub wake: String,
    pub pending: Vec<Value>,
    /// While `start` is delegate: the admins whose entry set it, and when
    /// (epoch ms).
    pub delegated: Option<(String, i64)>,
}

/// `start`: nobody, you, the network (you, and the network's sessions and
/// actors), or delegate: the network, and what a session or actor starts
/// here runs on your standing grant, within its limits
/// (`delivery::delegation`).
pub const STARTS: [&str; 4] = ["off", "you", "network", "delegate"];
/// `wake`: your consent per session (`xenonet live allow`), or any sender
/// in the network, within the budgets.
pub const WAKES: [&str; 2] = ["consent", "network"];

impl Default for Reach {
    fn default() -> Self {
        Reach { start: "you".into(), wake: "consent".into(), pending: Vec::new(), delegated: None }
    }
}

impl Reach {
    /// As presence carries it.
    pub fn to_json(&self) -> Value {
        let mut out = serde_json::json!({ "start": self.start, "wake": self.wake, "pending": self.pending });
        if let Some((by, at)) = &self.delegated {
            out["delegated"] = serde_json::json!({ "by": by, "at": at });
        }
        out
    }
}

/// What this build's XenoNet takes from other machines beyond messages,
/// which presence lists (`can`), so a sender knows before it asks:
/// `start` (a start request) and `answer` (an answer to a run's
/// permission prompt).
pub const CAN: [&str; 2] = ["start", "answer"];

/// The network as the engine sees it. The service puts xenonet-net's link
/// behind it; with no network, `networks` is empty and `send` refuses.
pub trait Net: Send + Sync {
    /// Every network this machine is in, with its host id in each: the one
    /// it founded or joined first, first.
    fn networks(&self) -> Vec<Me>;
    /// The first of `networks`, which the paths that name no network yet
    /// (the gateway's) go through; None when this machine is in none.
    fn me(&self) -> Option<Me> {
        self.networks().into_iter().next()
    }
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
    /// Tells `host` that this machine has applied every kept entry of its
    /// feed in `network` up to `high` of incarnation `inc`, so it resends
    /// only what's after. An author has a feed of its own in each network.
    fn ack(&self, _network: &str, _host: &str, _inc: u64, _high: u64) {}
    /// Asks `host` for the kept entries of its feed in `network` from
    /// `from` of incarnation `inc` on: one arrived past a gap.
    fn want(&self, _network: &str, _host: &str, _inc: u64, _from: u64) {}
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
    fn networks(&self) -> Vec<Me> {
        Vec::new()
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

/// An operation between XenoNets, typed by its body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "lowercase")]
pub enum Op {
    Presence(crate::network::Presence),
    Claims(crate::network::Claim),
    Message(crate::network::SendBody),
    Receipt(crate::network::Receipt),
    Handoff(crate::network::Handoff),
    Operator(crate::network::Operator),
    Relay(crate::network::Relay),
    Svc(crate::network::Svc),
}

impl Op {
    pub fn kind(&self) -> Kind {
        match self {
            Op::Presence(_) => Kind::Presence,
            Op::Claims(_) => Kind::Claims,
            Op::Message(_) => Kind::Message,
            Op::Receipt(_) => Kind::Receipt,
            Op::Handoff(_) => Kind::Handoff,
            Op::Operator(_) => Kind::Operator,
            Op::Relay(_) => Kind::Relay,
            Op::Svc(_) => Kind::Svc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_say_what_is_kept() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
            assert_eq!(serde_json::to_string(&k).unwrap(), format!("\"{}\"", k.as_str()));
        }
        assert!(Kind::Message.queued() && Kind::Handoff.queued() && Kind::Claims.queued() && Kind::Operator.queued() && Kind::Receipt.queued());
        assert!(!Kind::Presence.queued() && !Kind::Relay.queued() && !Kind::Svc.queued());
        assert!(Kind::Message.fed() && Kind::Receipt.fed() && Kind::Handoff.fed() && Kind::Claims.fed() && Kind::Intent.fed() && Kind::Assign.fed() && Kind::Report.fed());
        assert!(!Kind::Presence.fed() && !Kind::Operator.fed() && !Kind::Relay.fed());
        assert_eq!(Kind::parse("frame"), None);
        assert_eq!(Kind::Presence.schema(), "network/presence/v1");
        assert_eq!(Kind::Message.schema(), "network/send/v1");
    }

    #[test]
    fn no_net_refuses() {
        let net = NoNet;
        assert!(net.me().is_none() && net.networks().is_empty());
        assert_eq!(net.hosts().len(), 0);
        assert_eq!(net.roster_head(), None);
        let out = Outgoing::new(To::All, Kind::Presence, Value::Null);
        assert!(net.send(out).is_err());
    }
}
