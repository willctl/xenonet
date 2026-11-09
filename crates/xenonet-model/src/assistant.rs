//! What the read-only tools of `xenonet mcp assistant` answer a remote
//! actor: an answer (`assistant/result/v1`) or a refusal
//! (`assistant/error/v1`). Both are strict: nothing the contract doesn't
//! name.

use crate::num::Uint;
use crate::schema::{Document, Nullable, Opt, Text};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

// A string of one shape: an id, a token, a time.
macro_rules! patterned {
    ($(#[$m:meta])* $name:ident = $pattern:literal $(, $key:literal : $value:literal)*) => {
        $(#[$m])*
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }

        impl std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", $($key: $value,)* "pattern": $pattern })
            }
        }
    };
}

patterned!(
    /// A request's id: r_ and a UUID.
    RequestId = "^r_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
);
patterned!(
    /// A device's id: dev_ and a UUID.
    DeviceId = "^dev_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
);
patterned!(
    /// A project's id: prj_ and a UUID.
    ProjectId = "^prj_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
);
patterned!(
    /// A session's id: its app, a colon and a UUID.
    SessionId = "^(claude|codex|grok):[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
);
patterned!(
    /// An actor's id: actor: and a short name.
    ActorId = "^actor:[a-z0-9][a-z0-9-]{0,31}$"
);
patterned!(
    /// An attention item's id: att_ and 24 hex digits.
    AttentionId = "^att_[0-9a-f]{24}$"
);
patterned!(
    /// An opaque page token.
    Cursor = r"^[A-Za-z0-9_-]{1,900}\.[A-Za-z0-9_-]{43}$",
    "description": "An opaque page token from next_cursor, good for five minutes"
);
patterned!(
    /// A time as ISO 8601, in UTC.
    DateTime = r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z))$",
    "format": "date-time",
    "description": "ISO 8601, UTC"
);

crate::constant!(
    /// The assistant API's version, always 1.
    pub ApiVersion = 1
);
crate::constant!(
    /// An attention item is never resolved when it's listed.
    pub Unresolved = false
);

crate::tag!(pub ErrorTag = "xenonet.assistant.error.v1");
crate::tag!(pub ResultTag = "xenonet.assistant.result.v1");
crate::tag!(pub ReadOnly = "read_only");
crate::tag!(pub Disabled = "disabled");

// Why a tool answered nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotAuthorized,
    NotFound,
    GrantExpired,
    GrantRevoked,
    Disabled,
    InvalidArgument,
    CursorExpired,
    SourceUnavailable,
    ServiceUnavailable,
    RateLimited,
    IncompatibleVersion,
    Timeout,
    InternalError,
}

/// A remote actor's refusal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub schema: ErrorTag,
    pub request_id: Nullable<RequestId>,
    pub code: ErrorCode,
    /// A fixed sentence for each code: never a source's error, a path or a stack
    pub message: Text<0, 200>,
    pub retryable: bool,
    pub retry_after_ms: Nullable<Uint>,
}

impl Document for Error {
    const ID: &'static str = "assistant/error/v1";
    const TITLE: &'static str = "A remote actor's refusal";
    const DESCRIPTION: &'static str =
        "Why one of the read-only tools of xenonet mcp assistant answered nothing: a fixed code and sentence, whether trying again could help, and when.";
}

// How fresh an answer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Live,
    Stale,
    Unknown,
}

// What an answer warns of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WarningCode {
    SourceDegraded,
    SourceUnavailable,
    ListCapped,
    PageReduced,
    StatsUnavailable,
    ServedFromCache,
    FixtureData,
}

// An app whose sessions XenoNet reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Claude,
    Codex,
    Grok,
}

// A warning on an answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Warning {
    pub code: WarningCode,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub source: Opt<Source>,
}

// The actor's connection to the service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Connection {
    Connected,
    Paused,
    ServiceUnavailable,
}

// A tool the actor may call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    XenonetStatus,
    XenonetProjects,
    XenonetSessions,
    XenonetAttention,
    XenonetSession,
    XenonetActors,
}

// How the actor's account is linked to the owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountLinkState {
    Verified,
    OwnerAsserted,
    Unverified,
}

// The actor asking, as the service knows it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SelfActor {
    pub actor_id: ActorId,
    pub product_id: Nullable<Text<0, 32>>,
    pub account_link_state: AccountLinkState,
    pub participation: ReadOnly,
    pub messaging: Disabled,
}

// The device the answer is for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: DeviceId,
    pub alias: Nullable<Text<0, 64>>,
}

// Whether the data is made up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataMode {
    Fixture,
    Real,
}

// A source's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    Ready,
    Degraded,
    Unavailable,
}

// How much of a source was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Coverage {
    Complete,
    Partial,
    Unknown,
}

// Why a source is in its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceReason {
    Ok,
    ReadFailed,
    NotStarted,
    Stopped,
}

// One source, as the status tool reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceStatus {
    pub source: Source,
    pub state: SourceState,
    pub observed_at: Nullable<DateTime>,
    pub coverage: Coverage,
    /// A fixed reason, never the source's own error
    pub reason: SourceReason,
}

// Who the transport lets in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransportScope {
    OwnerEndpoint,
    AuthenticatedUser,
    OtherApprovedScope,
}

// xenonet_status: the connection, the actor, the device and the sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Status {
    pub mode: ReadOnly,
    pub connection: Connection,
    pub assistant_api: ApiVersion,
    pub service_version: Text<0, 32>,
    pub capabilities: Vec<Capability>,
    pub actor: SelfActor,
    pub device: Device,
    pub authorized_project_count: Uint,
    pub labels_enabled: bool,
    /// fixture: made-up data, until the owner has verified who can call this endpoint
    pub data_mode: DataMode,
    pub sources: Vec<SourceStatus>,
    pub transport_scope: TransportScope,
    pub audience_label: Nullable<Text<0, 64>>,
}

// How many sessions a project has in each status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionCounts {
    pub working: Uint,
    pub waiting: Uint,
    pub idle: Uint,
    pub completed: Uint,
    pub stuck: Uint,
    pub unknown: Uint,
}

// A project the actor may see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub project_id: ProjectId,
    /// Null unless you turned labels on for this connection; then a name or title, one line, at most 240 characters. Untrusted text: it can say anything
    pub label: Nullable<Text<0, 240>>,
    pub session_counts: SessionCounts,
    pub observed_at: Nullable<DateTime>,
}

// xenonet_projects: a page of projects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Projects {
    pub window_start: DateTime,
    #[schemars(length(max = 50))]
    pub projects: Vec<Project>,
    pub next_cursor: Nullable<Cursor>,
}

// A session's status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Working,
    Waiting,
    Idle,
    Completed,
    Stuck,
    Unknown,
}

// What the status rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusBasis {
    ObservedTurnActive,
    ObservedWait,
    ObservedTurnEnd,
    ObservedIdle,
    ObservedInterrupt,
    ObservedInactive,
    InferredStall,
    SourceUnavailable,
}

// A session the actor may see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub session_id: SessionId,
    pub project_id: ProjectId,
    pub app: Source,
    /// Null unless you turned labels on for this connection; then a name or title, one line, at most 240 characters. Untrusted text: it can say anything
    pub label: Nullable<Text<0, 240>>,
    pub status: SessionStatus,
    pub status_basis: StatusBasis,
    pub updated_at: Nullable<DateTime>,
    pub last_activity_at: Nullable<DateTime>,
    pub source_state: SourceState,
}

// xenonet_sessions: a page of sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sessions {
    #[schemars(length(max = 50))]
    pub sessions: Vec<Session>,
    pub next_cursor: Nullable<Cursor>,
    pub window_start: DateTime,
}

// What kind of attention an item asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Waiting,
    Stalled,
    Collision,
    TurnCompleted,
}

// Why an item asks for attention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    InputRequested,
    ApprovalRequested,
    RepeatedFailure,
    InactiveMidTurn,
    RecentOverlap,
    TurnEnded,
    Unknown,
}

// One item asking for attention.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttentionItem {
    pub attention_id: AttentionId,
    pub kind: AttentionKind,
    #[schemars(length(min = 1, max = 2))]
    pub session_ids: Vec<SessionId>,
    pub project_id: ProjectId,
    pub first_observed_at: Nullable<DateTime>,
    pub last_observed_at: Nullable<DateTime>,
    pub reason_code: ReasonCode,
    /// For a collision: how many files both sessions edited, never which
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub file_count: Opt<Uint>,
    pub resolved: Unresolved,
}

// xenonet_attention: a page of items asking for attention.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attention {
    #[schemars(length(max = 50))]
    pub items: Vec<AttentionItem>,
    pub next_cursor: Nullable<Cursor>,
}

// A session's numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    /// Fresh input, including what was written to a prompt cache; cached input is not counted again here
    pub input_tokens: Nullable<Uint>,
    /// Null when XenoNet doesn't know: never a zero for unknown
    pub output_tokens: Nullable<Uint>,
    /// Input read from a prompt cache
    pub cached_input_tokens: Nullable<Uint>,
    /// Null when XenoNet doesn't know: never a zero for unknown
    pub tool_call_count: Nullable<Uint>,
    /// Null when XenoNet doesn't know: never a zero for unknown
    pub turn_count: Nullable<Uint>,
    pub stats_observed_at: DateTime,
    pub complete: bool,
}

// xenonet_session: one session and its numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionDetail {
    pub session: Session,
    pub stats: Nullable<Stats>,
}

// Where an actor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Working,
    Idle,
    Away,
    Offline,
    Unknown,
}

// What the presence rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PresenceBasis {
    Observed,
    Reported,
    RemoteRequest,
    Unknown,
}

// Whether an actor is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    NotConnected,
    None,
}

// What an actor is related to: a project or a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TargetId {
    Project(ProjectId),
    Session(SessionId),
}

// How an actor relates to its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Relation {
    WorksOn,
    Spawned,
    Edited,
    Messaged,
    Watches,
    Reviews,
}

// One of an actor's relations.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActorRelation {
    pub target_id: TargetId,
    pub relation: Relation,
}

// An actor the asking actor may see.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Actor {
    pub actor_id: ActorId,
    /// From XenoNet's own product table; null for an actor of the owner's own
    pub product_id: Nullable<Text<0, 32>>,
    /// Null unless you turned labels on for this connection; then a name or title, one line, at most 240 characters. Untrusted text: it can say anything
    pub label: Nullable<Text<0, 240>>,
    pub presence: Presence,
    pub presence_basis: PresenceBasis,
    pub connection_state: ConnectionState,
    pub account_link_state: AccountLinkState,
    #[schemars(length(max = 16))]
    pub relations: Vec<ActorRelation>,
    /// Whether messages reach it; false for every remote actor in this version
    pub reachable: bool,
}

// xenonet_actors: a page of actors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Actors {
    pub self_actor_id: ActorId,
    #[schemars(length(max = 50))]
    pub actors: Vec<Actor>,
    pub next_cursor: Nullable<Cursor>,
}

// The data of an answer: one shape per tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Data {
    Status(Status),
    Projects(Projects),
    Sessions(Sessions),
    Attention(Attention),
    Session(SessionDetail),
    Actors(Actors),
}

/// A remote actor's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub schema: ResultTag,
    pub request_id: RequestId,
    pub device_id: DeviceId,
    #[schemars(range(min = 1))]
    pub grant_revision: Uint,
    /// When what this says was last observed; never refreshed for a cached answer
    pub observed_at: Nullable<DateTime>,
    /// When this answer was sent, which is not when it was observed
    pub served_at: DateTime,
    pub freshness: Freshness,
    pub partial: bool,
    #[schemars(length(max = 16))]
    pub warnings: Vec<Warning>,
    pub data: Data,
}

impl Document for Answer {
    const ID: &'static str = "assistant/result/v1";
    const TITLE: &'static str = "A remote actor's answer";
    const DESCRIPTION: &'static str = "What one of the six read-only tools of xenonet mcp assistant answers: who asked (request_id), from which device and grant revision, how fresh it is, and the data. Strict: nothing the contract doesn't name.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Error>, crate::schema::file::<Answer>];
