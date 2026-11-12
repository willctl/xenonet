//! The service's own parts the routes reach, each behind a trait the
//! composition root (xenonet-service) implements: the actor gateway, the
//! component service (Desktop's socket), the assistant's endpoint, A2A,
//! and the network across machines. The routes hold them as `dyn`, so this
//! crate needs none of the crates behind them.

use crate::cores::CoreError;
use async_trait::async_trait;
use serde_json::Value;
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

/// Outbound actors, the agents XenoNet calls over A2A (the service's
/// a2a/outbound.rs): the window registers them, sessions hand them work.
#[async_trait]
pub trait A2aOut: Send + Sync {
    /// Each one, what the person approved for it and its latest
    /// delegations; never a bearer.
    fn status(&self) -> Value;
    /// A card, fetched for the window to show before the person approves
    /// it.
    async fn preview(&self, card: &str, private_host: Option<&str>) -> Result<Value, String>;
    /// Registers one with what the window sent, asking for the person's
    /// presence.
    async fn register(&self, body: &Value) -> Result<Value, String>;
    async fn remove(&self, actor: &str) -> Result<bool, String>;
    /// A session hands one work, or reads on (`task`): the delegation as
    /// it stands. `project` is the session's as this machine's map has it.
    async fn delegate(
        &self,
        agent: &str,
        attestation: crate::cores::door::Attestation,
        project: Option<String>,
        actor: &str,
        text: Option<&str>,
        task: Option<&str>,
    ) -> Result<Value, String>;
}

/// What `PUT /api/net/settings` changes of this machine's place in the
/// network; what's None stays as it is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetChange {
    pub titles_only: Option<bool>,
    pub share_load: Option<bool>,
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

/// The networks across machines (the service's gateway/network.rs and the
/// nodes it runs), as `/api/net/…`, `/api/svc/…` and the tray reach them.
/// Every operation answers the network's document, or a refusal whose
/// status and code the routes answer as they are. What acts on one network
/// takes `network`, its id or name: None is the one network this machine
/// is in, and refused as ambiguous when it's in several.
#[async_trait]
pub trait Network: Send + Sync {
    /// `network/state/v1`, every network in `networks`.
    fn state(&self) -> Value;
    /// The first network's name, while this machine is in one.
    fn network_name(&self) -> Option<String>;
    /// This machine's id in `network`, while it's in it.
    fn host(&self, network: Option<&str>) -> Option<String>;
    /// A new network, founded here beside any other: `relay` is the relay
    /// the person named (their own iroh-relay's URL, or n0).
    async fn create(&self, name: &str, host: &str, relay: Option<&str>, at_window: bool) -> NetResult;
    /// Joins over join/2 with an invite and its code, finding the inviter
    /// at `at`, through the invite's relay, or by mDNS (`lan`).
    async fn join_invite(&self, invite: &str, code: &str, at: Option<&str>, lan: bool) -> NetResult;
    async fn leave(&self, network: Option<&str>, force: bool) -> NetResult;
    /// An invite for `host`, and its code apart, for join/2; `lan` has the
    /// inviter announce itself on the LAN by mDNS while it's open.
    async fn invite(&self, network: Option<&str>, host: &str, window: bool, lan: bool, at_window: bool) -> NetResult;
    async fn remove(&self, network: Option<&str>, host: &str) -> NetResult;
    async fn replace(&self, network: Option<&str>, host: &str, at_window: bool) -> NetResult;
    async fn approve(&self, network: Option<&str>, id: &str, at_window: bool) -> NetResult;
    /// Who may start and wake sessions on another machine.
    async fn set_reach(&self, network: Option<&str>, host: &str, start: Option<&str>, wake: Option<&str>, at_window: bool) -> NetResult;
    async fn settings(&self, network: Option<&str>, change: NetChange, at_window: bool) -> NetResult;
    /// What the person shares into `network`: "all", or a list of project
    /// ids. Sharing more publishes, so only from the window.
    fn share(&self, network: Option<&str>, sharing: &Value, at_window: bool) -> NetResult;
    /// A relay (`url`, or n0 for n0's public ones) into `network`'s chain
    /// (`add`) or out of it; with `mine`, into or out of the relays this
    /// machine adds of its own, for its own dials, in no chain.
    async fn relay(&self, network: Option<&str>, url: &str, add: bool, mine: bool, at_window: bool) -> NetResult;
    /// A member of `network` named one of its mailboxes (`add`, which
    /// widens: the window's, with presence) or taken off them (a window
    /// host's, or the mailbox's own).
    async fn mailbox(&self, network: Option<&str>, host: &str, add: bool, at_window: bool) -> NetResult;
    /// A new transport key for this machine in every network it's in, in
    /// place of one that was copied: asks the person, so only from the
    /// window.
    async fn rekey(&self, at_window: bool) -> NetResult;
    /// The services this machine publishes into `network`.
    fn svc_list(&self, network: Option<&str>) -> NetResult;
    /// Publishing asks the person (their presence), so only from the window.
    fn svc_publish(&self, network: Option<&str>, name: &str, target: &str, paths: &[String], hosts: &[String], at_window: bool) -> NetResult;
    /// Revokes another of the person's devices: a lost machine. Asks them,
    /// so only from the window.
    fn revoke_device(&self, device: &str, at_window: bool) -> NetResult;
    /// Takes a machine's feed again after a fork froze it (it signed two
    /// entries at one place): the person's call, so only from the window.
    fn unfreeze(&self, network: Option<&str>, author: &str, at_window: bool) -> NetResult;
    fn svc_unpublish(&self, network: Option<&str>, name: &str) -> NetResult;
    /// The port a service's target address names, when it's one.
    fn svc_port(&self, target: &str) -> Option<u16>;
    /// A call to the service `name` that `host` publishes, in the network
    /// `host` is in.
    async fn svc_call(&self, host: &str, name: &str, call: SvcCall) -> SvcAnswer;
    /// The window's read of `path` (a session's transcript or stats) on
    /// `host`, over svc/2 under the window's key: its status and JSON, or
    /// None when this machine's endpoint isn't running. Only the window
    /// reaches this.
    async fn window_read(&self, _host: &str, _path: &str) -> Option<(u16, Value)> {
        None
    }
    /// The latest load each other machine sent on link/2 (system/v1's
    /// fields, with `host`), for GET /api/system?hosts=1: only machines of
    /// this one's networks that share it, and only to a window host.
    fn loads(&self) -> Vec<Value> {
        Vec::new()
    }
    /// The machine another machine's agent is on, by the agt_ its presence
    /// lists it by.
    fn host_of_agent(&self, _id: &str) -> Option<String> {
        None
    }
}
