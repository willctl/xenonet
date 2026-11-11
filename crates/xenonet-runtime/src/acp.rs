//! The apps XenoNet starts over ACP (the registry's `acp`): what the
//! starts and the router ask of the ACP side, which the composition root
//! brings (`Runtime::acp`), since the generic ACP client (xenonet-acp) is
//! an adapter crate this one doesn't depend on.
//!
//! A start runs here as any other does (decided, limited, its folder
//! checked), then the ACP side starts the app's agent, opens a session in
//! the folder and sends the first prompt as a turn. The session stays open
//! between turns, and a message routed to it is its next turn, after the
//! router's consent and wake checks. Its permission requests are asks
//! held here (`asks::hold_acp`).

use crate::runtime::Runtime;
use crate::starts::Answer;
use serde_json::Value;
use std::sync::Arc;
use xenonet_authz::Information;

/// A start over ACP, once this machine took it.
pub struct Start {
    /// The app's id, and its agent's command as its adapter found it.
    pub app: String,
    pub program: String,
    pub args: Vec<String>,
    /// The folder it starts in, and as answers show it (~ for home).
    pub cwd: String,
    pub folder: String,
    /// Who asked (`{ kind, id, title }`), from which machine (None: this
    /// one), and the delegation it runs under (the machine and day).
    pub by: Value,
    pub from: Option<String>,
    pub delegation: Option<(String, String)>,
    pub prompt: Information<String>,
    /// Whoever asked hears the session's turns end, and its close.
    pub answer: Answer,
}

/// A session started over ACP: its id on the map (`<app>:<the agent's
/// own id>`), and the agent's process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    pub session: String,
    pub pid: Option<u32>,
}

/// Why a start over ACP didn't open a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failed {
    /// The agent asks to be signed in, which is the person's to do.
    SignIn(String),
    /// It didn't start, or didn't speak ACP v1 as XenoNet does: the app's
    /// headless start, where it has one, may go instead.
    Unproven(String),
}

/// A message routed to a session XenoNet holds open, as the router has it:
/// its id, who it's from by name, how XenoNet knows, its text, and whether
/// it's yours.
pub struct Message<'a> {
    pub id: &'a str,
    pub from: &'a str,
    pub verified: Option<&'a str>,
    pub text: &'a str,
    pub user: bool,
}

/// What the composition root brings.
pub trait Acp: Send + Sync {
    /// Starts the session (blocking until it's open, or why not), then
    /// runs its first turn.
    fn start(&self, w: &Arc<Runtime>, start: Start) -> Result<Started, Failed>;
    /// Whether it holds `agent`'s session open, between its turns.
    fn holds(&self, agent: &str) -> bool;
    /// A message as the session's next turn: where it went, or why not.
    fn deliver(&self, agent: &str, message: Message<'_>) -> Result<String, String>;
}
