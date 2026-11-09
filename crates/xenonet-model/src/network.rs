//! The network of hosts: what crosses the links between machines (frames
//! and what they carry), the roster that says who's in, and the network
//! as a host sees it.

use crate::ids;
use crate::num::{Int, Ms, Uint};
use crate::schema::{Any, Document, Nullable, Opt, Record, Text};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The host's id in the network, picked by the person: never the machine's own name
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HostId(pub ids::HostId);

/// What people compare: xnk1_ and the base32 of the first 20 bytes of the SHA-256 of both public keys
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct KeyId(#[schemars(regex(pattern = r"^xnk1_[a-z2-7]{32}$"))] pub String);

// A network's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct NetworkId(#[schemars(regex(pattern = r"^net_[0-9a-f]{32}$"))] pub String);

/// The message's id across the network (its envelope's mid)
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Mid(#[schemars(regex(pattern = r"^msg_[0-9a-f-]{36}$"))] pub String);

/// base64url, no padding
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Base64Url(#[schemars(regex(pattern = r"^[A-Za-z0-9_-]*$"))] pub String);

// base64url bytes whose field says what they are.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Base64Bytes(#[schemars(regex(pattern = r"^[A-Za-z0-9_-]*$"))] pub String);

// An agent's id on its host, as presence and claims carry it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AgentName(#[schemars(length(max = 200))] pub String);

// A path inside a repo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FilePath(#[schemars(length(max = 1000))] pub String);

// One line of what an agent said or why it seems stuck.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Line(#[schemars(length(max = 280))] pub String);

// Something a host's XenoNet takes beyond messages: start, answer…
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Capability(#[schemars(length(max = 32))] pub String);

/// strong: checked by the window key, or by process checks on macOS; weak: claimed, not checked
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Strong,
    Weak,
}

/// Who may start sessions there: nobody, you (from a window host's console), the network (you, and the sessions and actors on window hosts), or delegate (the network, and what a session or actor starts there runs on your standing delegation, within its limits)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StartReach {
    Off,
    You,
    Network,
    Delegate,
}

/// Who may wake its sessions for a message: your consent per session, or any sender in the network, within the budgets
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum WakeReach {
    Consent,
    Network,
}

// Which reach setting a widening is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReachSetting {
    Start,
    Wake,
}

// What a widening asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Widened {
    Network,
    Delegate,
}

// A widening another window host asked for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Widening {
    #[schemars(length(min = 1, max = 64))]
    pub id: String,
    pub setting: ReachSetting,
    pub value: Widened,
    /// The window host that asked
    pub by: HostId,
    pub at: Ms,
}

// Who put a machine on delegation, as its presence says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Delegated {
    /// The window host whose action put it on delegation: itself, for the person at its own keyboard
    pub by: HostId,
    pub at: Ms,
}

// Who may start and wake sessions on a host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Reach {
    pub start: StartReach,
    pub wake: WakeReach,
    /// Widenings to network (or, for a start, to delegate) another window host asked for, waiting ten minutes at most for the founder or a second window host to approve
    #[schemars(length(max = 4))]
    pub pending: Vec<Widening>,
    /// While start is delegate: who put the machine on it, and when
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub delegated: Opt<Delegated>,
}

// ---- network/claims/v1

crate::tag!(pub ClaimsTag = "network/claims/v1");

/// claim: the agent claimed it; edit: XenoNet saw it change
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClaimKind {
    Claim,
    Edit,
}

// One claim or edit on a host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostClaim {
    #[schemars(length(min = 1, max = 300))]
    pub repo_key: String,
    /// Inside the repo, with / between parts
    #[schemars(length(min = 1, max = 1000))]
    pub path: String,
    #[schemars(length(min = 1, max = 200))]
    pub agent: String,
    pub branch: Nullable<Text<0, 200>>,
    pub kind: ClaimKind,
    pub until: Nullable<Ms>,
}

// A claim an agent let go of.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReleasedClaim {
    #[schemars(length(max = 300))]
    pub repo_key: String,
    #[schemars(length(max = 1000))]
    pub path: String,
    #[schemars(length(max = 200))]
    pub agent: String,
}

/// Claims across clones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Claims {
    pub schema: ClaimsTag,
    pub full: bool,
    #[schemars(length(max = 5000))]
    pub claims: Vec<HostClaim>,
    #[schemars(length(max = 5000))]
    pub released: Vec<ReleasedClaim>,
}

impl Document for Claims {
    const ID: &'static str = "network/claims/v1";
    const TITLE: &'static str = "Claims across clones";
    const DESCRIPTION: &'static str = "What a host's agents claimed or are changing, by repo remote and path inside the repo, so agents on other hosts are warned.";
}

// ---- network/frame/v1

crate::constant!(
    /// The frame format's version: always 1.
    pub FrameVersion = 1
);

// A frame's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FrameId(#[schemars(regex(pattern = r"^frm_[0-9a-f-]{36}$"))] pub String);

// What a frame carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FrameKind {
    Hello,
    Ping,
    Gap,
    Roster,
    Removed,
    Presence,
    Claims,
    Message,
    Receipt,
    Handoff,
    Relay,
    Operator,
    Svc,
}

crate::tag!(pub Everyone = "*");

// A frame's recipient: one host, or every host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FrameTo {
    Host(HostId),
    All(Everyone),
}

/// A network frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    pub v: FrameVersion,
    pub id: FrameId,
    /// Set by the gateway, per recipient, within its epoch: what a host resumes after
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub seq: Opt<Uint>,
    pub kind: FrameKind,
    /// The host that sent it, whose key signed it
    pub from: HostId,
    /// The host it's for, or * for every host (presence and claims, sealed once per host)
    pub to: FrameTo,
    /// When the sender made it, by its clock
    pub at: f64,
    /// Whether body is sealed to the recipient's X25519 key; frames between hosts always are
    pub sealed: bool,
    /// The sealed bytes, or the JSON of an unsealed gateway frame
    pub body: Base64Bytes,
    /// Ed25519 over the head: xenonet-net/1, network id, id, kind, from, to, at, sealed (1 or 0) and the base64url SHA-256 of the body bytes, joined by newlines
    pub sig: Base64Bytes,
}

impl Document for Frame {
    const ID: &'static str = "network/frame/v1";
    const TITLE: &'static str = "A network frame";
    const DESCRIPTION: &'static str =
        "One line of a link's NDJSON: signed by the host that sent it and, between hosts, sealed for the host it's for. The gateway sees the head and never the body.";
}

// ---- network/handoff/v1

crate::tag!(pub HandoffTag = "network/handoff/v1");

// A git commit's hash, 7 to 64 hex digits.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CommitHash(#[schemars(regex(pattern = r"^[0-9a-f]{7,64}$"))] pub String);

// A handoff's id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HandoffId(#[schemars(regex(pattern = r"^hof_[0-9a-f-]{36}$"))] pub String);

/// offer: from the maker; accept: from a session that wants it; taken or lapsed: the maker's answer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HandoffOp {
    Offer,
    Accept,
    Taken,
    Lapsed,
}

// Who a handoff is for: any of an agent, a repo, a host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandoffTo {
    pub agent: Nullable<Text<0, 200>>,
    pub repo_key: Nullable<Text<0, 300>>,
    pub host: Nullable<HostId>,
}

/// A handoff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Handoff {
    pub schema: HandoffTag,
    pub id: HandoffId,
    pub op: HandoffOp,
    /// The maker, unqualified, on the frame's sender
    #[schemars(length(max = 200))]
    pub from: String,
    pub to: HandoffTo,
    pub goal: Nullable<Text<0, 2000>>,
    pub done: Nullable<Text<0, 8000>>,
    pub next: Nullable<Text<0, 8000>>,
    #[schemars(length(max = 200))]
    pub files: Vec<FilePath>,
    pub branch: Nullable<Text<0, 200>>,
    pub commit: Nullable<CommitHash>,
    /// For accept and taken: the session that took it, qualified
    pub by: Nullable<Text<0, 200>>,
    pub at: Ms,
}

impl Document for Handoff {
    const ID: &'static str = "network/handoff/v1";
    const TITLE: &'static str = "A handoff";
    const DESCRIPTION: &'static str = "Work one session leaves for another, perhaps on another machine; taken by the first that accepts it.";
}

// ---- network/message/v1

crate::tag!(pub MessageTag = "network/message/v1");

// Who sent a message: a session, an actor, or the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SenderKind {
    Agent,
    Actor,
    You,
}

/// How the sender's own host checked it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MessageAuth {
    #[schemars(length(max = 32))]
    pub method: String,
    pub strength: Strength,
}

// A message, or a handoff carried as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Message,
    Handoff,
}

/// A message between hosts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub schema: MessageTag,
    pub mid: Mid,
    /// The sender's id on its own host; the receiver qualifies it
    #[schemars(length(min = 1, max = 200))]
    pub sender: String,
    pub sender_kind: SenderKind,
    pub auth: MessageAuth,
    /// The recipient's id on the receiving host, unqualified
    #[schemars(length(min = 1, max = 200))]
    pub target: String,
    #[schemars(length(min = 1, max = 16000))]
    pub body: String,
    pub reply_to: Nullable<Mid>,
    /// direct, repo or all: how the sender addressed it
    pub scope: Nullable<Text<0, 64>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub kind: Opt<MessageKind>,
    /// Messages in its thread so far, across hosts, for the hop limit
    pub hops: Uint,
    pub at: Ms,
}

impl Document for Message {
    const ID: &'static str = "network/message/v1";
    const TITLE: &'static str = "A message between hosts";
    const DESCRIPTION: &'static str = "One message from an agent, actor or the person on one host to an agent or actor on another.";
}

// ---- network/operator/v1

crate::tag!(pub OperatorTag = "network/operator/v1");

// What the person did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OperatorAction {
    Message,
    Ticket,
    Remove,
    Replace,
    Settings,
    Answer,
}

/// window: the window key; person: a terminal, by process checks
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CheckedBy {
    Window,
    Person,
}

/// An action for the person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Operator {
    pub schema: OperatorTag,
    pub action: OperatorAction,
    /// For settings: { start?, wake? } or { approve } with a waiting widening's id. For answer: { ask, decision }, decision allow or deny
    pub args: Record<Any>,
    pub checked: CheckedBy,
    pub strength: Strength,
    pub nonce: Base64Url,
    pub at: Ms,
}

impl Document for Operator {
    const ID: &'static str = "network/operator/v1";
    const TITLE: &'static str = "An action for the person";
    const DESCRIPTION: &'static str = "Something the person did in a console on one host, for another host to carry out.";
}

// ---- network/presence/v1

crate::tag!(pub PresenceTag = "network/presence/v1");

// One agent on a host, as its presence tells it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresenceAgent {
    /// The agent's id on its own host (claude:<uuid>, codex:<id>, actor:<slug>): receivers qualify it as @<host>:<id>
    #[schemars(length(min = 1, max = 200))]
    pub id: String,
    pub app: Nullable<Text<1, 32>>,
    pub title: Nullable<Text<0, 200>>,
    /// What the live state says: working, waiting, idle…
    pub state: Nullable<Text<0, 32>>,
    /// What it said it's doing (focus)
    pub doing: Nullable<Text<0, 280>>,
    /// The normalized git remote of its folder (origin, else the first remote), or null when it has none
    pub repo_key: Nullable<Text<0, 300>>,
    pub branch: Nullable<Text<0, 200>>,
    /// Its folder, with the home folder as ~
    pub cwd: Nullable<Text<0, 500>>,
    pub last_active: Nullable<Ms>,
    /// A question or a stuck state waits on the person
    pub waits_on_you: bool,
    /// While it waits on the person: the text of its question, or of the permission it asks for, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 320))]
    pub question: Option<String>,
    /// What it said last, its last 280 characters, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 280))]
    pub last_message: Option<String>,
    /// Why it seems stuck, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 8))]
    pub stuck: Option<Vec<Line>>,
}

/// A host's presence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub schema: PresenceTag,
    /// Grows with every presence a host sends; a receiver drops one older than the newest it has
    pub counter: Uint,
    /// true: every agent on the host; false: only the agents listed changed, and removed went
    pub full: bool,
    #[schemars(length(max = 1000))]
    pub agents: Vec<PresenceAgent>,
    /// Agents that went since the last presence
    #[schemars(length(max = 1000))]
    pub removed: Vec<AgentName>,
    /// The person's last input was in this host's console: they're here
    pub you: bool,
    /// When the person last worked at this host's console (input in the last two minutes), to the half minute, by the gateway's clock: the consoles chime only where it's latest, or everywhere once it's five minutes old
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub you_at: Opt<Nullable<Ms>>,
    /// The hash of the sender's roster head, so hosts that see different rosters notice
    pub roster: Base64Bytes,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reach: Opt<Reach>,
    /// What the host's XenoNet takes beyond messages: start (a start request) and answer (an answer to a run's permission prompt). An older XenoNet says nothing
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 16))]
    pub can: Opt<Vec<Capability>>,
}

impl Document for Presence {
    const ID: &'static str = "network/presence/v1";
    const TITLE: &'static str = "A host's presence";
    const DESCRIPTION: &'static str = "Which agents are on a host, what each is doing, and whether the person is there.";
}

// ---- network/receipt/v1

crate::tag!(pub ReceiptTag = "network/receipt/v1");

/// stored: the receiving engine has it (the sender may drop it from its outbox); held: it waits because the session bypasses permission checks. For a start request: started, refused, then ended
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptState {
    Stored,
    Delivered,
    Seen,
    Held,
    Refused,
    Started,
    Ended,
}

// Which app a started run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RunApp {
    Claude,
    Codex,
    Grok,
}

/// A receipt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub schema: ReceiptTag,
    pub mid: Mid,
    /// The recipient, unqualified: xenonet:start for a start request
    #[schemars(length(min = 1, max = 200))]
    pub agent: String,
    pub state: ReceiptState,
    pub reason: Nullable<Text<0, 280>>,
    pub at: Ms,
    /// For started and ended: the new session's id (null for Codex until it names its thread)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(max = 200))]
    pub session: Option<String>,
    /// For started: its process
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pid: Opt<Uint>,
    /// For started: its folder, with the home folder as ~
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 1000))]
    pub folder: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub app: Opt<RunApp>,
    /// For ended: its exit code, null when it was stopped
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<Int>,
    /// For ended: how long it ran
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub minutes: Opt<Uint>,
    /// For ended: Claude Code there needs a sign-in for XenoNet's runs
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sign_in: Opt<bool>,
}

impl Document for Receipt {
    const ID: &'static str = "network/receipt/v1";
    const TITLE: &'static str = "A receipt";
    const DESCRIPTION: &'static str = "What happened to a message on the host it went to.";
}

// ---- network/relay/v1

crate::tag!(pub RelayTag = "network/relay/v1");

/// The subscription, made at random by the subscribing host
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SubscriptionId(#[schemars(regex(pattern = r"^sub_[0-9a-f]{32}$"))] pub String);

/// From the subscribing host. open: start, with a hello and a snapshot; get: one read from the relay's allowlist, as GET; time: the host's clock; renew: still watching (every 20 s; a subscription lapses after 60 s without one), with fresh for a new snapshot after a gap; close: stop
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RelayOp {
    Open,
    Get,
    Time,
    Renew,
    Close,
}

// What the subscribing host asks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelayAsk {
    pub schema: RelayTag,
    pub op: RelayOp,
    pub sub: SubscriptionId,
    /// For get and time: the console's question, which the answer's res or time frame names
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 1))]
    pub id: Opt<Uint>,
    /// For get: the path and query to read
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 1, max = 2048))]
    pub path: Opt<String>,
    /// For renew: send a fresh snapshot, since frames were lost
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub fresh: Opt<bool>,
}

// One of the SSH relay's frames, by its kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelayFrame {
    #[schemars(length(min = 1, max = 16))]
    pub t: String,
}

// What the watched host answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RelayAnswer {
    pub schema: RelayTag,
    pub sub: SubscriptionId,
    /// 1, 2, 3… within the subscription: a hole means frames were lost, and the console asks for a fresh snapshot
    #[schemars(range(min = 1))]
    pub seq: Uint,
    /// The relay's frames, in order
    #[schemars(length(max = 1000))]
    pub frames: Vec<RelayFrame>,
}

/// The map between hosts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Relay {
    Ask(RelayAsk),
    Answer(RelayAnswer),
}

impl Document for Relay {
    const ID: &'static str = "network/relay/v1";
    const TITLE: &'static str = "The map between hosts";
    const DESCRIPTION: &'static str = "A console's subscription to another host's map, and that host's answers: the SSH relay's frames, sealed between the two hosts.";
}

// ---- network/roster-entry/v1

// What a roster entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RosterOp {
    Found,
    Add,
    Remove,
    Window,
    Gateway,
}

// The host an entry is about, with its keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RosterHost {
    pub id: HostId,
    /// Ed25519 public key
    pub sign: Base64Bytes,
    /// X25519 public key
    pub seal: Base64Bytes,
    pub key_id: KeyId,
    /// A XenoNet window the person uses: this host may act for them
    pub window: bool,
}

// Where the gateway is, for op gateway.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RosterGateway {
    #[schemars(length(max = 300))]
    pub address: String,
    pub version: Uint,
}

// A host's signature on an entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Signer {
    pub host: HostId,
    pub sig: Base64Url,
}

/// A roster entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RosterEntry {
    pub n: Uint,
    /// base64url SHA-256 of the previous entry's canonical bytes; empty for entry 0
    pub prev: Base64Bytes,
    pub network: NetworkId,
    pub op: RosterOp,
    pub host: RosterHost,
    /// For op gateway: where the network's gateway is now
    pub gateway: Nullable<RosterGateway>,
    pub at: Ms,
    #[schemars(length(min = 1, max = 4))]
    pub signers: Vec<Signer>,
}

impl Document for RosterEntry {
    const ID: &'static str = "network/roster-entry/v1";
    const TITLE: &'static str = "A roster entry";
    const DESCRIPTION: &'static str = "One link in a network's chain of hosts: who joined, who left, who's the gateway, and which hosts signed it.";
}

// ---- network/state/v1

crate::tag!(pub StateTag = "network/state/v1");

// The network this machine is in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkInfo {
    pub id: NetworkId,
    #[schemars(length(max = 64))]
    pub name: String,
}

// This machine as a host of the network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SelfHost {
    pub id: HostId,
    pub key_id: KeyId,
    pub window: bool,
    pub founder: bool,
    pub gateway: bool,
}

// How the gateway is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum GatewayKind {
    Tunnel,
    Lan,
    Loopback,
}

// The state of this machine's link to the gateway.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Off,
    Connecting,
    Live,
    Offline,
    Refused,
    Removed,
}

// The network's gateway and this machine's link to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Gateway {
    #[schemars(length(max = 300))]
    pub address: String,
    pub key_id: KeyId,
    pub kind: GatewayKind,
    pub link: LinkState,
    pub since: Nullable<Ms>,
    pub reason: Nullable<Text<0, 280>>,
    pub skew_ms: Nullable<Int>,
}

// A host of the network, as this machine sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostRow {
    pub id: HostId,
    pub key_id: KeyId,
    pub window: bool,
    pub founder: bool,
    pub gateway: bool,
    pub online: bool,
    pub seen: Nullable<Ms>,
    pub agents: Uint,
    pub waits_on_you: Uint,
    pub you: bool,
    /// When the person last worked at its console, by this machine's clock
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub you_at: Opt<Nullable<Ms>>,
    pub platform: Nullable<Text<0, 32>>,
    /// This machine's own row
    #[serde(rename = "self", default, skip_serializing_if = "Opt::is_none")]
    pub is_self: Opt<bool>,
    /// Who may start and wake sessions there, as its presence said; null when its XenoNet doesn't say
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<Reach>,
    /// What the host's XenoNet takes beyond messages: start (a start request) and answer (an answer to a run's permission prompt). An older XenoNet says nothing
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 16))]
    pub can: Opt<Vec<Capability>>,
}

// A ticket that waits to be used.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ticket {
    pub host: HostId,
    pub window: bool,
    pub expires: Ms,
}

// What waits to be sent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outbox {
    pub frames: Uint,
    pub bytes: Uint,
    pub oldest: Nullable<Ms>,
}

// Who put this machine on delegation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Delegation {
    pub by: HostId,
    pub at: Ms,
}

// What waits at the gateway for one host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Queue {
    pub host: HostId,
    pub frames: Uint,
    pub bytes: Uint,
}

// A roster entry waiting for more signatures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Proposal {
    /// The entry's id, the hash of what it says apart from its signers: what POST /api/net/approve takes
    pub id: Base64Bytes,
    pub op: RosterOp,
    pub host: HostId,
    pub window: bool,
    /// For op gateway: the address it moves to
    pub gateway: Nullable<Text<0, 300>>,
    /// The hosts that signed it so far
    pub signers: Vec<HostId>,
    pub at: Ms,
}

/// This machine was removed from the network
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Removal {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub network: Opt<NetworkId>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<HostId>,
    /// The hosts that signed the removal
    pub by: Vec<HostId>,
    pub at: Ms,
    /// The removal's place in the roster
    pub n: Uint,
}

/// The network, as a host sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub schema: StateTag,
    /// null: this machine isn't in a network
    pub network: Nullable<NetworkInfo>,
    #[serde(rename = "self")]
    pub this_host: Nullable<SelfHost>,
    pub gateway: Nullable<Gateway>,
    pub hosts: Vec<HostRow>,
    /// Tickets that wait; the ticket itself is shown once, where it was made
    pub tickets: Vec<Ticket>,
    pub outbox: Outbox,
    pub roster_head: Nullable<Base64Url>,
    /// Hosts whose presence shows another roster head
    pub roster_mismatch: Vec<HostId>,
    pub titles_only: bool,
    /// Whether this machine sends its load (CPU, GPU, memory and what the parts are) to the window hosts' consoles that watch its map; on unless turned off
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub share_load: Opt<bool>,
    /// Who may start sessions on this machine; you unless widened
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub start: Opt<StartReach>,
    /// While start is delegate: the window host whose action put this machine on it, and when
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated: Option<Delegation>,
    /// Who may wake this machine's sessions for a message; consent unless widened
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub wake: Opt<WakeReach>,
    /// On the gateway's host: what waits at the gateway for each host
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub queues: Opt<Vec<Queue>>,
    /// On the gateway's host: its queues' epoch, which hosts resume within
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub epoch: Opt<Base64Bytes>,
    /// On the gateway's host: when the join route opens again, after 200 bad tickets in an hour; null while it's open
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub join_closed_until: Opt<Nullable<Ms>>,
    /// Roster entries signed by too few, waiting for the founder or a second window host to approve them
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub proposals: Opt<Vec<Proposal>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub removed: Opt<Removal>,
    /// The LAN address this machine's gateway listens on, when it's a LAN-only gateway
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 300))]
    pub lan: Opt<String>,
    /// XenoNet Dev's made-up network (simulate net-…)
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub simulated: Opt<bool>,
}

impl Document for State {
    const ID: &'static str = "network/state/v1";
    const TITLE: &'static str = "The network, as a host sees it";
    const DESCRIPTION: &'static str = "GET /api/net/state: the network this machine is in, its gateway and link, the other hosts, and what waits.";
}

// ---- network/svc/v1

crate::tag!(pub SvcTag = "network/svc/v1");
crate::tag!(pub SvcRequestOp = "request");
crate::tag!(pub SvcAnswerOp = "answer");

/// The request, made at random by the calling host; its answer names it
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SvcId(#[schemars(regex(pattern = r"^svq_[0-9a-f]{32}$"))] pub String);

// A service's name, as its host published it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ServiceName(#[schemars(regex(pattern = r"^[a-z][a-z0-9-]{1,31}$"))] pub String);

// An HTTP method.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HttpMethod(#[schemars(regex(pattern = r"^[A-Z]{1,16}$"))] pub String);

// One HTTP header, a name and a value, as a two-element array.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Header(pub (Text<1, 64>, Text<0, 8192>));

impl JsonSchema for Header {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Header")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let name = g.subschema_for::<Text<1, 64>>();
        let value = g.subschema_for::<Text<0, 8192>>();
        json_schema!({ "type": "array", "prefixItems": [name, value], "items": false, "minItems": 2, "maxItems": 2 })
    }
}

// Why XenoNet answered rather than the service.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct SvcCode(#[schemars(regex(pattern = r"^[a-z-]{1,32}$"))] pub String);

// A request to a published service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SvcRequest {
    pub schema: SvcTag,
    /// From the calling host
    pub op: SvcRequestOp,
    pub id: SvcId,
    /// The service, as its host published it
    pub name: ServiceName,
    /// A host passes on GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS
    pub method: HttpMethod,
    /// The path after /api/svc/<host>/<name>, with its query: the publishing host normalizes it and checks it against the paths it publishes
    #[schemars(length(max = 4096))]
    pub path: String,
    /// Only the caller's Authorization, Content-Type, Accept and User-Agent cross; the publishing host adds Host and Forwarded itself
    #[schemars(length(max = 64))]
    pub headers: Vec<Header>,
    /// The request's body, at most 1 MiB
    pub body: Base64Bytes,
}

// The service's answer, or XenoNet's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SvcAnswer {
    pub schema: SvcTag,
    /// From the publishing host
    pub op: SvcAnswerOp,
    pub id: SvcId,
    #[schemars(range(min = 200, max = 599))]
    pub status: Uint,
    /// The service's answer's headers, without Set-Cookie and hop-by-hop headers
    #[schemars(length(max = 64))]
    pub headers: Vec<Header>,
    /// The answer's body, at most 4 MiB
    pub body: Base64Bytes,
    /// Set when XenoNet answered rather than the service: not-published, not-allowed, rate, path-refused, too-big, timeout…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub code: Opt<SvcCode>,
}

/// A service between hosts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Svc {
    Request(SvcRequest),
    Answer(SvcAnswer),
}

impl Document for Svc {
    const ID: &'static str = "network/svc/v1";
    const TITLE: &'static str = "A service between hosts";
    const DESCRIPTION: &'static str = "A request to a service another host published to the network, and its answer, sealed between the two hosts.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[
    crate::schema::file::<Claims>,
    crate::schema::file::<Frame>,
    crate::schema::file::<Handoff>,
    crate::schema::file::<Message>,
    crate::schema::file::<Operator>,
    crate::schema::file::<Presence>,
    crate::schema::file::<Receipt>,
    crate::schema::file::<Relay>,
    crate::schema::file::<RosterEntry>,
    crate::schema::file::<State>,
    crate::schema::file::<Svc>,
];
