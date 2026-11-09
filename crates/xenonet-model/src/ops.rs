//! The coordination operations: what one XenoNet tells another, and what
//! an agent or actor asks of the engine (docs/design/direction.md, "Domain
//! model"). An operation's body is one of the network documents
//! (`crate::network`); its kind says which, and whether a gateway keeps it
//! for a host that's away.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What a body is, and so which frame kind carries it.
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

    /// Whether a gateway keeps it for a host that's away. Presence and
    /// claims aren't kept: a full set goes when the host is back. Nor is
    /// the map: a console asks for a fresh snapshot. Nor is a service's
    /// request, which may carry the service's own credential and is
    /// answered now or not at all.
    pub fn queued(self) -> bool {
        !matches!(self, Self::Presence | Self::Claims | Self::Relay | Self::Svc)
    }

    /// The schema of a body of this kind.
    pub fn schema(self) -> String {
        format!("network/{}/v1", self.as_str())
    }
}

/// Who a body is for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum To {
    /// One host, by its id.
    Host(crate::ids::HostId),
    /// Every other host in the network.
    All,
}

/// An operation between XenoNets, typed by its body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "body", rename_all = "lowercase")]
pub enum Op {
    Presence(crate::network::Presence),
    Claims(crate::network::Claims),
    Message(crate::network::Message),
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
        assert!(Kind::Message.queued() && Kind::Handoff.queued() && Kind::Operator.queued() && Kind::Receipt.queued());
        assert!(!Kind::Presence.queued() && !Kind::Claims.queued() && !Kind::Relay.queued() && !Kind::Svc.queued());
        assert_eq!(Kind::parse("frame"), None);
        assert_eq!(Kind::Presence.schema(), "network/presence/v1");
    }
}
