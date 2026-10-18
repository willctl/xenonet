//! What the Agent SDK reads and writes of Claude Code's own files, which
//! the JS reaches through @anthropic-ai/claude-agent-sdk (listSessions,
//! getSessionMessages, renameSession): the transcripts under
//! ~/.claude/projects/<folder>/<session>.jsonl.

use serde_json::Value;

/// The Agent SDK release whose readings these are, as the lenient
/// readers pick their schema by it (package.json's).
pub const VERSION: &str = "0.3.287";

/// listSessions(): every session's listing.
pub fn list_sessions(_projects_dir: &str) -> Vec<Value> {
    Vec::new()
}

/// getSessionMessages(id): a session's messages, in order.
pub fn session_messages(_projects_dir: &str, _id: &str) -> Vec<Value> {
    Vec::new()
}

/// renameSession(id, title).
pub fn rename_session(_projects_dir: &str, _id: &str, _title: &str) -> Result<(), String> {
    Err("XenoNet can't rename a Claude Code session here yet".into())
}
