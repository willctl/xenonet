//! The engine (core/engine/engine.js), connecting agents
//! (core/engine/connect.js), the process checks (core/engine/verify.js over
//! core/procs.js) and a Claude Code channel's queue
//! (core/delivery/channel.js), as the routes call them.

use super::{pending, refused, Pending, R};
use crate::http::Peer;
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;
use tokio::sync::watch;

const ENGINE: &str = "core/engine/engine.js";
const CONNECT: &str = "core/engine/connect.js";
const VERIFY: &str = "core/engine/verify.js";
const CHANNEL: &str = "core/delivery/channel.js";

#[async_trait]
pub trait Engine: Send + Sync {
    /// What the engine wants a hook's agent to hear, or None.
    async fn hook(&self, _app: &str, _event: Option<&str>, _body: &Value) -> R<Option<Value>> {
        pending(ENGINE)
    }
    async fn snapshot(&self) -> R<Value> {
        pending(ENGINE)
    }
    async fn agents(&self) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.whoHas(me, path)`, me null for a person asking.
    async fn who_has(&self, _me: Option<&str>, _path: &str) -> R<Value> {
        pending(ENGINE)
    }
    async fn whoami(&self, _me: &str) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.peers(me, { scope })`.
    async fn peers(&self, _me: &str, _scope: &str) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.send(me, { to, text, replyTo }, extra)`: `msg` holds to,
    /// text and replyTo as the route took them; `extra` the envelope.
    async fn send(&self, _me: &str, _msg: Value, _extra: Option<Value>) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.inbox(me, { waitMs })`.
    async fn inbox(&self, _me: &str, _wait: Duration) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.claim(me, { paths, note, minutes })`.
    async fn claim(&self, _me: &str, _args: Value) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.release(me, { paths })`, paths null for all of them.
    async fn release(&self, _me: &str, _paths: Value) -> R<Value> {
        pending(ENGINE)
    }
    async fn set_focus(&self, _me: &str, _text: Value) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.touch(me, { app, cwd, via, pid })`.
    fn touch(&self, _me: &str, _info: Value) -> R<()> {
        pending(ENGINE)
    }
    /// `engine.view(me)`, the agent as the engine sees it (its title).
    async fn view(&self, _me: &str) -> R<Option<Value>> {
        pending(ENGINE)
    }
    /// `engine.listen(me, { ms, signal })`: a message for a session's Stop
    /// hook, waiting up to `wait`; `gone` turns true when the hook stops
    /// waiting. `{ decision, reason, ids }`, or None.
    async fn listen(&self, _me: &str, _wait: Duration, _gone: watch::Receiver<bool>) -> R<Option<Value>> {
        pending(ENGINE)
    }
    /// A handoff (docs/design/network.md, section 11): offered with
    /// `{ to, host, goal, done, next, files, branch, commit }`, taken with
    /// `{ accept }`, or with neither, the ones open to `me`. `extra` holds
    /// the envelope, as send's does.
    async fn handoff(&self, _me: &str, _args: Value, _extra: Option<Value>) -> R<Value> {
        pending(ENGINE)
    }
    /// `engine.store.markUnseen(me, ids)`.
    fn mark_unseen(&self, _me: &str, _ids: &Value) -> R<()> {
        pending(ENGINE)
    }
    /// `engine.listening`, the sessions listening after a turn, as
    /// [agent, until] pairs.
    fn listening(&self) -> R<Vec<(String, Value)>> {
        pending(ENGINE)
    }
}

#[async_trait]
pub trait Connect: Send + Sync {
    async fn preview(&self) -> R<Value> {
        pending(CONNECT)
    }
    async fn apply(&self, _apps: Value) -> R<Value> {
        pending(CONNECT)
    }
    async fn remove(&self) -> R<Value> {
        pending(CONNECT)
    }
    /// Whether connect's own Claude Code hook for `event` is in Claude
    /// Code's settings, so the copy XenoNet's plugin posts stands by. None
    /// is, until connect is there.
    async fn hook_connected(&self, _event: &str) -> R<bool> {
        Ok(false)
    }
}

/// The process checks: each answers `{ ok, reason?, … }` for the process
/// at the other end of `peer`. One that can't run refuses.
#[async_trait]
pub trait Verify: Send + Sync {
    /// `verify.check(socket, id, { hook })`.
    async fn check(&self, _peer: Peer, _id: &str, _hook: bool) -> Value {
        refused(VERIFY)
    }
    async fn person(&self, _peer: Peer) -> Value {
        refused(VERIFY)
    }
    async fn window(&self, _peer: Peer) -> Value {
        refused(VERIFY)
    }
    /// The connected Desktop's own process (None when it went).
    async fn desktop(&self, _peer: Option<Peer>) -> Value {
        refused(VERIFY)
    }
    /// The tray app's own process, connecting to the component socket:
    /// XenoNet Tray.app in /Applications or `home`'s Applications folder.
    async fn tray(&self, _peer: Peer, _home: &str) -> Value {
        refused(VERIFY)
    }
    /// `verify.actor(socket, { slug, product }, { pid })`.
    async fn actor(&self, _peer: Peer, _slug: &str, _product: Value, _pid: Value) -> Value {
        refused(VERIFY)
    }
    /// `verify.register(socket, { product, pid })`.
    async fn register(&self, _peer: Peer, _product: Value, _pid: Value) -> Value {
        refused(VERIFY)
    }
    async fn asker(&self, _peer: Peer) -> Value {
        refused(VERIFY)
    }
    /// Which process is at the other end, as a key for the bad tokens'
    /// rates ("?" when it can't tell).
    async fn peer(&self, _peer: Peer) -> String {
        "?".into()
    }
    /// `verify.state`: what it refused because it couldn't see who sent it.
    fn state(&self) -> Value {
        Value::Null
    }
}

#[async_trait]
pub trait Channels: Send + Sync {
    /// Messages for a Claude Code channel session, waiting up to `wait`.
    async fn poll(&self, _me: &str, _wait: Duration) -> R<Value> {
        pending(CHANNEL)
    }
}

/// The engine's database (core/engine/store.js), for what the service keeps
/// in it itself: the gateway's settings, and when actors' tokens were made.
pub trait EngineStore: Send + Sync {
    /// `store.meta(key)`.
    fn meta(&self, _key: &str) -> R<Option<String>> {
        pending("core/engine/store.js")
    }
    fn set_meta(&self, _key: &str, _value: &str) -> R<()> {
        pending("core/engine/store.js")
    }
    /// `store.actorTokenTimes()`: `{ slug, hasToken, at, hasGateway, gatewayAt }`.
    fn actor_token_times(&self) -> R<Vec<Value>> {
        pending("core/engine/store.js")
    }
    /// `store.expiredActorTokens()`, for the doctor: `[{ slug, name }]`.
    fn expired_actor_tokens(&self) -> R<Value> {
        pending("core/engine/store.js")
    }
    /// `store.waitingActorRequests(now)`, for the connections' taken slugs.
    fn waiting_actor_requests(&self, _now: f64) -> R<Vec<Value>> {
        pending("core/engine/store.js")
    }
    /// `store.connection(id)`: a connection's row (its transport_kind).
    fn connection(&self, _id: &str) -> R<Option<Value>> {
        pending("core/engine/store.js")
    }
    /// The store itself, xenonet-engine's `store::Shared` as `Any`, for the
    /// cores in other crates that keep their tables in engine.sqlite as the
    /// JS's did (the remote actors' connections and grants, their audit,
    /// the tunnels' bindings), which take it whole. None until it's in Rust.
    fn native(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        None
    }
}

impl Engine for Pending {}
impl EngineStore for Pending {}
impl Connect for Pending {}
impl Verify for Pending {}
impl Channels for Pending {}
