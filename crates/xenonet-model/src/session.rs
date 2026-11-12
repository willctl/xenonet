//! One run of a harness, as XenoNet lists it (`session/v1`).

use crate::num::Ms;
use crate::schema::{Document, Nullable, Opt};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// The harness a session ran in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum App {
    Claude,
    Codex,
    Grok,
    GeminiCli,
    Cursor,
    Copilot,
    Vscode,
    Opencode,
    Openclaw,
    Antigravity,
    Factory,
    DevinCli,
    KimiCode,
    Pi,
    QwenCode,
    Cline,
    Amp,
    Auggie,
    Kiro,
    Goose,
    Kilo,
    Openhands,
    Crush,
    Junie,
    Windsurf,
    Cowork,
}

// How a session came from its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParentKind {
    Spawn,
    Fork,
    Subagent,
}

// The Claude account a session is filed under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClaudeAccount {
    pub account: String,
    pub org: String,
}

// How XenoNet knows of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Observed {
    Numbat,
    Gateway,
}

// Which reader read a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Reader {
    Experimental,
}

// A session's tokens, as its app counted them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub input: Nullable<f64>,
    pub output: Nullable<f64>,
    pub reasoning: Nullable<f64>,
    pub cache_read: Nullable<f64>,
    pub cache_write: Nullable<f64>,
}

// What a session used, as its app reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// What the app totalled for the whole session, in USD; null where it doesn't say. Not XenoNet's estimate at API prices
    pub cost_usd: Nullable<f64>,
    pub tokens: UsageTokens,
    pub provider: Nullable<String>,
    pub model: Nullable<String>,
}

/// One run of a harness.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// <app>:<the app's own id>, the id the rest of XenoNet uses on this machine
    #[schemars(regex(
        pattern = r"^(antigravity|gemini-cli|devin-cli|kimi-code|qwen-code|openhands|opencode|openclaw|windsurf|copilot|factory|claude|cursor|vscode|auggie|cowork|codex|cline|goose|crush|junie|grok|kiro|kilo|amp|pi):."
    ))]
    pub id: String,
    pub app: App,
    /// The app's own id: the Agent SDK's session id, or Codex's thread id
    pub raw_id: String,
    pub title: String,
    /// The first prompt, or Codex's preview
    pub preview: Nullable<String>,
    /// Where it ran; a Claude desktop worktree session gives the folder it branched from
    pub cwd: Nullable<String>,
    pub created_at: Nullable<Ms>,
    pub updated_at: Nullable<Ms>,
    pub archived: bool,
    pub model: Nullable<String>,
    pub branch: Nullable<String>,
    /// The session this one was spawned or forked from
    pub parent_id: Nullable<String>,
    pub parent_kind: Nullable<ParentKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Claude desktop's own id for it, local_…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_id: Option<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub in_desktop: Opt<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub starred: Opt<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<f64>,
    /// The Claude account and organization Claude desktop filed it under
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_account: Option<ClaudeAccount>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// The model provider, when it isn't your ChatGPT account's
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The repo's origin remote
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// What started it: cli, vscode, exec, subAgent…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub originator: Option<String>,
    /// The Codex desktop project it belongs to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_nickname: Option<String>,
    /// For a Codex sub-agent thread: spawned, or the kind of background work; for a Grok Build subagent, its type (general-purpose, explore…)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,
    /// Background work (codex exec, reviews, compaction) the map leaves out unless asked
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub background: Opt<bool>,
    /// How XenoNet knows of it: numbat's event records, or an actor's report through the gateway
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub observed: Opt<Observed>,
    /// The actor that reported it through the gateway
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_by: Option<String>,
    /// Read from the harness's own records by an experimental reader (core/native)
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub reader: Opt<Reader>,
    /// Reported by the app's own records (OpenCode's session row), whole session
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    pub usage: Opt<Usage>,
}

impl Document for Session {
    const ID: &'static str = "session/v1";
    const TITLE: &'static str = "Session";
    const DESCRIPTION: &'static str = "One run of a harness (a Claude Code session, a Codex thread, a Gemini CLI run…), as XenoNet lists it. The map's session nodes (graph/v1) are these with where they live and what they're doing added.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Session>];
