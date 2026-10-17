//! core/store.js: the small state XenoNet keeps in state.json (dragged
//! positions, dismissed alerts, markers, the projects folder's rules and
//! Codex's picks, identities). The file and its versions are the JSON
//! store's (xenonet-blocks' store, statefile); here is what a loaded
//! document must hold, and what each change does to it.

use serde_json::{json, Map, Value};
use xenonet_base::paths::is_under;
use xenonet_base::units::to_fixed;

fn is_object(v: &Value) -> bool {
    v.is_object()
}

/// The parts of the document prepare() fills in, in the order it adds
/// them.
pub const PARTS: [&str; 6] = ["workspace", "layout", "acks", "marks", "projects", "identities"];

/// Fills in every part a loaded document should have. A part that isn't
/// an object (a hand edit that left "identities": "x") starts afresh, and
/// is named in what this returns, so the doctor can say so.
pub fn prepare(state: &mut Map<String, Value>) -> Vec<String> {
    let mut repaired = Vec::new();
    fn part<'a>(parent: &'a mut Map<String, Value>, key: &str, at: &str, fresh: Value, repaired: &mut Vec<String>) -> &'a mut Map<String, Value> {
        match parent.get(key) {
            None | Some(Value::Null) => {
                parent.insert(key.into(), fresh);
            }
            Some(v) if !is_object(v) => {
                parent.insert(key.into(), fresh);
                repaired.push(at.into());
            }
            _ => {}
        }
        parent.get_mut(key).and_then(Value::as_object_mut).unwrap()
    }
    part(state, "workspace", "workspace", json!({ "root": null }), &mut repaired);
    for k in ["layout", "acks", "marks"] {
        part(state, k, k, json!({}), &mut repaired);
    }
    let projects = part(state, "projects", "projects", json!({}), &mut repaired);
    for k in ["rules", "codexByRoot"] {
        part(projects, k, &format!("projects.{k}"), json!({}), &mut repaired);
    }
    let identities = part(state, "identities", "identities", json!({}), &mut repaired);
    for k in ["names", "hidden", "pins", "known", "signins"] {
        part(identities, k, &format!("identities.{k}"), json!({}), &mut repaired);
    }
    repaired
}

/// What setLayout(positions) does to the layout: each id's new place (to
/// two decimals), or None to forget it. A place that isn't three finite
/// numbers is left alone.
pub fn layout_changes(positions: Option<&Value>) -> Vec<(String, Option<Value>)> {
    let Some(Value::Object(m)) = positions else { return Vec::new() };
    let mut out = Vec::new();
    for (id, p) in xenonet_base::jsstr::js_key_order(m.clone()) {
        if p.is_null() {
            out.push((id, None));
            continue;
        }
        let xyz: Vec<Option<f64>> = ["x", "y", "z"].iter().map(|k| p.get(*k).and_then(Value::as_f64).filter(|n| n.is_finite())).collect();
        if xyz.iter().all(Option::is_some) {
            let fixed = |n: f64| xenonet_base::json::number(to_fixed(n, 2).parse::<f64>().unwrap_or(f64::NAN) + 0.0);
            out.push((id, Some(json!({ "x": fixed(xyz[0].unwrap()), "y": fixed(xyz[1].unwrap()), "z": fixed(xyz[2].unwrap()) }))));
        }
    }
    out
}

/// The rules clearProjectRules takes out: every one, or with a projects
/// folder's path key, only those for the folders inside it.
pub fn rules_to_clear(keys: &[String], under_key: Option<&str>) -> Vec<String> {
    keys.iter().filter(|k| under_key.is_none_or(|u| is_under(k, u) && k.as_str() != u)).cloned().collect()
}

/// A projects folder's key for its picks: one is needed.
pub fn folder_key(root_key: Option<&Value>) -> Result<String, String> {
    match root_key {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        _ => Err("say which projects folder the picks are for".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_part_in_place() {
        let mut s = json!({ "version": 2, "identities": "x", "projects": { "rules": [1] } }).as_object().cloned().unwrap();
        let repaired = prepare(&mut s);
        assert_eq!(repaired, ["projects.rules", "identities"]);
        assert_eq!(s.keys().collect::<Vec<_>>(), ["version", "identities", "projects", "workspace", "layout", "acks", "marks"]);
        assert_eq!(s["identities"].as_object().unwrap().len(), 5);
        let ch = layout_changes(Some(&json!({ "a": { "x": 1.005, "y": -0.0, "z": 2 }, "b": null, "c": { "x": "1", "y": 0, "z": 0 } })));
        assert_eq!(ch, vec![("a".into(), Some(json!({ "x": 1, "y": 0, "z": 2 }))), ("b".into(), None)]);
        assert_eq!(rules_to_clear(&["/p".into(), "/p/a".into(), "/q".into()], Some("/p")), ["/p/a"]);
        assert!(folder_key(Some(&json!(""))).is_err());
    }
}
