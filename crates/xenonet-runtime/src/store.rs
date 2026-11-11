//! The engine's database (crates/xenonet-engine/src/store.rs), as the
//! service reads it (the EngineStore trait), and its health as a block.

use serde_json::Value;
use xenonet_api::cores::{CoreError, EngineStore as Trait, R};
use xenonet_store::engine::{row_json, Shared};
use xenonet_store::sql::SqlError;

fn core_error(e: SqlError) -> CoreError {
    CoreError::new(e.message().to_string())
}

/// The service's own reads of the engine's database.
pub struct Store(pub Shared);

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
        self.0.with(xenonet_store::engine::EngineStore::actor_token_times).map_err(core_error)
    }
    fn expired_actor_tokens(&self) -> R<Value> {
        self.0.with(xenonet_store::engine::EngineStore::expired_actor_tokens).map(Value::Array).map_err(core_error)
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
    let counters: serde_json::Map<String, Value> = l.counters.iter().map(|(k, v)| (k.to_string(), n(v))).collect();
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
    use xenonet_store::engine::EngineStore;

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
