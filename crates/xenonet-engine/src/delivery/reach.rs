//! What this machine lets a sender do to its sessions and actors, decided
//! where the message arrives: core/delivery/reach.js. A message from
//! another machine asks before it's stored, and each way the router wakes
//! a session asks before it's used. One place, so limits on who may reach
//! whom go here, never in the transport.
//!
//! ```text
//!   message  open: the sender's own machine checked its scope, and the
//!            policy's guards (the pause switch, rates, loops) still hold
//!   wake     your allowance for the session (xenonet live allow), which
//!            has to cover actors (--remote) when an actor sends
//! ```

use super::policy::{actor_of, is_you};
use crate::sql::Arg;
use crate::store::js_text;

/// Who sent it, as this machine sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    You,
    Session,
    Actor,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::You => "you",
            Kind::Session => "session",
            Kind::Actor => "actor",
        }
    }
}

/// A sender: you, a session or an actor, on this machine (no host) or
/// another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    pub kind: Kind,
    pub id: String,
    pub host: Option<String>,
}

impl Sender {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![("kind".into(), Arg::text(self.kind.as_str())), ("id".into(), Arg::text(self.id.clone())), ("host".into(), self.host.clone().map_or(Arg::Null, Arg::Text))])
    }

    /// A sender as the JS passed it ({ kind, id, host }).
    pub fn of(a: &Arg) -> Sender {
        let kind = match a.field("kind").as_str() {
            Some("you") => Kind::You,
            Some("actor") => Kind::Actor,
            _ => Kind::Session,
        };
        Sender { kind, id: js_text(&a.field("id")), host: a.field("host").as_str().map(str::to_string) }
    }
}

/// A sender by its id.
pub fn sender_of(id: &Arg) -> Sender {
    let s = if id.is_nullish() { String::new() } else { js_text(id) };
    let text = Arg::text(s.clone());
    if is_you(&text) {
        return Sender { kind: Kind::You, id: s, host: None };
    }
    if let Some((host, _)) = actor_of(&text) {
        return Sender { kind: Kind::Actor, id: s, host };
    }
    let host = xenonet_base::ids::parse_qualified(&s).and_then(|(h, _)| h).map(str::to_string);
    Sender { kind: Kind::Session, id: s, host }
}

/// What a sender asks to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Message,
    Wake,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "message" => Some(Action::Message),
            "wake" => Some(Action::Wake),
            _ => None,
        }
    }
}

/// Whether you allowed the target to be woken: from here, and by actors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Consent {
    pub here: bool,
    pub remote: bool,
}

/// What this machine is set to, as far as a decision needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub consent: Consent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Refuse(String),
}

impl Decision {
    pub fn allowed(&self) -> bool {
        *self == Decision::Allow
    }

    /// { ok: true }, or { ok: false, why }, as reach.js answers.
    pub fn to_arg(&self) -> Arg {
        match self {
            Decision::Allow => Arg::Obj(vec![("ok".into(), Arg::Bool(true))]),
            Decision::Refuse(why) => Arg::Obj(vec![("ok".into(), Arg::Bool(false)), ("why".into(), Arg::text(why.clone()))]),
        }
    }
}

/// Whether `sender` may do `action` to `target` here. The target isn't
/// read yet: it's there for the limits per target that come later.
pub fn decide(action: Action, sender: &Sender, _target: &str, settings: &Settings) -> Decision {
    match action {
        Action::Message => Decision::Allow,
        Action::Wake => {
            let c = settings.consent;
            let allowed = if sender.kind == Kind::Actor { c.remote } else { c.here };
            if allowed {
                Decision::Allow
            } else {
                Decision::Refuse("no consent".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn senders_are_you_sessions_or_actors_here_or_elsewhere() {
        let of = |id: &str| sender_of(&Arg::text(id));
        assert_eq!(of("actor:you").kind, Kind::You);
        assert_eq!(of("you").kind, Kind::You);
        assert_eq!((of("actor:muse").kind, of("actor:muse").host), (Kind::Actor, None));
        assert_eq!((of("@studio:actor:muse").kind, of("@studio:actor:muse").host), (Kind::Actor, Some("studio".into())));
        assert_eq!((of("claude:aaaaaaaa-0001").kind, of("claude:aaaaaaaa-0001").host), (Kind::Session, None));
        assert_eq!(of("@laptop:claude:aaaaaaaa-0001").host, Some("laptop".into()));
        assert_eq!(Sender::of(&of("@studio:actor:muse").to_arg()), of("@studio:actor:muse"));
    }

    #[test]
    fn a_message_is_open_and_a_wake_needs_your_allowance() {
        let consent = |here, remote| Settings { consent: Consent { here, remote } };
        let none = consent(false, false);
        let here = consent(true, false);
        let both = consent(true, true);
        for id in ["you", "claude:aaaaaaaa-0001", "@laptop:claude:aaaaaaaa-0001", "actor:muse", "@studio:actor:muse"] {
            let s = sender_of(&Arg::text(id));
            assert_eq!(decide(Action::Message, &s, "claude:x", &none), Decision::Allow, "{id}");
            assert_eq!(decide(Action::Wake, &s, "claude:x", &none), Decision::Refuse("no consent".into()), "{id}");
            assert_eq!(decide(Action::Wake, &s, "claude:x", &both), Decision::Allow, "{id}");
            // Your allowance from here covers you and sessions; an actor,
            // on this machine or another, needs it for actors too.
            let actor = s.kind == Kind::Actor;
            assert_eq!(decide(Action::Wake, &s, "claude:x", &here).allowed(), !actor, "{id}");
        }
        assert_eq!(Action::parse("start"), None);
    }
}
