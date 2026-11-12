//! What XenoNet saves for you, as the routes and the map ask it (the
//! Store core): the store's state domain (`xenonet_store::state`), with
//! what each change does to it, as core/store.js did it.

use serde_json::Value;
use std::sync::Arc;
use xenonet_api::cores::{CoreError, Store, R};
use xenonet_os::paths::is_under;
use xenonet_store::{Db, StoreError};
use xenonet_text::units::to_fixed;

/// The store's state domain, stamping what it keeps with `clock`.
pub struct Saved {
    db: Arc<Db>,
    clock: Arc<dyn Fn() -> f64 + Send + Sync>,
}

impl Saved {
    pub fn new(db: Arc<Db>) -> Saved {
        Saved::with_clock(db, Arc::new(xenonet_text::time::now_ms))
    }

    pub fn with_clock(db: Arc<Db>, clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Saved {
        Saved { db, clock }
    }
}

fn failed(e: StoreError) -> CoreError {
    CoreError::new(e.message)
}

/// What setLayout(positions) does to the layout: each id's new place (to
/// two decimals), or None to forget it. A place that isn't three finite
/// numbers is left alone.
pub fn layout_changes(positions: &Value) -> Vec<(String, Option<[f64; 3]>)> {
    let Value::Object(m) = positions else { return Vec::new() };
    let mut out = Vec::new();
    for (id, p) in xenonet_text::utf16::key_order(m.clone()) {
        if p.is_null() {
            out.push((id, None));
            continue;
        }
        let xyz: Vec<Option<f64>> = ["x", "y", "z"].iter().map(|k| p.get(*k).and_then(Value::as_f64).filter(|n| n.is_finite())).collect();
        if let [Some(x), Some(y), Some(z)] = xyz[..] {
            let fixed = |n: f64| to_fixed(n, 2).parse::<f64>().unwrap_or(f64::NAN) + 0.0;
            out.push((id, Some([fixed(x), fixed(y), fixed(z)])));
        }
    }
    out
}

/// The rules clearProjectRules takes out: every one, or with a projects
/// folder's path key, only those for the folders inside it.
pub fn rules_to_clear(keys: &[String], under_key: Option<&str>) -> Vec<String> {
    keys.iter().filter(|k| under_key.is_none_or(|u| is_under(k, u) && k.as_str() != u)).cloned().collect()
}

impl Store for Saved {
    fn ack(&self, session: &str) -> R<()> {
        self.db.ack(session, (self.clock)()).map_err(failed)
    }
    fn acks(&self) -> R<Value> {
        self.db.acks().map_err(failed)
    }
    fn marks(&self) -> R<Value> {
        self.db.marks().map_err(failed)
    }
    fn layout(&self) -> R<Value> {
        self.db.layout().map_err(failed)
    }
    fn set_layout(&self, positions: &Value) -> R<()> {
        self.db.set_layout(&layout_changes(positions)).map_err(failed)
    }
    fn clear_layout(&self) -> R<()> {
        self.db.clear_layout().map_err(failed)
    }
    fn theme(&self) -> R<Value> {
        self.db.setting("theme").map_err(failed)
    }
    fn set_theme(&self, id: &str) -> R<()> {
        self.db.set_setting("theme", Some(&Value::String(id.into()))).map_err(failed)
    }
    fn environment(&self) -> R<Value> {
        self.db.setting("environment").map_err(failed)
    }
    fn set_environment(&self, id: &str) -> R<()> {
        self.db.set_setting("environment", Some(&Value::String(id.into()))).map_err(failed)
    }
    fn workspace_root(&self) -> R<Value> {
        self.db.setting("workspace.root").map_err(failed)
    }
    fn set_workspace_root(&self, dir: Option<&str>) -> R<()> {
        self.db.set_setting("workspace.root", dir.map(|d| Value::String(d.into())).as_ref()).map_err(failed)
    }
    fn roots(&self) -> R<Value> {
        let v = self.db.setting("workspace.roots").map_err(failed)?;
        Ok(if v.is_array() { v } else { Value::Array(Vec::new()) })
    }
    fn set_roots(&self, roots: &[String]) -> R<()> {
        let v = (!roots.is_empty()).then(|| Value::Array(roots.iter().cloned().map(Value::String).collect()));
        self.db.set_setting("workspace.roots", v.as_ref()).map_err(failed)
    }
    fn reconcile_checkouts(&self, found: &Value) -> R<Value> {
        let rows: Vec<_> = found
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| crate::checkouts::found_of(f.get("path")?.as_str()?, crate::checkouts::Why::parse(f.get("why")?.as_str()?)?))
            .collect();
        let fixes = self.db.reconcile(&rows, (self.clock)() as i64).map_err(failed)?;
        let mut out = self.checkouts()?;
        out["fixes"] = Value::Array(fixes.iter().map(|f| Value::String(f.words())).collect());
        Ok(out)
    }
    fn agents_of(&self, sessions: &[String]) -> R<Value> {
        let all = self.db.agents_of(sessions, (self.clock)() as i64).map_err(failed)?;
        Ok(Value::Object(all.into_iter().map(|(s, a)| (s, Value::String(a.as_str().to_string()))).collect()))
    }
    fn checkouts(&self) -> R<Value> {
        let all = self.db.checkouts().map_err(failed)?;
        Ok(serde_json::json!({ "checkouts": all.iter().map(crate::checkouts::json_of).collect::<Vec<_>>() }))
    }
    fn space_tie(&self, marker: &Value) -> R<Value> {
        Ok(self.db.space_tie(marker, (self.clock)() as i64).map_err(failed)?.to_json())
    }
    fn confirm_checkout(&self, id: &str) -> R<Value> {
        let Ok(id) = xenonet_model::ids::CheckoutId::parse(id) else {
            return Err(CoreError::status(400, format!("{id} isn't a checkout's id")));
        };
        match self.db.confirm(&id) {
            Ok(ch) => Ok(crate::checkouts::json_of(&ch)),
            Err(e) if e.code == "not-found" => Err(CoreError::status(404, e.message)),
            Err(e) => Err(failed(e)),
        }
    }
    fn project_rules(&self) -> R<Value> {
        self.db.project_rules().map_err(failed)
    }
    fn set_project_rule(&self, key: &str, rule: Option<&Value>) -> R<()> {
        self.db.set_project_rule(key, rule).map_err(failed)
    }
    fn clear_project_rules(&self, under_key: Option<&str>) -> R<()> {
        let rules = self.db.project_rules().map_err(failed)?;
        let keys: Vec<String> = rules.as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
        self.db.drop_project_rules(&rules_to_clear(&keys, under_key)).map_err(failed)
    }
    fn app_setting(&self, app: &str, name: &str, scope: &str) -> R<Option<Value>> {
        self.db.app_setting(app, name, scope).map_err(failed)
    }
    fn set_app_setting(&self, app: &str, name: &str, scope: &str, value: Option<&Value>) -> R<()> {
        self.db.set_app_setting(app, name, scope, value).map_err(failed)
    }
    fn identities(&self) -> R<Value> {
        self.db.identities().map_err(failed)
    }
    fn set_identity_name(&self, id: &str, name: Option<&str>) -> R<()> {
        self.db.set_identity_name(id, name).map_err(failed)
    }
    fn set_identity_hidden(&self, id: &str, hidden: bool) -> R<()> {
        self.db.set_identity_hidden(id, hidden).map_err(failed)
    }
    fn set_identity_pin(&self, node: &str, identity: Option<&str>) -> R<()> {
        self.db.set_identity_pin(node, identity.map(|i| (i, (self.clock)()))).map_err(failed)
    }
    fn set_identity_known(&self, id: &str, value: &Value) -> R<()> {
        self.db.set_identity_known(id, value).map_err(failed)
    }
    fn set_signins(&self, app: &str, spans: &Value) -> R<()> {
        self.db.set_signins(app, spans).map_err(failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn changes_as_store_js_makes_them() {
        let s = Saved::with_clock(Arc::new(Db::memory().unwrap()), Arc::new(|| 1000.0));
        s.set_layout(&json!({ "a": { "x": 1.005, "y": -0.0, "z": 2 }, "b": { "x": 1, "y": 1, "z": 1 }, "c": { "x": "1", "y": 0, "z": 0 } })).unwrap();
        s.set_layout(&json!({ "b": null })).unwrap();
        assert_eq!(s.layout().unwrap(), json!({ "a": { "x": 1, "y": 0, "z": 2 } }));
        s.clear_layout().unwrap();
        assert_eq!(s.layout().unwrap(), json!({}));
        s.set_project_rule("/p", Some(&json!({ "path": "/p", "as": "group" }))).unwrap();
        s.set_project_rule("/p/labs", Some(&json!({ "path": "/p/labs", "as": "project" }))).unwrap();
        s.set_project_rule("/q/x", Some(&json!({ "path": "/q/x", "as": "group" }))).unwrap();
        s.clear_project_rules(Some("/p")).unwrap();
        assert_eq!(s.project_rules().unwrap().as_object().unwrap().keys().collect::<Vec<_>>(), ["/p", "/q/x"], "the folder's own rule stays");
        s.set_identity_pin("claude:a", Some("github:x/y")).unwrap();
        assert_eq!(s.identities().unwrap()["pins"]["claude:a"], json!({ "identity": "github:x/y", "at": 1000 }));
        s.set_identity_pin("claude:a", None).unwrap();
        assert_eq!(s.identities().unwrap()["pins"], json!({}));
        assert_eq!(s.theme().unwrap(), Value::Null);
        s.set_theme("rime").unwrap();
        assert_eq!(s.theme().unwrap(), "rime");
        s.ack("claude:a").unwrap();
        assert_eq!(s.acks().unwrap(), json!({ "claude:a": 1000 }));
        s.set_workspace_root(Some("/p")).unwrap();
        assert_eq!(s.workspace_root().unwrap(), "/p");
        s.set_workspace_root(None).unwrap();
        assert_eq!(s.workspace_root().unwrap(), Value::Null);
    }
}
