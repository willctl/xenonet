//! A JSON answer's fields read as the JS command reads them: in a template
//! string (`${a.title}`, "undefined" for one that isn't there), as truthy
//! or not, as a number, or as a list (none when it isn't one).

use serde_json::Value;
use std::fmt::Write as _;
use xenonet_base::text::{js_number, js_string, truthy};

/// `${v}`, for a value that may not be there.
pub fn text(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), js_string)
}

/// `${a[k]}`.
pub fn t(a: &Value, k: &str) -> String {
    text(a.get(k))
}

/// `String(a[k] ?? "")`.
pub fn or_empty(a: &Value, k: &str) -> String {
    a.get(k).filter(|v| !v.is_null()).map(js_string).unwrap_or_default()
}

/// `!!a[k]`.
pub fn on(a: &Value, k: &str) -> bool {
    truthy(a.get(k))
}

/// `Number(a[k])`: NaN when it isn't there.
pub fn num(a: &Value, k: &str) -> f64 {
    js_number(a.get(k))
}

/// `a[k]` when it's a string.
pub fn str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

/// `a[k]` as a number, None when it's null or not there: what money() and
/// tokens() take.
pub fn amount(a: &Value, k: &str) -> Option<f64> {
    have(a, k).map(|v| js_number(Some(v)))
}

/// `a[k]` as a list; empty when it's anything else.
pub fn list<'a>(a: &'a Value, k: &str) -> &'a [Value] {
    a.get(k).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `a[k]` when it's truthy (`a[k] && …`).
pub fn some<'a>(a: &'a Value, k: &str) -> Option<&'a Value> {
    a.get(k).filter(|v| truthy(Some(v)))
}

/// `a[k] ?? null`: the value, when it's there and not null.
pub fn have<'a>(a: &'a Value, k: &str) -> Option<&'a Value> {
    a.get(k).filter(|v| !v.is_null())
}

/// A value as `.length` counts it: a list's or a string's (in UTF-16
/// units), 0 otherwise.
pub fn length(v: Option<&Value>) -> usize {
    match v {
        Some(Value::Array(a)) => a.len(),
        Some(Value::String(s)) => s.encode_utf16().count(),
        _ => 0,
    }
}

/// `list.slice(0, n)`, n as JS takes it: cut toward zero, and counted
/// from the end when it's below zero.
pub fn slice_to<T>(list: &[T], n: f64) -> &[T] {
    let len = list.len() as f64;
    let n = if n.is_nan() { 0.0 } else { n.trunc() };
    let end = if n < 0.0 { (len + n).max(0.0) } else { n.min(len) };
    &list[..end as usize]
}

/// `encodeURIComponent(s)`.
pub fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fields_as_js_reads_them() {
        let a = json!({ "title": "x", "n": 2, "none": null, "list": [1] });
        assert_eq!((t(&a, "title"), t(&a, "n"), t(&a, "none"), t(&a, "gone")), ("x".into(), "2".into(), "null".into(), "undefined".into()));
        assert!(on(&a, "list") && !on(&a, "none"));
        assert!(num(&a, "gone").is_nan());
        assert_eq!(encode("/w/a b/é?&"), "%2Fw%2Fa%20b%2F%C3%A9%3F%26");
    }
}
