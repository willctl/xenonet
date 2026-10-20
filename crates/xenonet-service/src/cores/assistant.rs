//! Remote actors (docs/design/cloud-assistants.md): the connections you pair
//! (core/actors/connections.js), the tunnels XenoNet runs for them
//! (core/assistant/tunnels.js), the audit of their reads (audit.js), what
//! they read (source.js, projection.js) and the reads themselves
//! (service.js).

use super::{pending, Pending, R};
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::mpsc;

const CONNECTIONS: &str = "core/actors/connections.js";
const TUNNELS: &str = "core/assistant/tunnels.js";
const AUDIT: &str = "core/assistant/audit.js";
const SOURCE: &str = "core/assistant/source.js";
const SERVICE: &str = "core/assistant/service.js";

/// What a connection's event says: (revoked | rotated | expired | disabled
/// | enabled | changed, its data).
pub type ConnectionEvents = mpsc::UnboundedReceiver<(String, Value)>;

#[async_trait]
pub trait Connections: Send + Sync {
    fn list(&self) -> R<Value> {
        pending(CONNECTIONS)
    }
    fn disable(&self) -> R<Value> {
        pending(CONNECTIONS)
    }
    fn enable(&self) -> R<Value> {
        pending(CONNECTIONS)
    }
    /// `connections.pair(body, { places, identities })`: `places` the
    /// places on the map by id, `identities` the identities' ids.
    fn pair(&self, _body: &Value, _places: &Value, _identities: &[String]) -> R<Value> {
        pending(CONNECTIONS)
    }
    fn revoke(&self, _grant: &str) -> R<Value> {
        pending(CONNECTIONS)
    }
    fn rotate(&self, _grant: &str) -> R<Value> {
        pending(CONNECTIONS)
    }
    /// `connections.update(grant, { projects, labels }, { places })`.
    fn update(&self, _grant: &str, _projects: &Value, _labels: &Value, _places: &Value) -> R<Value> {
        pending(CONNECTIONS)
    }
    /// A hello's grant and credential: `{ ok, grant, actorId, connection,
    /// revision }` or `{ ok: false, code, reason }`.
    fn authenticate(&self, _grant: &Value, _token: &Value) -> Value {
        serde_json::json!({ "ok": false, "code": "SERVICE_UNAVAILABLE", "reason": "pending" })
    }
    /// An adapter connected (+1) or went (-1).
    fn bridge(&self, _connection: &str, _delta: i32) {}
    /// What a session may read: `{ deviceId, permissions }`.
    fn policy(&self, _session: &Value) -> R<Value> {
        pending(CONNECTIONS)
    }
    fn any_active(&self) -> bool {
        false
    }
    /// `connections.recheck(grant, -1)`: whether the grant still stands.
    fn recheck(&self, _grant: &str) -> Value {
        serde_json::json!({ "ok": false })
    }
    /// The state folder its credentials are in.
    fn dir(&self) -> String {
        String::new()
    }
    fn tick(&self) {}
    fn subscribe(&self) -> Option<ConnectionEvents> {
        None
    }
}

#[async_trait]
pub trait Tunnels: Send + Sync {
    async fn status(&self) -> R<Value> {
        pending(TUNNELS)
    }
    fn stop(&self, _grant: &str) -> R<Value> {
        pending(TUNNELS)
    }
    async fn forget_key(&self, _grant: &str) -> R<Value> {
        pending(TUNNELS)
    }
    async fn start(&self, _grant: &str) -> R<Value> {
        pending(TUNNELS)
    }
    async fn check(&self, _grant: &str) -> R<Value> {
        pending(TUNNELS)
    }
    async fn store_key(&self, _grant: &str, _key: &Value) -> R<Value> {
        pending(TUNNELS)
    }
    /// `tunnels.configure(grant, { tunnelId, keyTarget, autostart })`.
    fn configure(&self, _grant: &str, _spec: Value) -> R<Value> {
        pending(TUNNELS)
    }
    async fn autostart(&self) -> R<()> {
        pending(TUNNELS)
    }
    fn stop_all(&self) {}
    /// Tells `changed` when how they stand changed.
    fn subscribe(&self) -> Option<mpsc::UnboundedReceiver<()>> {
        None
    }
}

pub trait Audit: Send + Sync {
    fn recent(&self, _grant: Option<&str>, _limit: f64) -> R<Value> {
        pending(AUDIT)
    }
    fn access(&self, _record: Value) {}
}

#[async_trait]
pub trait AssistantSource: Send + Sync {
    /// The products that can be paired, with their transports
    /// (core/actors/adapters.js and products.js).
    fn products(&self) -> R<Value> {
        pending("core/actors/adapters.js")
    }
    /// `sourcesData(source.view())`.
    fn sources_data(&self) -> R<Value> {
        pending(SOURCE)
    }
    /// What a grant with these projects and labels would send (the
    /// route's preview): `known` the places on the map, by id.
    async fn preview(&self, _body: &Value, _known: &Value) -> R<Value> {
        pending(SOURCE)
    }
}

#[async_trait]
pub trait AssistantService: Send + Sync {
    /// `service.read(session, params)`, for an adapter let in.
    async fn read(&self, _session: &Value, _params: &Value) -> R<Value> {
        pending(SERVICE)
    }
    fn forget(&self, _grant: &str) {}
}

impl Connections for Pending {}
impl Tunnels for Pending {}
impl Audit for Pending {}
impl AssistantSource for Pending {}
impl AssistantService for Pending {}
