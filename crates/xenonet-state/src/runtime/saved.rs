//! What XenoNet saves for you, as the map reads and changes it: the Store
//! core (`xenonet_api::cores::Store`, kept in the store by the runtime).
//! A read that fails answers as nothing saved, and a change that fails is
//! logged, so the map goes on.

use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_api::cores::Store;

#[derive(Clone)]
pub struct Saved {
    store: Arc<dyn Store>,
}

// The app whose setting the projects folder's picks are: the sorter's,
// or XenoNet's own when no app sorts.
fn picks_app() -> &'static str {
    xenonet_adapter::apps::sorter().map_or("xenonet", |a| a.id.as_str())
}

// With no store (it couldn't be opened, and the service said why), there's
// nothing saved and nothing to say again.
fn read(what: &str, v: xenonet_api::cores::R<Value>, empty: Value) -> Value {
    v.unwrap_or_else(|e| {
        if !e.is_pending() {
            crate::runtime::util::log("warn", "store.read", &format!("{what} couldn't be read: {}", e.message));
        }
        empty
    })
}

fn wrote(what: &str, r: xenonet_api::cores::R<()>) {
    match r {
        Err(e) if !e.is_pending() => crate::runtime::util::log("error", "store.save", &format!("couldn't save {what}: {}", e.message)),
        _ => {}
    }
}

/// Saves what a made-up world remembers of identities (`{ names, known,
/// signins }`), as if it had been seen: the demo's.
pub fn seed(store: &Saved, remembered: &Value) {
    for (id, name) in remembered.get("names").and_then(Value::as_object).into_iter().flatten() {
        store.set_identity_name(id, name.as_str().map(String::from));
    }
    for (id, known) in remembered.get("known").and_then(Value::as_object).into_iter().flatten() {
        store.set_identity_known(id, known);
    }
    for (app, spans) in remembered.get("signins").and_then(Value::as_object).into_iter().flatten() {
        store.set_signins(app, spans);
    }
}

impl Saved {
    pub fn new(store: Arc<dyn Store>) -> Saved {
        Saved { store }
    }

    pub fn core(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn layout(&self) -> Value {
        read("the layout", self.store.layout(), json!({}))
    }

    /// Forgets each place in `ids`.
    pub fn forget_layout(&self, ids: &[String]) {
        let gone: serde_json::Map<String, Value> = ids.iter().map(|k| (k.clone(), Value::Null)).collect();
        wrote("the layout", self.store.set_layout(&Value::Object(gone)));
    }

    pub fn acks(&self) -> Value {
        read("the dismissed alerts", self.store.acks(), json!({}))
    }

    pub fn project_rules(&self) -> Value {
        read("the projects folder's rules", self.store.project_rules(), json!({}))
    }

    pub fn set_project_rule(&self, key: &str, rule: Option<Value>) {
        wrote("a rule", self.store.set_project_rule(key, rule.as_ref()));
    }

    pub fn clear_project_rules(&self, under_key: Option<&str>) {
        wrote("the rules", self.store.clear_project_rules(under_key));
    }

    /// The sorter's picks for a projects folder, by its path key.
    pub fn codex_picks(&self, root_key: Option<&str>) -> Option<Value> {
        let key = root_key.filter(|k| !k.is_empty())?;
        self.store.app_setting(picks_app(), "picks", key).ok().flatten().filter(|v| !v.is_null())
    }

    pub fn set_codex_picks(&self, root_key: Option<&str>, picks: Option<Value>) -> Result<(), String> {
        let key = root_key.filter(|k| !k.is_empty()).ok_or("say which projects folder the picks are for")?;
        self.store.set_app_setting(picks_app(), "picks", key, picks.as_ref()).map_err(|e| e.message)
    }

    pub fn workspace_root(&self) -> Value {
        read("the projects folder you chose", self.store.workspace_root(), Value::Null)
    }

    pub fn set_workspace_root(&self, dir: Option<String>) {
        wrote("the projects folder", self.store.set_workspace_root(dir.as_deref()));
    }

    /// `{ names, hidden, pins, known, signins }`.
    pub fn identities(&self) -> Value {
        read("the identities", self.store.identities(), json!({ "names": {}, "hidden": {}, "pins": {}, "known": {}, "signins": {} }))
    }

    pub fn set_identity_name(&self, id: &str, name: Option<String>) {
        wrote("an identity's name", self.store.set_identity_name(id, name.as_deref()));
    }

    pub fn set_identity_hidden(&self, id: &str, hidden: bool) {
        wrote("an identity", self.store.set_identity_hidden(id, hidden));
    }

    pub fn set_identity_pin(&self, node: &str, identity: Option<String>) {
        wrote("a pin", self.store.set_identity_pin(node, identity.as_deref()));
    }

    pub fn set_identity_known(&self, id: &str, value: &Value) {
        wrote("an identity", self.store.set_identity_known(id, value));
    }

    pub fn set_signins(&self, app: &str, spans: &Value) {
        wrote("the sign-ins", self.store.set_signins(app, spans));
    }
}

/// What a test saves: the service's Store core, over a store in memory.
#[cfg(test)]
pub fn in_memory() -> Saved {
    let db = Arc::new(xenonet_store::Db::memory().expect("a store in memory"));
    Saved::new(Arc::new(xenonet_runtime::saved::Saved::new(db)))
}
