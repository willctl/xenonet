//! The network of hosts: what crosses the links between machines (frames
//! and what they carry), the roster that says who's in, and the network
//! as a host sees it.

use crate::ids;
use crate::num::{Int, Ms, Ranged, Uint};
use crate::schema::{Bounded, Document, Nullable, Opt, Text};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The host's id in the network, picked by the person: never the machine's own name
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HostId(pub ids::HostId);

crate::pattern! {
    /// What people compare: xnk1_ and the base32 of the first 20 bytes of the SHA-256 of both public keys
    pub struct KeyId(r"^xnk1_[a-z2-7]{32}$");
}

crate::pattern! {
    // A network's id.
    pub struct NetworkId(r"^net_[0-9a-f]{32}$");
}

crate::pattern! {
    /// The message's id across the network (its envelope's mid)
    pub struct Mid(r"^msg_[0-9a-f-]{36}$");
}

crate::pattern! {
    /// base64url, no padding
    pub struct Base64Url(r"^[A-Za-z0-9_-]*$");
}

crate::pattern! {
    // base64url bytes whose field says what they are.
    pub struct Base64Bytes(r"^[A-Za-z0-9_-]*$");
}

// An agent's id on its host, as presence and claims carry it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct AgentName(pub Text<0, 200>);

// A path inside a repo.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct FilePath(pub Text<0, 1000>);

// One line of what an agent said or why it seems stuck.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Line(pub Text<0, 280>);

// Something a host's XenoNet takes beyond messages: start, answer…
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Capability(pub Text<0, 32>);

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
    pub id: Text<1, 64>,
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
    pub pending: Bounded<Widening, 0, 4>,
    /// While start is delegate: who put the machine on it, and when
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub delegated: Opt<Delegated>,
}

// ---- network/claim/v1

crate::tag!(pub ClaimTag = "network/claim/v1");

/// claim: an agent claimed the path; release: it let it go (with no path,
/// all its claims in the project)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ClaimOp {
    Claim,
    Release,
}

/// A claim in a project, or its release: a kept entry of its author's
/// feed, which goes to every member. The agent is the author's own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Claim {
    pub schema: ClaimTag,
    pub op: ClaimOp,
    pub project: ids::ProjectId,
    /// Inside the project, with / between parts: a file, a folder (ending
    /// in /) or a glob; empty in a release of all the agent's claims there
    pub path: Text<0, 1000>,
    pub agent: Text<1, 200>,
    pub branch: Nullable<Text<0, 200>>,
    pub note: Nullable<Text<0, 500>>,
    pub created_at: Nullable<Ms>,
    pub expires_at: Nullable<Ms>,
}

impl Document for Claim {
    const ID: &'static str = "network/claim/v1";
    const TITLE: &'static str = "A claim in a project";
    const DESCRIPTION: &'static str =
        "An agent's claim on a path inside a project, or its release, as a kept entry of its machine's feed, so agents on every machine with a checkout of the project are warned.";
}

// ---- network/handoff/v1

crate::tag!(pub HandoffTag = "network/handoff/v1");

crate::pattern! {
    // A git commit's hash, 7 to 64 hex digits.
    pub struct CommitHash(r"^[0-9a-f]{7,64}$");
}

crate::pattern! {
    // A handoff's id.
    pub struct HandoffId(r"^hof_[0-9a-f-]{36}$");
}

/// offer: from the maker; accept: from a session that wants it; taken or lapsed: the maker's answer
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum HandoffOp {
    Offer,
    Accept,
    Taken,
    Lapsed,
}

// Who a handoff is for: any of an agent, a project (its checkouts on
// every machine), a host.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HandoffTo {
    pub agent: Nullable<Text<0, 200>>,
    pub project_key: Nullable<Text<0, 300>>,
    pub host: Nullable<HostId>,
}

/// A handoff.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Handoff {
    pub schema: HandoffTag,
    pub id: HandoffId,
    pub op: HandoffOp,
    /// The maker, unqualified, on the entry's author's machine
    pub from: Text<0, 200>,
    pub to: HandoffTo,
    /// The one machine it's meant for: a handoff is a kept entry, which every member gets, and the others note it and pass it by. None: every member
    #[serde(default, rename = "for", skip_serializing_if = "Opt::is_none")]
    pub for_host: Opt<HostId>,
    pub goal: Nullable<Text<0, 2000>>,
    pub done: Nullable<Text<0, 8000>>,
    pub next: Nullable<Text<0, 8000>>,
    pub files: Bounded<FilePath, 0, 200>,
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

// ---- network/send/v1, network/message/v3

crate::tag!(pub SendTag = "network/send/v1");
crate::tag!(pub MessageTag = "network/message/v3");

// Who sent a message: a session, an actor, or the person.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SenderKind {
    Agent,
    Actor,
    Person,
}

crate::pattern! {
    /// A person's id: prs_ and 32 base32 characters
    pub struct PersonId(r"^prs_[a-z2-7]{32}$");
}

crate::pattern! {
    /// A signing key's id: xns1_ and 32 base32 characters
    pub struct SigningKeyId(r"^xns1_[a-z2-7]{32}$");
}

/// The person, by one of their devices: its signature over the act, or, for an act that takes a presence block, that block alone, which the device signs over the act's hash too
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PersonAuthority {
    pub person: PersonId,
    /// The device that signed, by its key's id
    pub device: SigningKeyId,
    /// The device's signature over the act's hash (xenonet-authz's act_hash), base64url; left out when the presence block stands for it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<Base64Bytes>,
}

// A presence block: the device's signed word that the person was there,
// for one act.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PresenceProof {
    pub device: SigningKeyId,
    pub what: PresenceWhat,
    /// The SHA-256 of the act, base64url
    pub act: Base64Bytes,
    pub nonce: Base64Bytes,
    pub at: Ms,
    pub sig: Base64Bytes,
}

/// What a presence block approves: an Allow (answer), admitting a machine, widening a network's settings, publishing a service, a revocation, pairing a device, a delegation, or issuing or widening an actor's grant
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PresenceWhat {
    Answer,
    Admit,
    Widen,
    Publish,
    Revoke,
    Pair,
    Delegate,
    Grant,
}

// A token root's certificate: the machine signing key's word that this root speaks for the machine, for a while.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RootCertificate {
    pub v: Uint,
    /// The machine, by its host id
    pub machine: Text<1, 128>,
    /// The root's Ed25519 key
    pub root: PublicKey,
    pub from: Ms,
    pub until: Ms,
    /// The machine signing key's signature over it
    pub sig: Base64Bytes,
}

/// What an act rests on, for the receiver to check against its own chains: the sender's word on its own strength is never part of it
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Authority {
    /// For an act by an actor: a token its machine minted for this act (xenonet_actor_ and a Biscuit, base64url)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<Text<1, 16384>>,
    /// The certificate of the token's root, which the receiver checks against the sending machine's signing key in the network's chain
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<RootCertificate>,
    /// For an act as the person: their device, and its signature over the act unless the presence block stands for it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<PersonAuthority>,
    /// For an act that needs the person there (an answer's Allow, an admit, a widening): a presence block for it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceProof>,
    /// The head of the person's chain the sender held: a receiver with an older one asks for the rest
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_head: Option<Base64Bytes>,
}

// A message, or a handoff carried as one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MessageKind {
    Message,
    Handoff,
}

/// A message between hosts: the body of a message entry in its sender's machine's feed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SendBody {
    pub schema: SendTag,
    pub mid: Mid,
    /// The sender's id on its own host; the receiver qualifies it
    pub sender: Text<1, 200>,
    pub sender_kind: SenderKind,
    /// The recipient's id on the receiving host, unqualified
    pub target: Text<1, 200>,
    pub body: Text<1, 16000>,
    pub reply_to: Nullable<Mid>,
    /// direct, repo or all: how the sender addressed it
    pub scope: Nullable<Text<0, 64>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub kind: Opt<MessageKind>,
    /// Messages in its thread so far, across hosts, for the hop limit
    pub hops: Uint,
    pub at: Ms,
}

impl Document for SendBody {
    const ID: &'static str = "network/send/v1";
    const TITLE: &'static str = "A message between hosts";
    const DESCRIPTION: &'static str = "One message from an agent, actor or the person on one host to an agent or actor on another: the body of a message entry in its machine's feed, whose frame carries the authority the act rests on.";
}

crate::pattern! {
    /// What a feed is scoped to: a network's net_ id or a space's spc_ id
    pub struct GroupId(r"^(net_[0-9a-f]{32}|spc_[a-z2-7]{32})$");
}

/// What a feed entry is about: claims, intents, handoffs, assigns, reports and compact entries are kept, in one gapless sequence per author and group; messages and receipts are loose, each taken once by its id
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Claim,
    Intent,
    Handoff,
    Assign,
    Report,
    Message,
    Receipt,
    Compact,
}

// A place in a feed: an incarnation and a number in it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedPlace {
    pub inc: Uint,
    pub seq: Uint,
}

// The kept entry before another: its place, and its id (the SHA-256 of what its author signed), none at 0, 0.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedLink {
    pub inc: Uint,
    pub seq: Uint,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub id: Opt<Base64Bytes>,
}

/// One entry of a machine's feed in a group, a network or a space (xenonet-feed/1), signed by its machine signing key
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedEntry {
    /// The author: its machine signing key, which the group's chain lists for a member
    pub author: SigningKeyId,
    pub group: GroupId,
    /// The author's incarnation: it changes only when the author's store was restored or it was set up again
    pub inc: Ranged<1>,
    /// From 1 in each incarnation, kept and loose kinds each counted on their own
    pub seq: Ranged<1>,
    /// On kept entries only: the kept entry before it; for an incarnation's first, the last kept entry before the incarnation, or 0, 0
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub prev: Opt<FeedLink>,
    pub kind: EntryKind,
    /// When the author made it, by its clock: shown, never a reason to drop it
    pub at: Ms,
    /// What it says: network/send/v1 for a message, network/receipt/v1, network/handoff/v1
    pub body: serde_json::Map<String, serde_json::Value>,
    /// The author's signature over xenonet-feed/1, a newline and the entry's header: its canonical JSON with the body as its SHA-256 (bodyHash), without the signature
    pub sig: Base64Bytes,
}

// A receiver's word that it holds every kept entry of the frame's recipient's feed up to high in incarnation inc.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedAck {
    pub inc: Uint,
    pub high: Uint,
}

// A receiver's ask, on a gap: every kept entry of the frame's recipient's feed from from in incarnation inc on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeedWant {
    pub inc: Uint,
    pub from: Uint,
}

/// What a message frame carries between hosts: an entry of its sender's machine's feed, with the authority the act rests on; or, for the recipient's feed, the sender's ack or want
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    pub schema: MessageTag,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub entry: Opt<FeedEntry>,
    /// For a message entry: what the act rests on, beside the entry since it's for this recipient alone
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub authority: Opt<Authority>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub ack: Opt<FeedAck>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub want: Opt<FeedWant>,
}

impl Document for Message {
    const ID: &'static str = "network/message/v3";
    const TITLE: &'static str = "A message frame's body";
    const DESCRIPTION: &'static str = "An entry of the sending machine's feed in the network, signed by its machine signing key, with the authority its act rests on; or that machine's ack of, or want for, the recipient's feed.";
}

// ---- network/operator/v2

crate::tag!(pub OperatorTag = "network/operator/v2");

/// allow or deny
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Allow,
    Deny,
}

// The person's answer to a run's permission prompt.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnswerAct {
    /// The prompt's id on the machine it waits on
    pub ask: Text<1, 64>,
    /// The session whose run asked it
    pub session: Text<1, 200>,
    pub decision: Decision,
}

/// An answer from another machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Operator {
    pub schema: OperatorTag,
    pub answer: AnswerAct,
    /// For an Allow: the person, by their presence for it; for a Deny, the person, an actor's token, or nothing, when the sending window host relays its window's Deny
    pub authority: Authority,
    pub at: Ms,
}

impl Document for Operator {
    const ID: &'static str = "network/operator/v2";
    const TITLE: &'static str = "An answer from another machine";
    const DESCRIPTION: &'static str =
        "Allow or deny for a run's permission prompt, from another machine: an Allow with the person's presence there, a Deny from its window or an actor with the right.";
}

// ---- network/presence/v1

crate::tag!(pub PresenceTag = "network/presence/v1");

// One agent on a host, as its presence tells it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresenceAgent {
    /// The agent's agt_, which its machine made: every machine names it so, with the machine it's on beside it
    pub id: ids::AgentId,
    pub app: Nullable<Text<1, 32>>,
    /// For an actor, its slug on its machine (actor:<slug>), which a scope's by:<slug> matches
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub actor: Opt<ids::ActorSlug>,
    pub title: Nullable<Text<0, 200>>,
    /// What the live state says: working, waiting, idle…
    pub state: Nullable<Text<0, 32>>,
    /// What it said it's doing (focus)
    pub doing: Nullable<Text<0, 280>>,
    /// The project its folder is a confirmed checkout of (prj_…), which agents on other machines are matched by, or null
    pub project_key: Nullable<Text<0, 300>>,
    /// What it's changing, inside its project, with / between parts
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub edits: Opt<Bounded<Text<0, 1000>, 0, 50>>,
    pub branch: Nullable<Text<0, 200>>,
    /// Its folder, with the home folder as ~
    pub cwd: Nullable<Text<0, 500>>,
    pub last_active: Nullable<Ms>,
    /// A question or a stuck state waits on the person
    pub waits_on_you: bool,
    /// While it waits on the person: the text of its question, or of the permission it asks for, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question: Option<Text<0, 320>>,
    /// What it said last, its last 280 characters, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<Text<0, 280>>,
    /// Why it seems stuck, with the home folder as ~
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Bounded<Line, 0, 8>>,
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
    pub agents: Bounded<PresenceAgent, 0, 1000>,
    /// Agents that went since the last presence
    pub removed: Bounded<AgentName, 0, 1000>,
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
    pub can: Opt<Bounded<Capability, 0, 16>>,
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
    pub agent: Text<1, 200>,
    pub state: ReceiptState,
    pub reason: Nullable<Text<0, 280>>,
    pub at: Ms,
    /// For started and ended: the new session's id (null for Codex until it names its thread)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<Text<0, 200>>,
    /// For started: its process
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pid: Opt<Uint>,
    /// For started: its folder, with the home folder as ~
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub folder: Opt<Text<0, 1000>>,
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

// ---- network/roster-entry/v2

// What a roster entry does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RosterOp {
    Found,
    Add,
    Remove,
    Window,
    Gateway,
    Settings,
    Relays,
    Mailboxes,
    Rekey,
}

// A signing key: Ed25519, or P-256 as a secure element signs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KeyAlg {
    Ed25519,
    P256,
}

// A public signing key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PublicKey {
    pub alg: KeyAlg,
    pub key: Base64Bytes,
}

// A machine's settings, for op settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RosterSettings {
    pub start: StartReach,
    pub wake: WakeReach,
}

// The host an entry is about, with its keys.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RosterHost {
    pub id: HostId,
    /// Its transport key, Ed25519: QUIC authentication, never an entry's signature
    pub sign: Base64Bytes,
    /// X25519 public key, what envelopes are sealed to
    pub seal: Base64Bytes,
    pub key_id: KeyId,
    /// An admin: a XenoNet window the person uses
    pub window: bool,
    /// The person's device key on it, by id; null on a machine with no way to ask them
    pub device: Nullable<SigningKeyId>,
    /// Its machine signing key: it signs the machine's entries, feeds and address records, and roots its tokens
    pub machine: PublicKey,
}

// Where the gateway is, for op gateway.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RosterGateway {
    pub address: Text<0, 300>,
    pub version: Uint,
}

// A host's signature on an entry by its machine signing key, with its device's presence where the entry admits or widens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Signer {
    pub host: HostId,
    pub sig: Base64Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presence: Option<PresenceProof>,
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
    /// For op found: the person the network is theirs
    pub owner: Nullable<PersonId>,
    /// For op settings: the machine's settings from now
    pub settings: Nullable<RosterSettings>,
    /// For op relays: the iroh relay URLs the network uses from now, with no credential in them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relays: Option<Bounded<Text<1, 300>, 0, 8>>,
    /// For op mailboxes: the members that keep sealed envelopes for away members from now
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailboxes: Option<Bounded<HostId, 0, 8>>,
    /// The owner's person chain head the entry was made under
    #[serde(rename = "personHead")]
    pub person_head: Nullable<Base64Bytes>,
    pub at: Ms,
    pub signers: Bounded<Signer, 1, 4>,
}

impl Document for RosterEntry {
    const ID: &'static str = "network/roster-entry/v2";
    const TITLE: &'static str = "A roster entry";
    const DESCRIPTION: &'static str = "One link in a network's chain of hosts: who joined, who left, who's the gateway, each one's settings and transport keys, the network's relays and mailboxes, and which hosts signed it, with the person's presence where it admits or widens.";
}

// ---- network/state/v1

crate::tag!(pub StateTag = "network/state/v1");

// The network this machine is in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NetworkInfo {
    pub id: NetworkId,
    pub name: Text<0, 64>,
}

// This machine as a host of the network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SelfHost {
    pub id: HostId,
    pub key_id: KeyId,
    pub window: bool,
    pub founder: bool,
}

// How this machine stands in the network: off while its node isn't
// running, live while it is, removed once a member took it out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LinkState {
    Off,
    Live,
    Removed,
}

// How this machine stands in the network, since when and why.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Link {
    pub state: LinkState,
    pub since: Nullable<Ms>,
    pub reason: Nullable<Text<0, 280>>,
}

// A host of the network, as this machine sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostRow {
    pub id: HostId,
    pub key_id: KeyId,
    pub window: bool,
    pub founder: bool,
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
    pub can: Opt<Bounded<Capability, 0, 16>>,
    /// How this machine reaches it on link/2: direct or relay; null while it isn't connected
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub path: Opt<Nullable<Text<0, 32>>>,
    /// The entries for it its mailboxes hold while it's away
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub held: Opt<Nullable<Uint>>,
}

// An invite that's open, for join/2: the code itself is shown once, where
// it was made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Invite {
    pub host: HostId,
    pub window: bool,
    pub expires: Ms,
    /// Wrong codes tried so far; five retire it
    pub wrong: Uint,
}

// An invite that closed unused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BurnedInvite {
    pub host: HostId,
    pub at: Ms,
    /// Retired after five wrong codes; false when it ran out of time
    pub retired: bool,
}

// The invites this machine made.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Invites {
    pub open: Vec<Invite>,
    pub burned: Vec<BurnedInvite>,
}

// Who put this machine on delegation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Delegation {
    pub by: HostId,
    pub at: Ms,
}

// A roster entry waiting for more signatures.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Proposal {
    /// The entry's id, the hash of what it says apart from its signers: what POST /api/net/approve takes
    pub id: Base64Bytes,
    pub op: RosterOp,
    pub host: HostId,
    pub window: bool,
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

/// How a device asks the person before it signs: biometric (Touch ID), or weaker (Windows Hello, polkit or a passphrase)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Biometric,
    Weaker,
}

/// One of the person's devices, as their chain holds it
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PersonDevice {
    pub id: SigningKeyId,
    /// The machine it was paired on, as the person named it then
    pub label: Text<0, 64>,
    pub kind: DeviceKind,
    /// Whether it can sign: false once it was removed or revoked
    pub active: bool,
    /// Whether it's this machine's own device
    pub here: bool,
}

/// Who this machine acts as: the person, its device here and how it asks, and whether the person owns the network
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PersonHere {
    /// null until an identity is set up here
    pub id: Nullable<PersonId>,
    /// Whether the person is the network's owner
    pub owner: bool,
    /// This machine's device, by its key's id; null where it has no way to ask the person
    pub device: Nullable<SigningKeyId>,
    pub kind: Nullable<DeviceKind>,
    /// Whether this machine's device is an active device of the person's, so it can act as them
    pub acts: bool,
    /// Whether the person's root key is sealed to this machine's device key (false: in an owner-only file); null where this machine doesn't hold it
    pub root_sealed: Nullable<bool>,
    /// The person's devices, every one their chain holds
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub devices: Opt<Vec<PersonDevice>>,
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
    /// null: this machine isn't in a network
    pub link: Nullable<Link>,
    pub hosts: Vec<HostRow>,
    /// The invites this machine made, while it's in a network
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub invites: Opt<Invites>,
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
    /// Roster entries signed by too few, waiting for the founder or a second window host to approve them
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub proposals: Opt<Vec<Proposal>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub removed: Opt<Removal>,
    /// XenoNet Dev's made-up network (simulate net-…)
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub simulated: Opt<bool>,
    /// Who this machine acts as, while it's in a network
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub person: Opt<PersonHere>,
    /// Members' feeds a fork froze here: nothing from them is taken until you unfreeze one at the window
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub frozen: Opt<Vec<FrozenFeed>>,
}

/// A member's feed that froze here: it signed two entries at one place
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrozenFeed {
    /// The member, by its machine signing key
    pub author: SigningKeyId,
    /// The member's host id, while the network's chain lists it
    pub host: Nullable<HostId>,
    /// Where the two entries are
    pub at: FeedPlace,
}

impl Document for State {
    const ID: &'static str = "network/state/v1";
    const TITLE: &'static str = "The network, as a host sees it";
    const DESCRIPTION: &'static str = "GET /api/net/state: the network this machine is in, how it stands in it, the other hosts, and what waits.";
}

// ---- network/svc/v2

/// A svc request's body, at most.
pub const SVC_REQUEST_BODY: usize = 8 << 20;
/// The header XenoNet's own refusals of a svc call carry, with their code,
/// so a caller can tell them from the service's answers.
pub const SVC_REFUSED_HEADER: &str = "x-xenonet-refused";

/// XenoNet's own answer to a svc call, rather than the service's: its
/// headers and its body, `{ error, code }`.
pub fn svc_refusal(code: &str, message: &str) -> (Vec<(String, String)>, Vec<u8>) {
    let headers = vec![("content-type".into(), "application/json; charset=utf-8".into()), ("cache-control".into(), "no-store".into()), (SVC_REFUSED_HEADER.into(), code.into())];
    (headers, serde_json::json!({ "error": message, "code": code }).to_string().into_bytes())
}

crate::tag!(pub SvcTag = "network/svc/v2");
crate::tag!(pub SvcRequestOp = "request");
crate::tag!(pub SvcWindowOp = "window");
crate::tag!(pub SvcAnswerOp = "answer");

crate::pattern! {
    /// The request, made at random by the calling host; its answer names it
    pub struct SvcId(r"^svq_[0-9a-f]{32}$");
}

crate::pattern! {
    // A service's name, as its host published it.
    pub struct ServiceName(r"^[a-z][a-z0-9-]{1,31}$");
}

crate::pattern! {
    // An HTTP method.
    pub struct HttpMethod(r"^[A-Z]{1,16}$");
}

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

crate::pattern! {
    // Why XenoNet answered rather than the service.
    pub struct SvcCode(r"^[a-z-]{1,32}$");
}

// A request to a published service: the head on its svc/2 stream, before
// its body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SvcRequest {
    pub schema: SvcTag,
    /// From the calling host
    pub op: SvcRequestOp,
    /// The network the service was published in: the connection carries it, and the caller is the member whose endpoint id the connection authenticated in its chain
    pub network: NetworkId,
    pub id: SvcId,
    /// The service, as its host published it
    pub name: ServiceName,
    /// A host passes on GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS
    pub method: HttpMethod,
    /// The path after /api/svc/<host>/<name>, with its query: the publishing host normalizes it and checks it against the paths it publishes
    pub path: Text<0, 4096>,
    /// Only the caller's Authorization, Content-Type, Accept and User-Agent cross; the publishing host adds Host and Forwarded itself
    pub headers: Bounded<Header, 0, 64>,
    /// The bytes of body after the head, at most 8 MiB
    pub length: Ranged<0, 8388608>,
}

// The person's device's word that a window key reads for them, from one
// machine's endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WindowCert {
    pub person: Text<1, 128>,
    /// The device's key id, in the person's chain
    pub device: Text<1, 128>,
    /// The window key
    pub key: PublicKey,
    /// The endpoint id of the machine whose window it is, in hex
    pub endpoint: Text<64, 64>,
    /// When it was certified (ms)
    pub at: Int,
    /// When it lapses (ms), 12 hours after at, at most
    pub until: Int,
    /// The device's signature
    pub sig: Base64Bytes,
}

// A read of the answering machine's own API for the person's window on
// another machine: a session's transcript or stats.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SvcWindow {
    pub schema: SvcTag,
    pub op: SvcWindowOp,
    pub network: NetworkId,
    /// /api/session/<id>/transcript (with cursor and limit) or /api/session/<id>/stats
    pub path: Text<1, 4096>,
    /// Taken once
    pub nonce: Text<16, 64>,
    /// When it was made (ms): within two minutes of the answering machine's clock
    pub at: Int,
    pub cert: WindowCert,
    /// The window key's signature over the network, both endpoint ids, the nonce, the time and the path
    pub sig: Base64Bytes,
    /// No body follows
    pub length: Ranged<0, 0>,
}

// The service's answer, or XenoNet's: the head on the stream, before its
// body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SvcAnswer {
    pub schema: SvcTag,
    /// From the publishing host
    pub op: SvcAnswerOp,
    pub status: Ranged<200, 599>,
    /// The service's answer's headers, without Set-Cookie and hop-by-hop headers
    pub headers: Bounded<Header, 0, 64>,
    /// The bytes of body after the head, at most 8 MiB
    pub length: Ranged<0, 8388608>,
    /// Set when XenoNet answered rather than the service: not-published, not-allowed, replayed, rate, path-refused, too-big, timeout…
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub code: Opt<SvcCode>,
}

/// A head on a svc/2 stream.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Svc {
    Request(SvcRequest),
    Window(SvcWindow),
    Answer(SvcAnswer),
}

impl Document for Svc {
    const ID: &'static str = "network/svc/v2";
    const TITLE: &'static str = "A head on a svc/2 stream";
    const DESCRIPTION: &'static str = "One stream per request between members of a network (xenonet/svc/2): a request to a service another host published, or the window's read of another machine's transcript, and the answer, each a head and then its body.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[
    crate::schema::file::<Claim>,
    crate::schema::file::<Handoff>,
    crate::schema::file::<Message>,
    crate::schema::file::<Operator>,
    crate::schema::file::<Presence>,
    crate::schema::file::<Receipt>,
    crate::schema::file::<RosterEntry>,
    crate::schema::file::<SendBody>,
    crate::schema::file::<State>,
    crate::schema::file::<Svc>,
];
