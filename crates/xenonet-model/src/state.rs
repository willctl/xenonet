//! XenoNet's state: `state.json` as it was before it had a version
//! (`state/v1`) and as it is written now (`state/v2`).

use crate::identity::Detail;
use crate::num::{Ms, Uint};
use crate::project::{Role, RuleAs};
use crate::schema::{Any, Document, Opt, Record, True};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

crate::constant!(pub Version1 = 1);
crate::constant!(pub Version2 = 2);

// Where a node was dragged to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Position {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

// Your rule for a folder in the projects folder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub path: String,
    #[serde(rename = "as")]
    pub as_: RuleAs,
    pub at: Ms,
}

// One of Codex's picks: a folder, and what it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pick {
    /// Inside the projects folder, with forward slashes
    pub path: String,
    #[serde(rename = "as")]
    pub as_: Role,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reason: Opt<String>,
}

// A run of Codex over a projects folder, and its picks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CodexRun {
    pub ran_at: Ms,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub ms: Opt<f64>,
    /// Of the folders it was sent, to tell when they've changed
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub fingerprint: Opt<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub sent: Opt<Uint>,
    /// Who made them: codex, or a decision model by its id (core/decide.js)
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub by: Opt<String>,
    pub picks: Vec<Pick>,
}

// The projects folder's rules and Codex's picks, before picks were kept per root.
// (Inline, like every type reached through `Opt`: `Opt<T>` shares `T`'s
// schema id, so the generator would otherwise write a `$ref`.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct ProjectsV1 {
    /// Your rules for folders in the projects folder, by path key
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub rules: Opt<Record<Rule>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexRun>,
}

// The projects folder's rules and Codex's picks for each root.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Projects {
    /// Your rules for folders in the projects folder, by path key
    pub rules: Record<Rule>,
    /// Codex's picks for each projects folder it was asked about, by the folder's path key
    pub codex_by_root: Record<CodexRun>,
}

// A session or project tied to an identity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Pin {
    pub identity: String,
    pub at: Ms,
}

// An identity XenoNet has seen.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Known {
    pub kind: String,
    #[schemars(with = "Opt<Option<String>>")]
    pub handle: Option<String>,
    pub detail: Detail,
    pub first_seen: Ms,
    pub last_seen: Ms,
}

// Who an app was signed in as, from when to when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Signin {
    pub id: String,
    pub from: Ms,
    pub to: Ms,
}

// Identities: your names and pins, and the sign-ins XenoNet has seen.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct Identities {
    /// Names you gave identities, by id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub names: Opt<Record<String>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub hidden: Opt<Record<True>>,
    /// Sessions and projects you tied to an identity, by node id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub pins: Opt<Record<Pin>>,
    /// Every identity XenoNet has seen, by id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub known: Opt<Record<Known>>,
    /// Who each app was signed in as, over time, by app
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub signins: Opt<Record<Vec<Signin>>>,
}

/// state.json before it had a version. Every part is optional: XenoNet filled in what was missing when it read the file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StateV1 {
    /// Not written: a file without a version is v1
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub version: Opt<Version1>,
    /// Where you dragged nodes, by node id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub layout: Opt<Record<Position>>,
    /// When you dismissed a session's "needs you", by session id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub acks: Opt<Record<Ms>>,
    /// Once-a-day markers, by name
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub marks: Opt<Record<Any>>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub projects: Opt<ProjectsV1>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub identities: Opt<Identities>,
    /// The theme in use, by id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub theme: Opt<String>,
}

impl Document for StateV1 {
    const ID: &'static str = "state/v1";
    const TITLE: &'static str = "XenoNet's state, v1";
    const DESCRIPTION: &'static str = "state.json before it had a version. Every part is optional: XenoNet filled in what was missing when it read the file.";
}

// The projects folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Workspace {
    /// The projects folder; null for XENONET_PROJECTS_DIR, or else ~/Projects
    #[schemars(with = "Opt<Option<String>>")]
    pub root: Option<String>,
}

/// state.json: where you dragged nodes, the alerts you dismissed, the projects folder and how to read it (your rules, and Codex's picks for each root), identities (your names and pins, and the sign-ins XenoNet has seen), and the theme.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct State {
    pub version: Version2,
    pub workspace: Workspace,
    /// Where you dragged nodes, by node id
    pub layout: Record<Position>,
    /// When you dismissed a session's "needs you", by session id
    pub acks: Record<Ms>,
    /// Once-a-day markers, by name
    pub marks: Record<Any>,
    pub projects: Projects,
    pub identities: Identities,
    /// The theme in use, by id
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub theme: Opt<String>,
    /// The environment around the map, independent of the theme
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub environment: Opt<String>,
}

impl Document for State {
    const ID: &'static str = "state/v2";
    const TITLE: &'static str = "XenoNet's state";
    const DESCRIPTION: &'static str = "state.json: where you dragged nodes, the alerts you dismissed, the projects folder and how to read it (your rules, and Codex's picks for each root), identities (your names and pins, and the sign-ins XenoNet has seen), and the theme.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<StateV1>, crate::schema::file::<State>];
