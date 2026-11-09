//! An object the readers make for JS, its fields in the order JS adds
//! them: Claude Code's and Codex's sessions (`session`), and the entries of
//! a transcript or a thread as the console shows them; its JSON, and what
//! the readers that make one share (an excerpt, a template's `${v}`, a
//! slice's index).

use crate::lines::Js;
use serde_json::{Map, Value};
use xenonet_base::text::js_string;
use xenonet_base::{json, units};

/// A field of an object made for JS: missing (undefined), a number JSON
/// can't hold (NaN), a JSON value, or a list of such.
#[derive(Clone, Debug, PartialEq)]
pub enum F {
    Undef,
    Num(f64),
    V(Value),
    Arr(Vec<F>),
}

impl F {
    pub fn of(v: Js<'_>) -> F {
        v.map_or(F::Undef, |x| F::V(x.clone()))
    }
    pub fn s(v: impl Into<String>) -> F {
        F::V(Value::String(v.into()))
    }
    pub fn null() -> F {
        F::V(Value::Null)
    }
    /// `v ?? null`
    pub fn or_null(v: Js<'_>) -> F {
        F::V(v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null))
    }
    pub fn as_value(&self) -> Option<&Value> {
        match self {
            F::V(v) => Some(v),
            _ => None,
        }
    }

    /// The field as JSON: None for undefined.
    pub fn json(&self) -> Option<Value> {
        match self {
            F::Undef => None,
            F::Num(n) => Some(json::number(*n)),
            F::V(v) => Some(v.clone()),
            F::Arr(a) => Some(Value::Array(a.iter().map(|x| x.json().unwrap_or(Value::Null)).collect())),
        }
    }
}

/// An object made for JS, its fields in order.
pub type Obj = Vec<(&'static str, F)>;

/// An object as JSON, its keys in order and its undefined fields left out.
pub fn obj_json(o: &Obj) -> Value {
    let mut m = Map::new();
    for (k, v) in o {
        if let Some(v) = v.json() {
            m.insert((*k).to_string(), v);
        }
    }
    Value::Object(m)
}

/// A value that isn't null (`v ?? undefined`).
pub fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

/// `${v}` in a template: undefined as "undefined".
pub fn tpl(v: Js<'_>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

/// excerpt(s, n) on any value, as units.js does it: a long string cut and
/// said so, nothing as "", and anything else as it is.
pub fn excerpt(s: Js<'_>, n: usize) -> F {
    match s {
        None | Some(Value::Null) => F::s(""),
        Some(Value::String(t)) => F::s(units::excerpt(t, n)),
        Some(Value::Array(a)) if a.len() > n => F::s(format!("{}\n… ({} more chars)", js_string(&Value::Array(a[..n].to_vec())), a.len() - n)),
        Some(v) => F::V(v.clone()),
    }
}

/// excerpt(JSON.stringify(v, null, 2), n): JSON.stringify of nothing is
/// undefined, which excerpt makes "".
pub fn excerpt_json(v: Js<'_>, n: usize) -> F {
    match v {
        None => F::s(""),
        Some(v) => F::s(units::excerpt(&json::stringify(v, 2), n)),
    }
}

/// `xs.slice(start, end)`'s index, as JS reads a number.
pub fn slice_index(i: f64, len: usize) -> usize {
    if i.is_nan() {
        return 0;
    }
    let i = i.trunc();
    if i < 0.0 {
        (len as f64 + i).max(0.0) as usize
    } else {
        i.min(len as f64) as usize
    }
}

/// Math.max(0, x): NaN stays NaN.
pub fn js_max0(x: f64) -> f64 {
    if x.is_nan() {
        x
    } else {
        x.max(0.0)
    }
}
