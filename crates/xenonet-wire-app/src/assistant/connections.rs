//! Remote actors' connections and their grants
//! (crates/xenonet-actors/src/connections.rs), over the engine's store.
//! Every check, what a grant may read and how it stands are the core's;
//! this answers what it asks of the world: the actors' registry, the
//! credentials' files (xenonet_assistant::credentials), the adapters, the
//! map's places and the identities for a pairing, and its events go to
//! whoever listens (the endpoint, the tunnels).

use super::{connection_error, row_json};
use crate::fx::{now_ms, Locked};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;
use xenonet_actors::account_links::Links;
use xenonet_actors::adapters::ADAPTERS;
use xenonet_actors::connections::{self as core, Adapter, Connections as Core, Host, Store};
use xenonet_actors::store::{self, Tx};
use xenonet_assistant::credentials as disk;
use xenonet_base::text::js_string;
use xenonet_engine::sql::SqlError;
use xenonet_engine::store::Shared;
use xenonet_service::cores::assistant::ConnectionEvents;
use xenonet_service::cores::{Actors, Connections, EngineStore, State, R};

pub struct Remote {
    core: Core,
    dir: String,
    actors: Arc<dyn Actors>,
    engine_store: Arc<dyn EngineStore>,
    state: Arc<dyn State>,
    listeners: Mutex<Vec<mpsc::UnboundedSender<(String, Value)>>>,
    /// The adapter a pairing picked, by its index among ADAPTERS.
    picked: Mutex<Option<usize>>,
}

// What a pairing adds: the places on the map, and the identities.
struct Also<'a> {
    places: &'a Value,
    identities: &'a [String],
}

struct Base<'a> {
    r: &'a Remote,
    also: Option<Also<'a>>,
}

/// What a file of the credentials couldn't do, as the core hears it.
fn fs(e: xenonet_base::fs::FsError) -> core::Fail {
    core::Fail::Error(store::Error { message: e.message, code: e.code })
}

/// What the actors' registry failed with, as the core hears it.
fn failed(e: xenonet_service::cores::CoreError) -> core::Fail {
    core::Fail::Error(store::Error { message: e.message, code: e.code })
}

struct Private;

impl disk::Private for Private {
    fn protect(&mut self, file: &str) -> Result<(), String> {
        let mut run = |f: &str, a: &[&str]| xenonet_base::private_file::run_private(f, a);
        xenonet_base::private_file::protect_file(file, cfg!(windows), &mut run).map_err(|e| e.message().to_string())
    }
    fn readers(&mut self, file: &str) -> Result<Vec<String>, String> {
        let mut run = |f: &str, a: &[&str]| xenonet_base::private_file::run_private(f, a);
        xenonet_base::private_file::other_readers(file, cfg!(windows), &mut run).map_err(|e| e.message().to_string())
    }
}

impl Host for Base<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn wall(&mut self) -> f64 {
        now_ms()
    }
    fn emit(&mut self, name: &str, what: Value) {
        self.r.emit(name, what);
    }
    fn describe(&mut self, place: &str) -> Value {
        self.r.state.node(place).ok().flatten().and_then(|n| n.get("name").cloned()).unwrap_or(Value::Null)
    }
    fn taken(&mut self, slug: &str) -> bool {
        let slug = json!(slug);
        self.r.engine_store.waiting_actor_requests(now_ms()).unwrap_or_default().iter().any(|q| q.get("slug") == Some(&slug))
    }
    fn adapter_for(&mut self, product: Option<&Value>) -> Option<Adapter> {
        let products: Vec<Option<Value>> = ADAPTERS.iter().map(|a| a.product.map(|p| json!(p))).collect();
        let custom = ADAPTERS.iter().position(|a| a.key == "custom");
        let i = xenonet_actors::adapters::adapter_for(product, &products, custom);
        *self.r.picked.locked() = i;
        let a = ADAPTERS.get(i?)?;
        Some(Adapter { id: a.id.into(), product: a.product.map(Into::into), provider: a.provider.into(), transports: a.transports.iter().map(ToString::to_string).collect() })
    }
    fn picked_capabilities(&mut self) -> core::Result<Vec<String>> {
        let i = (*self.r.picked.locked()).ok_or_else(|| core::Fail::Error(store::Error::new("picked.capabilities is not a function")))?;
        Ok(ADAPTERS.get(i).map(|a| a.capabilities.iter().map(ToString::to_string).collect()).unwrap_or_default())
    }
    fn write_credential(&mut self, grant: &str, token: &Value, device: &Value, expires_at: &Value) -> core::Result<()> {
        disk::write_credential(&self.r.dir, &json!(grant), token, Some(device), expires_at, &mut Private).map(|_| ()).map_err(fs)
    }
    fn places_has(&mut self, id: &str) -> bool {
        self.also.as_ref().is_some_and(|a| a.places.get(id).is_some())
    }
    fn identities_has(&mut self, id: &str) -> bool {
        self.also.as_ref().is_some_and(|a| a.identities.iter().any(|i| i == id))
    }
    fn identities(&mut self) -> Vec<String> {
        self.also.as_ref().map(|a| a.identities.to_vec()).unwrap_or_default()
    }
    fn is_disabled(&mut self) -> bool {
        disk::is_disabled(&self.r.dir)
    }
    fn is_marked_revoked(&mut self, grant: &str) -> bool {
        disk::is_marked_revoked(&self.r.dir, &json!(grant))
    }
    fn mark_revision(&mut self, grant: &str, revision: &Value) -> core::Result<()> {
        disk::mark_revision(&self.r.dir, &json!(grant), revision).map_err(fs)
    }
    fn mark_revoked(&mut self, grant: &str, at: f64) -> core::Result<()> {
        disk::mark_revoked(&self.r.dir, &json!(grant), at).map_err(fs)
    }
    fn delete_credential(&mut self, grant: &str) -> core::Result<()> {
        disk::delete_credential(&self.r.dir, &json!(grant)).map_err(fs)
    }
    fn marked_revoked(&mut self) -> Vec<String> {
        disk::marked_revoked(&self.r.dir)
    }
    fn new_token(&mut self) -> (String, String) {
        disk::new_token()
    }
    fn mark_disabled(&mut self, at: f64) -> core::Result<()> {
        disk::mark_disabled(&self.r.dir, at).map_err(fs)
    }
    fn clear_disabled(&mut self) -> core::Result<()> {
        disk::clear_disabled(&self.r.dir).map_err(fs)
    }
    fn is_token(&mut self, token: &Value) -> bool {
        disk::is_token(&js_string(token))
    }
    fn hash_token(&mut self, token: &Value) -> String {
        disk::hash_token(&js_string(token))
    }
    fn ids(&mut self) -> Vec<String> {
        self.r.actors.ids()
    }
    fn remote_of(&mut self, id: &str) -> Value {
        self.r.actors.remote_of(id).unwrap_or(Value::Null)
    }
    fn set_remote(&mut self, id: &str, remote: &Value) -> core::Result<()> {
        self.r.actors.set_remote(id, remote).map_err(failed)
    }
    fn has(&mut self, id: &str) -> bool {
        self.r.actors.has(id)
    }
    fn info(&mut self, id: &str) -> Value {
        self.r.actors.info(id).unwrap_or(Value::Null)
    }
    fn is_remote(&mut self, id: &str) -> bool {
        self.r.actors.is_remote(id)
    }
    fn upsert(&mut self, def: &Value) -> core::Result<()> {
        self.r.actors.upsert(def).map_err(failed)
    }
    fn add_via(&mut self, id: &str, via: &str) -> core::Result<()> {
        self.r.actors.add_via(id, via).map_err(failed)
    }
}

/// The connections' rows in the engine's store, each as JSON.
pub struct EngineRows(pub Shared);

fn sql(e: SqlError) -> store::Error {
    store::Error { message: e.message().to_string(), code: e.code().map(str::to_string) }
}

/// An id or a place as the requests name it.
fn text(v: &Value) -> String {
    js_string(v)
}

/// Rows answered, as JSON.
fn rows(r: Result<Vec<xenonet_engine::sql::Row>, SqlError>) -> store::Result<Vec<Value>> {
    r.map(|rows| rows.iter().map(row_json).collect()).map_err(sql)
}

/// A row answered, if any, as JSON.
fn row(r: Result<Option<xenonet_engine::sql::Row>, SqlError>) -> store::Result<Option<Value>> {
    r.map(|row| row.as_ref().map(row_json)).map_err(sql)
}

impl EngineRows {
    fn exec(&self, sql_text: &str) -> store::Result<()> {
        self.0.with(|s| s.db().and_then(|db| xenonet_engine::sql::exec(db, sql_text))).map_err(sql)
    }
}

impl Tx for EngineRows {
    fn in_transaction(&self) -> bool {
        self.0.with(|s| s.in_transaction())
    }
    fn begin(&self) -> store::Result<()> {
        self.exec("BEGIN")
    }
    fn commit(&self) -> store::Result<()> {
        self.exec("COMMIT")
    }
    fn rollback(&self) {
        let _ = self.exec("ROLLBACK");
    }
}

impl Links for EngineRows {
    fn account_links(&self, slug: &str) -> store::Result<Vec<Value>> {
        rows(self.0.with(|s| s.account_links(slug)))
    }
    fn add_account_link(&self, row: &Value) -> store::Result<()> {
        self.0.with(|s| s.add_account_link(row)).map_err(sql)
    }
    fn expire_account_link(&self, link_id: &Value, at: f64) -> store::Result<()> {
        self.0.with(|s| s.expire_account_link(&text(link_id), at)).map_err(sql)
    }
    fn external_ref(&self, key: &Value) -> store::Result<Option<Value>> {
        row(self.0.with(|s| s.external_ref(key)))
    }
    fn add_external_ref(&self, row: &Value) -> store::Result<()> {
        self.0.with(|s| s.add_external_ref(row)).map_err(sql)
    }
}

/// How a project id's making failed: the store's own way, or the maker's.
enum MakeFail {
    Sql(SqlError),
    Store(store::Error),
}

impl From<SqlError> for MakeFail {
    fn from(e: SqlError) -> Self {
        MakeFail::Sql(e)
    }
}

impl Store for EngineRows {
    fn meta(&self, key: &str) -> store::Result<Value> {
        self.0.with(|s| s.meta(key)).map_err(sql)
    }
    fn set_meta(&self, key: &str, value: &str) -> store::Result<()> {
        self.0.with(|s| s.set_meta(key, value)).map_err(sql)
    }
    fn grants(&self) -> store::Result<Vec<Value>> {
        rows(self.0.with(|s| s.grants()))
    }
    fn grant(&self, id: &str) -> store::Result<Option<Value>> {
        row(self.0.with(|s| s.grant(id)))
    }
    fn connection(&self, id: &str) -> store::Result<Option<Value>> {
        row(self.0.with(|s| s.connection(id)))
    }
    fn connections(&self) -> store::Result<Vec<Value>> {
        rows(self.0.with(|s| s.connections()))
    }
    fn connections_of(&self, slug: &str) -> store::Result<Vec<Value>> {
        rows(self.0.with(|s| s.connections_of(slug)))
    }
    fn add_connection(&self, row: &Value) -> store::Result<()> {
        self.0.with(|s| s.add_connection(row)).map_err(sql)
    }
    fn add_grant(&self, row: &Value) -> store::Result<()> {
        self.0.with(|s| s.add_grant(row)).map_err(sql)
    }
    fn change_grant(&self, id: &str, change: &Value) -> store::Result<bool> {
        self.0.with(|s| s.change_grant(id, change)).map_err(sql)
    }
    fn revoke_grant(&self, id: &str, reason: &str, at: f64) -> store::Result<bool> {
        self.0.with(|s| s.revoke_grant(id, reason, at)).map_err(sql)
    }
    fn end_connection(&self, id: &str, at: f64) -> store::Result<()> {
        self.0.with(|s| s.end_connection(id, at)).map_err(sql)
    }
    fn touch_grant(&self, id: &str, at: f64) -> store::Result<()> {
        self.0.with(|s| s.touch_grant(id, at)).map_err(sql)
    }
    fn add_audit(&self, row: &Value) -> store::Result<()> {
        self.0.with(|s| s.add_audit(row)).map_err(sql)
    }
    fn project_id_of(&self, place: &Value, make: &mut dyn FnMut() -> store::Result<Value>, now: f64) -> store::Result<Value> {
        let mut made = || make().map(|v| text(&v)).map_err(MakeFail::Store);
        self.0.with(|s| s.project_id_of(&text(place), &mut made, now)).map_err(|e| match e {
            MakeFail::Sql(e) => sql(e),
            MakeFail::Store(e) => e,
        })
    }
    fn project_id_for(&self, place: &Value) -> store::Result<Value> {
        self.0.with(|s| s.project_id_for(&text(place))).map_err(sql)
    }
    fn place_of_project(&self, project_id: &Value) -> store::Result<Value> {
        self.0.with(|s| s.place_of_project(&text(project_id))).map_err(sql)
    }
}

impl Remote {
    /// The connections over this store, their credentials in `dir`, or the
    /// error that kept them from starting.
    pub fn start(store: Shared, dir: &str, state_dir: &str, port: u16, actors: Arc<dyn Actors>, engine_store: Arc<dyn EngineStore>, state: Arc<dyn State>) -> R<Arc<Remote>> {
        let (host, user) = super::host_and_user();
        let fingerprint = core::fingerprint_of(&host, &user, state_dir);
        let r = Arc::new(Remote {
            core: Core::new(Arc::new(EngineRows(store)), &fingerprint, f64::from(port)),
            dir: dir.into(),
            actors: actors.clone(),
            engine_store,
            state,
            listeners: Mutex::new(Vec::new()),
            picked: Mutex::new(None),
        });
        // An actor's connections are revoked before its record goes.
        let weak: Weak<Remote> = Arc::downgrade(&r);
        actors.before_remove(Arc::new(move |slug: &str| {
            if let Some(r) = weak.upgrade() {
                let _ = r.core.before_remove(&mut r.base(), slug);
            }
        }));
        r.core.start(&mut r.base()).map_err(connection_error)?;
        Ok(r)
    }

    fn base(&self) -> Base<'_> {
        Base { r: self, also: None }
    }

    fn emit(&self, name: &str, data: Value) {
        self.listeners.locked().retain(|tx| tx.send((name.to_string(), data.clone())).is_ok());
    }

    /// Another listener of the connections' events.
    pub fn listen(&self) -> ConnectionEvents {
        let (tx, rx) = mpsc::unbounded_channel();
        self.listeners.locked().push(tx);
        rx
    }

    /// connections.list().connections.
    pub fn view_all(&self) -> R<Vec<Value>> {
        let l = self.core.list(&mut self.base()).map_err(connection_error)?;
        Ok(l["connections"].as_array().cloned().unwrap_or_default())
    }

    pub fn view(&self, grant: &str) -> R<Value> {
        self.core.view(&mut self.base(), grant).map_err(connection_error)
    }

    /// Whether every connection is off, as the credentials' files say.
    pub fn disabled(&self) -> R<bool> {
        Ok(disk::is_disabled(&self.dir))
    }

    pub fn id_key(&self) -> R<Value> {
        self.core.id_key(&mut self.base()).map_err(connection_error)
    }

    pub fn used(&self, grant: &str) {
        let _ = self.core.used(grant, now_ms());
    }

    pub fn recheck_at(&self, grant: &Value, revision: &Value) -> Value {
        match self.core.recheck(&mut self.base(), &js_string(grant), revision) {
            Ok(a) => a,
            Err(e) => json!({ "ok": false, "code": "SERVICE_UNAVAILABLE", "reason": connection_error(e).message }),
        }
    }

    pub fn policy_of(&self, standing: &Value) -> R<Value> {
        self.core.policy(&mut self.base(), standing).map_err(connection_error)
    }
}

/// The Connections trait, for the routes and the endpoint.
pub struct ConnectionsCore(pub Arc<Remote>);

impl Connections for ConnectionsCore {
    fn list(&self) -> R<Value> {
        self.0.core.list(&mut self.0.base()).map_err(connection_error)
    }

    fn disable(&self) -> R<Value> {
        self.0.core.disable(&mut self.0.base()).map_err(connection_error)
    }

    fn enable(&self) -> R<Value> {
        self.0.core.enable(&mut self.0.base()).map_err(connection_error)
    }

    fn pair(&self, body: &Value, places: &Value, identities: &[String]) -> R<Value> {
        let mut base = Base { r: &self.0, also: Some(Also { places, identities }) };
        self.0.core.pair(&mut base, body).map_err(connection_error)
    }

    fn revoke(&self, grant: &str) -> R<Value> {
        self.0.core.revoke(&mut self.0.base(), grant, "revoked").map_err(connection_error)
    }

    fn rotate(&self, grant: &str) -> R<Value> {
        self.0.core.rotate(&mut self.0.base(), grant).map_err(connection_error)
    }

    fn update(&self, grant: &str, projects: &Value, labels: &Value, places: &Value) -> R<Value> {
        let mut base = Base { r: &self.0, also: Some(Also { places, identities: &[] }) };
        let mut change = serde_json::Map::new();
        if !projects.is_null() {
            change.insert("projects".to_string(), projects.clone());
        }
        if !labels.is_null() {
            change.insert("labels".to_string(), labels.clone());
        }
        self.0.core.update(&mut base, grant, &Value::Object(change)).map_err(connection_error)
    }

    fn authenticate(&self, grant: &Value, token: &Value) -> Value {
        let ask = json!({ "grant": grant, "token": token });
        match self.0.core.authenticate(&mut self.0.base(), &ask) {
            Ok(s) => s.to_json(),
            Err(e) => json!({ "ok": false, "code": "SERVICE_UNAVAILABLE", "reason": connection_error(e).message }),
        }
    }

    fn bridge(&self, connection: &str, delta: i32) {
        let _ = self.0.core.bridge(&mut self.0.base(), connection, f64::from(delta));
    }

    fn policy(&self, session: &Value) -> R<Value> {
        self.0.policy_of(session)
    }

    fn any_active(&self) -> bool {
        self.0.core.any_active(&mut self.0.base()).unwrap_or(false)
    }

    fn recheck(&self, grant: &str) -> Value {
        self.0.recheck_at(&json!(grant), &json!(-1))
    }

    fn dir(&self) -> String {
        self.0.dir.clone()
    }

    fn tick(&self) {
        let _ = self.0.core.tick(&mut self.0.base(), now_ms());
    }

    fn subscribe(&self) -> Option<ConnectionEvents> {
        Some(self.0.listen())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use xenonet_service::cores::Pending;

    const ATLAS: &str = "project:/users/nova/projects/atlas-api";

    // The registry, as small as the connections need it: actors by id,
    // each with how it came and what its connections come to.
    type Hook = Arc<dyn Fn(&str) + Send + Sync>;

    struct FakeActors {
        store: Shared,
        known: Mutex<HashMap<String, Value>>,
        hooks: Mutex<Vec<Hook>>,
    }

    impl Actors for FakeActors {
        fn ids(&self) -> Vec<String> {
            self.known.locked().keys().cloned().collect()
        }
        fn has(&self, id: &str) -> bool {
            self.known.locked().contains_key(id)
        }
        fn info(&self, id: &str) -> Option<Value> {
            self.known.locked().get(id).cloned()
        }
        fn remote_of(&self, id: &str) -> Option<Value> {
            self.known.locked().get(id).map(|a| a["remote"].clone()).filter(|r| !r.is_null())
        }
        fn is_remote(&self, id: &str) -> bool {
            self.known.locked().get(id).is_some_and(|a| a["via"].as_array().is_some_and(|v| v.contains(&json!("remote"))))
        }
        fn set_remote(&self, id: &str, remote: &Value) -> R<()> {
            if let Some(a) = self.known.locked().get_mut(id) {
                a["remote"] = remote.clone();
            }
            Ok(())
        }
        fn add_via(&self, id: &str, via: &str) -> R<()> {
            if let Some(a) = self.known.locked().get_mut(id) {
                if let Some(v) = a["via"].as_array_mut() {
                    v.push(json!(via));
                }
            }
            Ok(())
        }
        fn upsert(&self, def: &Value) -> R<()> {
            let id = js_string(&def["id"]);
            let mut a = def.clone();
            a["via"] = json!([def.get("via").cloned().unwrap_or(json!("remote"))]);
            // The actor's row, which its connections refer to.
            let slug = id.trim_start_matches("actor:").to_string();
            let row = json!({ "slug": slug, "kind": "agent", "name": def["name"], "via": "[\"remote\"]", "capabilities": "[]", "created_at": now_ms() });
            self.store.with(|s| s.save_actor(&row)).map_err(|e| xenonet_service::cores::CoreError::new(e.to_string()))?;
            self.known.locked().insert(id, a);
            Ok(())
        }
        fn before_remove(&self, hook: Arc<dyn Fn(&str) + Send + Sync>) {
            self.hooks.locked().push(hook);
        }
    }

    // test/assistant-connections.test.js's first case, natively: pairing a
    // dot makes its actor, a connection and a grant, writes its credential,
    // and the grant then lets its own credential in; revoking ends it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dot_paired_let_in_and_revoked() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-connections-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Shared::new(xenonet_engine::store::EngineStore::open(&dir.join("engine.sqlite").to_string_lossy(), now_ms()).unwrap());
        let creds = dir.join("assistant").to_string_lossy().into_owned();
        let actors = Arc::new(FakeActors { store: store.clone(), known: Mutex::new(HashMap::new()), hooks: Mutex::new(Vec::new()) });
        let remote = Remote::start(store.clone(), &creds, &dir.to_string_lossy(), 47999, actors.clone(), Arc::new(Pending), Arc::new(Pending)).unwrap();
        assert_eq!(actors.hooks.locked().len(), 1, "its connections go before an actor does");
        let c = ConnectionsCore(remote.clone());
        let places = json!({ ATLAS: { "place": ATLAS, "name": "atlas-api" } });
        let ask = json!({ "label": "My dot", "product": "dots", "transport": "openai-secure-mcp-tunnel", "audience": { "description": "Only my ChatGPT account, in my workspace", "verification": "owner_verified", "scope": "owner_endpoint" }, "projects": [ATLAS] });
        let v = c.pair(&ask, &places, &["chatgpt:demo-workspace".to_string()]).unwrap();
        assert_eq!((v["actor_id"].clone(), v["state"].clone(), v["data_mode"].clone()), (json!("actor:dots"), json!("active"), json!("real")));
        let grant = v["grant_id"].as_str().unwrap().to_string();
        let token = disk::read_credential(&creds, &json!(grant)).unwrap()["token"].clone();
        let s = c.authenticate(&json!(grant), &token);
        assert_eq!(s["ok"], true, "{s}");
        assert_eq!(c.authenticate(&json!(grant), &json!("xenonet_assistant_nope"))["ok"], false);
        let p = c.policy(&s).unwrap();
        assert_eq!(p["dataMode"], "real");
        assert_eq!(c.recheck(&grant)["ok"], true);
        assert!(c.any_active());
        assert!(c.list().unwrap()["connections"].as_array().unwrap().iter().any(|x| x["grant_id"] == json!(grant)));
        // A grant whose audience isn't verified reads made-up data only,
        // through every check a real read goes through, and is audited.
        let unverified = json!({ "label": "Work dot", "name": "Work dot", "product": "dots", "transport": "openai-secure-mcp-tunnel", "audience": { "description": "Anyone in my workspace", "verification": "unverified", "scope": "authenticated_user" }, "projects": [ATLAS] });
        let w = c.pair(&unverified, &places, &[]).unwrap();
        assert_eq!(w["data_mode"], "fixture");
        let st = store.clone();
        let reads = super::super::reads::Reads::new(
            Some(remote.clone()),
            Arc::new(Pending),
            Some(actors.clone()),
            Arc::new(Pending),
            Some(st),
            "0.0.0",
            xenonet_service::wire::Emitter::new(),
        );
        let session = json!({ "grantId": w["grant_id"], "actorId": w["actor_id"], "connectionId": w["connection_id"] });
        let read = reads.read(&session, &json!({ "request_id": "r_00000000-0000-4000-8000-000000000000", "operation": "status", "arguments": {} })).await;
        assert_eq!(read["ok"], true, "{read}");
        assert!(read["result"]["warnings"].as_array().unwrap().iter().any(|x| x["code"] == "FIXTURE_DATA"));
        let bad = reads.read(&session, &json!({ "request_id": "r_00000000-0000-4000-8000-000000000001", "operation": "sessions", "arguments": { "path": "/etc" } })).await;
        assert_eq!(bad["error"]["code"], "INVALID_ARGUMENT");
        let audit = super::super::reads::AuditCore(reads.clone());
        use xenonet_service::cores::Audit;
        let recent = audit.recent(w["grant_id"].as_str(), 10.0).unwrap();
        assert_eq!(recent.as_array().unwrap().iter().filter(|r| r["kind"] == "access").count(), 2, "both reads are in the audit: {recent}");
        let mut events = c.subscribe().unwrap();
        c.revoke(&grant).unwrap();
        assert_eq!(c.recheck(&grant)["ok"], false);
        assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|(name, _)| name == "revoked"));
        assert!(c.pair(&json!({ "label": "" }), &places, &[]).unwrap_err().is(xenonet_service::cores::Kind::Connection));
        let _ = std::fs::remove_dir_all(&dir); // Windows keeps a folder whose files are still open.
    }
}
