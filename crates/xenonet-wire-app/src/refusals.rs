//! Provider refusals (core/refusals/store.js, query.js, demo.js;
//! crates/xenonet-sources/src/refusals): every one XenoNet sees, kept in the
//! state folder's refusals/ with its context when you've turned that on,
//! and the demo's made up.

use crate::fx::now_ms;
use crate::fx::Locked;
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;
use xenonet_service::cores::{CoreError, Refusals, R};
use xenonet_sources::refusals::{demo, query, store};

pub struct RefusalStore {
    dir: String,
    core: Mutex<store::Refusals>,
}

impl RefusalStore {
    pub fn new(state_dir: &str, home: &str) -> Self {
        let dir = Path::new(state_dir).join("refusals").to_string_lossy().into_owned();
        RefusalStore { core: Mutex::new(store::Refusals::new(&dir, home)), dir }
    }

    // contextOn(dir), with XENONET_REFUSAL_CONTEXT over what you chose.
    fn context(&self) -> bool {
        store::context_on(Path::new(&self.dir), std::env::var("XENONET_REFUSAL_CONTEXT").ok().as_deref())
    }
}

fn err(e: store::StoreError) -> CoreError {
    let mut c = CoreError::new(e.message);
    c.code = e.code;
    c
}

impl Refusals for RefusalStore {
    fn record(&self, signal: &Value, event: &Value) -> R<Option<Value>> {
        let mut core = self.core.locked();
        if core.known(signal, event) {
            return Ok(None);
        }
        core.record(signal, event, now_ms(), self.context()).map_err(err)
    }

    fn list(&self) -> R<Value> {
        Ok(Value::Array(self.core.locked().list(0.0, None, None)))
    }

    fn get(&self, id: &str) -> R<Option<Value>> {
        Ok(self.core.locked().get(id))
    }

    fn dir(&self) -> String {
        self.dir.clone()
    }

    fn context_on(&self) -> R<bool> {
        Ok(self.context())
    }

    fn set_context(&self, on: bool) -> R<bool> {
        store::set_context(Path::new(&self.dir), on).map_err(err)
    }

    fn query(&self, records: &Value, params: &Value) -> R<Value> {
        let records = records.as_array().ok_or_else(|| CoreError::new("records.filter is not a function"))?;
        Ok(query::query_refusals(records, params))
    }

    fn demo_records(&self) -> R<Value> {
        Ok(Value::Array(demo::demo_refusals(now_ms())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_demo_and_an_empty_store() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-refusals-{}", std::process::id()));
        let r = RefusalStore::new(&dir.to_string_lossy(), "/home/nova");
        assert_eq!(r.list().unwrap(), json!([]));
        assert_eq!(r.get("20261017-0000abcd").unwrap(), None);
        let demo = r.demo_records().unwrap();
        assert!(!demo.as_array().unwrap().is_empty());
        let q = r.query(&demo, &json!({})).unwrap();
        assert!(q.is_object());
        assert!(r.dir().ends_with("refusals"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
