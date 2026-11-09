//! What the engine and the actors read of the map, through the State
//! trait: the engine's narrow readers (`host::MapReader`: live ids, the
//! index, a node, a live session, whether one is only observed, a
//! session's home, the identities) and the actors' map adapter (ready, has,
//! sessions, recentSession, describe, placeOf). A map that isn't made yet
//! reads as an empty one, so the engine still answers from its own store.

use crate::js::arg;
use crate::js::Locked;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_engine::engine::Fail;
use xenonet_engine::host::IndexEntry;
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

/// A map answer as the engine takes it.
fn engine<T>(r: Result<T>) -> xenonet_engine::engine::Result<T> {
    r.map_err(Fail::from)
}

/// An object with the fields a node has, of these, in this order.
fn pick(n: &Value, keys: &[&str]) -> Value {
    Value::Object(keys.iter().filter_map(|k| n.get(*k).map(|v| ((*k).to_string(), v.clone()))).collect::<serde_json::Map<String, Value>>())
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

    /// Each live session's id and when its transcript last moved.
    pub fn live_ids(&self) -> xenonet_engine::engine::Result<Vec<(String, f64)>> {
        let ids = engine(read(self.state.live_ids(), Vec::new))?;
        Ok(ids.into_iter().map(|(id, at)| (id, xenonet_base::text::js_number(Some(&at)))).collect())
    }

    /// Every node, as the engine indexes it.
    pub fn index(&self) -> xenonet_engine::engine::Result<Vec<IndexEntry>> {
        let s = |n: &Value, k: &str| n.get(k).and_then(Value::as_str).map(str::to_string);
        Ok(engine(self.nodes())?
            .iter()
            .map(|n| IndexEntry {
                id: s(n, "id").unwrap_or_default(),
                kind: s(n, "kind"),
                archived: crate::js::truthy_json(&n["archived"]),
                updated_at: n.get("updatedAt").and_then(Value::as_f64),
                name: s(n, "name"),
                path: s(n, "path"),
            })
            .collect())
    }

    /// A node as the engine reads it, or None.
    pub fn node(&self, id: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        let Some(n) = engine(self.node_value(id))? else { return Ok(None) };
        let identities: Vec<Value> = n.get("identities").and_then(Value::as_array).map(|l| l.iter().map(|i| pick(i, &["id"])).collect()).unwrap_or_default();
        let mut out = pick(&n, &["id", "kind", "title", "name", "worktree", "cwd", "updatedAt", "branch", "reader", "observed", "path", "archived"]);
        out["identities"] = Value::Array(identities);
        Ok(Some(out))
    }

    /// A live session (or an observed run) as the engine reads it, or None.
    pub fn live(&self, id: &str, now: f64) -> xenonet_engine::engine::Result<Option<Value>> {
        engine(read(self.state.live_info(id, now), || None))
    }

    pub fn observed_live(&self, id: &str) -> xenonet_engine::engine::Result<bool> {
        engine(read(self.state.observed_live(id), || false))
    }

    /// The project or zone a session counts under, or None.
    pub fn home_of(&self, id: &str) -> xenonet_engine::engine::Result<Option<Value>> {
        let v = engine(read(self.state.home_of(id), || Value::Null))?;
        Ok((!v.is_null()).then_some(v))
    }

    /// The identities on the map, as { id, handle }.
    pub fn identities(&self) -> xenonet_engine::engine::Result<Vec<Value>> {
        Ok(engine(read(self.state.identities(), Vec::new))?.iter().map(|i| pick(i, &["id", "handle"])).collect())
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
        let n = m.node("claude:s1").unwrap().unwrap();
        assert_eq!(n["identities"], json!([{ "id": "claude:a:o" }]));
        assert_eq!(n.as_object().unwrap().keys().collect::<Vec<_>>(), ["id", "kind", "title", "cwd", "identities"], "the fields it has, in the engine's order");
        // What the map can't say yet reads as nothing there.
        assert_eq!(m.live_ids().unwrap(), vec![]);
        assert!(!m.observed_live("claude:s1").unwrap());
    }
}
