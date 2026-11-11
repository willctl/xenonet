//! What a request is made of. Every type here is closed: an id is a short
//! word from a fixed alphabet, an act is one of a fixed list with typed
//! fields, and nothing has room for message text, so the words of a message,
//! a handoff or an ask can't reach a decision through a request. The caller
//! parses these from the signed act and its envelope's header, never from a
//! body.

use crate::{AuthzError, Role, Settings, R};
use std::fmt;
use xenonet_id::codec::sha256;

/// The longest id a request carries.
const ID_MAX: usize = 128;
/// The longest folder a start names.
const FOLDER_MAX: usize = 4096;

// An id is letters, digits and the few marks XenoNet's own ids use, so it
// can't carry a sentence, a newline or a quote into a token's Datalog.
fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= ID_MAX && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:@/+=~-".contains(&b))
}

macro_rules! id {
    ($(#[$doc:meta])* $name:ident, $what:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// The id, when it's a short word of letters, digits and `._:@/+=~-`.
            pub fn new(s: &str) -> R<$name> {
                if valid_id(s) {
                    Ok($name(s.to_string()))
                } else {
                    Err(AuthzError::new("id", format!("{s:?} isn't {}", $what)))
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// A machine: one XenoNet service, one OS user on one host.
    MachineId,
    "a machine's id"
);
id!(
    /// A network, by the id its chain gives it.
    NetworkId,
    "a network's id"
);
id!(
    /// A person, by the id their chain gives them.
    PersonId,
    "a person's id"
);
id!(
    /// One of a person's devices: the device key's id (`xns1_…`).
    DeviceId,
    "a device's id"
);
id!(
    /// An agent: a harness session, by the id its machine minted.
    AgentId,
    "an agent's id"
);
id!(
    /// An actor, by its slug on its machine.
    ActorSlug,
    "an actor's slug"
);
id!(
    /// A session on this machine, for the facts only this machine holds
    /// (its consent rows).
    SessionId,
    "a session's id"
);
id!(
    /// A permission question a run asked.
    AskId,
    "a question's id"
);
id!(
    /// An app a run is started in, as its adapter names it.
    AppId,
    "an app's id"
);
id!(
    /// A service a machine publishes to its network.
    ServiceId,
    "a service's id"
);
id!(
    /// A project, by the name this machine's map gives it (its repo's).
    ProjectId,
    "a project's id"
);

/// The folder a run starts in. A path, so longer than an id and with any
/// printable character, but never a control character, so it can't hold a
/// message's lines.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Folder(String);

impl Folder {
    pub fn new(s: &str) -> R<Folder> {
        if s.is_empty() || s.len() > FOLDER_MAX || s.chars().any(char::is_control) {
            return Err(AuthzError::new("id", "that isn't a folder"));
        }
        Ok(Folder(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How the machine that vouches for an agent checked it, weakest first:
/// policy reads this, never a strength the sender claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Attestation {
    /// Nothing checked the process: anything on the account could be it.
    Unchecked,
    /// The machine checked the process (its parent, its binary).
    ProcessChecked,
    /// The harness signed for it.
    HarnessSigned,
}

impl Attestation {
    pub const ALL: [Attestation; 3] = [Attestation::Unchecked, Attestation::ProcessChecked, Attestation::HarnessSigned];

    pub fn as_str(self) -> &'static str {
        match self {
            Attestation::Unchecked => "unchecked",
            Attestation::ProcessChecked => "process-checked",
            Attestation::HarnessSigned => "harness-signed",
        }
    }
}

/// Who asks, as the receiver knows them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    /// A machine's service, acting for itself.
    Machine { id: MachineId },
    /// An actor registered on `machine`; it proves itself with a token.
    Actor { slug: ActorSlug, machine: MachineId },
    /// An agent on `machine`, vouched for with how it was checked.
    Agent { id: AgentId, machine: MachineId, attestation: Attestation },
    /// The person, at one of their devices: only once the device's
    /// signature over the act was checked.
    Person(crate::PersonSigned),
}

impl Principal {
    /// The machine it's on; a person is on none.
    pub fn machine(&self) -> Option<&MachineId> {
        match self {
            Principal::Machine { id } => Some(id),
            Principal::Actor { machine, .. } | Principal::Agent { machine, .. } => Some(machine),
            Principal::Person(_) => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Principal::Machine { .. } => "machine",
            Principal::Actor { .. } => "actor",
            Principal::Agent { .. } => "agent",
            Principal::Person(_) => "person",
        }
    }
}

/// What a request does. Each is one word in a token's `right(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Message,
    Wake,
    Start,
    Answer,
    Claim,
    Handoff,
    Read,
    Admit,
    Widen,
    Publish,
    RegisterActor,
    UseGateway,
    Revoke,
    /// Calling an outbound actor: another agent XenoNet reaches over A2A.
    Delegate,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Message,
        Action::Wake,
        Action::Start,
        Action::Answer,
        Action::Claim,
        Action::Handoff,
        Action::Read,
        Action::Admit,
        Action::Widen,
        Action::Publish,
        Action::RegisterActor,
        Action::UseGateway,
        Action::Revoke,
        Action::Delegate,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Action::Message => "message",
            Action::Wake => "wake",
            Action::Start => "start",
            Action::Answer => "answer",
            Action::Claim => "claim",
            Action::Handoff => "handoff",
            Action::Read => "read",
            Action::Admit => "admit",
            Action::Widen => "widen",
            Action::Publish => "publish",
            Action::RegisterActor => "register-actor",
            Action::UseGateway => "use-gateway",
            Action::Revoke => "revoke",
            Action::Delegate => "delegate",
        }
    }

    pub fn parse(s: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.as_str() == s)
    }
}

/// What a request is done to. A resource on a machine names it, and a
/// machine decides only for itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    Machine(MachineId),
    Network(NetworkId),
    /// A run on `machine`: one app in one folder.
    Run {
        machine: MachineId,
        app: AppId,
        folder: Folder,
    },
    /// A session here, for the facts only this machine holds.
    Session(SessionId),
    /// One of this machine's outbound actors, which it calls.
    Actor(ActorSlug),
}

/// How a started run's permission prompts are answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StartMode {
    /// They come to the person.
    Asks,
    /// The person's standing grant judges them.
    Delegated,
}

/// The person's answer to a permission prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnswerDecision {
    Allow,
    Deny,
}

/// What a permission prompt asks to do, as the machine whose run asked it
/// sorted the tool call: what an actor's allow scope or the person's
/// delegation names when it lets an Allow through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PromptKind {
    /// Reading or searching files.
    Read,
    /// Creating or changing files.
    Edit,
    /// Running a command.
    Command,
    /// Reaching the web.
    Web,
    /// Any other tool.
    Tool,
}

impl PromptKind {
    pub const ALL: [PromptKind; 5] = [PromptKind::Read, PromptKind::Edit, PromptKind::Command, PromptKind::Web, PromptKind::Tool];

    pub fn as_str(self) -> &'static str {
        match self {
            PromptKind::Read => "read",
            PromptKind::Edit => "edit",
            PromptKind::Command => "command",
            PromptKind::Web => "web",
            PromptKind::Tool => "tool",
        }
    }

    pub fn parse(s: &str) -> Option<PromptKind> {
        PromptKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// What a read reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadScope {
    /// Who's there and what they're doing: the map.
    Map,
    /// What sessions said.
    Transcripts,
    /// A search over them.
    Search,
    /// What this machine holds beyond the map: the messages between agents
    /// (the engine's state), delivery's records, other machines' state.
    State,
}

impl ReadScope {
    pub const ALL: [ReadScope; 4] = [ReadScope::Map, ReadScope::Transcripts, ReadScope::Search, ReadScope::State];

    pub fn as_str(self) -> &'static str {
        match self {
            ReadScope::Map => "map",
            ReadScope::Transcripts => "transcripts",
            ReadScope::Search => "search",
            ReadScope::State => "state",
        }
    }
}

/// What a revocation takes back.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Revocation {
    /// A token, by one of its Biscuit revocation ids.
    Token(Vec<u8>),
    /// One of the person's devices.
    Device(DeviceId),
}

/// The act itself, with the fields a decision reads, as the caller parsed
/// them from the signed act. Text (a message, a handoff's note, a prompt)
/// isn't here: it travels beside the request as `Information`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    Message,
    Wake,
    Start {
        app: AppId,
        folder: Folder,
        mode: StartMode,
    },
    Answer {
        ask: AskId,
        decision: AnswerDecision,
    },
    /// What's claimed doesn't change who may claim; claims name (project,
    /// path) once projects have stored ids.
    Claim,
    Handoff,
    /// `filters` is the hash of the filters a page was read with, which a
    /// page cursor is bound to.
    Read {
        scope: ReadScope,
        filters: Option<[u8; 32]>,
    },
    Admit {
        machine: MachineId,
        role: Role,
    },
    Widen {
        machine: MachineId,
        settings: Settings,
    },
    /// Publishing a service to the network: its name, and the hash of
    /// what it publishes (its address, paths and callers), so a presence
    /// for one publication can't be put on a wider one.
    Publish {
        service: ServiceId,
        spec: [u8; 32],
    },
    /// Issuing an actor's grant, or widening it: its rights and its
    /// scopes (the hash of the rows the receiver keeps of them), so a
    /// presence for one grant can't be put on a wider one.
    RegisterActor {
        slug: ActorSlug,
        rights: Vec<Action>,
        scopes: [u8; 32],
    },
    UseGateway,
    Revoke {
        what: Revocation,
    },
    /// Handing work to an outbound actor. `project` is the project of the
    /// session that delegates, as this machine's own map holds it, never as
    /// the session says; None for the person, or a session in no project.
    /// The work's words are Information, beside the request.
    Delegate {
        project: Option<ProjectId>,
    },
}

impl Act {
    pub fn action(&self) -> Action {
        match self {
            Act::Message => Action::Message,
            Act::Wake => Action::Wake,
            Act::Start { .. } => Action::Start,
            Act::Answer { .. } => Action::Answer,
            Act::Claim => Action::Claim,
            Act::Handoff => Action::Handoff,
            Act::Read { .. } => Action::Read,
            Act::Admit { .. } => Action::Admit,
            Act::Widen { .. } => Action::Widen,
            Act::Publish { .. } => Action::Publish,
            Act::RegisterActor { .. } => Action::RegisterActor,
            Act::UseGateway => Action::UseGateway,
            Act::Revoke { .. } => Action::Revoke,
            Act::Delegate { .. } => Action::Delegate,
        }
    }

    /// What a presence block for this act approves, for the acts that may
    /// take the person there. An answer takes one either way, so the block
    /// can stand for the device's signature, but only an Allow needs it.
    pub fn presence_what(&self) -> Option<&'static str> {
        match self {
            Act::Answer { .. } => Some("answer"),
            Act::Admit { .. } => Some("admit"),
            Act::Widen { .. } => Some("widen"),
            Act::Publish { .. } => Some("publish"),
            Act::Revoke { .. } => Some("revoke"),
            Act::RegisterActor { .. } => Some("grant"),
            _ => None,
        }
    }

    /// Whether this act widens what anyone may do, so the person must be
    /// there for it: every act that takes a presence, but a Deny, which
    /// can only narrow.
    pub fn needs_presence(&self) -> bool {
        match self {
            Act::Answer { decision, .. } => *decision == AnswerDecision::Allow,
            _ => self.presence_what().is_some(),
        }
    }
}

/// Where a request came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Door {
    /// A surface on this machine: the console, the command, the hooks, the
    /// MCP servers.
    Local,
    /// Another machine of a network this machine is in.
    Network,
    /// This machine's gateway, which actors outside reach it through.
    Gateway,
    /// The A2A front door.
    A2a,
}

impl Door {
    pub const ALL: [Door; 4] = [Door::Local, Door::Network, Door::Gateway, Door::A2a];

    pub fn as_str(self) -> &'static str {
        match self {
            Door::Local => "local",
            Door::Network => "network",
            Door::Gateway => "gateway",
            Door::A2a => "a2a",
        }
    }
}

/// One act to decide: who, what, to what, how it came in, when (this
/// machine's clock, in milliseconds since 1970), and the network its
/// envelope named. The action is the act's, so the two can't disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    principal: Principal,
    action: Action,
    resource: Resource,
    act: Act,
    door: Door,
    at: i64,
    network: Option<NetworkId>,
}

impl Request {
    pub fn new(principal: Principal, resource: Resource, act: Act, door: Door, at: i64, network: Option<NetworkId>) -> Request {
        Request { principal, action: act.action(), resource, act, door, at, network }
    }

    pub fn principal(&self) -> &Principal {
        &self.principal
    }

    pub fn action(&self) -> Action {
        self.action
    }

    pub fn resource(&self) -> &Resource {
        &self.resource
    }

    pub fn act(&self) -> &Act {
        &self.act
    }

    pub fn door(&self) -> Door {
        self.door
    }

    pub fn at(&self) -> i64 {
        self.at
    }

    pub fn network(&self) -> Option<&NetworkId> {
        self.network.as_ref()
    }

    /// The hash a person's device signs, and a presence block names, for
    /// this request's act.
    pub fn act_hash(&self) -> [u8; 32] {
        act_hash(&self.resource, &self.act)
    }
}

// Each field is written with its length first, so no two acts write the
// same bytes.
struct Canon(Vec<u8>);

impl Canon {
    fn field(&mut self, s: &str) -> &mut Canon {
        self.bytes(s.as_bytes())
    }

    fn bytes(&mut self, b: &[u8]) -> &mut Canon {
        self.0.extend_from_slice(format!("{}:", b.len()).as_bytes());
        self.0.extend_from_slice(b);
        self
    }
}

/// The SHA-256 of an act and what it's done to, in canonical bytes: what a
/// person's device signs for it and what a presence block for it names. It
/// covers every field, so a signature or a presence made for one act
/// (allow) can't be put on another (deny, another question, another
/// machine).
pub fn act_hash(resource: &Resource, act: &Act) -> [u8; 32] {
    let mut c = Canon(b"xenonet-authz-act/1\n".to_vec());
    match resource {
        Resource::Machine(m) => c.field("machine").field(m.as_str()),
        Resource::Network(n) => c.field("network").field(n.as_str()),
        Resource::Run { machine, app, folder } => c.field("run").field(machine.as_str()).field(app.as_str()).field(folder.as_str()),
        Resource::Session(s) => c.field("session").field(s.as_str()),
        Resource::Actor(a) => c.field("actor").field(a.as_str()),
    };
    c.field(act.action().as_str());
    match act {
        Act::Message | Act::Wake | Act::Claim | Act::Handoff | Act::UseGateway => &mut c,
        Act::Start { app, folder, mode } => c.field(app.as_str()).field(folder.as_str()).field(match mode {
            StartMode::Asks => "asks",
            StartMode::Delegated => "delegated",
        }),
        Act::Answer { ask, decision } => c.field(ask.as_str()).field(match decision {
            AnswerDecision::Allow => "allow",
            AnswerDecision::Deny => "deny",
        }),
        Act::Read { scope, filters } => c.field(scope.as_str()).bytes(filters.as_ref().map_or(&[][..], |f| &f[..])),
        Act::Admit { machine, role } => c.field(machine.as_str()).field(role.as_str()),
        Act::Widen { machine, settings } => c.field(machine.as_str()).field(settings.start.as_str()).field(settings.wake.as_str()),
        Act::Publish { service, spec } => c.field(service.as_str()).bytes(spec),
        Act::RegisterActor { slug, rights, scopes } => {
            c.field(slug.as_str());
            let mut rights = rights.clone();
            rights.sort();
            rights.dedup();
            for r in rights {
                c.field(r.as_str());
            }
            c.bytes(scopes)
        }
        Act::Revoke { what } => match what {
            Revocation::Token(id) => c.field("token").bytes(id),
            Revocation::Device(d) => c.field("device").field(d.as_str()),
        },
        Act::Delegate { project } => c.field(project.as_ref().map_or("", ProjectId::as_str)),
    };
    sha256(&c.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{StartSetting, WakeSetting};

    #[test]
    fn ids_are_short_words() {
        assert!(MachineId::new("laptop").is_ok());
        assert!(MachineId::new("xnk1_abc").is_ok());
        assert!(MachineId::new("").is_err());
        assert!(MachineId::new("two words").is_err());
        assert!(MachineId::new("quote\"d").is_err());
        assert!(MachineId::new("line\nbreak").is_err());
        assert!(MachineId::new(&"a".repeat(129)).is_err());
        assert!(Folder::new("~/Projects/my app").is_ok());
        assert!(Folder::new("a\nb").is_err());
        assert_eq!(Action::ALL.map(|a| Action::parse(a.as_str())), Action::ALL.map(Some));
    }

    #[test]
    fn every_field_of_an_act_changes_its_hash() {
        let m = Resource::Machine(MachineId::new("m1").unwrap());
        let run = |folder: &str| Resource::Run { machine: MachineId::new("m1").unwrap(), app: AppId::new("claude").unwrap(), folder: Folder::new(folder).unwrap() };
        let answer = |ask: &str, decision| Act::Answer { ask: AskId::new(ask).unwrap(), decision };
        let hashes = [
            act_hash(&m, &Act::Message),
            act_hash(&Resource::Machine(MachineId::new("m2").unwrap()), &Act::Message),
            act_hash(&m, &Act::Claim),
            act_hash(&run("/a"), &answer("q1", AnswerDecision::Allow)),
            act_hash(&run("/a"), &answer("q1", AnswerDecision::Deny)),
            act_hash(&run("/a"), &answer("q2", AnswerDecision::Allow)),
            act_hash(&run("/b"), &answer("q1", AnswerDecision::Allow)),
            act_hash(&m, &Act::Read { scope: ReadScope::Search, filters: None }),
            act_hash(&m, &Act::Read { scope: ReadScope::Search, filters: Some([0; 32]) }),
            act_hash(&m, &Act::Widen { machine: MachineId::new("m2").unwrap(), settings: Settings { start: StartSetting::Network, wake: WakeSetting::Consent } }),
            act_hash(&m, &Act::Widen { machine: MachineId::new("m2").unwrap(), settings: Settings { start: StartSetting::Network, wake: WakeSetting::Network } }),
            act_hash(&Resource::Actor(ActorSlug::new("scout").unwrap()), &Act::Delegate { project: None }),
            act_hash(&Resource::Actor(ActorSlug::new("scout").unwrap()), &Act::Delegate { project: Some(ProjectId::new("atlas").unwrap()) }),
            act_hash(&Resource::Actor(ActorSlug::new("wren").unwrap()), &Act::Delegate { project: None }),
        ];
        let distinct: std::collections::HashSet<_> = hashes.iter().collect();
        assert_eq!(distinct.len(), hashes.len());
    }
}
