//! A project, or a group of projects, as the scan of the projects folder
//! finds it (`project/v1`).

use crate::num::Ms;
use crate::schema::{Document, Nullable, Opt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// Whether the folder is a project of its own or a group of projects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Project,
    Group,
}

/// What made it one: a project's own repo, a rule, being a submodule, a workspace file, sessions run in it, or being a plain top-level folder; a group's projects (contains) or a rule
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Detected {
    Repo,
    Rule,
    Submodule,
    Workspace,
    Sessions,
    Folder,
    Contains,
}

// What git says about the folder.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Git {
    #[schemars(with = "Opt<Option<String>>")]
    pub branch: Option<String>,
    /// The origin remote's URL
    #[schemars(with = "Opt<Option<String>>")]
    pub origin: Option<String>,
    /// For a linked worktree, the folder of the repo it belongs to
    #[schemars(with = "Opt<Option<String>>")]
    pub worktree_of: Option<String>,
    #[schemars(with = "Opt<Option<String>>")]
    pub submodule_of: Option<String>,
}

// What a rule makes of a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RuleAs {
    Project,
    Group,
    Ignore,
}

// Who set a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RuleBy {
    You,
    Codex,
}

/// The rule that applies to its folder: yours, or one of Codex's picks
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub path: String,
    #[serde(rename = "as")]
    pub as_: RuleAs,
    pub by: RuleBy,
    /// When you set it; yours only
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub at: Opt<f64>,
    /// Why Codex picked it; Codex's only
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reason: Opt<String>,
}

/// A project, or a group of projects, in the projects folder, as XenoNet's scan finds it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub name: String,
    /// The folder, spelled as on disk
    pub path: String,
    /// The folder's path key (core/paths.js): forward slashes, lowercase, no trailing slash
    pub key: String,
    /// The folder inside the projects folder, with forward slashes
    pub rel: String,
    pub role: Role,
    pub detected: Detected,
    // A required field that may be null is written through `Opt<Option<T>>`
    // (a `type` list, as the file has it; `Nullable<T>` is the `anyOf`
    // form, for a value with a description), since a plain `Option` would
    // leave the key optional.
    /// The file that marks it: a version control folder (.git, .hg, …) or a workspace file
    #[schemars(with = "Opt<Option<String>>")]
    pub marker: Option<String>,
    /// The key of the group or project it's inside
    #[schemars(with = "Opt<Option<String>>")]
    pub parent: Option<String>,
    #[schemars(with = "Opt<Option<Git>>")]
    pub git: Option<Git>,
    /// What it's built with, from marker files: node, typescript, python…
    pub stack: Vec<String>,
    #[schemars(with = "Opt<Option<String>>")]
    pub description: Option<String>,
    pub modified_at: Nullable<Ms>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
}

impl Document for Project {
    const ID: &'static str = "project/v1";
    const TITLE: &'static str = "Project";
    const DESCRIPTION: &'static str = "A project, or a group of projects, in the projects folder, as XenoNet's scan finds it.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Project>];
