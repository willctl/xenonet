//! The cores the service's routes call, each behind a trait: the map's
//! state, the engine and delivery, the actors, the other machines, the
//! assistant, the doctor and the console's services, JSON in and out. The
//! composition root (the service's `compose` and the command's `serve`)
//! fills `Cores` with each crate's implementation.
//!
//! A trait's methods have default bodies for a XenoNet started without
//! that part (the actors off, no hosts, a test that wires only one core):
//! a route that reaches one answers 501 with `{ error, code: "pending" }`,
//! and a check that can't run refuses (the process check, the schema
//! checks), never lets through. `Cores::pending()` starts every core so.

pub mod actors;
pub mod assistant;
pub mod delivery;
pub mod door;
pub mod engine;
pub mod forwards;
pub mod hosts;
pub mod intents;
pub mod services;
pub mod settings;
pub mod state;

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_model::ops::Net;
use xenonet_net::sink::SinkSlot;

pub use actors::{Actors, Registration, Reports, Runs, Spend, Wakes};
pub use assistant::{AssistantService, AssistantSource, Audit, Connections, Grants, Tunnels};
pub use delivery::{Policy, Router};
pub use door::{Apps, LocalDoor};
pub use engine::{Channels, Connect, Engine, EngineStore, Verify};
pub use forwards::Forwards;
pub use hosts::{HttpApi, Hub, HubNet, SelfApi};
pub use intents::{Intents, Reporter};
pub use services::{Decider, Diagnostics, Doctor, DriftReport, Integrations, McpInfo, OpenRouter, Refusals, Spotify, SystemStats, Terminals};
pub use settings::{Command, Demo, Opener, Schemas, Store, Themes};
pub use state::State;

/// Which JS error a core's failure stands for: the routes answer
/// ActorError, WakeError and the others each in their own way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Error,
    Actor,
    Wake,
    Connection,
    Tunnel,
    Integration,
    Decision,
    Spotify,
    /// A core that isn't in Rust yet.
    Pending,
}

/// A core's failure: `err.message`, and the `status`, `code` and `extra`
/// the JS errors carry.
#[derive(Clone, Debug)]
pub struct CoreError {
    pub kind: Kind,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
    pub extra: Value,
}

impl CoreError {
    pub fn new(message: impl Into<String>) -> Self {
        CoreError { kind: Kind::Error, status: None, code: None, message: message.into(), extra: Value::Null }
    }

    /// An error with a status, as `Object.assign(new Error(…), { status })`.
    pub fn status(status: u16, message: impl Into<String>) -> Self {
        CoreError { status: Some(status), ..CoreError::new(message) }
    }

    /// One of the JS error classes, with its status and code.
    pub fn of(kind: Kind, status: u16, code: &str, message: impl Into<String>) -> Self {
        CoreError { kind, status: Some(status), code: Some(code.to_string()), message: message.into(), extra: Value::Null }
    }

    #[must_use]
    pub fn with_extra(mut self, extra: Value) -> Self {
        self.extra = extra;
        self
    }

    pub fn pending(module: &str) -> Self {
        CoreError {
            kind: Kind::Pending,
            status: Some(501),
            code: Some("pending".into()),
            message: format!("{module} isn't in the Rust service yet: run the JS service for this"),
            extra: Value::Null,
        }
    }

    pub fn is_pending(&self) -> bool {
        self.kind == Kind::Pending
    }

    pub fn is(&self, kind: Kind) -> bool {
        self.kind == kind
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub type R<T> = Result<T, CoreError>;

/// The default body of every method of a core not in Rust yet.
pub fn pending<T>(module: &str) -> R<T> {
    Err(CoreError::pending(module))
}

/// A check that can't run, refused, as `{ ok: false, reason }`.
pub fn refused(module: &str) -> Value {
    json!({ "ok": false, "reason": format!("XenoNet can't check this yet: {module} isn't in the Rust service") })
}

/// `x.ok` of a check's verdict.
pub fn ok(v: &Value) -> bool {
    v.get("ok").and_then(Value::as_bool) == Some(true)
}

/// `x.reason` of a refusal, as a string.
pub fn reason(v: &Value) -> String {
    match v.get("reason") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => "undefined".into(),
        Some(other) => xenonet_text::text::display(other),
    }
}

/// Every core the service holds, as the JS service's top level makes them.
/// Optional ones are None when a switch turns them off
/// (XENONET_ACTORS=off, XENONET_ASSISTANT=off), and the hub until the
/// service makes it.
#[derive(Clone)]
pub struct Cores {
    pub state: Arc<dyn State>,
    pub store: Arc<dyn Store>,
    pub schemas: Arc<dyn Schemas>,
    pub drift: Arc<dyn DriftReport>,
    pub engine: Arc<dyn Engine>,
    pub engine_store: Arc<dyn EngineStore>,
    pub connect: Arc<dyn Connect>,
    pub verify: Arc<dyn Verify>,
    pub channels: Arc<dyn Channels>,
    pub actors: Option<Arc<dyn Actors>>,
    pub registration: Option<Arc<dyn Registration>>,
    pub reports: Arc<dyn Reports>,
    pub wakes: Option<Arc<dyn Wakes>>,
    pub runs: Option<Arc<dyn Runs>>,
    pub spend: Arc<dyn Spend>,
    pub policy: Arc<dyn Policy>,
    pub router: Arc<dyn Router>,
    pub hub: Option<Arc<dyn Hub>>,
    pub connections: Option<Arc<dyn Connections>>,
    pub tunnels: Option<Arc<dyn Tunnels>>,
    pub audit: Option<Arc<dyn Audit>>,
    pub assistant_source: Option<Arc<dyn AssistantSource>>,
    pub assistant: Option<Arc<dyn AssistantService>>,
    /// Actors' grants, minted and decided by the engine's driver.
    pub grants: Arc<dyn Grants>,
    pub doctor: Arc<dyn Doctor>,
    pub diagnostics: Arc<dyn Diagnostics>,
    pub themes: Arc<dyn Themes>,
    pub opener: Arc<dyn Opener>,
    pub command: Arc<dyn Command>,
    pub spotify: Arc<dyn Spotify>,
    pub decider: Arc<dyn Decider>,
    pub openrouter: Arc<dyn OpenRouter>,
    pub system: Arc<dyn SystemStats>,
    pub refusals: Arc<dyn Refusals>,
    pub integrations: Arc<dyn Integrations>,
    pub mcp_info: Arc<dyn McpInfo>,
    pub terminals: Arc<dyn Terminals>,
    pub demo: Arc<dyn Demo>,
    /// The person's intents, made here and heard of from the network.
    pub intents: Arc<dyn Intents>,
    /// The local door's decisions, and the app registry as the routes ask
    /// it: the composition root's (xenonet-authz, the adapters).
    pub door: Arc<dyn LocalDoor>,
    pub apps: Arc<dyn Apps>,
    /// The actors' forwards over this machine's endpoint.
    pub forwards: Arc<dyn Forwards>,
    /// What plugs the engine into the network's link once the service has
    /// made it (the service's gateway/network.rs): the engine's driver sets
    /// it, and the service calls it once, before the link starts. The link
    /// reads the network as it stands, so a network created or joined later
    /// reaches the engine through the same plug.
    pub on_net: Option<NetPlug>,
}

/// This machine's link to its network, as the service makes it: the
/// network itself, and the slot where what arrives from it goes.
pub trait NetLink {
    fn net(&self) -> Arc<dyn Net>;
    fn sinks(&self) -> &Arc<SinkSlot>;
    /// The network's chains and the person on this machine, which the
    /// engine decides every incoming act on.
    fn governance(&self) -> Option<Arc<dyn xenonet_net::ground::Governance>> {
        None
    }
}

/// See `Cores::on_net`.
pub type NetPlug = Arc<dyn Fn(&dyn NetLink) + Send + Sync>;

/// The stand-in for every core that isn't in Rust yet.
#[derive(Clone, Copy, Default)]
pub struct Pending;

impl Cores {
    /// Every core pending, with the optional ones there as the JS service's
    /// switches have them.
    pub fn pending(actors: bool, assistant: bool) -> Cores {
        let p = Arc::new(Pending);
        Cores {
            state: p.clone(),
            store: p.clone(),
            schemas: p.clone(),
            drift: p.clone(),
            engine: p.clone(),
            engine_store: p.clone(),
            connect: p.clone(),
            verify: p.clone(),
            channels: p.clone(),
            actors: actors.then(|| p.clone() as Arc<dyn Actors>),
            registration: actors.then(|| p.clone() as Arc<dyn Registration>),
            reports: p.clone(),
            wakes: actors.then(|| p.clone() as Arc<dyn Wakes>),
            runs: actors.then(|| p.clone() as Arc<dyn Runs>),
            spend: p.clone(),
            policy: p.clone(),
            router: p.clone(),
            hub: None,
            connections: (actors && assistant).then(|| p.clone() as Arc<dyn Connections>),
            tunnels: (actors && assistant).then(|| p.clone() as Arc<dyn Tunnels>),
            audit: (actors && assistant).then(|| p.clone() as Arc<dyn Audit>),
            assistant_source: (actors && assistant).then(|| p.clone() as Arc<dyn AssistantSource>),
            assistant: (actors && assistant).then(|| p.clone() as Arc<dyn AssistantService>),
            grants: p.clone(),
            doctor: p.clone(),
            diagnostics: p.clone(),
            themes: p.clone(),
            opener: p.clone(),
            command: p.clone(),
            spotify: p.clone(),
            decider: p.clone(),
            openrouter: p.clone(),
            system: p.clone(),
            refusals: p.clone(),
            integrations: p.clone(),
            mcp_info: p.clone(),
            terminals: p.clone(),
            demo: p.clone(),
            intents: p.clone(),
            door: p.clone(),
            apps: p.clone(),
            forwards: p,
            on_net: None,
        }
    }
}
