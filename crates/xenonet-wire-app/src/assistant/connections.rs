//! Remote actors' connections and their grants (core/actors/connections.js,
//! crates/xenonet-actors/src/connections.rs), over the engine's store.
//! Every check, what a grant may read and how it stands are the Rust's;
//! the actors' registry, the credentials' files
//! (core/assistant/credentials.js) and the adapters are called as the JS
//! called them, and its events go to whoever listens (the endpoint, the
//! tunnels).

use super::{arg, connection_error, thrown_of};
use crate::fx::{now_ms, Locked};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;
use xenonet_actors::adapters::ADAPTERS;
use xenonet_actors::connections::{self as core, Connections as Core};
use xenonet_assistant::credentials as disk;
use xenonet_engine::peer::{Calls, Result as CallResult, Thrown};
use xenonet_engine::sql::Arg;
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

fn text(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

fn value(a: &Arg) -> Value {
    a.to_json()
}

fn fs(e: xenonet_base::fs::FsError) -> Thrown {
    let mut t = Thrown::new(e.message);
    t.code = e.code.map(|c| Box::new(Value::String(c)));
    t
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

impl Base<'_> {
    fn disk(method: &str, args: &[Arg]) -> CallResult<Arg> {
        let a = |i: usize| args.get(i).map_or(Value::Null, value);
        let dir = || args.first().map(text).unwrap_or_default();
        Ok(match method {
            "hashToken" => Arg::text(disk::hash_token(&text(args.first().unwrap_or(&Arg::Undef)))),
            "isToken" => Arg::Bool(disk::is_token(&text(args.first().unwrap_or(&Arg::Undef)))),
            "newToken" => {
                let (token, hash) = disk::new_token();
                Arg::Obj(vec![("token".into(), Arg::text(token)), ("hash".into(), Arg::text(hash))])
            }
            "deleteCredential" => {
                disk::delete_credential(&dir(), &a(1)).map_err(fs)?;
                Arg::Undef
            }
            "markRevision" => {
                disk::mark_revision(&dir(), &a(1), &a(2)).map_err(fs)?;
                Arg::Undef
            }
            "markRevoked" => {
                disk::mark_revoked(&dir(), &a(1), args.get(2).and_then(Arg::as_f64).unwrap_or_else(now_ms)).map_err(fs)?;
                Arg::Undef
            }
            "isMarkedRevoked" => Arg::Bool(disk::is_marked_revoked(&dir(), &a(1))),
            "markedRevoked" => Arg::List(disk::marked_revoked(&dir()).into_iter().map(Arg::text).collect()),
            "markDisabled" => {
                disk::mark_disabled(&dir(), args.get(1).and_then(Arg::as_f64).unwrap_or_else(now_ms)).map_err(fs)?;
                Arg::Undef
            }
            "isDisabled" => Arg::Bool(disk::is_disabled(&dir())),
            "clearDisabled" => {
                disk::clear_disabled(&dir()).map_err(fs)?;
                Arg::Undef
            }
            "revisionOf" => disk::revision_of(&dir(), &a(1)).map_or(Arg::Null, Arg::num),
            "readCredential" => disk::read_credential(&dir(), &a(1)).map_or(Arg::Null, |v| arg(&v)),
            other => return Err(Thrown::new(format!("disk.{other} is not a function"))),
        })
    }
}

impl Calls for Base<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> CallResult<Arg> {
        let r = self.r;
        match (path, method) {
            ("actors", m) => r.actors.registry(m, args.iter().map(value).collect()).map(|v| arg(&v)).map_err(|e| thrown_of(&e)),
            ("disk", m) => Self::disk(m, &args),
            ("", "now" | "wall") => Ok(Arg::num(now_ms())),
            ("", "emit") => {
                let name = args.first().map(text).unwrap_or_default();
                r.emit(&name, args.get(1).map_or(Value::Null, value));
                Ok(Arg::Undef)
            }
            ("", "describe") => {
                let place = args.first().map(text).unwrap_or_default();
                Ok(r.state.node(&place).ok().flatten().and_then(|n| n.get("name").cloned()).map_or(Arg::Null, |n| arg(&n)))
            }
            ("", "taken") => {
                let slug = args.first().map_or(Value::Null, value);
                Ok(Arg::Bool(r.engine_store.waiting_actor_requests(now_ms()).unwrap_or_default().iter().any(|q| q.get("slug") == Some(&slug))))
            }
            ("", "adapterFor") => {
                let product = args.first().map(value).filter(|p| !p.is_null());
                let products: Vec<Option<Value>> = ADAPTERS.iter().map(|a| a.product.map(|p| json!(p))).collect();
                let custom = ADAPTERS.iter().position(|a| a.key == "custom");
                let i = xenonet_actors::adapters::adapter_for(product.as_ref(), &products, custom);
                *r.picked.locked() = i;
                Ok(match i.and_then(|i| ADAPTERS.get(i)) {
                    Some(a) => arg(&json!({ "id": a.id, "product": a.product, "provider": a.provider, "transports": a.transports })),
                    None => Arg::Null,
                })
            }
            ("picked", "capabilities") => {
                let i = (*r.picked.locked()).ok_or_else(|| Thrown::new("picked.capabilities is not a function"))?;
                Ok(Arg::List(ADAPTERS.get(i).map(|a| a.capabilities.iter().map(|c| Arg::text(*c)).collect()).unwrap_or_default()))
            }
            ("", "writeCredential") => {
                let c = args.first().map_or(Value::Null, value);
                let file = disk::write_credential(&r.dir, &c["grant"], &c["token"], c.get("device"), c.get("expiresAt").unwrap_or(&Value::Null), &mut Private).map_err(fs)?;
                Ok(Arg::text(file))
            }
            ("places", "has") => {
                let id = args.first().map(text).unwrap_or_default();
                Ok(Arg::Bool(self.also.as_ref().is_some_and(|a| a.places.get(&id).is_some())))
            }
            ("identities", "has") => {
                let id = args.first().map(text).unwrap_or_default();
                Ok(Arg::Bool(self.also.as_ref().is_some_and(|a| a.identities.contains(&id))))
            }
            (p, m) => Err(Thrown::new(format!("{}{m} is not a function", if p.is_empty() { String::new() } else { format!("{p}.") }))),
        }
    }

    fn get(&mut self, path: &str) -> CallResult<Arg> {
        match path {
            "identities" => Ok(Arg::List(self.also.as_ref().map(|a| a.identities.iter().map(|i| Arg::text(i.clone())).collect()).unwrap_or_default())),
            "places" => Ok(self.also.as_ref().map_or(Arg::Undef, |a| arg(a.places))),
            _ => Ok(Arg::Undef),
        }
    }

    fn kind(&mut self, path: &str) -> CallResult<String> {
        Ok(match path {
            "actors" | "disk" => "object".into(),
            "places" | "identities" if self.also.is_some() => "object".into(),
            _ => "undefined".into(),
        })
    }

    fn now(&mut self) -> f64 {
        now_ms()
    }
}

impl Remote {
    /// The connections over this store, their credentials in `dir`, or the
    /// error that kept them from starting.
    pub fn start(
        store: xenonet_engine::store::Shared,
        dir: &str,
        state_dir: &str,
        port: u16,
        actors: Arc<dyn Actors>,
        engine_store: Arc<dyn EngineStore>,
        state: Arc<dyn State>,
    ) -> R<Arc<Remote>> {
        let (host, user) = super::host_and_user();
        let fingerprint = core::fingerprint_of(&Arg::text(host), &Arg::text(user), &Arg::text(state_dir));
        let r = Arc::new(Remote {
            core: Core::new(store, Arg::text(dir), Arg::text(fingerprint), Arg::num(f64::from(port))),
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
                let _ = r.core.before_remove(&mut r.base(), &Arg::text(slug));
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
        let l = self.core.list(&mut self.base()).map(|a| value(&a)).map_err(connection_error)?;
        Ok(l["connections"].as_array().cloned().unwrap_or_default())
    }

    pub fn view(&self, grant: &str) -> R<Value> {
        self.core.view(&mut self.base(), &Arg::text(grant)).map(|a| value(&a)).map_err(connection_error)
    }

    pub fn disabled(&self) -> R<bool> {
        self.core.disabled(&mut self.base()).map_err(connection_error)
    }

    pub fn id_key(&self) -> R<Value> {
        self.core.id_key(&mut self.base()).map_err(connection_error)
    }

    pub fn used(&self, grant: &str) {
        let _ = self.core.used(&Arg::text(grant), now_ms());
    }

    pub fn recheck_at(&self, grant: &Value, revision: &Value) -> Value {
        match self.core.recheck(&mut self.base(), &arg(grant), &arg(revision)) {
            Ok(a) => value(&a),
            Err(e) => json!({ "ok": false, "code": "SERVICE_UNAVAILABLE", "reason": connection_error(e).message }),
        }
    }

    pub fn policy_of(&self, standing: &Value) -> R<Value> {
        self.core.policy(&mut self.base(), &arg(standing)).map(|a| value(&a)).map_err(connection_error)
    }
}

/// The Connections trait, for the routes and the endpoint.
pub struct ConnectionsCore(pub Arc<Remote>);

impl Connections for ConnectionsCore {
    fn list(&self) -> R<Value> {
        self.0.core.list(&mut self.0.base()).map(|a| value(&a)).map_err(connection_error)
    }

    fn disable(&self) -> R<Value> {
        self.0.core.disable(&mut self.0.base()).map(|a| value(&a)).map_err(connection_error)
    }

    fn enable(&self) -> R<Value> {
        self.0.core.enable(&mut self.0.base()).map(|a| value(&a)).map_err(connection_error)
    }

    fn pair(&self, body: &Value, places: &Value, identities: &[String]) -> R<Value> {
        let mut base = Base { r: &self.0, also: Some(Also { places, identities }) };
        self.0.core.pair(&mut base, &arg(body)).map(|a| value(&a)).map_err(connection_error)
    }

    fn revoke(&self, grant: &str) -> R<Value> {
        self.0.core.revoke(&mut self.0.base(), &Arg::text(grant), &Arg::text("revoked")).map(|a| value(&a)).map_err(connection_error)
    }

    fn rotate(&self, grant: &str) -> R<Value> {
        self.0.core.rotate(&mut self.0.base(), &Arg::text(grant)).map(|a| value(&a)).map_err(connection_error)
    }

    fn update(&self, grant: &str, projects: &Value, labels: &Value, places: &Value) -> R<Value> {
        let mut base = Base { r: &self.0, also: Some(Also { places, identities: &[] }) };
        let mut change = Vec::new();
        if !projects.is_null() {
            change.push(("projects".to_string(), arg(projects)));
        }
        if !labels.is_null() {
            change.push(("labels".to_string(), arg(labels)));
        }
        self.0.core.update(&mut base, &Arg::text(grant), &Arg::Obj(change)).map(|a| value(&a)).map_err(connection_error)
    }

    fn authenticate(&self, grant: &Value, token: &Value) -> Value {
        let ask = Arg::Obj(vec![("grant".into(), arg(grant)), ("token".into(), arg(token))]);
        match self.0.core.authenticate(&mut self.0.base(), &ask) {
            Ok(s) => value(&s.to_arg()),
            Err(e) => json!({ "ok": false, "code": "SERVICE_UNAVAILABLE", "reason": connection_error(e).message }),
        }
    }

    fn bridge(&self, connection: &str, delta: i32) {
        let _ = self.0.core.bridge(&mut self.0.base(), &Arg::text(connection), f64::from(delta));
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
        store: xenonet_engine::store::Shared,
        known: Mutex<HashMap<String, Value>>,
        hooks: Mutex<Vec<Hook>>,
    }

    impl Actors for FakeActors {
        fn registry(&self, method: &str, args: Vec<Value>) -> R<Value> {
            let mut known = self.known.locked();
            let id = args.first().map(xenonet_base::text::js_string).unwrap_or_default();
            Ok(match method {
                "ids" => json!(known.keys().cloned().collect::<Vec<_>>()),
                "has" => json!(known.contains_key(&id)),
                "info" => known.get(&id).cloned().unwrap_or(Value::Null),
                "remoteOf" => known.get(&id).map_or(Value::Null, |a| a["remote"].clone()),
                "isRemote" => json!(known.get(&id).is_some_and(|a| a["via"].as_array().is_some_and(|v| v.contains(&json!("remote"))))),
                "setRemote" => {
                    if let Some(a) = known.get_mut(&id) {
                        a["remote"] = args.get(1).cloned().unwrap_or(Value::Null);
                    }
                    Value::Null
                }
                "addVia" => {
                    if let Some(a) = known.get_mut(&id) {
                        if let Some(v) = a["via"].as_array_mut() {
                            v.push(args.get(1).cloned().unwrap_or(Value::Null));
                        }
                    }
                    Value::Null
                }
                "upsert" => {
                    let def = args.first().cloned().unwrap_or(Value::Null);
                    let id = xenonet_base::text::js_string(&def["id"]);
                    let mut a = def.clone();
                    a["via"] = json!([def.get("via").cloned().unwrap_or(json!("remote"))]);
                    // The actor's row, which its connections refer to.
                    let slug = id.trim_start_matches("actor:").to_string();
                    let row = arg(&json!({ "slug": slug, "kind": "agent", "name": def["name"], "via": "[\"remote\"]", "capabilities": "[]", "created_at": now_ms() }));
                    self.store.with(|s| s.save_actor(&row)).map_err(|e| xenonet_service::cores::CoreError::new(e.to_string()))?;
                    known.insert(id, a);
                    Value::Null
                }
                _ => Value::Null,
            })
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
        let store = xenonet_engine::store::Shared::new(xenonet_engine::store::EngineStore::open(&dir.join("engine.sqlite").to_string_lossy(), now_ms()).unwrap());
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
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
