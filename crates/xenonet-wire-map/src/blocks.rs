//! The map's building blocks (core/blocks/lifecycle.js's Blocks): each
//! with a name, a kind and its health, for /api/health and the doctor. The
//! ones that read for a source of the map say which, so a source that
//! fails is reported once.

use crate::util::{lock, now_ms, num};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use xenonet_blocks::lifecycle::{Lifecycle, Registry};

/// A block's health: { name, kind, state, since, lastOk, lastError,
/// counters, budget }.
pub trait Block: Send + Sync {
    fn name(&self) -> String;
    fn health(&self) -> Value;
}

/// What a lifecycle says, as health() reports it, with the kind's own
/// counters after its own.
pub fn health_of(name: &str, kind: &str, life: &Lifecycle, more: &[(&str, f64)], budget: Value) -> Value {
    let mut counters = Map::new();
    for (k, v) in life.counters.iter() {
        counters.insert(k.to_string(), num(v));
    }
    for (k, v) in more {
        counters.insert((*k).to_string(), num(*v));
    }
    json!({
        "name": name,
        "kind": kind,
        "state": life.state.as_str(),
        "since": num(life.since),
        "lastOk": life.last_ok.map_or(Value::Null, num),
        "lastError": life.last_error.as_ref().map_or(Value::Null, |e| json!({ "at": num(e.at), "message": e.message })),
        "counters": counters,
        "budget": budget,
    })
}

/// A block that is a lifecycle and nothing more: the state store, the
/// normalizers, a client.
pub struct Life {
    pub name: String,
    pub kind: &'static str,
    pub life: Mutex<Lifecycle>,
}

impl Life {
    pub fn new(name: &str, kind: &'static str) -> Arc<Life> {
        Arc::new(Life { name: name.into(), kind, life: Mutex::new(Lifecycle::new(now_ms())) })
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut Lifecycle) -> T) -> T {
        f(&mut lock(&self.life))
    }

    pub fn ok(&self) {
        self.with(|l| l.ok(now_ms()));
    }

    pub fn problem(&self, message: &str) {
        self.with(|l| l.problem(message, now_ms()));
    }

    pub fn count(&self, name: &str) {
        self.with(|l| l.count(name, 1.0));
    }
}

impl Block for Life {
    fn name(&self) -> String {
        self.name.clone()
    }
    fn health(&self) -> Value {
        health_of(&self.name, self.kind, &lock(&self.life), &[], Value::Null)
    }
}

/// A block another part of XenoNet keeps (the engine's store, an
/// observer), by the health it reports.
pub struct Reported(pub Arc<dyn Fn() -> Value + Send + Sync>);

impl Block for Reported {
    fn name(&self) -> String {
        (self.0)().get("name").and_then(Value::as_str).unwrap_or_default().to_string()
    }
    fn health(&self) -> Value {
        (self.0)()
    }
}

#[derive(Default)]
pub struct Blocks {
    inner: Mutex<(Registry, Vec<Arc<dyn Block>>)>,
}

impl Blocks {
    /// Adds a block: a name already taken by another is refused.
    pub fn add(&self, block: Arc<dyn Block>, source: Option<&str>) -> Result<(), String> {
        let mut g = lock(&self.inner);
        let name = block.name();
        let same = g.1.iter().any(|b| Arc::ptr_eq(b, &block));
        g.0.add(&name, same, source)?;
        if !same {
            g.1.push(block);
        }
        Ok(())
    }

    /// Each block's health, with the source it reads for when it has one.
    pub fn health(&self) -> Vec<Value> {
        let (names, list): (Vec<Option<String>>, Vec<Arc<dyn Block>>) = {
            let g = lock(&self.inner);
            (g.1.iter().map(|b| g.0.source(&b.name()).map(str::to_string)).collect(), g.1.clone())
        };
        list.iter()
            .zip(names)
            .map(|(b, source)| {
                let h = b.health();
                let mut m = Map::new();
                m.insert("name".into(), h.get("name").cloned().unwrap_or(Value::Null));
                m.insert("kind".into(), h.get("kind").cloned().unwrap_or(Value::Null));
                if let Some(s) = source {
                    m.insert("source".into(), Value::String(s));
                }
                for (k, v) in h.as_object().into_iter().flatten() {
                    if k != "name" && k != "kind" {
                        m.insert(k.clone(), v.clone());
                    }
                }
                Value::Object(m)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_as_the_api_lists_it() {
        let blocks = Blocks::default();
        let a = Life::new("state", "store");
        blocks.add(a.clone(), None).unwrap();
        blocks.add(a.clone(), Some("x")).unwrap();
        assert!(blocks.add(Life::new("state", "store"), None).is_err(), "one block a name");
        a.ok();
        let h = blocks.health();
        assert_eq!(h.len(), 1);
        let keys: Vec<&str> = h[0].as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["name", "kind", "source", "state", "since", "lastOk", "lastError", "counters", "budget"]);
        assert_eq!(h[0]["state"], "running");
        assert_eq!(h[0]["counters"]["errors"], 0);
    }
}
