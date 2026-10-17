//! core/export-labels.js: the map's own labels during an export of the day:
//! star names in place of the names of projects, groups, folders, pages,
//! shared systems and machines, and no label on a session.

use serde_json::Value;
use xenonet_base::text::{js_string, truthy};

pub const LABELLED_KINDS: [&str; 8] = ["core", "project", "zone", "native", "wiki", "page", "resource", "host"];

pub fn is_labelled(kind: Option<&Value>) -> bool {
    kind.and_then(Value::as_str).is_some_and(|k| LABELLED_KINDS.contains(&k))
}

/// What a node's label shows under `labels` ({ id: star name }): the star
/// name in capitals, with a group's slash; None keeps the label hidden.
pub fn export_label(node: Option<&Value>, labels: Option<&Value>) -> Option<String> {
    let (Some(Value::Object(n)), Some(labels)) = (node, labels) else { return None };
    if !truthy(Some(labels)) || !is_labelled(n.get("kind")) {
        return None;
    }
    // labels[node.id]: the id as a property key.
    let key = n.get("id").map_or_else(|| "undefined".to_string(), js_string);
    let name = labels.as_object()?.get(&key)?.as_str().filter(|s| !s.is_empty())?;
    let upper = name.to_uppercase();
    Some(if n.get("kind").and_then(Value::as_str) == Some("project") && truthy(n.get("group")) { format!("{upper} /") } else { upper })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn star_names_in_capitals() {
        let labels = json!({ "project:a": "Vela", "zone:b": "Lyra" });
        assert_eq!(export_label(Some(&json!({ "id": "project:a", "kind": "project", "group": true })), Some(&labels)).as_deref(), Some("VELA /"));
        assert_eq!(export_label(Some(&json!({ "id": "zone:b", "kind": "zone" })), Some(&labels)).as_deref(), Some("LYRA"));
        assert_eq!(export_label(Some(&json!({ "id": "zone:b", "kind": "session" })), Some(&labels)), None);
    }
}
