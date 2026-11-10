//! A JSON value read as one of XenoNet's document types
//! (crates/xenonet-model): the type is the check. What fails is the first
//! problem serde meets, with the path to it as a message names it
//! (`events.0.kind`) and serde's own words ("missing field `kind`").

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt::Write as _;

/// The first problem a value has against its type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    /// Keys and indexes with dots, empty at the root.
    pub path: String,
    pub message: String,
}

impl Issue {
    /// `<path> <message>`, with `root` standing for an empty path ("the
    /// body", "it").
    pub fn worded(&self, root: &str) -> String {
        format!("{} {}", if self.path.is_empty() { root } else { &self.path }, self.message)
    }
}

/// The value as `T`, or where it first fails to be one.
pub fn read<T: DeserializeOwned>(value: &Value) -> Result<T, Issue> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let mut path = String::new();
        for segment in e.path() {
            use serde_path_to_error::Segment;
            if !path.is_empty() {
                path.push('.');
            }
            match segment {
                Segment::Seq { index } => {
                    let _ = write!(path, "{index}");
                }
                Segment::Map { key } => path.push_str(key),
                Segment::Enum { variant } => path.push_str(variant),
                Segment::Unknown => path.push('?'),
            }
        }
        Issue { path, message: e.into_inner().to_string() }
    })
}

/// Whether the value is a `T`.
pub fn fits<T: DeserializeOwned>(value: &Value) -> bool {
    read::<T>(value).is_ok()
}

/// The value read as `T` and written back: what the type declares, and
/// nothing else, or None when it isn't a `T`.
pub fn clean<T: DeserializeOwned + serde::Serialize>(value: &Value) -> Option<Value> {
    to_json(&read::<T>(value).ok()?)
}

/// A typed value back as JSON with its numbers as JSON written them: a
/// whole number is an integer, never `5.0`.
pub fn to_json<T: serde::Serialize>(value: &T) -> Option<Value> {
    let mut v = serde_json::to_value(value).ok()?;
    whole_numbers(&mut v);
    Some(v)
}

fn whole_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|_| !n.is_i64() && !n.is_u64()) {
                *v = xenonet_text::json::number(f);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(whole_numbers),
        Value::Object(o) => o.values_mut().for_each(whole_numbers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    #[derive(Debug, Deserialize, serde::Serialize)]
    struct Item {
        kind: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
    }

    #[derive(Debug, Deserialize)]
    struct Doc {
        items: Vec<Item>,
    }

    #[test]
    fn the_first_problem_is_named_by_its_path() {
        let e = read::<Doc>(&json!({ "items": [{ "kind": "a" }, { "kind": 3 }] })).unwrap_err();
        assert_eq!(e.path, "items.1.kind");
        assert!(e.message.starts_with("invalid type: integer `3`"), "{}", e.message);
        let e = read::<Doc>(&json!({ "items": [{}] })).unwrap_err();
        assert_eq!((e.path.as_str(), e.message.as_str()), ("items.0", "missing field `kind`"));
        let e = read::<Doc>(&json!(7)).unwrap_err();
        assert_eq!(e.path, "");
        assert_eq!(e.worded("it"), "it invalid type: integer `7`, expected struct Doc");
        assert!(fits::<Doc>(&json!({ "items": [] })));
        assert_eq!(read::<Doc>(&json!({ "items": [{ "kind": "a" }] })).unwrap().items.len(), 1);
    }

    #[test]
    fn a_typed_value_writes_whole_numbers_as_integers() {
        let item: Item = read(&json!({ "kind": "a", "at": 5 })).unwrap();
        assert_eq!(to_json(&item), Some(json!({ "kind": "a", "at": 5 })));
        let item: Item = read(&json!({ "kind": "a", "at": 5.5 })).unwrap();
        assert_eq!(to_json(&item), Some(json!({ "kind": "a", "at": 5.5 })));
        assert_eq!(clean::<Item>(&json!({ "kind": "a", "extra": true })), Some(json!({ "kind": "a" })));
        assert_eq!(clean::<Item>(&json!({ "at": 1 })), None);
    }
}
