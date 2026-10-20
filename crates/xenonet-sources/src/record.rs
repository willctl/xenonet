//! An object the readers make for JS, its fields in the order JS adds
//! them: Claude Code's and Codex's sessions (`session`), and the entries of
//! a transcript or a thread as the console shows them.

use crate::lines::Js;
use serde_json::Value;

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
}

/// An object made for JS, its fields in order.
pub type Obj = Vec<(&'static str, F)>;
