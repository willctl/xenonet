//! state.json's versions (core/statefile.js): version 2 is this XenoNet's,
//! and the step from version 1.

use serde_json::{Map, Value};
use xenonet_base::text::truthy;

pub const STATE_VERSION: f64 = 2.0;

fn object(v: Option<&Value>) -> Option<&Map<String, Value>> {
    v.and_then(Value::as_object)
}

/// v1 → v2. v2 adds the version, the workspace's root (null:
/// XENONET_PROJECTS_DIR, else ~/Projects) and Codex's picks for each root.
/// v1's picks were made for the projects folder of the day, so they go
/// under its key. A root or picks per root already there (written by code
/// from before the versions met) are kept. Keys keep the order the JS step
/// gives them.
pub fn up_1_to_2(s: &Value, root_key: Option<&str>) -> Value {
    let empty = Map::new();
    let s = s.as_object().unwrap_or(&empty);
    let projects = object(s.get("projects")).unwrap_or(&empty);
    let codex = projects.get("codex").filter(|c| !c.is_null());

    let mut workspace = Map::new();
    workspace.insert("root".into(), Value::Null);
    for (k, v) in object(s.get("workspace")).unwrap_or(&empty) {
        workspace.insert(k.clone(), v.clone());
    }

    let mut by_root = Map::new();
    if let (true, Some(key), Some(c)) = (truthy(codex), root_key, codex) {
        by_root.insert(key.to_string(), c.clone());
    }
    for (k, v) in object(projects.get("codexByRoot")).unwrap_or(&empty) {
        by_root.insert(k.clone(), v.clone());
    }
    let mut proj = Map::new();
    for (k, v) in projects {
        if k != "codex" && k != "codexByRoot" {
            proj.insert(k.clone(), v.clone());
        }
    }
    proj.insert("codexByRoot".into(), Value::Object(by_root));

    let mut out = Map::new();
    out.insert("version".into(), Value::from(2));
    out.insert("workspace".into(), Value::Object(workspace));
    for (k, v) in s {
        if !matches!(k.as_str(), "version" | "workspace" | "projects") {
            out.insert(k.clone(), v.clone());
        }
    }
    out.insert("projects".into(), Value::Object(proj));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn v1_to_v2() {
        let v2 = up_1_to_2(&json!({ "layout": {}, "projects": { "codex": { "picks": [] }, "rules": {} } }), Some("/users/nova/projects"));
        assert_eq!(
            xenonet_base::json::stringify(&v2, 0),
            r#"{"version":2,"workspace":{"root":null},"layout":{},"projects":{"rules":{},"codexByRoot":{"/users/nova/projects":{"picks":[]}}}}"#
        );
        assert_eq!(up_1_to_2(&json!({}), None), json!({ "version": 2, "workspace": { "root": null }, "projects": { "codexByRoot": {} } }));
    }
}
