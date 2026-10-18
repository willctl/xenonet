//! XenoNetState (core/state.js), as server/index.js and its routes use it:
//! the map, the live state, search, stats, transcripts, the projects
//! folder and identities. Its events (pulse, activity, refusal, tokens,
//! graph, live) go to the service's Events, which the real core is given.

use super::{pending, Pending, R};
use async_trait::async_trait;
use serde_json::Value;

const M: &str = "core/state.js";

#[async_trait]
pub trait State: Send + Sync {
    /// `state.sources`: each source's name, ok and count.
    fn sources(&self) -> R<Value> {
        pending(M)
    }
    /// `state.blocks.health()`, each without its budget.
    fn blocks(&self) -> R<Value> {
        pending(M)
    }
    /// `state.graph`, null before the first refresh.
    fn graph(&self) -> R<Option<Value>> {
        pending(M)
    }
    async fn refresh(&self) -> R<Value> {
        pending(M)
    }
    fn live_snapshot(&self) -> R<Value> {
        pending(M)
    }
    /// `state.live.historySince(since)`.
    fn history_since(&self, _since: f64) -> R<Value> {
        pending(M)
    }
    async fn search_transcripts(&self, _q: &str) -> R<Value> {
        pending(M)
    }
    /// `state.search.status`.
    fn search_status(&self) -> R<Value> {
        pending(M)
    }
    /// `state.index.get(id)`.
    fn node(&self, _id: &str) -> R<Option<Value>> {
        pending(M)
    }
    /// `[...state.index.values()]`, in the index's order.
    fn nodes(&self) -> R<Vec<Value>> {
        pending(M)
    }
    /// `state.homeOf(id)`: the project or zone a session counts under.
    fn home_of(&self, _id: &str) -> R<Value> {
        pending(M)
    }
    fn title_of(&self, _id: Option<&str>) -> R<Value> {
        pending(M)
    }
    /// `state.live.collisionDiffs(a, b)`.
    fn collision_diffs(&self, _a: Option<&str>, _b: Option<&str>) -> R<Value> {
        pending(M)
    }
    async fn handoff(&self, _node: Option<&str>, _target: &str) -> R<Value> {
        pending(M)
    }
    fn digest(&self, _since: f64) -> R<Value> {
        pending(M)
    }
    fn day_log(&self) -> R<Value> {
        pending(M)
    }
    fn constellation(&self, _names: bool) -> R<Value> {
        pending(M)
    }
    fn export_labels(&self) -> R<Value> {
        pending(M)
    }
    /// The words for `kind` (day or away) since then.
    async fn narrate(&self, _kind: &str, _since: f64) -> R<Value> {
        pending(M)
    }
    async fn project_detection(&self) -> R<Value> {
        pending(M)
    }
    async fn set_project_rule(&self, _dir: Value, _as: Value) -> R<Value> {
        pending(M)
    }
    async fn clear_project_rules(&self, _all: bool) -> R<Value> {
        pending(M)
    }
    async fn rescan_projects(&self) -> R<Value> {
        pending(M)
    }
    async fn codex_outline(&self) -> R<Value> {
        pending(M)
    }
    /// `state.askCodexAboutProjects(fingerprint, { decider })`, with the
    /// decision model when `via` is the decider.
    async fn ask_codex_about_projects(&self, _fingerprint: Value, _decider: bool) -> R<Value> {
        pending(M)
    }
    async fn forget_codex_picks(&self) -> R<Value> {
        pending(M)
    }
    fn identity_detection(&self) -> R<Value> {
        pending(M)
    }
    async fn name_identity(&self, _id: Value, _name: Value) -> R<Value> {
        pending(M)
    }
    async fn hide_identity(&self, _id: Value, _hidden: Value) -> R<Value> {
        pending(M)
    }
    async fn pin_identity(&self, _node: Value, _identity: Value) -> R<Value> {
        pending(M)
    }
    async fn rescan_identities(&self) -> R<Value> {
        pending(M)
    }
    fn summary(&self) -> R<Value> {
        pending(M)
    }
    /// A session's stats, None when its transcript can't be found.
    async fn session_stats(&self, _id: &str) -> R<Option<Value>> {
        pending(M)
    }
    async fn read_transcript(&self, _node: &Value, _cursor: Option<&str>, _limit: f64) -> R<Value> {
        pending(M)
    }
    async fn rename_session(&self, _node: Value, _title: Value) -> R<Value> {
        pending(M)
    }
    /// `state.live.noteHook(id, body)`, a Claude Code hook's event.
    fn note_hook(&self, _id: &str, _body: &Value) -> R<()> {
        pending(M)
    }
    fn root_info(&self) -> R<Value> {
        pending(M)
    }
    async fn set_root(&self, _path: Value) -> R<Value> {
        pending(M)
    }
    async fn check_root(&self, _path: Value) -> R<Value> {
        pending(M)
    }
    /// The live state's own simulations (needs-you, collision, stuck,
    /// turn-end, wiki-write, and clear's part), from server/index.js's
    /// simulate(): what it did, or None for no such session.
    fn simulate(&self, _kind: &str, _a: Option<String>, _b: Option<String>) -> R<Option<Value>> {
        pending(M)
    }
    /// Whether a live session waits on you now (Spotify's dip).
    fn someone_waits(&self, _now: f64) -> R<bool> {
        pending(M)
    }
    /// `state.refreshAfterChange()`, after a run an actor reported.
    async fn refresh_after_change(&self) -> R<()> {
        pending(M)
    }
    // ---------- what the other cores read of the map ----------

    /// The live sessions' ids, each with its last event (the engine's
    /// liveIds).
    fn live_ids(&self) -> R<Vec<(String, Value)>> {
        pending(M)
    }
    /// A live session as the engine reads it: `{ observed, lastEventAt,
    /// endedAt, status, needsYou (its kind), edits: [{ path, ts }] }`, the
    /// observed runs' too; None when it isn't live.
    fn live_info(&self, _id: &str, _now: f64) -> R<Option<Value>> {
        pending(M)
    }
    /// `!!state.observed?.live?.(id)`.
    fn observed_live(&self, _id: &str) -> R<bool> {
        pending(M)
    }
    /// `state.graph?.identities ?? []`.
    fn identities(&self) -> R<Vec<Value>> {
        pending(M)
    }
    /// mapOf's recentSession(): the live session on the map last at work,
    /// else the first session on the map.
    fn recent_session(&self) -> R<Option<String>> {
        pending(M)
    }
    /// The readers' health as the assistant's source reads it: `{ claude,
    /// codex }`, each sourceOf(state.claudeSessions / codexThreads, the
    /// live tailer).
    fn assistant_sources(&self, _now: f64) -> R<Value> {
        pending(M)
    }
    /// How many times the map has been made: a cache keyed on the graph
    /// (mapOf's places and sessions) is good while this stays the same.
    fn graph_version(&self) -> u64 {
        0
    }
    /// The layout and identities a hub sends with this machine's map
    /// (store.getLayout, store.getIdentities), and the layout's ids under a
    /// prefix taken away (forgetLayout): how many went.
    fn forget_layout(&self, _prefix: &str) -> R<usize> {
        pending(M)
    }

    /// `state.start()`: the readers' first refresh and their watches.
    async fn start(&self) -> R<()> {
        pending(M)
    }
    fn stop(&self) {}
}

impl State for Pending {}
