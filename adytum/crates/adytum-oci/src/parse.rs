//! Reads `ps -a --format json` from podman (one JSON array) and docker (one
//! JSON object per line).

use adytum_driver::Listed;
use adytum_model::{CellName, CellState};
use serde_json::Value;

use crate::CELL_LABEL;

fn values(text: &str) -> Result<Vec<Value>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    if text.starts_with('[') {
        return match serde_json::from_str::<Value>(text).map_err(|e| e.to_string())? {
            Value::Array(items) => Ok(items),
            _ => Err("expected a list".to_owned()),
        };
    }
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str::<Value>(l).map_err(|e| e.to_string())).collect()
}

/// Podman gives `Labels` as an object, docker as `k=v,k=v`.
fn has_cell_label(item: &Value) -> bool {
    match item.get("Labels") {
        Some(Value::Object(map)) => map.contains_key(CELL_LABEL),
        Some(Value::String(text)) => text.split(',').any(|kv| kv.split('=').next() == Some(CELL_LABEL)),
        _ => false,
    }
}

/// Podman gives `Names` as a list, docker as a comma-separated string.
fn names(item: &Value) -> Vec<String> {
    match item.get("Names") {
        Some(Value::Array(list)) => list.iter().filter_map(|n| n.as_str().map(str::to_owned)).collect(),
        Some(Value::String(text)) => text.split(',').map(str::to_owned).collect(),
        _ => Vec::new(),
    }
}

fn state_of(state: &str) -> CellState {
    match state.to_lowercase().as_str() {
        "running" => CellState::Running,
        "created" | "exited" | "stopped" | "configured" => CellState::Stopped,
        "removing" => CellState::Removing,
        _ => CellState::Broken,
    }
}

/// The Adytum cells in a `ps` answer. A container without the cell label, or
/// whose name is not Adytum's, is dropped even if the tool listed it.
pub(crate) fn ps(text: &str) -> Result<Vec<Listed>, String> {
    let mut cells = Vec::new();
    for item in values(text)? {
        if !has_cell_label(&item) {
            continue;
        }
        let state = state_of(item.get("State").and_then(Value::as_str).unwrap_or(""));
        if let Some(name) = names(&item).iter().find_map(|n| n.parse::<CellName>().ok()) {
            cells.push(Listed { name, state });
        }
    }
    Ok(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn podman_array_and_docker_lines_both_read() {
        let podman = r#"[{"Names":["adytum-a"],"State":"running","Labels":{"adytum.cell":"cell_x"}},{"Names":["adytum-b"],"State":"exited","Labels":{"adytum.cell":"cell_y"}}]"#;
        let docker = "{\"Names\":\"adytum-a\",\"State\":\"running\",\"Labels\":\"adytum.cell=cell_x,adytum.wall=wall_t\"}\n{\"Names\":\"adytum-b\",\"State\":\"created\",\"Labels\":\"adytum.cell=cell_y\"}\n";
        for text in [podman, docker] {
            let got = ps(text).unwrap();
            assert_eq!(got.len(), 2);
            assert_eq!((got[0].name.as_str(), got[0].state), ("adytum-a", CellState::Running));
            assert_eq!((got[1].name.as_str(), got[1].state), ("adytum-b", CellState::Stopped));
        }
        assert_eq!(ps("[]").unwrap(), Vec::new());
        assert_eq!(ps("").unwrap(), Vec::new());
        assert!(ps("nonsense").is_err());
    }

    #[test]
    fn a_container_without_our_label_or_name_is_never_listed() {
        let text = r#"[{"Names":["adytum-a"],"State":"running","Labels":{"other":"x"}},{"Names":["adytum-b"],"State":"running","Labels":null},{"Names":["web"],"State":"running","Labels":{"adytum.cell":"cell_x"}}]"#;
        assert_eq!(ps(text).unwrap(), Vec::new());
        let docker = "{\"Names\":\"adytum-a\",\"State\":\"running\",\"Labels\":\"adytum.cellular=1\"}\n";
        assert_eq!(ps(docker).unwrap(), Vec::new());
    }
}
