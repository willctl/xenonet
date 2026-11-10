//! An app XenoNet knows sessions of, as the app registry lists it
//! (`app/v1`).

use crate::num::Uint;
use crate::schema::{Document, Opt, Record};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// A vendor product, <vendor>/<product>.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Product(#[schemars(regex(pattern = r"^[a-z0-9-]+\/[a-z0-9-]+$"))] pub String);

// A path under the home folder, spelled with ~.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct HomePath(#[schemars(regex(pattern = r"^~(\/.*)?$"))] pub String);

// Where the app keeps its files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Roots {
    /// The environment variable that moves its folder
    pub env: String,
    /// Its folder when the variable isn't set
    pub default: HomePath,
    /// Files the variable moves too, which are elsewhere when it isn't set: each file's name, and the folder it's in then
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub elsewhere: Opt<Record<HomePath>>,
}

// The kind of account an app runs under. (Inline, like every type reached
// through `Opt`: `Opt<T>` shares `T`'s schema id, so the generator would
// otherwise write a `$ref`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum IdentityKind {
    Claude,
    Chatgpt,
}

// Whose plan limits an app has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum Limits {
    Claude,
    Codex,
}

// Where an app keeps its hooks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum HookFormat {
    ClaudeSettings,
    CodexHooks,
    GrokHooks,
}

// A hook event XenoNet adds: its name, with the matcher it takes, if any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum HookEvent {
    #[schemars(extend("items" = false))]
    Event((String,)),
    #[schemars(extend("items" = false))]
    Matched((String, String)),
}

// How XenoNet hooks into the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Connect {
    /// Where and how XenoNet adds its hooks: Claude Code's settings.json, Codex's hooks.json, or Grok Build's hooks folder and config.toml
    pub format: HookFormat,
    /// The hook events XenoNet adds, in order, each with the matcher it takes, if any
    pub events: Vec<HookEvent>,
    /// The hook events whose answers it passes over: XenoNet tells it nothing then, and keeps its messages for an event it hears
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub unheard: Opt<Vec<String>>,
}

// An app as a candidate for a routing suggestion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Routing {
    /// Where it goes among the candidates, from 1
    #[schemars(range(min = 1))]
    pub order: Uint,
    /// What it's good for, as the decider reads it
    pub description: String,
}

// What no actor may pass for of an app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Reserved {
    /// The names it goes by, which no actor's name or product reads as
    pub names: Vec<String>,
    /// Its makers, which no actor's product reads as
    pub makers: Vec<String>,
    /// The actor slugs kept for it
    pub slugs: Vec<String>,
}

// How a handoff carries a session of an app or to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Handoff {
    /// The entries of one of its sessions' transcripts a handoff reads
    #[schemars(range(min = 1))]
    pub entries: Uint,
    /// The characters of where the work left off that a brief for it keeps
    #[schemars(range(min = 1))]
    pub clip: Uint,
}

// What XenoNet can do with an app.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Capability {
    Lines,
    Sessions,
    Native,
    Connect,
    Delivery,
    Processes,
    Caller,
    Open,
}

/// An app XenoNet knows sessions of, and what's fixed about it, as the app registry (core/apps.js) lists it. The apps XenoNet only observes have their id, name and numbat's names only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct App {
    /// What its session ids begin with: <id>:<the run's id>
    #[schemars(regex(pattern = r"^[a-z0-9][a-z0-9-]*$"))]
    pub id: String,
    /// What it's called
    pub name: String,
    /// The names numbat records its runs under
    pub numbat: Vec<String>,
    /// Its vendor products (<vendor>/<product>), each one of the provider drift check's, which lists their sources
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub products: Opt<Vec<Product>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub roots: Opt<Roots>,
    /// Its context window, in tokens, until a transcript says otherwise
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(range(min = 1))]
    pub context: Opt<Uint>,
    /// The kind of account it runs under, as identity/v1 has it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub identity: Opt<IdentityKind>,
    /// Its plan limits, as live/snapshot/v1's limits name them
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub limits: Opt<Limits>,
    /// The developer team that signs it, codesign's TeamIdentifier
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(regex(pattern = r"^[A-Z0-9]{10}$"))]
    pub team: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub connect: Opt<Connect>,
    /// For a native reader, the tools whose calls are edits
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub edit_tools: Opt<Vec<String>>,
    /// What XenoNet can do with it at most; what it can do with a session is the session's own
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub has: Opt<Vec<Capability>>,
    /// The map's stats count its sessions, under its id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub counted: Opt<bool>,
    /// The arguments that run one turn with the prompt after them, for an agent an OpenShell sandbox runs
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub headless: Opt<Vec<String>>,
    /// As a candidate for a routing suggestion: its place among the candidates and what it's good for
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub routing: Opt<Routing>,
    /// Its own server lists the MCP servers it runs and their catalogs, which XenoNet asks when one of its sessions uses a server
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub mcp_status: Opt<bool>,
    /// What no actor may pass for: the names it goes by and its makers, and the actor slugs kept for it
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reserved: Opt<Reserved>,
    /// Its sessions report what they spent, which the spend view shows under its name
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reports_spend: Opt<bool>,
    /// XenoNet runs turns of its own with it: the narration, and the sort of the projects folder into projects, whose picks name it as theirs
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sorts: Opt<bool>,
    /// How a handoff carries a session of it or to it, where that isn't 80 transcript entries read and 1500 characters kept of where the work left off
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub handoff: Opt<Handoff>,
}

impl Document for App {
    const ID: &'static str = "app/v1";
    const TITLE: &'static str = "An app";
    const DESCRIPTION: &'static str = "An app XenoNet knows sessions of, and what's fixed about it, as the app registry (core/apps.js) lists it. The apps XenoNet only observes have their id, name and numbat's names only.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<App>];
