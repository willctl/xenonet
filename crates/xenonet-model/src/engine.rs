//! The engine's view of the sessions on this machine: an agent at work,
//! its claims, when it was last in touch, and the messages between agents.

use crate::num::{Ms, Uint};
use crate::schema::{Document, Nullable, Opt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An agent: claude:<session id>, codex:<thread id> or grok:<session id>, or an actor, actor:<slug>; another machine's, by the agt_ its machine made
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AgentId(pub String);

// Which app runs an agent, or that it's an actor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum App {
    Claude,
    Codex,
    Grok,
    Actor,
}

// ---- engine/agent/v1

// Whether an agent is at work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Working,
    Idle,
    Ended,
}

// What an agent waits on the person for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NeedsYou {
    Question,
    Permission,
}

// How an agent's records are read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Reader {
    Experimental,
}

// A file an agent edited lately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Edit {
    pub path: String,
    pub at: Ms,
    /// An actor's edit XenoNet inferred from what it observes; the actor didn't report it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub observed: Opt<bool>,
}

// What an agent is editing: the files, or how many.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Editing {
    Files(Vec<Edit>),
    Count(Uint),
}

/// A claim, as an agent's claims list it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaimBrief {
    /// An absolute path, a folder (ending in /) or a glob
    pub pattern: String,
    pub note: Nullable<String>,
    pub until: Ms,
}

/// An agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub id: AgentId,
    pub app: App,
    #[schemars(length(max = 90))]
    pub title: String,
    pub project: Nullable<String>,
    pub branch: Nullable<String>,
    /// The map node it lives in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Its working tree's top
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    /// Its repo's shared git folder, the same for every worktree
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub status: Opt<Status>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub ended: Opt<bool>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub needs_you: Opt<Nullable<NeedsYou>>,
    /// Read from its own records by an experimental reader (core/native), so no message reaches it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reader: Opt<Nullable<Reader>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub last_active: Opt<Ms>,
    /// What it says it's doing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<String>,
    /// Files it edited in the last half hour; the console's snapshot sends how many
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub editing: Opt<Editing>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub claims: Opt<Vec<ClaimBrief>>,
    /// In touch directly lately, so messages reach it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reachable: Opt<bool>,
    /// From peers: files you've both edited lately
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub overlap: Opt<Vec<String>>,
    /// From send: when it will see the message
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub delivery: Opt<String>,
    /// In a XenoNet network, the machine an agent on another machine is on; its id is its agt_
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// For an agent on another host: when that host was last heard from
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub seen: Opt<Nullable<Ms>>,
    /// For an agent on another host: whether its host's link is up; offline since seen when it isn't
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub online: Opt<bool>,
    /// For an agent on another host: the project its folder is a checkout of (prj_…), which its checkouts on every machine share
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_key: Option<String>,
    /// For an agent on another host: a question or a stuck state waits on the person
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub waits_on_you: Opt<bool>,
    /// For an agent on another host, in the console's snapshot only: the text of what waits on the person, as its host's presence carries it; null when it shares titles only
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<String>,
    /// For an agent on another host, in the console's snapshot only: what it said last, at most 280 characters
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<String>,
    /// For an agent on another host, in the console's snapshot only: why it seems stuck
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Vec<String>>,
}

impl Document for Agent {
    const ID: &'static str = "engine/agent/v1";
    const TITLE: &'static str = "An agent";
    const DESCRIPTION: &'static str = "A session at work, as the engine sees it. The brief form (id, title, app, project, branch) names an agent in messages and answers; the agents list and peers send the whole view; the console's snapshot sends the brief form with status, focus, claims, reachability and how many files it's editing.";
}

// ---- engine/claim/v1

/// As engine.sqlite keeps it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaimRow {
    pub id: Uint,
    pub agent: AgentId,
    pub pattern: String,
    pub note: Nullable<String>,
    pub created_at: Ms,
    pub expires_at: Ms,
}

/// A claim.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Claim {
    Brief(ClaimBrief),
    Row(ClaimRow),
}

impl Document for Claim {
    const ID: &'static str = "engine/claim/v1";
    const TITLE: &'static str = "A claim";
    const DESCRIPTION: &'static str = "A file, folder or glob an agent has said it's working on, until a time.";
}

// ---- engine/contact/v1

// How an agent was last in touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Hook,
    Mcp,
}

/// A contact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Contact {
    pub agent: AgentId,
    pub app: App,
    pub cwd: Nullable<String>,
    pub via: Via,
    pub at: Ms,
    /// When its SessionEnd hook said it ended
    pub ended_at: Nullable<Ms>,
    /// Its process, to tell when it's gone
    pub pid: Nullable<Uint>,
}

impl Document for Contact {
    const ID: &'static str = "engine/contact/v1";
    const TITLE: &'static str = "A contact";
    const DESCRIPTION: &'static str =
        "When XenoNet last heard from an agent directly, through a hook or a tool call, as engine.sqlite keeps it: how it knows messages reach the agent.";
}

// ---- engine/message/v1

// How XenoNet knew who sent a message, as the snapshot carries it: a
// known way, or one this XenoNet doesn't name.
crate::open!(How of KnownHow {
    Session = "session",
    Token = "token",
    Window = "window",
    Grant = "grant",
    Gateway = "gateway",
});

// One recipient of a message in the snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Recipient {
    pub id: AgentId,
    pub seen: bool,
    /// For an agent on another host: its host
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// For an agent on another host: what its host last said of it (stored, held, delivered, seen or refused), or unread when it was stored there and no one read it in a week; null before it said
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<String>,
}

/// In the console's snapshot, GET /api/engine/state
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotMessage {
    pub id: Uint,
    pub from: AgentId,
    pub target: String,
    pub text: String,
    pub at: Ms,
    /// How XenoNet knows who sent it (its envelope's method); null for one sent before XenoNet kept that
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub how: Option<How>,
    pub to: Vec<Recipient>,
}

// The brief form of the agent that sent a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sender {
    pub id: AgentId,
    pub title: String,
    pub app: App,
    pub project: Nullable<String>,
    pub branch: Nullable<String>,
}

crate::tag!(pub EnvelopeTag = "message/envelope/v1");

// Who a sender is: a session, an actor, or the person, or a kind this
// XenoNet doesn't name.
crate::open!(SenderKind of KnownSenderKind { Agent = "agent", Actor = "actor", You = "you" });

// The sender, as the envelope names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EnvelopeSender {
    pub id: String,
    pub kind: SenderKind,
    pub name: Nullable<String>,
    pub product: Nullable<String>,
}

// How a sender proved who it is: a known method, or one this XenoNet
// doesn't name.
crate::open!(AuthMethod of KnownAuthMethod {
    Session = "session",
    Token = "token",
    Window = "window",
    Grant = "grant",
    Gateway = "gateway",
    Host = "host",
});

// What XenoNet saw of the sending program: a known check, or one this
// XenoNet doesn't name.
crate::open!(ProcessCheck of KnownProcessCheck {
    Signed = "signed",
    Checked = "checked",
    Unchecked = "unchecked",
    NotApplicable = "n/a",
});

// How the machine that vouches for an agent checked it (xenonet-authz's
// Attestation), or n/a for the person and for what isn't an agent's.
crate::open!(Attestation of KnownAttestation {
    Unchecked = "unchecked",
    ProcessChecked = "process-checked",
    HarnessSigned = "harness-signed",
    NotApplicable = "n/a",
});

// How the envelope's sender was checked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EnvelopeAuth {
    /// How it proved who it is; host: another host of the network signed the frame that brought it
    pub method: AuthMethod,
    /// What XenoNet saw of the program that sent it
    pub process: ProcessCheck,
    /// How the agent was checked, as policy reads it
    pub attestation: Attestation,
}

// The program a message came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Client {
    pub exe: String,
    pub name: String,
}

// What a message from another host rests on, as this one decided it:
// the person's device signature, a token the sending machine minted for
// its actor, or the sending machine's word.
crate::open!(OriginalAuthority of KnownOriginalAuthority { Person = "person", Token = "token", Machine = "machine" });

/// For transport network: who sent it and what this host checked of it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OriginalCheck {
    pub kind: String,
    pub authority: OriginalAuthority,
}

// How XenoNet knows who sent a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Envelope {
    pub schema: EnvelopeTag,
    pub mid: String,
    pub at: Ms,
    pub sender: EnvelopeSender,
    pub auth: EnvelopeAuth,
    /// The program it came from, with the home folder as ~, when XenoNet saw it
    pub client: Nullable<Client>,
    /// this machine, tunnel, gateway, or network: from another host of a XenoNet network
    pub transport: String,
    /// For transport network: the host it came from
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub original: Opt<OriginalCheck>,
    /// For transport network: the messages before it in its thread, across hosts
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub hops: Opt<Uint>,
    /// The same in a sentence: verified: …
    pub line: String,
}

/// In an agent's inbox
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InboxMessage {
    pub id: Uint,
    pub from: Sender,
    /// "you", or what it was sent to
    pub to: String,
    pub text: String,
    pub at: Ms,
    pub reply_to: Nullable<Uint>,
    /// How XenoNet knows who sent it; null for one sent before XenoNet kept that
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<Envelope>,
}

/// The agent-message event
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageEvent {
    pub id: Uint,
    pub from: AgentId,
    pub to: Vec<AgentId>,
    pub target: String,
    pub text: String,
    pub ts: Ms,
    /// How XenoNet knows who sent it, in a sentence
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub verified: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub attestation: Opt<Attestation>,
}

/// As engine.sqlite keeps it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageRow {
    pub id: Uint,
    pub sender: AgentId,
    pub target: String,
    /// For a message to a repo, the repo's git folder or project
    pub scope: Nullable<String>,
    pub body: String,
    pub reply_to: Nullable<Uint>,
    pub created_at: Ms,
    /// How XenoNet knew who sent it, as message/envelope/v1 JSON (core/engine/attest.js); null before it kept that
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope: Option<String>,
    /// The host of a XenoNet network it came from; null for one made on this machine
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Its id across the network (its envelope's mid), for one that went to or came from another host
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mid: Option<String>,
}

/// A message between agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Message {
    Snapshot(SnapshotMessage),
    // Boxed: the envelope makes an inbox message far larger than the others.
    Inbox(Box<InboxMessage>),
    Event(MessageEvent),
    Row(MessageRow),
}

impl Document for Message {
    const ID: &'static str = "engine/message/v1";
    const TITLE: &'static str = "A message between agents";
    const DESCRIPTION: &'static str = "A message from one agent to another, to everyone in a repo (target repo:<name>) or to all (\"all\"), at most 4,000 characters.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Agent>, crate::schema::file::<Claim>, crate::schema::file::<Contact>, crate::schema::file::<Message>];
