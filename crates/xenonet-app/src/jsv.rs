//! JSON values read the way the JS reads its objects: `obj?.key`, a value
//! that's an object or else `{}`, a string cut to a length or else null,
//! truthiness, and `??`.

use crate::jstext::slice16;
use serde_json::{Map, Value};
use std::sync::OnceLock;

fn empty() -> &'static Map<String, Value> {
    static E: OnceLock<Map<String, Value>> = OnceLock::new();
    E.get_or_init(Map::new)
}

/// `v && typeof v === "object" && !Array.isArray(v) ? v : {}`.
pub fn object(v: Option<&Value>) -> &Map<String, Value> {
    match v {
        Some(Value::Object(m)) => m,
        _ => empty(),
    }
}

/// `v?.[k]`, with null read as missing, as `?? fallback` reads it.
pub fn get<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k)).filter(|x| !x.is_null())
}

/// `v?.[k]`, null kept.
pub fn raw<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k))
}

/// `typeof v === "string" ? v.slice(0, max) : null`.
pub fn text(v: Option<&Value>, max: usize) -> Value {
    match v {
        Some(Value::String(s)) => Value::String(slice16(s, max)),
        _ => Value::Null,
    }
}

pub fn truthy(v: Option<&Value>) -> bool {
    xenonet_base::text::truthy(v)
}

/// `a ?? b`.
pub fn or(a: Option<&Value>, b: Value) -> Value {
    a.filter(|v| !v.is_null()).cloned().unwrap_or(b)
}

/// `Object.keys(v).length`, for the values JSON has (null as none).
pub fn key_count(v: &Value) -> usize {
    match v {
        Value::Object(m) => m.len(),
        Value::Array(a) => a.len(),
        Value::String(s) => crate::jstext::utf16_len(s),
        _ => 0,
    }
}

/// `a === b` for JSON values: the same primitive (objects and arrays are
/// never === a copy).
pub fn strict_eq(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// `a < b` as JS compares two JSON values: strings by their units, else as
/// numbers.
pub fn less(a: Option<&Value>, b: Option<&Value>) -> bool {
    match (a, b) {
        (Some(Value::String(x)), Some(Value::String(y))) => x.encode_utf16().lt(y.encode_utf16()),
        _ => {
            let n = |v: Option<&Value>| match v {
                Some(Value::Array(_)) | Some(Value::Object(_)) => xenonet_base::text::js_number(Some(&Value::String(v.map(xenonet_base::text::js_string).unwrap_or_default()))),
                other => xenonet_base::text::js_number(other),
            };
            n(a) < n(b)
        }
    }
}

/// `typeof v` for a JSON value.
pub fn type_name(v: &Value) -> &'static str {
    xenonet_base::text::type_of(Some(v))
}
