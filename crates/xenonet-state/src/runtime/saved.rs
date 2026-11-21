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
    let roots: std::collections::BTreeMap<String, Vec<String>> = remembered
        .get("appRoots")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(app, l)| (app.clone(), l.as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect()))
        .collect();
    if !roots.is_empty() {
        let _ = store.set_app_roots(&roots);
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

    /// The folders you named to look for projects in, beside the projects
    /// folder.
    pub fn roots(&self) -> Vec<String> {
        read("the roots you named", self.store.roots(), Value::Array(Vec::new())).as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
    }

    pub fn set_roots(&self, roots: &[String]) -> Result<(), String> {
        self.store.set_roots(roots).map_err(|e| e.message)
    }

    /// The folders you named for each app beside its default one:
    /// app to paths, in the order you named them.
    pub fn app_roots(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        read("the app folders you named", self.store.app_roots(), json!({}))
            .as_object()
            .into_iter()
            .flatten()
            .map(|(app, v)| (app.clone(), v.as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)).collect()))
            .collect()
    }

    pub fn set_app_roots(&self, roots: &std::collections::BTreeMap<String, Vec<String>>) -> Result<(), String> {
        let v: serde_json::Map<String, Value> = roots.iter().filter(|(_, l)| !l.is_empty()).map(|(a, l)| (a.clone(), json!(l))).collect();
        self.store.set_app_roots(&Value::Object(v)).map_err(|e| e.message)
    }

    /// The agents of this machine's sessions, made the first time each is
    /// seen: session to `agt_…`. Nothing without a store.
    pub fn agents_of(&self, sessions: &[String]) -> std::collections::HashMap<String, String> {
        let v = read("the agents", self.store.agents_of(sessions), json!({}));
        v.as_object().into_iter().flatten().filter_map(|(k, a)| Some((k.clone(), a.as_str()?.to_string()))).collect()
    }

    /// What the map found (`[{ path, why }]`) diffed into the store's
    /// checkouts, each change logged: this machine's checkouts by path key.
    /// None without a store.
    pub fn reconcile_checkouts(&self, found: Vec<Value>) -> Option<std::collections::HashMap<String, Value>> {
        let out = self.store.reconcile_checkouts(&Value::Array(found));
        let out = match out {
            Ok(v) => v,
            Err(e) => {
                if !e.is_pending() {
                    crate::runtime::util::log("warn", "projects.reconcile", &format!("the checkouts couldn't be kept: {}", e.message));
                }
                return None;
            }
        };
        for fix in out.get("fixes").and_then(Value::as_array).into_iter().flatten() {
            crate::runtime::util::log("info", "projects.reconcile", fix.as_str().unwrap_or_default());
        }
        Some(
            out.get("checkouts")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| c.get("gone") != Some(&Value::Bool(true)))
                .filter_map(|c| Some((c.get("pathKey")?.as_str()?.to_string(), c.clone())))
                .collect(),
        )
    }

    /// The checkouts the store holds now, the gone ones left out: Err when
    /// the store can't say, an empty list without a store.
    pub fn kept_checkouts(&self) -> Result<Vec<Value>, String> {
        match self.store.checkouts() {
            Ok(v) => Ok(v.get("checkouts").and_then(Value::as_array).into_iter().flatten().filter(|c| c.get("gone") != Some(&Value::Bool(true))).cloned().collect()),
            Err(e) if e.is_pending() => Ok(Vec::new()),
            Err(e) => Err(e.message),
        }
    }

    /// How the map ties a space, from its folder's marker (the Store core's
    /// `space_tie`); by its name without a store, or when the store can't
    /// say.
    pub fn space_tie(&self, marker: &Value) -> Value {
        read("a space's marker", self.store.space_tie(marker), json!({ "tie": "name" }))
    }

    /// `{ names, hidden, pins, rules, known, signins }`.
    pub fn identities(&self) -> Value {
        read("the identities", self.store.identities(), json!({ "names": {}, "hidden": {}, "pins": {}, "known": {}, "signins": {} }))
    }

    /// `identities`, except that a store that can't say is an Err: a rule
    /// that couldn't be read is not the same as no rule, and what places a
    /// run by its rule must not take the one for the other. Empty without
    /// a store.
    pub fn identities_strict(&self) -> Result<Value, String> {
        match self.store.identities() {
            Ok(v) => Ok(v),
            Err(e) if e.is_pending() => Ok(json!({ "names": {}, "hidden": {}, "pins": {}, "known": {}, "signins": {} })),
            Err(e) => Err(e.message),
        }
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

    pub fn set_identity_rule(&self, project: &str, rule: Option<&Value>) {
        wrote("a rule", self.store.set_identity_rule(project, rule));
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
    Saved::new(Arc::new(xenonet_saved::Saved::new(db)))
}

/// A store whose reading of the identities (the rules among them) fails
/// while the flag is set, over one in memory: what a test uses to show a
/// store error is never taken for "no rule".
#[cfg(test)]
pub struct Flaky(Arc<dyn Store>, pub Arc<std::sync::atomic::AtomicBool>);

#[cfg(test)]
pub fn flaky() -> (Saved, Arc<std::sync::atomic::AtomicBool>) {
    let db = Arc::new(xenonet_store::Db::memory().expect("a store in memory"));
    let fails = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (Saved::new(Arc::new(Flaky(Arc::new(xenonet_saved::Saved::new(db)), fails.clone()))), fails)
}

#[cfg(test)]
use std::sync::atomic::Ordering;

#[cfg(test)]
impl Store for Flaky {
    fn ack(&self, session: &str) -> xenonet_api::cores::R<()> {
        self.0.ack(session)
    }
    fn acks(&self) -> xenonet_api::cores::R<Value> {
        self.0.acks()
    }
    fn marks(&self) -> xenonet_api::cores::R<Value> {
        self.0.marks()
    }
    fn layout(&self) -> xenonet_api::cores::R<Value> {
        self.0.layout()
    }
    fn set_layout(&self, positions: &Value) -> xenonet_api::cores::R<()> {
        self.0.set_layout(positions)
    }
    fn clear_layout(&self) -> xenonet_api::cores::R<()> {
        self.0.clear_layout()
    }
    fn theme(&self) -> xenonet_api::cores::R<Value> {
        self.0.theme()
    }
    fn set_theme(&self, id: &str) -> xenonet_api::cores::R<()> {
        self.0.set_theme(id)
    }
    fn environment(&self) -> xenonet_api::cores::R<Value> {
        self.0.environment()
    }
    fn set_environment(&self, id: &str) -> xenonet_api::cores::R<()> {
        self.0.set_environment(id)
    }
    fn workspace_root(&self) -> xenonet_api::cores::R<Value> {
        self.0.workspace_root()
    }
    fn set_workspace_root(&self, dir: Option<&str>) -> xenonet_api::cores::R<()> {
        self.0.set_workspace_root(dir)
    }
    fn roots(&self) -> xenonet_api::cores::R<Value> {
        self.0.roots()
    }
    fn set_roots(&self, roots: &[String]) -> xenonet_api::cores::R<()> {
        self.0.set_roots(roots)
    }
    fn app_roots(&self) -> xenonet_api::cores::R<Value> {
        self.0.app_roots()
    }
    fn set_app_roots(&self, roots: &Value) -> xenonet_api::cores::R<()> {
        self.0.set_app_roots(roots)
    }
    fn reconcile_checkouts(&self, found: &Value) -> xenonet_api::cores::R<Value> {
        self.0.reconcile_checkouts(found)
    }
    fn agents_of(&self, sessions: &[String]) -> xenonet_api::cores::R<Value> {
        self.0.agents_of(sessions)
    }
    fn checkouts(&self) -> xenonet_api::cores::R<Value> {
        self.0.checkouts()
    }
    fn space_tie(&self, marker: &Value) -> xenonet_api::cores::R<Value> {
        self.0.space_tie(marker)
    }
    fn confirm_checkout(&self, id: &str) -> xenonet_api::cores::R<Value> {
        self.0.confirm_checkout(id)
    }
    fn project_rules(&self) -> xenonet_api::cores::R<Value> {
        self.0.project_rules()
    }
    fn set_project_rule(&self, key: &str, rule: Option<&Value>) -> xenonet_api::cores::R<()> {
        self.0.set_project_rule(key, rule)
    }
    fn clear_project_rules(&self, under_key: Option<&str>) -> xenonet_api::cores::R<()> {
        self.0.clear_project_rules(under_key)
    }
    fn app_setting(&self, app: &str, name: &str, scope: &str) -> xenonet_api::cores::R<Option<Value>> {
        self.0.app_setting(app, name, scope)
    }
    fn set_app_setting(&self, app: &str, name: &str, scope: &str, value: Option<&Value>) -> xenonet_api::cores::R<()> {
        self.0.set_app_setting(app, name, scope, value)
    }
    fn identities(&self) -> xenonet_api::cores::R<Value> {
        if self.1.load(Ordering::SeqCst) {
            return Err(xenonet_api::cores::CoreError::new("the store is unreadable"));
        }
        self.0.identities()
    }
    fn set_identity_name(&self, id: &str, name: Option<&str>) -> xenonet_api::cores::R<()> {
        self.0.set_identity_name(id, name)
    }
    fn set_identity_hidden(&self, id: &str, hidden: bool) -> xenonet_api::cores::R<()> {
        self.0.set_identity_hidden(id, hidden)
    }
    fn set_identity_pin(&self, node: &str, identity: Option<&str>) -> xenonet_api::cores::R<()> {
        self.0.set_identity_pin(node, identity)
    }
    fn set_identity_rule(&self, project: &str, rule: Option<&Value>) -> xenonet_api::cores::R<()> {
        self.0.set_identity_rule(project, rule)
    }
    fn set_identity_known(&self, id: &str, value: &Value) -> xenonet_api::cores::R<()> {
        self.0.set_identity_known(id, value)
    }
    fn set_signins(&self, app: &str, spans: &Value) -> xenonet_api::cores::R<()> {
        self.0.set_signins(app, spans)
    }
}
