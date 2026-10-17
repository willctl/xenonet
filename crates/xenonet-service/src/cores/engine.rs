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

impl Engine for Pending {}
impl Connect for Pending {}
impl Verify for Pending {}
impl Channels for Pending {}
