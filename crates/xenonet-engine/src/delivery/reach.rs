//! What this machine lets a sender do to its sessions and actors, decided
//! where the message arrives: core/delivery/reach.js. A message from
//! another machine asks before it's stored, each way the router wakes a
//! session asks before it's used, and a start asks before anything runs.
//! One place, so limits on who may reach whom go here, never in the
//! transport.
//!
//! ```text
//!   message  open: the sender's own machine checked its scope, and the
//!            policy's guards (the pause switch, rates, loops) still hold
//!   wake     wake: network lets any sender wake a session, within the
//!            budgets; else your allowance for the session (xenonet live
//!            allow), which has to cover actors (--remote) when one sends
//!   start    start: off refuses all; a start comes only from a window
//!            host (this machine counts as one); you need a strong check
//!            (the window key); sessions and actors need start: network
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
/// another; with whether the machine it came from is a window host (this
/// one counts as one) and whether that machine checked it strongly (the
/// window key, or process checks on macOS).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    pub kind: Kind,
    pub id: String,
    pub host: Option<String>,
    pub window: bool,
    pub strong: bool,
}

impl Sender {
    pub fn to_arg(&self) -> Arg {
        Arg::Obj(vec![
            ("kind".into(), Arg::text(self.kind.as_str())),
            ("id".into(), Arg::text(self.id.clone())),
            ("host".into(), self.host.clone().map_or(Arg::Null, Arg::Text)),
            ("window".into(), Arg::Bool(self.window)),
            ("strong".into(), Arg::Bool(self.strong)),
        ])
    }

    /// A sender as the JS passed it ({ kind, id, host, window, strong }):
    /// window unless it says otherwise, strong only when it says so.
    pub fn of(a: &Arg) -> Sender {
        let kind = match a.field("kind").as_str() {
            Some("you") => Kind::You,
            Some("actor") => Kind::Actor,
            _ => Kind::Session,
        };
        let window = !matches!(a.field("window"), Arg::Bool(false));
        Sender { kind, id: js_text(&a.field("id")), host: a.field("host").as_str().map(str::to_string), window, strong: a.field("strong").truthy() }
    }
}

/// A sender by its id, as a message names it: from a window host, checked
/// weakly, unless the caller knows better.
pub fn sender_of(id: &Arg) -> Sender {
    let s = if id.is_nullish() { String::new() } else { js_text(id) };
    let text = Arg::text(s.clone());
    let make = |kind, host| Sender { kind, id: s.clone(), host, window: true, strong: false };
    if is_you(&text) {
        return make(Kind::You, None);
    }
    if let Some((host, _)) = actor_of(&text) {
        return make(Kind::Actor, host);
    }
    let host = xenonet_base::ids::parse_qualified(&s).and_then(|(h, _)| h).map(str::to_string);
    make(Kind::Session, host)
}

/// What a sender asks to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Message,
    Wake,
    Start,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        match s {
            "message" => Some(Action::Message),
            "wake" => Some(Action::Wake),
            "start" => Some(Action::Start),
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

/// Who may start sessions here (network.json's start).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Start {
    Off,
    #[default]
    You,
    Network,
}

impl Start {
    /// off, you or network; anything else is you.
    pub fn parse(s: &str) -> Start {
        match s {
            "off" => Start::Off,
            "network" => Start::Network,
            _ => Start::You,
        }
    }
}

/// Who may wake sessions here (network.json's wake).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Wake {
    #[default]
    Consent,
    Network,
}

impl Wake {
    /// consent or network; anything else is consent.
    pub fn parse(s: &str) -> Wake {
        if s == "network" {
            Wake::Network
        } else {
            Wake::Consent
        }
    }
}

/// What this machine is set to, as far as a decision needs it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Settings {
    pub consent: Consent,
    pub start: Start,
    pub wake: Wake,
}

impl Settings {
    /// As the JS passes it: { consent: { here, remote }, start, wake }.
    pub fn of(a: &Arg) -> Settings {
        let consent = a.field("consent");
        Settings {
            consent: Consent { here: consent.field("here").truthy(), remote: consent.field("remote").truthy() },
            start: Start::parse(a.field("start").as_str().unwrap_or("")),
            wake: Wake::parse(a.field("wake").as_str().unwrap_or("")),
        }
    }
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
    let refuse = |why: String| Decision::Refuse(why);
    match action {
        Action::Message => Decision::Allow,
        Action::Wake => {
            if settings.wake == Wake::Network {
                return Decision::Allow;
            }
            let c = settings.consent;
            let allowed = if sender.kind == Kind::Actor { c.remote } else { c.here };
            if allowed {
                Decision::Allow
            } else {
                refuse("no consent".into())
            }
        }
        Action::Start => {
            if settings.start == Start::Off {
                return refuse("this machine starts no sessions: its start setting is off".into());
            }
            // The receiver can tell only which machine signed the request
            // and whether it's a window host; what it says of its sender is
            // that machine's word.
            if !sender.window {
                let from = sender.host.as_deref().unwrap_or("that machine");
                return refuse(format!("{from} isn't a window host, and only window hosts start sessions"));
            }
            match sender.kind {
                Kind::You if sender.strong => Decision::Allow,
                Kind::You => refuse("a start as you comes only from a window host's console, checked by its window key".into()),
                _ if settings.start == Start::Network => Decision::Allow,
                _ => refuse("only you start sessions here: its start setting is you".into()),
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
        assert!(of("you").window && !of("you").strong, "window unless told otherwise; strong only when told");
    }

    #[test]
    fn a_message_is_open_and_a_wake_needs_your_allowance_or_wake_network() {
        let consent = |here, remote| Settings { consent: Consent { here, remote }, ..Settings::default() };
        let none = consent(false, false);
        let here = consent(true, false);
        let both = consent(true, true);
        let network = Settings { wake: Wake::Network, ..none };
        for id in ["you", "claude:aaaaaaaa-0001", "@laptop:claude:aaaaaaaa-0001", "actor:muse", "@studio:actor:muse"] {
            let s = sender_of(&Arg::text(id));
            assert_eq!(decide(Action::Message, &s, "claude:x", &none), Decision::Allow, "{id}");
            assert_eq!(decide(Action::Wake, &s, "claude:x", &none), Decision::Refuse("no consent".into()), "{id}");
            assert_eq!(decide(Action::Wake, &s, "claude:x", &both), Decision::Allow, "{id}");
            assert_eq!(decide(Action::Wake, &s, "claude:x", &network), Decision::Allow, "{id}: wake network needs no allowance");
            // Your allowance from here covers you and sessions; an actor,
            // on this machine or another, needs it for actors too.
            let actor = s.kind == Kind::Actor;
            assert_eq!(decide(Action::Wake, &s, "claude:x", &here).allowed(), !actor, "{id}");
        }
        assert_eq!(Action::parse("start"), Some(Action::Start));
        assert_eq!(Action::parse("update"), None);
    }

    #[test]
    fn a_start_comes_from_a_window_host_as_you_strongly_or_under_start_network() {
        let set = |start| Settings { start, ..Settings::default() };
        let from = |id: &str, window, strong| Sender { window, strong, ..sender_of(&Arg::text(id)) };
        let you = from("you", true, true);
        let weak = from("you", true, false);
        let session = from("@laptop:claude:aaaaaaaa-0001", true, false);
        let actor = from("actor:muse", true, true);
        let server = Sender { host: Some("desk".into()), ..from("you", false, true) };
        let d = |s: &Sender, st| decide(Action::Start, s, "", &set(st));
        // You, strongly, from a window host: under you and network.
        assert_eq!(d(&you, Start::You), Decision::Allow);
        assert_eq!(d(&you, Start::Network), Decision::Allow);
        assert_eq!(d(&weak, Start::Network), Decision::Refuse("a start as you comes only from a window host's console, checked by its window key".into()));
        // Sessions and actors on window hosts: only under network.
        for s in [&session, &actor] {
            assert_eq!(d(s, Start::You), Decision::Refuse("only you start sessions here: its start setting is you".into()));
            assert_eq!(d(s, Start::Network), Decision::Allow);
        }
        // A machine that isn't a window host starts nothing, whoever it
        // names as the sender.
        assert_eq!(d(&server, Start::Network), Decision::Refuse("desk isn't a window host, and only window hosts start sessions".into()));
        // off: nobody.
        assert_eq!(d(&you, Start::Off), Decision::Refuse("this machine starts no sessions: its start setting is off".into()));
        assert_eq!((Start::parse("network"), Start::parse("x"), Wake::parse("network"), Wake::parse("")), (Start::Network, Start::You, Wake::Network, Wake::Consent));
        let js = Arg::Obj(vec![("start".into(), Arg::text("network")), ("wake".into(), Arg::text("network"))]);
        assert_eq!(Settings::of(&js), Settings { consent: Consent::default(), start: Start::Network, wake: Wake::Network });
    }
}
