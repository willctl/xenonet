//! The service's own parts the routes reach, each behind a trait the
//! composition root (xenonet-service) implements: the actor gateway, the
//! component service (Desktop's socket), the assistant's endpoint, A2A,
//! and the network across machines. The routes hold them as `dyn`, so this
//! crate needs none of the crates behind them.

use crate::cores::CoreError;
use async_trait::async_trait;
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;

/// What a network operation answers: its document, or a refusal with its
/// status and code.
pub type NetResult = Result<Value, CoreError>;

/// The actor gateway (the service's gateway/control.rs), as the Actors
/// panel, the tray, A2A and the network routes read and set it.
#[async_trait]
pub trait Gateway: Send + Sync {
    /// The gateway's public URL, or null while it has none.
    fn url(&self) -> Value;
    /// What the tray shows of it: `{ on, state, url, problem }`.
    fn brief(&self) -> Value;
    /// Its whole status, for the Actors panel.
    async fn status(&self) -> Result<Value, CoreError>;
    /// Turns it on or off, picks or takes away its tunnel, checks it now,
    /// or repairs it.
    async fn set(self: Arc<Self>, on: Option<bool>, pick: Option<Value>, check: bool, repair: bool) -> Result<Value, CoreError>;
    /// Its tunnel as the panel shows it (`{ kind, … }`).
    fn tunnel_view(&self) -> Value;
    /// The host its tunnel is reached at, when it has one.
    fn public_host(&self) -> Option<String>;
    /// The port it listens on.
    fn port(&self) -> u16;
}

/// The component service: Desktop's connection to this service.
#[async_trait]
pub trait Components: Send + Sync {
    /// What it says of itself (`{ desktop, … }`).
    fn info(&self) -> Value;
    /// A notification the desktop app shows.
    fn notify(&self, note: Value);
    /// An app's link, a folder or a page, opened by Desktop: false when it
    /// didn't.
    async fn open(&self, target: &str) -> bool;
    /// Desktop's window shown, at a session or an actor: false when it
    /// wasn't.
    async fn show(&self, select: Option<&str>) -> bool;
    /// XenoNet Dev's control channel: a call to the connected Desktop's
    /// console, its result or why not.
    async fn console(&self, body: Value) -> Result<Value, String>;
}

/// Remote actors' private endpoint (the service's assistant_endpoint.rs).
pub trait Endpoint: Send + Sync {
    /// Whether it listens, and the sessions on it.
    fn info(&self) -> Value;
}

/// A2A agents' side of the gateway (the service's a2a/), as the window
/// sets it.
pub trait A2a: Send + Sync {
    /// Whether A2A agents may reach XenoNet, the card at `address`, and
    /// the agents that called.
    fn status(&self, address: Option<&str>) -> Value;
    fn set_enabled(&self, on: bool) -> Result<(), String>;
    /// An agent's default target, or none.
    fn set_default_target(&self, actor: &str, target: Option<&str>) -> Result<(), String>;
}

/// What `PUT /api/net/settings` changes of this machine's place in the
/// network; what's None stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetChange {
    pub titles_only: Option<bool>,
    pub share_load: Option<bool>,
    /// The gateway's LAN address, or None inside to take it away.
    pub lan: Option<Option<String>>,
    pub start: Option<String>,
    pub wake: Option<String>,
}

/// A request for a service another host published, as `/api/svc/…` takes
/// it.
#[derive(Clone, Debug)]
pub struct SvcCall {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The service's answer, or XenoNet's when it refused.
#[derive(Clone, Debug)]
pub struct SvcAnswer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl SvcAnswer {
    /// XenoNet's own answer: `{ error, code }`, with the refused header.
    pub fn refused(status: u16, code: &str, message: &str) -> SvcAnswer {
        let (headers, body) = xenonet_model::network::svc_refusal(code, message);
        SvcAnswer { status, headers, body }
    }
}

/// The network across machines (the service's gateway/network.rs and the
/// node it runs), as `/api/net/…`, `/api/svc/…` and the tray reach it.
/// Every operation answers the network's document, or a refusal whose
/// status and code the routes answer as they are.
#[async_trait]
pub trait Network: Send + Sync {
    /// `network/state/v1`.
    fn state(&self) -> Value;
    /// The network's name, while this machine is in one.
    fn network_name(&self) -> Option<String>;
    /// This machine's id in its network, while it's in one.
    fn host(&self) -> Option<String>;
    /// Whether this machine is the network's gateway.
    fn is_gateway(&self) -> bool;
    /// The LAN listener, at `addr`.
    async fn listen(&self, addr: SocketAddr) -> Result<(), CoreError>;
    fn unlisten(&self);
    /// The LAN listener's address, while it listens.
    fn lan(&self) -> Option<SocketAddr>;
    /// A new network, founded here: `address` is the gateway's public host,
    /// `lan` its LAN address.
    fn create(&self, name: &str, host: &str, address: Option<&str>, lan: Option<&str>, at_window: bool) -> NetResult;
    async fn join(&self, address: &str, ticket: &str) -> NetResult;
    async fn leave(&self, force: bool) -> NetResult;
    /// The address a ticket names for the gateway, through its tunnel.
    fn tunnel_address(&self, address: Option<&str>);
    async fn ticket(&self, host: &str, window: bool, at_window: bool) -> NetResult;
    async fn remove(&self, host: &str) -> NetResult;
    async fn replace(&self, host: &str, at_window: bool) -> NetResult;
    async fn approve(&self, id: &str, at_window: bool) -> NetResult;
    async fn take_gateway(&self, address: Option<&str>, lan: Option<&str>, at_window: bool) -> NetResult;
    async fn set_gateway(&self, address: &str) -> NetResult;
    /// Who may start and wake sessions on another machine.
    async fn set_reach(&self, host: &str, start: Option<&str>, wake: Option<&str>, at_window: bool) -> NetResult;
    async fn settings(&self, change: NetChange, at_window: bool) -> NetResult;
    /// The services this machine publishes.
    fn svc_list(&self) -> Value;
    fn svc_publish(&self, name: &str, target: &str, paths: &[String], hosts: &[String]) -> NetResult;
    fn svc_unpublish(&self, name: &str) -> NetResult;
    /// The port a service's target address names, when it's one.
    fn svc_port(&self, target: &str) -> Option<u16>;
    /// A call to the service `name` that `host` publishes.
    async fn svc_call(&self, host: &str, name: &str, call: SvcCall) -> SvcAnswer;
}
