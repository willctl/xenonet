//! Actors (core/actors/): the list and its events, registrations and their
//! tokens, the reports an actor sends, waking actors, the runs an actor
//! reports from elsewhere, and what actors' work cost.

use super::{pending, Pending, R};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

const ACTORS: &str = "core/actors/index.js";
const REGISTRATION: &str = "core/actors/registration.js";
const REPORTS: &str = "core/actors/reports.js";
const WAKE: &str = "core/actors/wake.js";
const RUNS: &str = "core/actors/runs.js";
const SPEND: &str = "core/actors/spend.js";

#[async_trait]
pub trait Actors: Send + Sync {
    /// actor/list/v1.
    fn list(&self) -> R<Value> {
        pending(ACTORS)
    }
    /// `actors.snapshot(id, { view, counts })`: `view` "window" for the
    /// window's own view, `counts` false for a snapshot without its
    /// 24-hour counts. None when there's no such actor.
    fn snapshot(&self, _id: &str, _view: Option<&str>, _counts: bool) -> R<Option<Value>> {
        pending(ACTORS)
    }
    /// `actors.events({ since, only, view, limit })`: since None for the
    /// default.
    fn events(&self, _since: Option<f64>, _only: Option<&str>, _view: Option<&str>, _limit: f64) -> R<Value> {
        pending(ACTORS)
    }
    /// `actors.you.forget()`.
    fn you_forget(&self) -> R<()> {
        pending(ACTORS)
    }
    /// `actors.you.report(body)`: `{ accepted, dropped }`.
    fn you_report(&self, _body: &Value) -> R<Value> {
        pending(ACTORS)
    }
    /// `actors.you.window(state)`: shown, hidden or closed.
    fn you_window(&self, _state: &Value) {}
    /// `actors.pidsOf(id)`: the processes it named as its own.
    fn pids_of(&self, _id: &str) -> R<Vec<u32>> {
        pending(ACTORS)
    }
    fn tokens(&self, _now: f64) -> R<Value> {
        pending(ACTORS)
    }
    fn can(&self, _id: &str, _capability: &str) -> bool {
        false
    }
    /// `actors.info(id)`: its name and product.
    fn info(&self, _id: &str) -> Option<Value> {
        None
    }
    /// A message an actor sent, which becomes its said event.
    fn said(&self, _message: &Value) {}
    /// The map changed.
    fn refresh(&self) {}
    /// `actors.target(actor, id, relation)`.
    fn target(&self, _actor: &str, _id: &str, _relation: &str) -> R<()> {
        pending(ACTORS)
    }
    /// The actor-… simulations (dev only).
    fn simulate(&self, _kind: &str, _a: Option<String>, _b: Option<String>) -> R<Option<Value>> {
        pending(ACTORS)
    }
    fn is_demo(&self) -> bool {
        false
    }
    fn ids(&self) -> Vec<String> {
        Vec::new()
    }
    /// The made-up remote of a demo actor (its transports, whether active).
    fn remote_of(&self, _id: &str) -> Option<Value> {
        None
    }
    fn targets_of(&self, _id: &str) -> Vec<Value> {
        Vec::new()
    }
    /// `observers.status()`, for the doctor: what each observer reads.
    fn observers_status(&self) -> Value {
        Value::Array(Vec::new())
    }
    /// The doctor's look for actors' tokens left in repos
    /// (createTokenScan(…).look({ byHand })).
    async fn token_files(&self, _by_hand: bool) -> R<Value> {
        pending("core/actors/token-file.js")
    }
    fn start(&self) {}
    fn stop(&self) {}
}

#[async_trait]
pub trait Registration: Send + Sync {
    /// The actor a token belongs to: `{ ok, id, slug, state }` or
    /// `{ ok: false, code, error, reason? }`.
    fn authenticate(&self, _token: &str) -> Value {
        serde_json::json!({ "ok": false, "code": "pending", "error": format!("{REGISTRATION} isn't in the Rust service yet") })
    }
    /// The same for a gateway token, with `status` and `actor` on a refusal.
    fn authenticate_gateway(&self, _token: &str) -> Value {
        serde_json::json!({ "ok": false, "status": 501, "code": "pending", "error": format!("{REGISTRATION} isn't in the Rust service yet") })
    }
    fn product_of(&self, _auth: &Value) -> Value {
        Value::Null
    }
    fn register(&self, _body: &Value, _verdict: &Value) -> R<Value> {
        pending(REGISTRATION)
    }
    /// actor/me/v1, with `extra` (accepted, dropped) when there's some.
    fn me(&self, _auth: &Value, _extra: Option<Value>) -> R<Value> {
        pending(REGISTRATION)
    }
    fn remove(&self, _slug: &str, _reason: Option<&str>) -> R<Value> {
        pending(REGISTRATION)
    }
    fn add(&self, _body: &Value) -> R<Value> {
        pending(REGISTRATION)
    }
    fn decide(&self, _request: &str, _decision: &Value) -> R<Value> {
        pending(REGISTRATION)
    }
    fn rotate(&self, _slug: &str) -> R<Value> {
        pending(REGISTRATION)
    }
    fn drop_gateway(&self, _slug: &str) -> R<Value> {
        pending(REGISTRATION)
    }
    /// A gateway token for the actor, capabilities None for its own.
    fn gateway(&self, _slug: &str, _capabilities: Option<Value>) -> R<Value> {
        pending(REGISTRATION)
    }
    fn ask_gateway(&self, _slug: &str, _verdict: &Value) -> R<Value> {
        pending(REGISTRATION)
    }
    fn requests(&self) -> R<Value> {
        pending(REGISTRATION)
    }
    fn observers(&self) -> R<Value> {
        pending(REGISTRATION)
    }
    fn observe(&self, _product: &Value, _on: &Value) -> R<Value> {
        pending(REGISTRATION)
    }
    /// How many programs wait for you to allow them.
    fn waiting(&self) -> usize {
        0
    }
    /// actor-request (a made-up registration that waits; dev only).
    fn simulate(&self, _kind: &str, _a: Option<String>) -> R<Option<Value>> {
        pending(REGISTRATION)
    }
    fn tick(&self) {}
}

/// What an actor reports, taken (core/actors/reports.js), and the checks
/// on it (core/schema/actor.js, core/actors/detail.js, verify.ownOf).
#[async_trait]
pub trait Reports: Send + Sync {
    fn take_heartbeat(&self, _id: &str, _body: &Value) -> R<()> {
        pending(REPORTS)
    }
    /// `takeEvents(actors, id, list, { own, lineage, skip })`: `{ accepted,
    /// dropped }`; list items None where the route dropped one already.
    async fn take_events(&self, _id: &str, _list: Vec<Value>, _own: Option<Vec<u32>>, _skip: Vec<usize>) -> R<Value> {
        pending(REPORTS)
    }
    /// `takeTargets(actors, id, { targets, remove, replace }, { own, skip })`.
    async fn take_targets(&self, _id: &str, _targets: Vec<Value>, _remove: Vec<Value>, _replace: bool, _own: Option<Vec<u32>>, _skip: Vec<usize>) -> R<Value> {
        pending(REPORTS)
    }
    /// `statusLine(status)`: `{ ok, status }`.
    fn status_line(&self, _status: &Value) -> Value {
        serde_json::json!({ "ok": false })
    }
    /// `ReportEvent.safeParse(e)`: the event as parsed, or the first issue
    /// as `<path> <message>`.
    fn parse_event(&self, _event: &Value) -> Result<Value, String> {
        Err(format!("it can't be checked yet: {REPORTS} isn't in the Rust service"))
    }
    /// `ReportTarget.safeParse(t)`.
    fn parse_target(&self, _target: &Value) -> Result<Value, String> {
        Err(format!("it can't be checked yet: {REPORTS} isn't in the Rust service"))
    }
    /// `ownOf(chain)`: the processes below the first app in a verdict's chain.
    fn own_of(&self, _chain: &Value) -> Vec<u32> {
        Vec::new()
    }
}

/// An event for an actor's own stream: (event, data); the stream ends when
/// the sender goes.
pub type WakeStream = mpsc::Receiver<(String, Value)>;

#[async_trait]
pub trait Wakes: Send + Sync {
    fn list(&self) -> R<Value> {
        pending(WAKE)
    }
    /// `wakes.set(slug, { kind, url, key })`.
    async fn set(&self, _slug: &str, _spec: Value) -> R<Value> {
        pending(WAKE)
    }
    async fn test(&self, _slug: &str) -> R<Value> {
        pending(WAKE)
    }
    async fn forget(&self, _slug: &str) -> R<()> {
        pending(WAKE)
    }
    fn subscribe(&self, _id: &str) -> R<WakeStream> {
        pending(WAKE)
    }
    fn stop(&self) {}
}

/// Runs an actor reports from elsewhere (state.reportedRuns).
pub trait Runs: Send + Sync {
    /// `runs.report(actor, { run, app, title, presence })`: `{ ok, id }` or
    /// `{ ok: false, error }`.
    fn report(&self, _actor: &str, _run: Value) -> Value {
        serde_json::json!({ "ok": false, "error": format!("{RUNS} isn't in the Rust service yet") })
    }
}

/// What each actor's work cost today (core/actors/spend.js).
pub trait Spend: Send + Sync {
    fn actor_spend(&self) -> R<Value> {
        pending(SPEND)
    }
}

impl Actors for Pending {}
impl Registration for Pending {}
impl Reports for Pending {}
impl Wakes for Pending {}
impl Runs for Pending {}
impl Spend for Pending {}
