//! What the engine and the actors read of the map, through the State
//! trait: the engine twin's narrow readers (liveIds, index, node, live,
//! observedLive, homeOf, identities) and the actors' map adapter (mapOf in
//! core/actors/index.js: ready, has, sessions, recentSession, describe,
//! placeOf). A map that isn't in Rust yet reads as an empty one, so the
//! engine still answers from its own store.

use crate::js::Locked;
use crate::js::{arg, json, obj};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_service::cores::{CoreError, State, R};

/// A State answer: what a pending map stands in for, or what failed.
fn read<T>(r: R<T>, empty: impl FnOnce() -> T) -> Result<T> {
    match r {
        Ok(v) => Ok(v),
        Err(e) if e.is_pending() => Ok(empty()),
        Err(e) => Err(thrown(e)),
    }
}

pub fn thrown(e: CoreError) -> Thrown {
    Thrown { message: e.message, code: e.code.map(|c| Box::new(Value::String(c))), handle: None, stdout: None }
}

fn field(n: &Value, k: &str) -> Arg {
    n.get(k).map_or(Arg::Undef, arg)
}

/// The places (projects and zones) by their path key, and the sessions as
/// the observers match them, made again when the map is.
#[derive(Default)]
struct Kept {
    version: Option<u64>,
    places: HashMap<String, Value>,
    sessions: Vec<Value>,
}

pub struct Map {
    pub state: Arc<dyn State>,
    kept: Mutex<Kept>,
}

impl Map {
    pub fn new(state: Arc<dyn State>) -> Map {
        Map { state, kept: Mutex::new(Kept::default()) }
    }

    fn nodes(&self) -> Result<Vec<Value>> {
        read(self.state.nodes(), Vec::new)
    }

    pub fn node_value(&self, id: &str) -> Result<Option<Value>> {
        read(self.state.node(id), || None)
    }

    // ---------- the engine's readers ----------

    /// [id, lastEventAt] of each live session.
    pub fn live_ids(&self) -> Result<Arg> {
        let ids = read(self.state.live_ids(), Vec::new)?;
        Ok(Arg::List(ids.into_iter().map(|(id, at)| Arg::List(vec![Arg::Text(id), arg(&at)])).collect()))
    }

    /// [id, { id, kind, archived, updatedAt, name, path }] of each node.
    pub fn index(&self) -> Result<Arg> {
        Ok(Arg::List(
            self.nodes()?
                .iter()
                .map(|n| {
                    let id = field(n, "id");
                    Arg::List(vec![
                        id.clone(),
                        obj(vec![
                            ("id", id),
                            ("kind", field(n, "kind")),
                            ("archived", field(n, "archived")),
                            ("updatedAt", field(n, "updatedAt")),
                            ("name", field(n, "name")),
                            ("path", field(n, "path")),
                        ]),
                    ])
                })
                .collect(),
        ))
    }

    /// A node as the engine reads it, or null.
    pub fn node(&self, id: &Arg) -> Result<Arg> {
        let Some(n) = self.node_value(&crate::js::text(id))? else { return Ok(Arg::Null) };
        let identities =
            n.get("identities").and_then(Value::as_array).map(|l| l.iter().map(|i| obj(vec![("id", i.get("id").map_or(Arg::Undef, arg))])).collect()).unwrap_or_default();
        Ok(obj(vec![
            ("id", field(&n, "id")),
            ("kind", field(&n, "kind")),
            ("title", field(&n, "title")),
            ("name", field(&n, "name")),
            ("worktree", field(&n, "worktree")),
            ("cwd", field(&n, "cwd")),
            ("updatedAt", field(&n, "updatedAt")),
            ("branch", field(&n, "branch")),
            ("reader", field(&n, "reader")),
            ("observed", field(&n, "observed")),
            ("path", field(&n, "path")),
            ("archived", field(&n, "archived")),
            ("identities", Arg::List(identities)),
        ]))
    }

    /// A live session (or an observed run) as the engine reads it, or null.
    pub fn live(&self, id: &Arg, now: f64) -> Result<Arg> {
        Ok(read(self.state.live_info(&crate::js::text(id), now), || None)?.as_ref().map_or(Arg::Null, arg))
    }

    pub fn observed_live(&self, id: &Arg) -> Result<bool> {
        read(self.state.observed_live(&crate::js::text(id)), || false)
    }

    /// The project or zone a session counts under, or null.
    pub fn home_of(&self, id: &Arg) -> Result<Arg> {
        let v = read(self.state.home_of(&crate::js::text(id)), || Value::Null)?;
        Ok(if v.is_null() { Arg::Null } else { arg(&v) })
    }

    /// The identities on the map, as { id, handle }.
    pub fn identities(&self) -> Result<Arg> {
        Ok(Arg::List(
            read(self.state.identities(), Vec::new)?
                .iter()
                .map(|i| obj(vec![("id", i.get("id").map_or(Arg::Undef, arg)), ("handle", i.get("handle").map_or(Arg::Undef, arg))]))
                .collect(),
        ))
    }

    // ---------- the actors' map ----------

    fn kept(&self) -> Result<std::sync::MutexGuard<'_, Kept>> {
        let version = self.state.graph_version();
        let fresh = self.kept.locked().version != Some(version);
        if fresh {
            let nodes = self.nodes()?;
            let mut places = HashMap::new();
            let mut sessions = Vec::new();
            for n in &nodes {
                let kind = n.get("kind").and_then(Value::as_str);
                if matches!(kind, Some("project" | "zone")) {
                    if let Some(k) = n.get("path").and_then(Value::as_str).filter(|p| !p.is_empty()).and_then(xenonet_base::paths::path_key) {
                        places.insert(k, n.get("id").cloned().unwrap_or(Value::Null));
                    }
                }
                if kind == Some("session") {
                    let f = |k: &str| n.get(k).cloned().unwrap_or(Value::Null);
                    sessions
                        .push(json!({ "id": f("id"), "app": f("app"), "cwd": f("cwd"), "worktree": f("worktree"), "createdAt": f("createdAt"), "originator": f("originator") }));
                }
            }
            let mut k = self.kept.locked();
            *k = Kept { version: Some(version), places, sessions };
        }
        Ok(self.kept.locked())
    }

    /// Whether the map has been made.
    pub fn ready(&self) -> bool {
        matches!(self.state.graph(), Ok(Some(_)))
    }

    pub fn has(&self, id: &str) -> Result<bool> {
        Ok(self.node_value(id)?.is_some())
    }

    pub fn sessions(&self) -> Result<Value> {
        Ok(Value::Array(self.kept()?.sessions.clone()))
    }

    pub fn recent_session(&self) -> Result<Value> {
        Ok(read(self.state.recent_session(), || None)?.map_or(Value::Null, Value::String))
    }

    pub fn describe(&self, id: &str) -> Result<Value> {
        let Some(n) = self.node_value(id)? else { return Ok(Value::Null) };
        let f = |k: &str| n.get(k).cloned().unwrap_or(Value::Null);
        if n.get("kind").and_then(Value::as_str) != Some("session") {
            return Ok(json!({ "kind": f("kind"), "app": f("app"), "name": f("name"), "place": null, "placeId": null, "path": f("path") }));
        }
        let place_id = read(self.state.home_of(id), || Value::Null)?;
        let place = match place_id.as_str() {
            Some(p) if !p.is_empty() => self.node_value(p)?.and_then(|n| n.get("name").cloned()).unwrap_or(Value::Null),
            _ => Value::Null,
        };
        Ok(json!({ "kind": "session", "app": f("app"), "name": null, "place": place, "placeId": place_id, "path": null }))
    }

    /// The deepest project or folder holding an absolute path: its id, or
    /// null.
    pub fn place_of(&self, p: &str) -> Result<Value> {
        let kept = self.kept()?;
        let mut k = xenonet_base::paths::path_key(p).unwrap_or_default();
        while !k.is_empty() {
            if let Some(id) = kept.places.get(&k).filter(|v| crate::js::truthy_json(v)) {
                return Ok(id.clone());
            }
            k = match k.rfind('/') {
                Some(i) => k[..i].to_string(),
                None => String::new(),
            };
        }
        Ok(Value::Null)
    }

    /// The actors' map, as its calls answer them.
    pub fn call(&self, method: &str, args: &[Arg]) -> Result<Arg> {
        let s = || crate::js::text(&crate::js::at(args, 0));
        Ok(match method {
            "ready" => Arg::Bool(self.ready()),
            "has" => Arg::Bool(self.has(&s())?),
            "sessions" => arg(&self.sessions()?),
            "recentSession" => arg(&self.recent_session()?),
            "describe" => arg(&self.describe(&s())?),
            "placeOf" => arg(&self.place_of(&s())?),
            _ => return Err(crate::js::no_method("actors.map", method)),
        })
    }

    pub fn kind(method: &str) -> &'static str {
        match method {
            "" => "object",
            "ready" | "has" | "sessions" | "recentSession" | "describe" | "placeOf" => "function",
            _ => "undefined",
        }
    }
}

/// A map node as JSON, for what the routes hand back.
pub fn node_json(n: &Arg) -> Value {
    json(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct Fake;

    #[async_trait]
    impl State for Fake {
        fn graph(&self) -> R<Option<Value>> {
            Ok(Some(json!({})))
        }
        fn nodes(&self) -> R<Vec<Value>> {
            Ok(vec![
                json!({ "id": "project:/w/app", "kind": "project", "name": "app", "path": "/w/app" }),
                json!({ "id": "zone:/w", "kind": "zone", "name": "w", "path": "/w" }),
                json!({ "id": "claude:s1", "kind": "session", "app": "claude", "title": "T", "cwd": "/w/app", "identities": [{ "id": "claude:a:o", "x": 1 }] }),
            ])
        }
        fn node(&self, id: &str) -> R<Option<Value>> {
            Ok(self.nodes()?.into_iter().find(|n| n["id"] == id))
        }
        fn home_of(&self, id: &str) -> R<Value> {
            Ok(if id == "claude:s1" { json!("project:/w/app") } else { Value::Null })
        }
    }

    #[test]
    fn the_map_is_read_as_the_engine_and_the_actors_read_it() {
        let m = Map::new(Arc::new(Fake));
        assert_eq!(m.place_of("/W/App/src/x.ts").unwrap(), json!("project:/w/app"));
        assert_eq!(m.place_of("/w/other").unwrap(), json!("zone:/w"));
        assert_eq!(m.place_of("/elsewhere").unwrap(), Value::Null);
        assert_eq!(m.describe("claude:s1").unwrap()["place"], json!("app"));
        assert_eq!(m.sessions().unwrap()[0]["cwd"], json!("/w/app"));
        let n = m.node(&Arg::text("claude:s1")).unwrap();
        assert_eq!(json(&n.field("identities")), json!([{ "id": "claude:a:o" }]));
        // What the map can't say yet reads as nothing there.
        assert_eq!(m.live_ids().unwrap(), Arg::List(vec![]));
        assert!(!m.observed_live(&Arg::text("claude:s1")).unwrap());
    }
}
