//! The engine's tables in the one store (`xenonet_store::engine`): as the
//! engine keeps what it knows (`xenonet_engine::store::Store`), as the
//! service reads them (the EngineStore core), and their health as a block.

use serde_json::{Map, Value};
use xenonet_api::cores::{CoreError, EngineStore as Trait, R};
use xenonet_engine::engine::Thrown;
use xenonet_engine::store::{Kept, Pin, Store as Engine};
use xenonet_store::engine::{row_json, EngineStore, Shared};
use xenonet_store::sql::{Result, Row, SqlError};
use xenonet_store::StoreError;

fn core_error(e: SqlError) -> CoreError {
    CoreError::new(e.message().to_string())
}

/// The engine's tables, for the engine and for the service's own reads.
#[derive(Clone)]
pub struct Store(pub Shared);

/// A failure as the engine passes it on: the store's words and code.
pub(crate) fn thrown(e: SqlError) -> Thrown {
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

// A row goes to the engine as `row_json` reads it, as the engine read the
// store's rows before it had a store of its own.
impl Engine for Store {
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
    fn agent_known(&self, session: &str) -> Kept<Option<String>> {
        let db = self.0.with(|s| s.db().clone());
        db.agent_known(session).map(|a| a.map(|a| a.as_str().to_string())).map_err(kept)
    }
    fn session_of(&self, agent: &xenonet_model::ids::AgentId) -> Kept<Option<String>> {
        let db = self.0.with(|s| s.db().clone());
        db.session_of(agent).map_err(kept)
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

impl Trait for Store {
    fn meta(&self, key: &str) -> R<Option<String>> {
        let v = self.0.with(|s| s.meta(key)).map_err(core_error)?;
        Ok(match v {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(xenonet_text::text::display(&other)),
        })
    }
    fn set_meta(&self, key: &str, value: &str) -> R<()> {
        self.0.with(|s| s.set_meta(key, value)).map_err(core_error)
    }
    fn actor_token_times(&self) -> R<Vec<Value>> {
        self.0.with(EngineStore::actor_token_times).map_err(core_error)
    }
    fn expired_actor_tokens(&self) -> R<Value> {
        self.0.with(EngineStore::expired_actor_tokens).map(Value::Array).map_err(core_error)
    }
    fn waiting_actor_requests(&self, now: f64) -> R<Vec<Value>> {
        self.0.with(|s| s.waiting_actor_requests(now)).map(|r| r.iter().map(row_json).collect()).map_err(core_error)
    }
    fn native(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(std::sync::Arc::new(self.0.clone()))
    }
    fn connection(&self, id: &str) -> R<Option<Value>> {
        self.0.with(|s| s.connection(id)).map(|r| r.as_ref().map(row_json)).map_err(core_error)
    }
}

/// A building block's health, as /api/health lists it: its name and kind,
/// then what its lifecycle says.
pub fn block_health(name: &str, kind: &str, l: &xenonet_blocks::lifecycle::Lifecycle) -> Value {
    let n = xenonet_text::json::number;
    let counters: Map<String, Value> = l.counters.iter().map(|(k, v)| (k.to_string(), n(v))).collect();
    serde_json::json!({
        "name": name,
        "kind": kind,
        "state": l.state.as_str(),
        "since": n(l.since),
        "lastOk": l.last_ok.map(n),
        "lastError": l.last_error.as_ref().map(|e| serde_json::json!({ "at": n(e.at), "message": e.message })),
        "counters": counters,
        "budget": null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_reads_meta_and_registrations() {
        let s = Shared::new(EngineStore::memory(1000.0).unwrap());
        s.with(|s| s.pin("claude:a", 7.0, 2000.0, true, 1000.0)).unwrap();
        let pins = s.with(|s| s.pins("claude:a")).unwrap();
        assert_eq!(pins.len(), 1);
        // An integer column reads as a number.
        assert_eq!(row_json(&pins[0])["pid"], serde_json::json!(7));
        assert!(s.with(|s| s.actor_row("nobody")).unwrap().is_none());
        assert!(!s.with(|s| s.has_actor_ref("actor:a", "r", 0.0)).unwrap());
        let t = Store(s);
        t.set_meta("gateway", "{}").unwrap();
        assert_eq!(t.meta("gateway").unwrap().as_deref(), Some("{}"));
        assert_eq!(t.meta("none").unwrap(), None);
        assert_eq!(t.waiting_actor_requests(0.0).unwrap(), [] as [Value; 0]);
    }
}
