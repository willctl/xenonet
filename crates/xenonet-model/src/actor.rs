//! Actors: the programs and people XenoNet knows of beside its sessions.
//! An actor registers (`actor/registration/v1`) and gets its credentials
//! (`actor/credentials/v1`), reports what it does (`actor/report/v1`) and
//! sees itself (`actor/me/v1`); the console draws each one
//! (`actor/snapshot/v1`, `actor/list/v1`) and what they did
//! (`actor/event/v1`, `actor/events/v1`), and the owner sees what waits
//! for an Allow (`actor/requests/v1`), the tokens (`actor/tokens/v1`, the
//! gateway's in `actor/gateway-ask/v1` and `actor/gateway-credentials/v1`)
//! and the remote connections (`actor/connection/v1`). The person at the
//! console is an actor too (`actor/console/v1`).

use crate::ids::{ActorId, ActorSlug};
use crate::num::{Ms, PosInt, Uint};
use crate::schema::{Document, Nullable, Opt, Ref, Text};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

// ---------- shared vocabularies ----------

crate::open!(ActorState of KnownActorState { Waiting = "waiting", Allowed = "allowed" });

crate::open!(Presence of KnownPresence {
    Working = "working",
    Idle = "idle",
    Away = "away",
    Offline = "offline",
    Unknown = "unknown",
});

crate::open!(
    /// observed: XenoNet reads its state; attributed: XenoNet traces work here back to it; registered: it reports itself; remote: it reads through a connection you approved; demo: made up
    Via of KnownVia {
        Observed = "observed",
        Attributed = "attributed",
        Registered = "registered",
        Remote = "remote",
        Demo = "demo",
    }
);

crate::open!(Capability of KnownCapability {
    SpawnSessions = "spawn-sessions",
    EditRepos = "edit-repos",
    RunCommands = "run-commands",
    MessagePeople = "message-people",
    MessageAgents = "message-agents",
    Schedule = "schedule",
    Browse = "browse",
    UseComputer = "use-computer",
});

crate::open!(Relation of KnownRelation {
    WorksOn = "works-on",
    Spawned = "spawned",
    Edited = "edited",
    Messaged = "messaged",
    Watches = "watches",
    Reviews = "reviews",
});

// What a registered program says it is, as the console and the requests
// list carry it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Product {
    /// A slug: hermes-agent, grok-bot, muse, dots
    #[schemars(regex(pattern = r"^[a-z0-9][a-z0-9-]{0,63}$"))]
    pub id: String,
    pub vendor: Nullable<String>,
    pub version: Nullable<String>,
}

// The process that asked, as XenoNet looked it up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Asker {
    pub program: String,
    /// Its parents, up to the first app or terminal
    pub parents: Vec<String>,
    pub folder: Nullable<String>,
}

// ---------- actor/connection/v1 ----------

crate::tag!(pub ConnectionTag = "actor/connection/v1");

// The one thing a remote connection may do.
crate::tag!(pub ReadPermission = "read");

// ISO 8601 in UTC, to the second or finer, with a real calendar date.
const DATE_TIME_PATTERN: &str = r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z))$";

// A time written as ISO 8601 in UTC.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DateTime(pub String);

impl JsonSchema for DateTime {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("DateTime")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "format": "date-time",
            "pattern": DATE_TIME_PATTERN,
            "description": "ISO 8601, UTC"
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    OwnerVerified,
    Unverified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AudienceScope {
    OwnerEndpoint,
    AuthenticatedUser,
    OtherApprovedScope,
}

// Who may call the connection's endpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Audience {
    /// Who can call the endpoint, in your words
    #[schemars(length(max = 200))]
    pub description: String,
    pub verification: Verification,
    pub scope: AudienceScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountLinkState {
    Verified,
    OwnerAsserted,
    Unverified,
}

// The account the actor acts as through the connection, and how sure
// XenoNet is of it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AccountLink {
    pub state: AccountLinkState,
    pub identity: Nullable<Text<0, 200>>,
}

// The device the connection was approved for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    #[schemars(regex(pattern = r"^dev_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"))]
    pub id: String,
    pub alias: Nullable<Text<0, 64>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataMode {
    Fixture,
    Real,
}

// A project the connection may read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProject {
    #[schemars(regex(pattern = r"^prj_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"))]
    pub project_id: String,
    /// The project's node id on your map, for this window only
    pub place: Nullable<String>,
    pub name: Nullable<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Active,
    Expired,
    Revoked,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bridge {
    Connected,
    NotConnected,
}

// The command the tunnel client runs for the connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub command: String,
    pub args: Vec<String>,
}

/// A remote connection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    pub schema: ConnectionTag,
    #[schemars(regex(pattern = r"^cn_[0-9a-f]{24}$"))]
    pub connection_id: String,
    #[schemars(regex(pattern = r"^gr_[0-9a-f]{24}$"))]
    pub grant_id: String,
    #[schemars(regex(pattern = r"^actor:[a-z0-9][a-z0-9-]{0,31}$"))]
    pub actor_id: String,
    /// The name you gave it
    #[schemars(length(min = 1, max = 64))]
    pub label: String,
    #[schemars(length(max = 32))]
    pub adapter_id: String,
    #[schemars(length(max = 32))]
    pub provider: String,
    #[schemars(length(max = 48))]
    pub transport_kind: String,
    pub audience: Audience,
    pub account_link: AccountLink,
    pub device: Device,
    pub permissions: Vec<ReadPermission>,
    pub data_mode: DataMode,
    pub projects: Vec<ConnectionProject>,
    pub labels: bool,
    #[schemars(range(min = 1, max = 30))]
    pub lookback_days: Uint,
    #[schemars(range(min = 1))]
    pub revision: Uint,
    pub state: ConnectionState,
    /// Whether an adapter is connected now; it says nothing about whether the actor itself is working
    pub bridge: Bridge,
    pub created_at: DateTime,
    pub expires_at: DateTime,
    pub revoked_at: Nullable<DateTime>,
    pub revoked_reason: Nullable<Text<0, 32>>,
    pub last_used_at: Nullable<DateTime>,
    /// What the tunnel client runs: the adapter for this grant, holding no secret
    pub launch: Launch,
}

impl Document for Connection {
    const ID: &'static str = "actor/connection/v1";
    const TITLE: &'static str = "A remote connection";
    const DESCRIPTION: &'static str = "One way a remote actor reaches XenoNet, as the owner sees it in XenoNet's window: who may call it, what it may read, until when, and whether it's in use. Never its credential.";
}

// ---------- actor/console/v1 ----------

crate::tag!(pub ConsoleTag = "actor/console/v1");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConsolePresence {
    Working,
    Idle,
    Away,
}

crate::open!(ConsoleAction of KnownConsoleAction {
    Looked = "looked",
    Read = "read",
    Opened = "opened",
    HandedOff = "handed-off",
    Started = "started",
    Terminal = "terminal",
});

crate::open!(ConsoleApp of KnownConsoleApp { Claude = "claude", Codex = "codex" });

/// Something you did, as a node id and an action's name: never text
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Did {
    /// looked: a selection that held 3 s; read: a preview or stats; opened: in its app; handed-off and started: a session; terminal: one opened in a folder
    pub what: ConsoleAction,
    /// The graph node it was done to
    #[schemars(length(min = 1, max = 1024))]
    pub node: String,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub at: Opt<Ms>,
    /// For handed-off: the app it went to
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub to: Opt<ConsoleApp>,
    /// For started: the app the session starts in
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub app: Opt<ConsoleApp>,
}

/// The person at the console.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Console {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub schema: Opt<ConsoleTag>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub presence: Opt<ConsolePresence>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 20))]
    pub did: Opt<Vec<Did>>,
}

impl Document for Console {
    const ID: &'static str = "actor/console/v1";
    const TITLE: &'static str = "The person at the console";
    const DESCRIPTION: &'static str = "What XenoNet's own window reports of you to POST /api/actors/you, with the key only the window holds: your presence, and what you did as node ids and action names. Never what you type.";
}

// ---------- actor/credentials/v1 ----------

crate::tag!(pub CredentialsTag = "actor/credentials/v1");

/// An actor's credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Credentials {
    pub schema: CredentialsTag,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub id: ActorId,
    /// Shown once: XenoNet keeps only its SHA-256
    #[schemars(regex(pattern = r"^xenonet_actor_[0-9a-f]{64}$"))]
    pub token: String,
    pub state: ActorState,
    /// While it waits: when the request lapses. Left out once it's allowed
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub expires_at: Opt<f64>,
    /// Capabilities asked for that XenoNet doesn't know
    pub ignored: Vec<String>,
}

impl Document for Credentials {
    const ID: &'static str = "actor/credentials/v1";
    const TITLE: &'static str = "An actor's credentials";
    const DESCRIPTION: &'static str = "The answer to registering, to adding an actor from XenoNet's window and to a new token: the actor's id and its token, once.";
}

// ---------- actor/event/v1 ----------

crate::tag!(pub EventTag = "actor/event/v1");

crate::open!(EventKind of KnownEventKind {
    Presence = "presence",
    Status = "status",
    Spawned = "spawned",
    Edited = "edited",
    Ran = "ran",
    Messaged = "messaged",
    Said = "said",
    Finished = "finished",
    Failed = "failed",
});

/// An actor's event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub schema: EventTag,
    /// Unique for the actor, such as wren:41
    pub id: String,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub actor: ActorId,
    pub at: Ms,
    pub kind: EventKind,
    /// A graph node id, an actor id, or null
    pub target: Nullable<String>,
    /// What and where, written by XenoNet from facts: never the text of a message or a conversation, and never a secret
    pub detail: Nullable<Text<0, 200>>,
    pub via: Via,
}

impl Document for Event {
    const ID: &'static str = "actor/event/v1";
    const TITLE: &'static str = "An actor's event";
    const DESCRIPTION: &'static str =
        "Something an actor did: came or went, changed what it's doing, spawned a session, edited, ran a command, messaged someone, said something, finished or failed.";
}

// ---------- actor/events/v1 ----------

crate::tag!(pub EventsTag = "actor/events/v1");

/// Actors' events since a time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Events {
    pub schema: EventsTag,
    pub at: Ms,
    /// Oldest first
    pub events: Vec<Ref<Event>>,
}

impl Document for Events {
    const ID: &'static str = "actor/events/v1";
    const TITLE: &'static str = "Actors' events";
    const DESCRIPTION: &'static str =
        "What actors did since a time, at most the last 7 days, for replay and digests: GET /api/actors/events?since=<ms>, which leaves out the person at the console's.";
}

// ---------- actor/gateway-ask/v1 ----------

crate::tag!(pub GatewayAskTag = "actor/gateway-ask/v1");

// The only state an ask is ever answered in: the token comes later, and
// only to the window.
crate::tag!(pub WaitingState = "waiting");

/// A session's ask for a gateway token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GatewayAsk {
    pub schema: GatewayAskTag,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub id: ActorId,
    pub state: WaitingState,
    /// The ask's id, which the window answers with POST /api/actors/requests/<id>
    #[schemars(length(min = 1))]
    pub request: String,
    /// When the ask lapses unanswered, in epoch milliseconds
    pub expires_at: f64,
    /// The actor's app has a gateway token already, which Allow replaces
    pub replaces: bool,
}

impl Document for GatewayAsk {
    const ID: &'static str = "actor/gateway-ask/v1";
    const TITLE: &'static str = "A session's ask for a gateway token";
    const DESCRIPTION: &'static str = "The answer to asking for an actor's gateway token: that it waits for an Allow in XenoNet's window. It never holds a token, so whoever asks never sees one: the token is made, and shown masked, only in the window.";
}

// ---------- actor/gateway-credentials/v1 ----------

crate::tag!(pub GatewayCredentialsTag = "actor/gateway-credentials/v1");

/// An actor's gateway credentials.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GatewayCredentials {
    pub schema: GatewayCredentialsTag,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub id: ActorId,
    /// Shown once: XenoNet keeps only its SHA-256. It works only at XenoNet's actor gateway, as Authorization: Bearer
    #[schemars(regex(pattern = r"^xenonet_gateway_[0-9a-f]{64}$"))]
    pub token: String,
    /// The gateway's address, when XenoNet knows it: where the program sends its MCP requests
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl Document for GatewayCredentials {
    const ID: &'static str = "actor/gateway-credentials/v1";
    const TITLE: &'static str = "An actor's gateway credentials";
    const DESCRIPTION: &'static str = "The answer to making a gateway token in XenoNet's window: the actor's id and a token that works only at XenoNet's actor gateway, once.";
}

// ---------- actor/list/v1 ----------

crate::tag!(pub ListTag = "actor/list/v1");

/// The actors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct List {
    pub schema: ListTag,
    pub at: Ms,
    pub actors: Vec<Ref<Snapshot>>,
}

impl Document for List {
    const ID: &'static str = "actor/list/v1";
    const TITLE: &'static str = "The actors";
    const DESCRIPTION: &'static str = "Every actor XenoNet knows of, as GET /api/actors and the actors event send them.";
}

// ---------- actor/me/v1 ----------

crate::tag!(pub MeTag = "actor/me/v1");

// One part of a report XenoNet left out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Dropped {
    pub index: Uint,
    pub reason: String,
}

/// An actor, as it sees itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Me {
    pub schema: MeTag,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub id: ActorId,
    pub state: ActorState,
    pub presence: Presence,
    pub heartbeat_ms: Nullable<PosInt>,
    /// Engine messages waiting for it
    pub unread: Uint,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub accepted: Opt<Uint>,
    /// The parts of the report XenoNet left out, and why
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub dropped: Opt<Vec<Dropped>>,
}

impl Document for Me {
    const ID: &'static str = "actor/me/v1";
    const TITLE: &'static str = "An actor, as it sees itself";
    const DESCRIPTION: &'static str = "The answer to an actor's own calls (GET /api/actors/me, and its reports) and to the person's reports: whether it's allowed, its presence, its unread messages, and what XenoNet took.";
}

// ---------- actor/registration/v1 ----------

crate::tag!(pub RegistrationTag = "actor/registration/v1");

// What a registering program says it is: only the id is needed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RegistrationProduct {
    #[schemars(regex(pattern = r"^[a-z0-9][a-z0-9-]{0,63}$"))]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// Registering an actor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub schema: Opt<RegistrationTag>,
    #[schemars(length(min = 1, max = 64))]
    pub name: String,
    /// Without one, XenoNet makes one from the name
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub slug: Opt<ActorSlug>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<RegistrationProduct>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub capabilities: Opt<Vec<Capability>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub heartbeat_ms: Opt<PosInt>,
    /// Its process, which XenoNet checks on macOS
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pid: Opt<PosInt>,
}

impl Document for Registration {
    const ID: &'static str = "actor/registration/v1";
    const TITLE: &'static str = "Registering an actor";
    const DESCRIPTION: &'static str =
        "What a program sends to POST /api/actors/register to ask to be an actor. It's answered at once, and its token works once you allow it in XenoNet's window.";
}

// ---------- actor/report/v1 ----------

crate::tag!(pub ReportTag = "actor/report/v1");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReportPresence {
    Working,
    Idle,
    Offline,
}

crate::open!(ReportEventKind of KnownReportEventKind {
    Spawned = "spawned",
    Edited = "edited",
    Ran = "ran",
    Messaged = "messaged",
    Finished = "finished",
    Failed = "failed",
});

crate::open!(ReportApp of KnownReportApp { Claude = "claude", Codex = "codex", Other = "other" });

crate::open!(Platform of KnownPlatform {
    Telegram = "telegram",
    Discord = "discord",
    Slack = "slack",
    Whatsapp = "whatsapp",
    Signal = "signal",
    Matrix = "matrix",
    Mattermost = "mattermost",
    Email = "email",
    Sms = "sms",
    Imessage = "imessage",
    Teams = "teams",
    Other = "other",
});

crate::open!(Subject of KnownSubject {
    Session = "session",
    Task = "task",
    Job = "job",
    Turn = "turn",
    Command = "command",
});

/// Something it did, as facts: XenoNet writes the detail
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReportEvent {
    pub kind: ReportEventKind,
    /// A graph node id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub target: Opt<String>,
    /// An absolute path, mapped to the project or folder holding it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub path: Opt<String>,
    /// When it happened; now when left out
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub at: Opt<f64>,
    /// Its own id for the event, so a retry never counts twice
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(regex(pattern = r"^[A-Za-z0-9._:-]{1,64}$"))]
    pub r#ref: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 1, max = 10000))]
    pub count: Opt<Uint>,
    /// For edited: absolute paths
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 20))]
    pub paths: Opt<Vec<String>>,
    /// For spawned, with a session not on the map yet
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub app: Opt<ReportApp>,
    /// For messaged
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub platform: Opt<Platform>,
    /// For finished and failed
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub subject: Opt<Subject>,
}

/// Something it's acting on
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ReportTarget {
    /// A graph node id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub id: Opt<String>,
    /// An absolute path, mapped to the project or folder holding it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub path: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub relation: Opt<Relation>,
}

/// An actor's report.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub schema: Opt<ReportTag>,
    /// Which of its processes this is: main when left out
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub instance: Opt<ActorSlug>,
    /// offline says goodbye; only XenoNet decides away and unknown
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub presence: Opt<ReportPresence>,
    /// What it's doing: cut to 140 characters on one line, and an empty string clears it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub status: Opt<String>,
    /// Where it's working, an absolute path
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub cwd: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pid: Opt<PosInt>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 50))]
    pub events: Opt<Vec<ReportEvent>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub targets: Opt<Vec<ReportTarget>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub remove: Opt<Vec<ReportTarget>>,
    /// targets is the whole list
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub replace: Opt<bool>,
}

impl Document for Report {
    const ID: &'static str = "actor/report/v1";
    const TITLE: &'static str = "An actor's report";
    const DESCRIPTION: &'static str = "What a registered actor posts with its token: a heartbeat (POST /api/actors/heartbeat), what it did (POST /api/actors/events) or what it's acting on (POST /api/actors/targets).";
}

// ---------- actor/requests/v1 ----------

crate::tag!(pub RequestsTag = "actor/requests/v1");

// A program waiting for an Allow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    /// The request's own id: POST /api/actors/requests/<id> answers it
    #[schemars(length(min = 1))]
    pub id: String,
    /// The actor it would be
    pub actor: ActorId,
    #[schemars(length(min = 1, max = 64))]
    pub name: String,
    /// The product it says it is; null for a program of your own
    pub product: Nullable<Product>,
    /// What it asks to be allowed
    pub capabilities: Vec<Capability>,
    pub heartbeat_ms: Nullable<PosInt>,
    /// The process it named, if any
    pub pid: Nullable<PosInt>,
    /// Who asked, with ~ for the home folder; null where XenoNet couldn't look
    pub asker: Nullable<Asker>,
    /// It would join an actor XenoNet already knows, such as a product's observed one
    pub joins: bool,
    /// When it asked, in epoch milliseconds
    pub at: f64,
    /// When it lapses unanswered, in epoch milliseconds
    pub expires_at: f64,
}

// A session's ask for an actor's gateway token, waiting for an Allow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PendingGatewayAsk {
    /// The ask's own id: POST /api/actors/requests/<id> answers it, and Allow makes the token
    #[schemars(length(min = 1))]
    pub id: String,
    /// The actor whose app would get the token
    pub actor: ActorId,
    #[schemars(length(min = 1, max = 64))]
    pub name: String,
    /// The actor's app has a gateway token already, which Allow replaces
    pub replaces: bool,
    /// Who asked, with ~ for the home folder; null where XenoNet couldn't look
    pub asker: Nullable<Asker>,
    /// When it asked, in epoch milliseconds
    pub at: f64,
    /// When it lapses unanswered, in epoch milliseconds
    pub expires_at: f64,
}

/// Registrations waiting for you.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Requests {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub schema: Opt<RequestsTag>,
    pub at: Ms,
    /// Oldest first
    pub requests: Vec<Request>,
    /// Sessions' asks for an actor's gateway token, oldest first. Never a token: it's made only when you Allow
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub gateway_asks: Opt<Vec<PendingGatewayAsk>>,
}

impl Document for Requests {
    const ID: &'static str = "actor/requests/v1";
    const TITLE: &'static str = "Registrations waiting for you";
    const DESCRIPTION: &'static str = "What GET /api/actors/requests answers XenoNet's own window, with its key: each program waiting for an Allow, what it asks, who asked and when it lapses, and any session's ask for an actor's gateway token.";
}

// ---------- actor/snapshot/v1 ----------

crate::tag!(pub SnapshotTag = "actor/snapshot/v1");

crate::open!(ActorKind of KnownActorKind { Agent = "agent", Human = "human" });

crate::open!(NeedKind of KnownNeedKind { Question = "question", Permission = "permission", Review = "review" });

crate::open!(RemoteBridge of KnownRemoteBridge { Connected = "connected", NotConnected = "not_connected" });

crate::open!(RemoteAccountLink of KnownRemoteAccountLink {
    Verified = "verified",
    OwnerAsserted = "owner_asserted",
    Unverified = "unverified",
});

crate::open!(RemotePermission of KnownRemotePermission { Read = "read" });

crate::open!(Transport of KnownTransport {
    OpenaiSecureMcpTunnel = "openai-secure-mcp-tunnel",
    LocalStdio = "local-stdio",
});

// What the actor waits on you for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NeedsYou {
    pub kind: NeedKind,
    #[schemars(length(max = 140))]
    pub text: String,
    pub since: Ms,
}

// Tokens by kind, as the app's own records add them up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub input: f64,
    pub output: f64,
    pub reasoning: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

// One model's share of the usage.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageModel {
    pub provider: Nullable<String>,
    pub model: Nullable<String>,
    pub mode: Nullable<String>,
    pub tokens: f64,
    pub cost_usd: Nullable<f64>,
}

// What the app's own records say its sessions used and cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// When XenoNet began adding up the sessions below
    pub since: f64,
    /// The sessions its own records show at work since then, counted by their whole totals
    pub sessions: Uint,
    /// What the app totalled for them, in USD: the sessions it priced; null when it priced none. Not XenoNet's estimate at API prices
    pub cost_usd: Nullable<f64>,
    /// Some of costUsd is the app's own estimate, not what the provider billed
    pub estimated: bool,
    /// Sessions with no price: covered by a subscription, or the app doesn't say
    pub unpriced: Uint,
    pub tokens: UsageTokens,
    /// The five that used the most tokens
    #[schemars(length(max = 5))]
    pub models: Vec<UsageModel>,
}

// Something the actor is acting on, as the console draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// A graph node id
    pub id: String,
    pub relation: Relation,
    pub since: Ms,
    pub last_at: Ms,
}

// The last day's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Counts {
    pub events_24h: Uint,
    pub spawned_24h: Uint,
    pub messages_24h: Uint,
}

// The actor's remote connections, summed up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    /// Its remote connections, active or not
    pub connections: Uint,
    /// Whether one is active: approved, not revoked or expired
    pub active: bool,
    /// Whether an adapter is connected through one now; it says nothing of whether the actor itself is at work
    pub bridge: RemoteBridge,
    /// How sure XenoNet is of the account it acts as: verified by its provider, your word only, or neither
    pub account_link: RemoteAccountLink,
    /// What it may do through them, apart from its capabilities elsewhere
    pub permissions: Vec<RemotePermission>,
    /// The routes its active connections take: a tunnel such as OpenAI's Secure MCP Tunnel, or a local MCP client
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub transports: Opt<Vec<Transport>>,
}

/// An actor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub schema: SnapshotTag,
    /// actor:<slug>, or @<host>:actor:<slug> from another host; the person at the console is actor:you
    pub id: ActorId,
    pub kind: ActorKind,
    #[schemars(length(min = 1))]
    pub name: String,
    /// What it is; null for a custom actor
    pub product: Nullable<Product>,
    /// Null on this machine, else the host id, and the id is then qualified
    pub host: Nullable<String>,
    /// How XenoNet knows of it
    pub via: Vec<Via>,
    /// away: it missed two heartbeats
    pub presence: Presence,
    pub presence_since: Nullable<Ms>,
    pub last_seen: Nullable<Ms>,
    pub heartbeat_ms: Nullable<PosInt>,
    /// What it says, or XenoNet infers, it's doing
    pub status: Nullable<Text<0, 140>>,
    /// When it waits on you, for the alert strip; null or left out otherwise
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_you: Option<NeedsYou>,
    /// What the app's own records say its sessions used and cost (Hermes's session database); left out when it keeps none
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub usage: Opt<Usage>,
    pub capabilities: Vec<Capability>,
    /// What it's acting on
    pub targets: Vec<Target>,
    /// Its last events, newest last
    #[schemars(length(max = 20))]
    pub recent: Vec<Ref<Event>>,
    pub counts: Counts,
    /// The identity it acts as (claude:…, chatgpt:…, github:…), if XenoNet knows it
    pub identity: Nullable<String>,
    /// Left out for an actor with no remote connection
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub remote: Opt<Remote>,
}

impl Document for Snapshot {
    const ID: &'static str = "actor/snapshot/v1";
    const TITLE: &'static str = "An actor";
    const DESCRIPTION: &'static str =
        "An actor as the console draws it: who or what it is, how XenoNet knows of it, whether it's there and what it's doing, and what it's acting on.";
}

// ---------- actor/tokens/v1 ----------

crate::tag!(pub TokensTag = "actor/tokens/v1");

crate::open!(TokenState of KnownTokenState { Works = "works", Expired = "expired", None = "none" });

crate::open!(GatewayTokenState of KnownGatewayTokenState { Works = "works" });

// An actor's gateway token, while it has one that works.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GatewayToken {
    pub state: GatewayTokenState,
    /// When it stops working unless it's used before then
    pub expires_at: f64,
    /// When it was last used, to within the hour; null for one never used
    pub used_at: Nullable<Ms>,
}

// One registered actor's token, as the owner sees it: never the token.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    /// An actor that reports itself
    pub actor: ActorId,
    /// works; expired: 30 days unused, until you give it a new token; none: the demo's made-up actor has none
    pub state: TokenState,
    /// When it stops working unless it's used before then; null unless it works
    pub expires_at: Nullable<Ms>,
    /// When it was last used: exactly the first time, and to within the hour after that; null for a token never used, or none
    pub used_at: Nullable<Ms>,
    /// Its gateway token, while it has one that works (server/actor-gateway.js)
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub gateway: Opt<GatewayToken>,
}

/// Registered actors' tokens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tokens {
    pub schema: TokensTag,
    pub at: Ms,
    /// Oldest actor first
    pub tokens: Vec<Token>,
}

impl Document for Tokens {
    const ID: &'static str = "actor/tokens/v1";
    const TITLE: &'static str = "Registered actors' tokens";
    const DESCRIPTION: &'static str = "What GET /api/actors/tokens answers XenoNet's own window, with its key: for each actor that reports itself, whether its token works, when it expires unless it's used, and when it was last used. Never a token, nor its hash.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[
    crate::schema::file::<Connection>,
    crate::schema::file::<Console>,
    crate::schema::file::<Credentials>,
    crate::schema::file::<Event>,
    crate::schema::file::<Events>,
    crate::schema::file::<GatewayAsk>,
    crate::schema::file::<GatewayCredentials>,
    crate::schema::file::<List>,
    crate::schema::file::<Me>,
    crate::schema::file::<Registration>,
    crate::schema::file::<Report>,
    crate::schema::file::<Requests>,
    crate::schema::file::<Snapshot>,
    crate::schema::file::<Tokens>,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_snapshot_round_trips_with_words_it_does_not_know() {
        let text = r#"{"schema":"actor/snapshot/v1","id":"@vm1:actor:kite","kind":"agent","name":"Kite","product":null,"host":"vm1","via":["observed","telepathy"],"presence":"working","presenceSince":1.0,"lastSeen":null,"heartbeatMs":30000,"status":null,"capabilities":["browse"],"targets":[],"recent":[],"counts":{"events24h":1,"spawned24h":0,"messages24h":0},"identity":null}"#;
        let s: Snapshot = serde_json::from_str(text).unwrap();
        assert_eq!(s.id.to_string(), "@vm1:actor:kite");
        assert_eq!(s.kind, ActorKind::Known(KnownActorKind::Agent));
        assert_eq!(s.via, vec![Via::Known(KnownVia::Observed), Via::Other("telepathy".into())]);
        assert_eq!((s.presence_since.0, s.last_seen.0, s.heartbeat_ms.0), (Some(Ms(1.0)), None, Some(PosInt(30000))));
        assert!(s.needs_you.is_none() && s.usage.is_none() && s.remote.is_none());
        let back: serde_json::Value = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, serde_json::from_str::<serde_json::Value>(text).unwrap(), "left-out fields stay left out, nulls stay null");
    }

    #[test]
    fn a_report_keeps_only_what_was_sent() {
        let r: Report = serde_json::from_str(r#"{"presence":"idle","events":[{"kind":"edited","paths":["/a"],"ref":"e1"}]}"#).unwrap();
        assert_eq!(r.presence.as_ref(), Some(&ReportPresence::Idle));
        let e = &r.events.as_ref().unwrap()[0];
        assert_eq!((e.r#ref.as_ref().map(String::as_str), e.count.as_ref()), (Some("e1"), None));
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"presence":"idle","events":[{"kind":"edited","ref":"e1","paths":["/a"]}]}"#);
        assert!(serde_json::from_str::<Connection>(r#"{"schema":"actor/connection/v1","extra":1}"#).is_err(), "a connection is strict");
    }
}
