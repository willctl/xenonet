//! What the agents are doing now: one thing that happened in a session
//! (`live/activity/v1`), one session as the tailer sees it
//! (`live/session/v1`) and the whole live state (`live/snapshot/v1`).

use crate::num::{Int, Ms, Uint};
use crate::schema::{Any, Document, Nullable, Opt, Record, Ref, True};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// An app the tailer follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum App {
    Claude,
    Codex,
    Grok,
}

// What kind of tool a call was.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ToolCategory {
    Mcp,
    Shell,
    Edit,
    Read,
    Web,
    Agent,
    Code,
    Other,
}

// Where the name of an MCP server with a uuid for an id came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NameSource {
    Declared,
    Instructions,
    Results,
}

// ---- live/activity/v1 ----

// Which session something happened in, and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub session_id: String,
    pub app: App,
    pub ts: Ms,
    /// The host it's from, for a session on another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
}

// A tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    #[serde(flatten)]
    pub origin: Origin,
    pub tool: String,
    pub category: ToolCategory,
    pub detail: String,
    pub files: Vec<String>,
    /// The MCP server, for an MCP tool
    pub server: Nullable<String>,
    /// The subagent that called it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// The context pages a write to the shared context names: a slug, or <space>/<slug> for a page of another Xenolith space than this machine's
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(max = 6))]
    pub pages: Opt<Vec<String>>,
}

// Something with a text: a prompt, a message, a question, a refusal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Note {
    #[serde(flatten)]
    pub origin: Origin,
    pub detail: String,
}

// A turn ending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TurnEnd {
    #[serde(flatten)]
    pub origin: Origin,
    pub duration: Nullable<f64>,
    /// You stopped it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub interrupted: Opt<True>,
}

// A tool call that failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    #[serde(flatten)]
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

// A subagent starting or finishing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SubagentEvent {
    #[serde(flatten)]
    pub origin: Origin,
    pub agent_id: String,
    pub label: String,
}

// A Claude Code hook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Hook {
    #[serde(flatten)]
    pub origin: Origin,
    /// The hook's event name
    pub detail: String,
}

// A nudge to commit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShipIt {
    #[serde(flatten)]
    pub origin: Origin,
    pub detail: String,
    pub project_id: String,
}

// An app one signs in to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SignInApp {
    Claude,
    Chatgpt,
    Gh,
}

// A sign-in that changed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SignIn {
    pub app: SignInApp,
    pub identity: Nullable<String>,
    pub before: Nullable<String>,
    pub ts: Ms,
}

/// One thing that happened in a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Activity {
    /// A tool call
    Tool(ToolCall),
    /// A prompt, the start of a turn
    Prompt(Note),
    /// Something the agent said
    Message(Note),
    /// A Codex turn starting
    TurnStart(Origin),
    /// A turn ending
    TurnEnd(TurnEnd),
    /// A question for you, or a plan to approve
    NeedsYou(Note),
    /// A tool call that failed
    ToolError(ToolError),
    /// The provider refused the turn (core/refusals); the detail is its own words or category
    Refusal(Note),
    /// A subagent starting
    SubagentStart(SubagentEvent),
    /// A subagent finishing
    SubagentStop(SubagentEvent),
    /// A Claude Code hook
    Hook(Hook),
    /// A nudge to commit: a turn ended in a repo with uncommitted work (not kept in the history)
    ShipIt(ShipIt),
    /// A sign-in that changed (not kept in the history)
    Identity(SignIn),
}

impl Document for Activity {
    const ID: &'static str = "live/activity/v1";
    const TITLE: &'static str = "Activity";
    const DESCRIPTION: &'static str = "One thing that happened in a session, as the activity feed, the replay and the history get it.";
}

// ---- live/session/v1 ----

// What a session is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Tool,
    Thinking,
    Idle,
}

// Tokens over a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tokens {
    pub processed: f64,
    pub output: f64,
}

// How full the context is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Context {
    pub used: f64,
    pub window: f64,
}

// Where the context is heading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Forecast {
    /// How full the context is, 0 to 1
    pub pct: f64,
    pub growth_per_turn: Nullable<f64>,
    /// Turns until 90% full, at its usual growth
    pub turns_left: Nullable<Int>,
}

// The last tool a session called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LastTool {
    pub name: String,
    pub category: ToolCategory,
    pub detail: String,
    pub ts: Ms,
}

// A subagent of a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Subagent {
    pub id: String,
    pub label: String,
    pub started_at: Ms,
    pub last_at: Ms,
    pub tools: Uint,
    pub done: bool,
    pub active: bool,
}

// A file a session touched, and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FileTouch {
    pub path: String,
    pub ts: Ms,
}

// An MCP server a session called, and when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct McpCall {
    pub server: String,
    pub ts: Ms,
}

// What a session waits on you for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum NeedKind {
    Permission,
    Question,
    Review,
}

// A session waiting on you.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NeedsYou {
    pub kind: NeedKind,
    pub text: String,
    pub since: Ms,
    /// From a host that isn't live: when it last was
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub as_of: Nullable<Ms>,
}

// Why a session seems stuck.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Stuck {
    #[schemars(length(min = 1))]
    pub reasons: Vec<String>,
}

/// One session as the tailer sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    pub id: String,
    pub app: App,
    /// A tool call in flight, a turn under way, or neither
    pub status: SessionStatus,
    /// Over the last 24 hours of its transcript
    pub tokens: Tokens,
    pub tokens_today: f64,
    /// Tokens in the last minute
    pub tpm: f64,
    pub context: Context,
    pub forecast: Forecast,
    pub model: Nullable<String>,
    pub turn_started_at: Nullable<Ms>,
    pub last_tool: Nullable<LastTool>,
    /// Tool calls by category
    pub tool_counts: Record<Uint>,
    #[schemars(length(max = 12))]
    pub subagents: Vec<Subagent>,
    /// Files it touched in the last 20 minutes, newest first
    #[schemars(length(max = 10))]
    pub files: Vec<FileTouch>,
    pub mcp: Vec<McpCall>,
    /// Waiting on you; left out once you've seen it, and for archived and background sessions
    pub needs_you: Nullable<NeedsYou>,
    pub stuck: Nullable<Stuck>,
    pub last_message: Nullable<String>,
    pub last_turn_end_at: Nullable<Ms>,
    /// When you stopped its last turn (a Claude interruption, or a Codex turn you aborted), until its next prompt: it waits on you, but isn't ready for review
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub interrupted_at: Nullable<Ms>,
    pub last_event_at: Ms,
    /// How it asks before it acts (default, acceptEdits, plan, bypassPermissions…), as its hooks last said
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub permission_mode: Opt<String>,
    /// The host it's from, for a session on another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
    /// Its host isn't live: it's shown idle, as of when the host last was
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub stale: Opt<bool>,
}

impl Document for Session {
    const ID: &'static str = "live/session/v1";
    const TITLE: &'static str = "A live session";
    const DESCRIPTION: &'static str =
        "One session as the tailer sees it: what it's doing, its tokens and context, its tools, files and subagents, and whether it needs you or seems stuck.";
}

// ---- live/snapshot/v1 ----

// Tokens a second, by app.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Series {
    pub claude: Vec<f64>,
    pub codex: Vec<f64>,
    pub grok: Vec<f64>,
}

// One of Codex's rate-limit windows.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexWindow {
    /// Used, 0 to 100
    pub pct: Nullable<f64>,
    /// How long the window is
    pub window_minutes: Nullable<f64>,
    /// When it resets
    pub resets_at: Nullable<Ms>,
}

// Codex's rate limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexLimits {
    pub primary: Nullable<CodexWindow>,
    pub secondary: Nullable<CodexWindow>,
    pub observed_at: Ms,
    /// The identity it was read for (claude:…, chatgpt:…, openai:api-key), the one signed in now; null when XenoNet doesn't know it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The host it was read on, when that's another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
}

// When Claude's windows reset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeResets {
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub five_hour: Nullable<Ms>,
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub seven_day: Nullable<Ms>,
}

// Where a Claude plan reading came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ClaudeSource {
    Anthropic,
    ClaudeDesktop,
}

// A Claude plan reading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeReading {
    pub five_hour: Nullable<f64>,
    pub seven_day: Nullable<f64>,
    pub resets: ClaudeResets,
    pub observed_at: Ms,
    pub source: ClaudeSource,
    /// The identity it was read for (claude:…, chatgpt:…, openai:api-key), the one signed in now; null when XenoNet doesn't know it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The host it was read on, when that's another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
}

// A Claude plan reading that's run out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeStale {
    pub five_hour: (),
    pub seven_day: (),
    pub observed_at: Nullable<Ms>,
    pub stale: True,
    pub reason: Nullable<String>,
    /// The identity it was read for (claude:…, chatgpt:…, openai:api-key), the one signed in now; null when XenoNet doesn't know it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The host it was read on, when that's another machine
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub host: Opt<String>,
}

// Claude plan usage: a reading, or none recent enough.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ClaudeLimits {
    Reading(ClaudeReading),
    /// No reading recent enough: when the last one was, and why there's none since
    Stale(ClaudeStale),
}

// A reading of an account signed in only on another machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OtherReading {
    pub app: App,
    pub identity: Nullable<String>,
    pub host: String,
    pub reading: Record<Any>,
}

// Plan usage, by app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Limits {
    /// Codex's rate limits, as its last token count reported them, and when
    pub codex: Nullable<CodexLimits>,
    /// Claude plan usage, in percent
    pub claude: Nullable<ClaudeLimits>,
    /// With hosts on: readings of accounts signed in only on other machines, the newest of each, as limits.claude and limits.codex have them
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub others: Opt<Vec<OtherReading>>,
}

// Where Codex's limit is heading.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexForecast {
    /// Used, 0 to 100
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pct: Opt<f64>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub per_hour: Opt<f64>,
    /// When it runs out at this pace
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub eta: Nullable<Ms>,
    #[serde(default, skip_serializing_if = "Nullable::is_none")]
    pub resets_at: Nullable<Ms>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub hits_before_reset: Opt<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<f64>,
}

// Where one of Claude's windows is heading.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeForecast {
    /// Used, 0 to 100
    pub pct: f64,
    pub per_hour: f64,
    /// When it runs out at this pace
    pub eta: Nullable<Ms>,
    pub resets_at: Nullable<Ms>,
    pub hits_before_reset: bool,
}

// Where the limits are heading.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LimitForecast {
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub codex: Opt<CodexForecast>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_five_hour: Option<ClaudeForecast>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_seven_day: Option<ClaudeForecast>,
}

// Today's cost, by app.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CostByApp {
    /// Dollars; null when the app's calls today have no price
    pub claude: Nullable<f64>,
    /// Dollars; null when the app's calls today have no price
    pub codex: Nullable<f64>,
    /// Dollars; null when the app's calls today have no price
    pub grok: Nullable<f64>,
}

// What today's calls would cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Costs {
    /// Dollars; null when no call today has a price, so what it cost isn't known
    pub today: Nullable<f64>,
    pub by_app: CostByApp,
    /// Dollars an hour at this minute's pace
    pub per_hour: f64,
    /// Models used today with no price: none in XenoNet's table, and none their app said
    pub unpriced: Vec<String>,
    /// When the prices were last checked against the providers' (YYYY-MM-DD)
    pub checked: String,
}

crate::tag!(pub FileTag = "file");

// Two sessions that touched the same files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Collab {
    pub a: String,
    pub b: String,
    pub kind: FileTag,
    pub files: Vec<String>,
}

// Two sessions that both edited a file lately.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Collision {
    pub a: String,
    pub b: String,
    pub files: Vec<String>,
    pub paths: Vec<String>,
    pub ts: Ms,
    #[schemars(extend("items" = false))]
    pub branches: (Option<String>, Option<String>),
    pub same_branch: bool,
    #[schemars(extend("items" = false))]
    pub titles: (String, String),
}

// An MCP server sessions share, and when it was last called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resource {
    pub server: String,
    pub ts: Ms,
    /// What an MCP server with a uuid for an id is called: its name in Claude desktop, or what its instructions or results say
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub name: Opt<String>,
    /// Where the name came from: Claude desktop's list of connectors, or the server's own instructions or results
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub name_source: Opt<NameSource>,
}

// How the tailer is doing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Tailer {
    pub watching: bool,
    pub files: Uint,
    pub lines: Uint,
    pub errors: Uint,
}

/// What the agents are doing now.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub at: Ms,
    /// Sessions with anything in the last 24 hours
    pub sessions: Vec<Ref<Session>>,
    /// Tokens a second over the last five minutes, oldest first
    pub series: Series,
    pub tpm: f64,
    #[serde(rename = "total24h")]
    pub total_24h: f64,
    pub today: f64,
    pub limits: Limits,
    pub limit_forecast: LimitForecast,
    /// What today's calls would cost at API prices (core/pricing.js), or at what their app said they cost (Grok Build's), in dollars
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub costs: Opt<Costs>,
    /// Sessions that touched the same files lately
    pub collab: Vec<Collab>,
    /// Sessions on the map that both edited a file in the last ten minutes
    pub collisions: Vec<Collision>,
    #[schemars(length(max = 8))]
    pub resources: Vec<Resource>,
    pub status: Tailer,
}

impl Document for Snapshot {
    const ID: &'static str = "live/snapshot/v1";
    const TITLE: &'static str = "Live state";
    const DESCRIPTION: &'static str =
        "What the agents are doing now: each session's live state, token rates, plan usage and its forecast, and the sessions working on the same files.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Activity>, crate::schema::file::<Session>, crate::schema::file::<Snapshot>];
