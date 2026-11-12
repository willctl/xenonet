//! What the engine keeps, as it asks for it: messages and who has seen
//! them, claims, what each agent says it's doing, when it last heard from
//! each agent, which process holds each session (pins), the actors'
//! targets, and delivery's policy, wake budget and audit.
//!
//! The engine defines this and depends on no store: the composition gives
//! it one (the service's `compose::runtime::store::Store`, over xenonet-store's tables).
//! A row comes back as a JSON object in its columns' order, every integer
//! a number, as the engine has always read them; a failure as the store
//! words it, with its code (`ERR_SQLITE_ERROR`) when it has one, which
//! the engine passes on to its caller as it came.

use crate::engine::Thrown;
use serde_json::{Map, Value};
use xenonet_model::ids::AgentId;

pub type Kept<T> = Result<T, Thrown>;

/// A process holding a session: its pid and start time, and whether the
/// session's own process said so (`sure`) or the engine guessed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pin {
    pub pid: f64,
    pub start: f64,
    pub sure: bool,
}

pub trait Store: Send + Sync {
    // ---------- messages ----------

    /// A message, delivered to its recipients: its id. `m` is `{ sender,
    /// target, scope, body, replyTo, recipients, envelope }`.
    fn send(&self, m: &Value, now: f64) -> Kept<f64>;
    fn deliver(&self, id: f64, agent: &str) -> Kept<()>;
    fn message(&self, id: f64) -> Kept<Option<Value>>;
    /// Messages to everyone in a repo since `since`, oldest first.
    fn scoped(&self, scope: &str, since: f64) -> Kept<Vec<Value>>;
    /// The messages to `agent` it hasn't seen, oldest first.
    fn unseen(&self, agent: &str) -> Kept<Vec<Value>>;
    fn mark_seen(&self, agent: &str, ids: &[f64], now: f64) -> Kept<()>;
    /// Who a message went to, and who has seen it: `{ agent, seen_at }`.
    fn deliveries(&self, id: f64) -> Kept<Vec<Value>>;
    /// The messages since `since`, oldest first.
    fn recent(&self, since: f64) -> Kept<Vec<Value>>;

    // ---------- claims ----------

    /// A claim: `{ project, path, agent, author, host, checkout, pattern,
    /// branch, note, created_at, expires_at }`, in place of the same
    /// agent's claim of that path in that project.
    fn claim(&self, row: &Value) -> Kept<()>;
    /// This machine's claims let go: all of an agent's (`patterns` None),
    /// or those of `patterns`. The rows that went.
    fn release(&self, agent: &str, patterns: Option<&[String]>) -> Kept<Vec<Value>>;
    /// The claims that haven't expired, earliest first, ties by author.
    fn claims(&self, now: f64) -> Kept<Vec<Value>>;
    /// The same, in one project.
    fn claims_in(&self, project: &str, now: f64) -> Kept<Vec<Value>>;

    // ---------- focus ----------

    /// What an agent says it's doing; an empty text takes it away.
    fn set_focus(&self, agent: &str, text: &str, now: f64) -> Kept<()>;
    /// `{ text, at }`, or None.
    fn focus(&self, agent: &str) -> Kept<Option<Value>>;

    // ---------- contacts ----------

    /// An agent heard from directly: `contact` is `{ app, cwd, via, pid }`.
    fn touch(&self, agent: &str, contact: &Value, now: f64) -> Kept<()>;
    fn ended(&self, agent: &str, now: f64) -> Kept<()>;
    fn contact(&self, agent: &str) -> Kept<Option<Value>>;
    /// The contacts heard from since `since`.
    fn contacts(&self, since: f64) -> Kept<Vec<Value>>;

    // ---------- agents ----------

    /// This machine's session's own id (`agt_…`), when it has one.
    fn agent_known(&self, session: &str) -> Kept<Option<String>>;
    /// The session an agent of this machine's is, `<app>:<vendor id>`.
    fn session_of(&self, agent: &AgentId) -> Kept<Option<String>>;

    // ---------- pins ----------

    fn pins(&self, agent: &str) -> Kept<Vec<Pin>>;
    fn pin(&self, agent: &str, pid: f64, start: f64, sure: bool, now: f64) -> Kept<()>;
    fn unpin(&self, agent: &str, pid: f64, start: f64) -> Kept<()>;
    /// Takes back the guesses for an agent once its own process has spoken.
    fn drop_guesses(&self, agent: &str) -> Kept<()>;

    // ---------- actors ----------

    /// What the actors said they act on.
    fn actor_targets(&self) -> Kept<Vec<Value>>;

    // ---------- delivery ----------

    /// The policy's data as it was last kept (`{ paused, consent, … }`), or
    /// None when nothing was.
    fn delivery_policy(&self) -> Kept<Option<Map<String, Value>>>;
    fn put_delivery_policy(&self, data: &Map<String, Value>) -> Kept<()>;
    /// The wake budget's times, by its key.
    fn wake_budget(&self) -> Kept<Vec<(String, Vec<f64>)>>;
    fn put_wake_budget(&self, budget: &[(String, Vec<f64>)]) -> Kept<()>;
    /// A line of the audit log, which keeps the newest lines only.
    fn add_delivery_audit(&self, line: &Value) -> Kept<()>;
    /// The audit log's lines kept, oldest first.
    fn delivery_audit(&self) -> Kept<Vec<Value>>;
}
