//! The service's other parts: the doctor and the diagnostics bundle, the
//! drift report, Spotify, the decision model and OpenRouter's spend, the
//! system panel, refusals, integrations, MCP servers' catalog, and the
//! terminals the component service owns.

use super::{pending, Pending, R};
use async_trait::async_trait;
use serde_json::Value;
use xenonet_schema::drift::DriftView;

const DOCTOR: &str = "core/doctor/service.js";
const DIAGNOSTICS: &str = "core/diagnostics.js";
const VENDOR: &str = "core/vendor.js";
const SPOTIFY: &str = "core/spotify.js";
const DECIDE: &str = "core/decide.js";
const OPENROUTER: &str = "core/openrouter-usage.js";
const SYSTEM: &str = "core/system-stats.js";
const REFUSALS: &str = "core/refusals/store.js";
const INTEGRATIONS: &str = "core/integrations/index.js";
const MCP_INFO: &str = "core/mcp-info.js";
const TERMINALS: &str = "server/terminals.js (a pty)";

#[async_trait]
pub trait Doctor: Send + Sync {
    /// `doctor.last`: the last run, None before the first.
    fn last(&self) -> Option<Value> {
        None
    }
    /// `doctor.check({ fresh, byHand })`.
    async fn check(&self, _fresh: bool, _by_hand: bool) -> R<Value> {
        pending(DOCTOR)
    }
    /// `doctor.report(run)`, a run made elsewhere (the doctor simulation).
    fn report(&self, _run: Value) -> R<()> {
        pending(DOCTOR)
    }
    /// `summarize(results)` (core/doctor/index.js).
    fn summarize(&self, _results: &Value) -> R<Value> {
        pending("core/doctor/index.js")
    }
    fn start(&self) {}
    fn stop(&self) {}
}

#[async_trait]
pub trait Diagnostics: Send + Sync {
    /// `makeBundle(…)`, with what the service knows.
    async fn bundle(&self) -> R<Value> {
        pending(DIAGNOSTICS)
    }
    fn markdown(&self, _bundle: &Value) -> R<String> {
        pending(DIAGNOSTICS)
    }
    /// `writeLook(stateDir, bundle, { sources })`: the folder.
    fn write_look(&self, _bundle: &Value) -> R<String> {
        pending(DIAGNOSTICS)
    }
    /// `askCommand({ app, model, dir, sources })`: `{ args }`.
    fn ask_command(&self, _app: &str, _model: Option<&str>, _dir: &str) -> R<Value> {
        pending(DIAGNOSTICS)
    }
}

/// `driftReport({ log, all, home, now })`, which reads what the vendor
/// readers have read; the log itself (core/drift.js) is in Rust. It takes
/// the log's view rather than the log: the readers hold their own lock
/// while they record drift in the log, so a report that asked them what
/// they've read with the log still locked could wait on a reader that
/// waits on it.
pub trait DriftReport: Send + Sync {
    fn report(&self, _view: DriftView, _all: bool, _home: &str, _now: f64) -> R<Value> {
        pending(VENDOR)
    }
}

#[async_trait]
pub trait Spotify: Send + Sync {
    async fn status(&self) -> R<Value> {
        pending(SPOTIFY)
    }
    async fn player(&self) -> R<Value> {
        pending(SPOTIFY)
    }
    async fn command(&self, _op: &str, _value: &Value) -> R<()> {
        pending(SPOTIFY)
    }
    async fn callback(&self, _code: Option<&str>, _state: Option<&str>, _error: Option<&str>) -> R<()> {
        pending(SPOTIFY)
    }
    async fn disconnect(&self) -> R<Value> {
        pending(SPOTIFY)
    }
    fn set_app(&self, _client_id: &Value) -> R<Value> {
        pending(SPOTIFY)
    }
    fn begin(&self) -> R<Value> {
        pending(SPOTIFY)
    }
    /// `spotify.set({ device, duck })`.
    fn set(&self, _device: &Value, _duck: &Value) -> R<Value> {
        pending(SPOTIFY)
    }
    /// `duckPlan({ waiting, player, device, held })`.
    fn duck_plan(&self, _waiting: bool, _player: &Value, _device: &Value, _held: &Value) -> R<Value> {
        pending(SPOTIFY)
    }
}

#[async_trait]
pub trait Decider: Send + Sync {
    async fn status(&self) -> R<Value> {
        pending(DECIDE)
    }
    async fn store_key(&self, _key: &Value) -> R<Value> {
        pending(DECIDE)
    }
    async fn forget_key(&self) -> R<Value> {
        pending(DECIDE)
    }
    /// `decider.decide({ kind, state, questions })`: `{ answers, model,
    /// provider, cost, ms }`, for the projects folder's pass by a decision
    /// model (core/classify.js's askDeciderForPicks).
    async fn decide(&self, _request: Value) -> R<Value> {
        pending(DECIDE)
    }
}

#[async_trait]
pub trait OpenRouter: Send + Sync {
    async fn status(&self) -> R<Value> {
        pending(OPENROUTER)
    }
    fn forget(&self) {}
}

#[async_trait]
pub trait SystemStats: Send + Sync {
    async fn read(&self) -> R<Value> {
        pending(SYSTEM)
    }
    fn stop(&self) {}
}

pub trait Refusals: Send + Sync {
    /// `refusals.record(signal, ev)`: the record kept, or None.
    fn record(&self, _signal: &Value, _event: &Value) -> R<Option<Value>> {
        pending(REFUSALS)
    }
    fn list(&self) -> R<Value> {
        pending(REFUSALS)
    }
    fn get(&self, _id: &str) -> R<Option<Value>> {
        pending(REFUSALS)
    }
    fn dir(&self) -> String {
        String::new()
    }
    /// `contextOn(dir)`.
    fn context_on(&self) -> R<bool> {
        pending(REFUSALS)
    }
    /// `setContext(dir, on)`.
    fn set_context(&self, _on: bool) -> R<bool> {
        pending(REFUSALS)
    }
    /// `queryRefusals(records, params)` (core/refusals/query.js).
    fn query(&self, _records: &Value, _params: &Value) -> R<Value> {
        pending("core/refusals/query.js")
    }
    /// The demo's made-up records (core/refusals/demo.js).
    fn demo_records(&self) -> R<Value> {
        pending("core/refusals/demo.js")
    }
}

/// core/integrations: the method each route calls, by its name in JS
/// (status, history, shell.review, routing.route…), with the arguments the
/// route passes it.
#[async_trait]
pub trait Integrations: Send + Sync {
    async fn call(&self, _method: &str, _args: Vec<Value>) -> R<Value> {
        pending(INTEGRATIONS)
    }
}

#[async_trait]
pub trait McpInfo: Send + Sync {
    /// `catalog.get(server, sessions, { refresh })`.
    async fn get(&self, _server: &str, _sessions: &Value, _refresh: bool) -> R<Value> {
        pending(MCP_INFO)
    }
    /// `demoMcpInfo(server)`.
    fn demo(&self, _server: &str) -> R<Value> {
        pending(MCP_INFO)
    }
}

/// The terminals the service owns (server/terminals.js, on node-pty).
pub trait Terminals: Send + Sync {
    fn profiles(&self) -> R<Value> {
        pending(TERMINALS)
    }
    fn list(&self) -> Value {
        Value::Array(Vec::new())
    }
    fn create(&self, _params: &Value) -> R<Value> {
        pending(TERMINALS)
    }
    fn read(&self, _params: &Value) -> R<Value> {
        pending(TERMINALS)
    }
    fn write(&self, _params: &Value) -> R<()> {
        pending(TERMINALS)
    }
    fn resize(&self, _params: &Value) -> R<()> {
        pending(TERMINALS)
    }
    fn kill(&self, _id: &Value) -> R<()> {
        pending(TERMINALS)
    }
    fn kill_all(&self) {}
}

impl Doctor for Pending {}
impl Diagnostics for Pending {}
impl DriftReport for Pending {}
impl Spotify for Pending {}
impl Decider for Pending {}
impl OpenRouter for Pending {}
impl SystemStats for Pending {}
impl Refusals for Pending {}
impl Integrations for Pending {}
impl McpInfo for Pending {}
impl Terminals for Pending {}
