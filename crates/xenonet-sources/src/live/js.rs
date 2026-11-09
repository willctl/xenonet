//! JS's ways, as the telemetry and the apps' line handlers read records
//! and build events: a Map's keys as SameValueZero tells them apart,
//! objects built in order, `??`, `===`, template literals, and a stable
//! sort by a comparator that gives a number.

use serde_json::{Map, Value};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use xenonet_base::json::{number, number_string};
use xenonet_base::jsstr;
use xenonet_base::text::{js_number, js_string};

use crate::lines::opt_prop;
/// An event's fields as JS has them, in order: a field that's there with
/// an undefined value is None.
pub use xenonet_adapter::reader::Fields;

static OBJECT_KEYS: AtomicU64 = AtomicU64::new(0);

/// A JS Map's key, as SameValueZero tells keys apart: by value for what
/// isn't an object, and an object only as itself, so one read from a line
/// matches no other.
pub fn key_tag(k: Option<&Value>) -> String {
    match k {
        None => "u".into(),
        Some(Value::Null) => "z".into(),
        Some(Value::Bool(true)) => "t".into(),
        Some(Value::Bool(false)) => "f".into(),
        Some(Value::Number(n)) => {
            let f = n.as_f64().unwrap_or(f64::NAN);
            format!("n{}", number_string(if f == 0.0 { 0.0 } else { f }))
        }
        Some(Value::String(s)) => format!("s{s}"),
        Some(_) => format!("o{}", OBJECT_KEYS.fetch_add(1, AtomicOrdering::Relaxed)),
    }
}

pub fn str_tag(s: &str) -> String {
    format!("s{s}")
}

/// `Math.max(a, b)`: NaN when either is.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math.round`.
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// `v ?? fallback`: the value unless it's null or undefined.
pub fn nn(v: Option<&Value>) -> Option<&Value> {
    v.filter(|x| !x.is_null())
}

/// A value in a template literal: `${v}`.
pub fn tpl(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => js_string(v),
    }
}

pub fn is_str(v: Option<&Value>, s: &str) -> bool {
    v.and_then(Value::as_str) == Some(s)
}

/// `a === b`, for what a record holds: an object or a list is only itself,
/// which two values read apart never are.
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

/// `x?.[0]`: a list's first item, a string's first unit, an object's "0".
pub fn index0(v: Option<&Value>) -> Option<Value> {
    match v {
        Some(Value::Array(a)) => a.first().cloned(),
        Some(Value::String(s)) => s.encode_utf16().next().map(|u| Value::String(String::from_utf16_lossy(&[u]))),
        Some(Value::Object(m)) => m.get("0").cloned(),
        _ => None,
    }
}

/// `Object.keys(v ?? {})`.
pub fn object_keys(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Object(m)) => jsstr::js_key_order(m.clone()).keys().cloned().collect(),
        Some(Value::Array(a)) => (0..a.len()).map(|i| i.to_string()).collect(),
        Some(Value::String(s)) => (0..jsstr::len(s)).map(|i| i.to_string()).collect(),
        _ => Vec::new(),
    }
}

pub fn obj(fields: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in fields {
        m.insert(k.into(), v);
    }
    Value::Object(m)
}

/// A JS object built in order, with `Some` for each field that's there.
pub fn obj_opt(fields: Vec<(&str, Option<Value>)>) -> Value {
    Value::Object(map_opt(fields))
}

/// obj_opt's fields, as a map.
pub fn map_opt(fields: Vec<(&str, Option<Value>)>) -> Map<String, Value> {
    let mut m = Map::new();
    for (k, v) in fields {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    }
    m
}

pub fn field<'a>(f: &'a Fields, key: &str) -> Option<&'a Value> {
    f.iter().find(|(k, _)| k == key).and_then(|(_, v)| v.as_ref())
}

/// A field set as an object literal sets it: in its place when it's
/// there, else last.
pub fn put(f: &mut Fields, key: &str, v: Option<Value>) {
    match f.iter_mut().find(|(k, _)| k == key) {
        Some(slot) => slot.1 = v,
        None => f.push((key.to_string(), v)),
    }
}

/// `pages ? { pages } : undefined`.
pub fn pages_extra(pages: Option<Vec<String>>) -> Option<Map<String, Value>> {
    pages.map(|p| {
        let mut m = Map::new();
        m.insert("pages".into(), Value::Array(p.into_iter().map(Value::String).collect()));
        m
    })
}

pub fn ev(fields: Vec<(&str, Option<Value>)>) -> Fields {
    fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

pub fn sv(v: &str) -> Option<Value> {
    Some(Value::String(v.into()))
}

pub fn nv(v: f64) -> Option<Value> {
    Some(number(v))
}

/// A stable sort by a comparator that gives a number, as JS's sort takes
/// one: negative puts `a` first, and NaN counts as 0. (Rust's own sort may
/// panic on an order that isn't total.) The items move once, at the end:
/// the merges work on their places, as the history holds a hundred
/// thousand events with maps in them at start.
pub fn sort_by_num<T>(v: &mut Vec<T>, cmp: impl Fn(&T, &T) -> f64) {
    let n = v.len();
    if n < 2 {
        return;
    }
    let mut order: Vec<usize> = (0..n).collect();
    let mut spare = vec![0usize; n];
    let mut width = 1;
    while width < n {
        let mut start = 0;
        while start < n {
            let mid = (start + width).min(n);
            let end = (start + 2 * width).min(n);
            let (mut i, mut j, mut k) = (start, mid, start);
            while i < mid && j < end {
                // The right one goes first only when it's strictly before.
                if cmp(&v[order[i]], &v[order[j]]) > 0.0 {
                    spare[k] = order[j];
                    j += 1;
                } else {
                    spare[k] = order[i];
                    i += 1;
                }
                k += 1;
            }
            spare[k..k + (mid - i)].copy_from_slice(&order[i..mid]);
            k += mid - i;
            spare[k..k + (end - j)].copy_from_slice(&order[j..end]);
            start = end;
        }
        std::mem::swap(&mut order, &mut spare);
        width *= 2;
    }
    let mut taken: Vec<Option<T>> = std::mem::take(v).into_iter().map(Some).collect();
    v.extend(order.into_iter().map(|i| taken[i].take().expect("each place once")));
}

pub fn num_of(v: Option<&Value>, key: &str) -> f64 {
    js_number(opt_prop(v, key))
}

/// `(v ?? 0)` as a number.
pub fn or0(v: Option<&Value>) -> f64 {
    nn(v).map_or(0.0, |v| js_number(Some(v)))
}
