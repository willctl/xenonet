//! The engine's store for its tests: the real tables, in a store in
//! memory, as xenonet-runtime's `store::Store` gives them to the engine.
//! A copy of that impl, since the engine's own tests can't link the
//! runtime's.

use crate::engine::Thrown;
use crate::store::{Kept, Pin, Store};
use serde_json::{Map, Value};
use xenonet_store::engine::{row_json, EngineStore, Shared};
use xenonet_store::sql::{Result, Row, SqlError};
use xenonet_store::StoreError;

pub struct Tables(pub Shared);

impl Tables {
    pub fn memory() -> Tables {
        Tables::at(0.0)
    }
    pub fn at(now: f64) -> Tables {
        Tables(Shared::new(EngineStore::memory(now).unwrap()))
    }
}

fn thrown(e: SqlError) -> Thrown {
    Thrown { message: e.message().to_string(), code: e.code().map(str::to_string) }
}
fn kept(e: StoreError) -> Thrown {
    Thrown::coded(e.message, e.code)
}
fn one(r: Result<Option<Row>>) -> Kept<Option<Value>> {
    r.map(|r| r.as_ref().map(row_json)).map_err(thrown)
}
fn all(r: Result<Vec<Row>>) -> Kept<Vec<Value>> {
    r.map(|r| r.iter().map(row_json).collect()).map_err(thrown)
}

impl Store for Tables {
    fn send(&self, m: &Value, now: f64) -> Kept<f64> {
        self.0.with(|s| s.send(m, now)).map_err(thrown)
    }
    fn deliver(&self, id: f64, agent: &str) -> Kept<()> {
        self.0.with(|s| s.deliver(id, agent)).map_err(thrown)
    }
    fn message(&self, id: f64) -> Kept<Option<Value>> {
        one(self.0.with(|s| s.message(id)))
    }
    fn scoped(&self, scope: &str, since: f64) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.scoped(scope, since)))
    }
    fn unseen(&self, agent: &str) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.unseen(agent)))
    }
    fn mark_seen(&self, agent: &str, ids: &[f64], now: f64) -> Kept<()> {
        self.0.with(|s| s.mark_seen(agent, ids, now)).map_err(thrown)
    }
    fn deliveries(&self, id: f64) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.deliveries(id)))
    }
    fn recent(&self, since: f64) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.recent(since)))
    }
    fn claim(&self, agent: &str, pattern: &str, note: &Value, expires_at: f64, now: f64) -> Kept<()> {
        self.0.with(|s| s.claim(agent, pattern, note, expires_at, now)).map_err(thrown)
    }
    fn release(&self, agent: &str, patterns: Option<&[String]>) -> Kept<f64> {
        self.0.with(|s| s.release(agent, patterns)).map_err(thrown)
    }
    fn claims(&self, now: f64) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.claims(now)))
    }
    fn set_focus(&self, agent: &str, text: &str, now: f64) -> Kept<()> {
        self.0.with(|s| s.set_focus(agent, text, now)).map(|_| ()).map_err(thrown)
    }
    fn focus(&self, agent: &str) -> Kept<Option<Value>> {
        one(self.0.with(|s| s.focus(agent)))
    }
    fn touch(&self, agent: &str, contact: &Value, now: f64) -> Kept<()> {
        self.0.with(|s| s.touch(agent, contact, now)).map_err(thrown)
    }
    fn ended(&self, agent: &str, now: f64) -> Kept<()> {
        self.0.with(|s| s.ended(agent, now)).map_err(thrown)
    }
    fn contact(&self, agent: &str) -> Kept<Option<Value>> {
        one(self.0.with(|s| s.contact(agent)))
    }
    fn contacts(&self, since: f64) -> Kept<Vec<Value>> {
        all(self.0.with(|s| s.contacts(since)))
    }
    fn pins(&self, agent: &str) -> Kept<Vec<Pin>> {
        let rows = self.0.with(|s| s.pins(agent)).map_err(thrown)?;
        Ok(rows.iter().map(|p| Pin { pid: p.num("pid").unwrap_or(f64::NAN), start: p.num("start").unwrap_or(f64::NAN), sure: p.num("sure").is_some_and(|s| s != 0.0) }).collect())
    }
    fn pin(&self, agent: &str, pid: f64, start: f64, sure: bool, now: f64) -> Kept<()> {
        self.0.with(|s| s.pin(agent, pid, start, sure, now)).map_err(thrown)
    }
    fn unpin(&self, agent: &str, pid: f64, start: f64) -> Kept<()> {
        self.0.with(|s| s.unpin(agent, pid, start)).map_err(thrown)
    }
    fn drop_guesses(&self, agent: &str) -> Kept<()> {
        self.0.with(|s| s.drop_guesses(agent)).map_err(thrown)
    }
    fn actor_targets(&self) -> Kept<Vec<Value>> {
        all(self.0.with(EngineStore::actor_targets))
    }
    fn delivery_policy(&self) -> Kept<Option<Map<String, Value>>> {
        self.0 .0.db().delivery_policy().map_err(kept)
    }
    fn put_delivery_policy(&self, data: &Map<String, Value>) -> Kept<()> {
        self.0 .0.db().put_delivery_policy(data).map_err(kept)
    }
    fn wake_budget(&self) -> Kept<Vec<(String, Vec<f64>)>> {
        self.0 .0.db().wake_budget().map_err(kept)
    }
    fn put_wake_budget(&self, budget: &[(String, Vec<f64>)]) -> Kept<()> {
        self.0 .0.db().put_wake_budget(budget).map_err(kept)
    }
    fn add_delivery_audit(&self, line: &Value) -> Kept<()> {
        self.0 .0.db().add_delivery_audit(line).map_err(kept)
    }
    fn delivery_audit(&self) -> Kept<Vec<Value>> {
        self.0 .0.db().delivery_audit().map_err(kept)
    }
}
