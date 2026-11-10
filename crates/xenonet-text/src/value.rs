//! JSON values read the way XenoNet's answers and files read them: a
//! field that's an object or else none, a field with null read as missing,
//! a string cut to a length or else null, a value's count of keys, items
//! or units, two primitives being the same, and one less than another.

use serde_json::{Map, Value};
use std::sync::OnceLock;

fn empty() -> &'static Map<String, Value> {
    static E: OnceLock<Map<String, Value>> = OnceLock::new();
    E.get_or_init(Map::new)
}

/// The value when it's an object (not an array), else an empty one.
pub fn object(v: Option<&Value>) -> &Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m,
        _ => empty(),
    }
}

/// A field, with null read as missing.
pub fn get<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k)).filter(|x| !x.is_null())
}

/// A field, null kept.
pub fn raw<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k))
}

/// A string cut to `max` UTF-16 units, or null for anything else.
pub fn cut(v: Option<&Value>, max: usize) -> Value {
    match v {
        Some(Value::String(s)) => Value::String(crate::utf16::head(s, max)),
        _ => Value::Null,
    }
}

/// `a`, unless it's missing or null: then `b`.
pub fn or(a: Option<&Value>, b: Value) -> Value {
    a.filter(|v| !v.is_null()).cloned().unwrap_or(b)
}

/// How many keys an object has, items an array, or UTF-16 units a string
/// (none for anything else).
pub fn key_count(v: &Value) -> usize {
    match v {
        Value::Object(m) => m.len(),
        Value::Array(a) => a.len(),
        Value::String(s) => crate::utf16::len(s),
        _ => 0,
    }
}

/// Whether two values are the same primitive: two objects or arrays never
/// are, even when they're equal.
pub fn same(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// Whether `a` is less than `b`: two strings by their UTF-16 units, else
/// as numbers (an object or array by its text).
pub fn less(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::String(x)), Some(Value::String(y))) => x.encode_utf16().lt(y.encode_utf16()),
        _ => {
            let n = |v: Option<&Value>| match v {
                Some(Value::Array(_) | Value::Object(_)) => crate::text::to_number(Some(&Value::String(v.map(crate::text::display).unwrap_or_default()))),
                other => crate::text::to_number(other),
            };
            n(a) < n(b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn read_as_the_answers_read_them() {
        assert!(object(Some(&json!([1]))).is_empty());
        assert_eq!(get(Some(&json!({ "a": null })), "a"), None);
        assert_eq!(raw(Some(&json!({ "a": null })), "a"), Some(&Value::Null));
        assert_eq!(cut(Some(&json!("a😀b")), 2), json!("a\u{fffd}"));
        assert_eq!(cut(Some(&json!(7)), 2), Value::Null);
        assert_eq!(key_count(&json!("a😀")), 3);
        assert!(same(Some(&json!(1)), Some(&json!(1.0))));
        assert!(!same(Some(&json!({})), Some(&json!({}))));
        assert!(less(Some(&json!("B")), Some(&json!("a"))));
        assert!(less(Some(&json!(2)), Some(&json!("10"))));
    }
}
