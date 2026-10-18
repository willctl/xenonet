//! Text as JS reads it: what `trim()` takes off, and what a value is as a
//! string or a number, for the readers that treat a record's fields as JS
//! did.

use crate::json::number_string;
use serde_json::Value;

/// Whether JS's trim() takes a character off: its white space and line
/// terminators. (Rust's own white space differs: it has U+0085 and not
/// U+FEFF.)
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

pub fn js_trim_start(s: &str) -> &str {
    s.trim_start_matches(is_js_space)
}

/// Whether a line is blank as `!line.trim()` says.
pub fn is_blank(s: &str) -> bool {
    js_trim(s).is_empty()
}

/// A value as String(value) writes it; for null, "null". (Array.join
/// writes null and a missing value as "", which `join_item` does.)
pub fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_i64().map_or_else(|| number_string(n.as_f64().unwrap_or(f64::NAN)), |i| i.to_string()),
        Value::String(s) => s.clone(),
        Value::Array(a) => join(a.iter().map(Some), ","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// An item as Array.prototype.join writes it: null and missing as "".
pub fn join_item(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

pub fn join<'a>(items: impl Iterator<Item = Option<&'a Value>>, sep: &str) -> String {
    items.map(join_item).collect::<Vec<_>>().join(sep)
}

/// A value as Number(value) reads it.
pub fn js_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => string_number(s),
        Some(Value::Array(a)) => string_number(&js_string(&Value::Array(a.clone()))),
        Some(Value::Object(_)) => f64::NAN,
    }
}

// StringToNumber: white space around it, "" as 0, hex, octal and binary
// literals, Infinity, and otherwise a decimal or NaN.
fn string_number(s: &str) -> f64 {
    let t = js_trim(s);
    if t.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(d) = t.strip_prefix(prefix) {
            if d.is_empty() {
                return f64::NAN;
            }
            return d.chars().try_fold(0.0, |n, c| Some(n * f64::from(radix) + f64::from(c.to_digit(radix)?))).unwrap_or(f64::NAN);
        }
    }
    let (sign, body) = match t.as_bytes()[0] {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "Infinity" {
        return sign * f64::INFINITY;
    }
    // A decimal literal: digits, a point, digits, an exponent; no "inf",
    // "nan" or "1_000", which Rust's parse would take.
    let b = body.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let from = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - from
    };
    let int = digits(&mut i);
    let mut frac = 0;
    if i < b.len() && b[i] == b'.' {
        i += 1;
        frac = digits(&mut i);
    }
    if int + frac == 0 {
        return f64::NAN;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return f64::NAN;
        }
    }
    if i != b.len() {
        return f64::NAN;
    }
    sign * body.parse::<f64>().unwrap_or(f64::NAN)
}

/// Whether a value is truthy, as `!!value` says.
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Some(Value::String(s)) => !s.is_empty(),
        Some(_) => true,
    }
}

/// `typeof value`, for a JSON value or a missing one.
pub fn type_of(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null | Value::Array(_) | Value::Object(_)) => "object",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn as_js_reads_it() {
        assert_eq!(js_trim("\u{feff} a \u{2028}"), "a");
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
        assert_eq!(
            join([Some(&json!("a")), None, Some(&json!(null)), Some(&json!(1.5)), Some(&json!([1, [2, null]])), Some(&json!({}))].into_iter(), " "),
            "a   1.5 1,2, [object Object]"
        );
        for (v, n) in [
            (json!(" 12 "), 12.0),
            (json!(""), 0.0),
            (json!("0x1f"), 31.0),
            (json!("-Infinity"), f64::NEG_INFINITY),
            (json!(true), 1.0),
            (json!(null), 0.0),
            (json!([7]), 7.0),
            (json!([]), 0.0),
        ] {
            assert_eq!(js_number(Some(&v)), n, "{v}");
        }
        for v in [json!("1_0"), json!("inf"), json!("."), json!("1e"), json!({}), json!([1, 2])] {
            assert!(js_number(Some(&v)).is_nan(), "{v}");
        }
        assert!(!truthy(Some(&json!(0))) && truthy(Some(&json!("0"))) && truthy(Some(&json!([]))) && !truthy(None));
    }
}
