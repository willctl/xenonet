//! Values as the cores and their collaborators hand them to each other:
//! `Arg`, which is what a twin's JS would have given, and JSON, which is
//! what the service's traits take. A JS Map comes across as its [key,
//! value] pairs and a Set as its values, as the twins' calls convert them.

use serde_json::{Map, Value};
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::{Arg, Cell, Row};

/// A JSON value as JS would hand it over.
pub fn arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::Text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), arg(v))).collect()),
    }
}

/// `x ?? null` as JSON, an undefined read as null.
pub fn json(a: &Arg) -> Value {
    a.to_json()
}

/// A value of a JS call, None for undefined.
pub fn opt_json(a: &Arg) -> Option<Value> {
    match a {
        Arg::Undef => None,
        a => Some(a.to_json()),
    }
}

/// A cell as node:sqlite reads it: every integer a number.
pub fn cell(c: &Cell) -> Arg {
    match c {
        Cell::Int(i) => Arg::Num(*i as f64),
        other => other.arg(),
    }
}

pub fn row(r: &Row) -> Arg {
    Arg::Obj(r.0.iter().map(|(k, v)| (k.clone(), cell(v))).collect())
}

pub fn rows(rs: &[Row]) -> Arg {
    Arg::List(rs.iter().map(row).collect())
}

/// An object literal.
pub fn obj(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// A JSON object literal.
pub fn jobj(fields: Vec<(&str, Value)>) -> Value {
    Value::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect::<Map<String, Value>>())
}

/// The i-th argument of a call, undefined past the end.
pub fn at(args: &[Arg], i: usize) -> Arg {
    args.get(i).cloned().unwrap_or(Arg::Undef)
}

/// Number(x), as JS reads it.
pub fn num(a: &Arg) -> f64 {
    match a {
        Arg::Num(n) => *n,
        Arg::Big(n) => *n as f64,
        Arg::Null => 0.0,
        Arg::Bool(b) => f64::from(u8::from(*b)),
        Arg::Undef | Arg::Other => f64::NAN,
        other => xenonet_base::text::js_number(Some(&other.to_json())),
    }
}

/// String(x), as JS writes it.
pub fn text(a: &Arg) -> String {
    xenonet_engine::store::js_text(a)
}

/// `x ?? ""` as text.
pub fn text_or_empty(a: &Arg) -> String {
    if a.is_nullish() {
        String::new()
    } else {
        text(a)
    }
}

/// A list's items, or none.
pub fn list(a: Arg) -> Vec<Arg> {
    match a {
        Arg::List(l) => l,
        _ => Vec::new(),
    }
}

/// A JSON list's items, or none.
pub fn items(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

/// A Thrown made of a message.
pub fn thrown(message: impl Into<String>) -> Thrown {
    Thrown::new(message)
}

/// `x is not a function`, for a method nothing answers.
pub fn no_method(path: &str, method: &str) -> Thrown {
    let at = if path.is_empty() { method.to_string() } else { format!("{path}.{method}") };
    Thrown::coded(format!("{at} is not a function"), "TypeError")
}

/// process.platform, as Node names this system.
pub fn platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// Date.now().
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64() * 1000.0).floor()
}

/// The JSON of a value that might be a number written as an integer.
pub fn number(n: f64) -> Value {
    xenonet_base::json::number(n)
}

/// typeof of an Arg, as `kind` answers it.
pub fn type_of(a: &Arg) -> &'static str {
    match a {
        Arg::Undef => "undefined",
        Arg::Null => "null",
        Arg::Num(_) => "number",
        Arg::Big(_) => "bigint",
        Arg::Text(_) => "string",
        Arg::Bool(_) => "boolean",
        Arg::Other => "function",
        _ => "object",
    }
}

/// Whether a process still runs, a process of another user's included.
pub fn alive(pid: f64) -> bool {
    pid.is_finite() && pid.fract() == 0.0 && pid > 0.0 && pid <= f64::from(i32::MAX) && xenonet_base::process::running(pid as u32, true)
}

/// JS's truthiness of a JSON value.
pub fn truthy_json(v: &Value) -> bool {
    xenonet_base::text::truthy(Some(v))
}

/// `new Date(ms).toLocaleTimeString([], { hour: "numeric", minute:
/// "2-digit" })` in an English locale: 9:04 AM, in local time.
pub fn time_of(ms: f64) -> String {
    if !ms.is_finite() {
        return "Invalid Date".into();
    }
    let local = ms + xenonet_base::jsstr::local_offset_ms(ms);
    let minutes = (local / 60e3).floor() as i64;
    let (h, m) = (minutes.div_euclid(60).rem_euclid(24), minutes.rem_euclid(60));
    let (h12, half) = (if h % 12 == 0 { 12 } else { h % 12 }, if h < 12 { "AM" } else { "PM" });
    format!("{h12}:{m:02} {half}")
}

/// The local day's start (new Date(y, m, d).getTime()) of a time.
pub fn day_start(ms: f64) -> f64 {
    let off = xenonet_base::jsstr::local_offset_ms(ms);
    let start = ((ms + off) / 86400e3).floor() * 86400e3;
    // Midnight's own offset, which differs on a day the clocks change.
    start - xenonet_base::jsstr::local_offset_ms(start - off)
}

/// A mutex taken, whatever a thread that panicked while holding it left:
/// what's kept here is plain data, never half-written by a panic of ours.
pub trait Locked<T> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> Locked<T> for std::sync::Mutex<T> {
    fn locked(&self) -> std::sync::MutexGuard<'_, T> {
        self.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn values_cross_as_js_hands_them_over() {
        let v = json!({ "a": [1, "x", null, true], "b": { "c": 2.5 } });
        assert_eq!(json(&arg(&v)), v);
        assert_eq!(num(&Arg::text(" 42 ")), 42.0);
        assert!(num(&Arg::Undef).is_nan());
        assert_eq!(text(&Arg::Num(3.0)), "3");
        assert_eq!(cell(&Cell::Int(7)), Arg::Num(7.0));
        assert_eq!(type_of(&Arg::List(vec![])), "object");
    }

    #[test]
    fn this_process_is_alive_and_nonsense_is_not() {
        assert!(alive(f64::from(std::process::id())));
        assert!(!alive(-1.0));
        assert!(!alive(f64::NAN));
    }
}
