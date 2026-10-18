//! JSON values to and from JS, as JS itself would make them: the way
//! JSON.parse builds a value, and the way JSON.stringify reads one.
//!
//! napi's own conversion of serde_json values differs from JS where it
//! matters to the twins: a whole number past 2^53 becomes a BigInt, and a
//! "__proto__" key sets the object's prototype rather than making a key.
//! Here every number is a JS number, "__proto__" is an own key as
//! JSON.parse makes it, and from JS, what JSON.stringify leaves out of an
//! object (undefined, functions, symbols) is left out, a Date is its ISO
//! time, and a number that isn't finite is null.

use napi::bindgen_prelude::{FromNapiValue, ToNapiValue};
use napi::{sys, Error, Result, Status};
use serde_json::{Map, Value};
use std::ptr;
use xenonet_base::{json, time};

/// A JSON value on its way to or from JS.
pub struct Json(pub Value);

const MAX_DEPTH: usize = 512;

fn check(status: sys::napi_status, what: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Error::new(Status::GenericFailure, format!("couldn't {what} ({status})")))
    }
}

pub(crate) unsafe fn string(env: sys::napi_env, s: &str) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    // SAFETY: env is the call's, `s` goes in as its bytes and length; napi
    // writes to the local `out`.
    check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &raw mut out) }, "make a string")?;
    Ok(out)
}

pub(crate) unsafe fn to_js(env: sys::napi_env, v: &Value, depth: usize) -> Result<sys::napi_value> {
    if depth > MAX_DEPTH {
        return Err(Error::new(Status::InvalidArg, "a value nested too deeply".to_string()));
    }
    let mut out = ptr::null_mut();
    match v {
        // SAFETY: env is the call's; napi writes to the local `out`.
        Value::Null => check(unsafe { sys::napi_get_null(env, &raw mut out) }, "make null")?,
        // SAFETY: env is the call's; napi writes to the local `out`.
        Value::Bool(b) => check(unsafe { sys::napi_get_boolean(env, *b, &raw mut out) }, "make a boolean")?,
        // SAFETY: env is the call's; napi writes to the local `out`.
        Value::Number(n) => check(unsafe { sys::napi_create_double(env, n.as_f64().unwrap_or(f64::NAN), &raw mut out) }, "make a number")?,
        // SAFETY: env is the call's.
        Value::String(s) => out = unsafe { string(env, s)? },
        Value::Array(a) => {
            // SAFETY: env is the call's; napi writes to the local `out`.
            check(unsafe { sys::napi_create_array_with_length(env, a.len(), &raw mut out) }, "make an array")?;
            for (i, x) in a.iter().enumerate() {
                // SAFETY: env is the call's.
                let item = unsafe { to_js(env, x, depth + 1)? };
                // SAFETY: env, `out` and `item` are the call's.
                check(unsafe { sys::napi_set_element(env, out, i as u32, item) }, "fill an array")?;
            }
        }
        Value::Object(m) => {
            // SAFETY: env is the call's; napi writes to the local `out`.
            check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
            for (k, x) in m {
                // SAFETY: env is the call's.
                let key = unsafe { string(env, k)? };
                // SAFETY: env is the call's.
                let value = unsafe { to_js(env, x, depth + 1)? };
                if k == "__proto__" {
                    // Defined, not set: setting __proto__ would change the
                    // object's prototype.
                    let attributes = sys::PropertyAttributes::writable | sys::PropertyAttributes::enumerable | sys::PropertyAttributes::configurable;
                    let d = sys::napi_property_descriptor { utf8name: ptr::null(), name: key, method: None, getter: None, setter: None, value, attributes, data: ptr::null_mut() };
                    // SAFETY: env and `out` are the call's, `d` is a local napi reads.
                    check(unsafe { sys::napi_define_properties(env, out, 1, &raw const d) }, "define a key")?;
                } else {
                    // SAFETY: env, `out`, `key` and `value` are the call's.
                    check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill an object")?;
                }
            }
        }
    }
    Ok(out)
}

pub(crate) unsafe fn type_of(env: sys::napi_env, v: sys::napi_value) -> Result<i32> {
    let mut t = 0;
    // SAFETY: env and `v` are the call's; napi writes to the local `t`.
    check(unsafe { sys::napi_typeof(env, v, &raw mut t) }, "tell a value's type")?;
    Ok(t)
}

// What JSON.stringify leaves out of an object, and writes as null in an
// array.
fn skipped(t: i32) -> bool {
    use sys::ValueType::{napi_function, napi_symbol, napi_undefined};
    t == napi_undefined || t == napi_function || t == napi_symbol
}

pub(crate) unsafe fn from_js(env: sys::napi_env, v: sys::napi_value, depth: usize) -> Result<Value> {
    use sys::ValueType::{napi_boolean, napi_null, napi_number, napi_object, napi_string};
    if depth > MAX_DEPTH {
        return Err(Error::new(Status::InvalidArg, "a value nested too deeply, or in a cycle".to_string()));
    }
    // SAFETY: env and `v` are the call's.
    let t = unsafe { type_of(env, v)? };
    Ok(match t {
        _ if t == napi_null || skipped(t) => Value::Null,
        // SAFETY: env and `v` are the call's.
        _ if t == napi_boolean => Value::Bool(unsafe { bool::from_napi_value(env, v)? }),
        // SAFETY: env and `v` are the call's.
        _ if t == napi_number => json::number(unsafe { f64::from_napi_value(env, v)? }),
        // SAFETY: env and `v` are the call's.
        _ if t == napi_string => Value::String(unsafe { String::from_napi_value(env, v)? }),
        _ if t == napi_object => {
            let mut is = false;
            // SAFETY: env and `v` are the call's; napi writes to the local `is`.
            check(unsafe { sys::napi_is_array(env, v, &raw mut is) }, "tell an array")?;
            if is {
                let mut n = 0u32;
                // SAFETY: env and `v` are the call's; napi writes to the local `n`.
                check(unsafe { sys::napi_get_array_length(env, v, &raw mut n) }, "read an array's length")?;
                let mut a = Vec::with_capacity(n as usize);
                for i in 0..n {
                    let mut item = ptr::null_mut();
                    // SAFETY: env and `v` are the call's; napi writes to the local `item`.
                    check(unsafe { sys::napi_get_element(env, v, i, &raw mut item) }, "read an array")?;
                    // SAFETY: env and `item` are the call's.
                    let it = unsafe { type_of(env, item)? };
                    // SAFETY: env and `item` are the call's.
                    a.push(if skipped(it) { Value::Null } else { unsafe { from_js(env, item, depth + 1)? } });
                }
                return Ok(Value::Array(a));
            }
            // SAFETY: env and `v` are the call's; napi writes to the local `is`.
            check(unsafe { sys::napi_is_date(env, v, &raw mut is) }, "tell a date")?;
            if is {
                let mut ms = 0.0;
                // SAFETY: env and `v` are the call's; napi writes to the local `ms`.
                check(unsafe { sys::napi_get_date_value(env, v, &raw mut ms) }, "read a date")?;
                return Ok(if ms.is_finite() { Value::String(time::iso(ms)) } else { Value::Null });
            }
            let mut keys = ptr::null_mut();
            check(
                // SAFETY: env and `v` are the call's; napi writes to the
                // local `keys`.
                unsafe {
                    sys::napi_get_all_property_names(
                        env,
                        v,
                        sys::KeyCollectionMode::own_only,
                        sys::KeyFilter::enumerable | sys::KeyFilter::skip_symbols,
                        sys::KeyConversion::numbers_to_strings,
                        &raw mut keys,
                    )
                },
                "list an object's keys",
            )?;
            let mut n = 0u32;
            // SAFETY: env and `keys` are the call's; napi writes to the local `n`.
            check(unsafe { sys::napi_get_array_length(env, keys, &raw mut n) }, "count an object's keys")?;
            let mut m = Map::new();
            for i in 0..n {
                let mut key = ptr::null_mut();
                // SAFETY: env and `keys` are the call's; napi writes to the local `key`.
                check(unsafe { sys::napi_get_element(env, keys, i, &raw mut key) }, "read a key")?;
                let mut item = ptr::null_mut();
                // SAFETY: env, `v` and `key` are the call's; napi writes to the local `item`.
                check(unsafe { sys::napi_get_property(env, v, key, &raw mut item) }, "read a property")?;
                // SAFETY: env and `item` are the call's.
                let it = unsafe { type_of(env, item)? };
                if skipped(it) {
                    continue;
                }
                // SAFETY: env, `key` and `item` are the call's.
                m.insert(unsafe { String::from_napi_value(env, key)? }, unsafe { from_js(env, item, depth + 1)? });
            }
            Value::Object(m)
        }
        // A BigInt, which JSON.stringify refuses, or an external.
        _ => Value::Null,
    })
}

impl ToNapiValue for Json {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        // SAFETY: env is the call's.
        unsafe { to_js(env, &val.0, 0) }
    }
}

impl ToNapiValue for &Json {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        // SAFETY: env is the call's.
        unsafe { to_js(env, &val.0, 0) }
    }
}

impl FromNapiValue for Json {
    unsafe fn from_napi_value(env: sys::napi_env, v: sys::napi_value) -> Result<Self> {
        // SAFETY: env and `v` are the call's.
        Ok(Json(unsafe { from_js(env, v, 0)? }))
    }
}

impl napi::bindgen_prelude::TypeName for Json {
    fn type_name() -> &'static str {
        "Json"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

impl napi::bindgen_prelude::ValidateNapiValue for Json {}

/// One of an object's fields, built in order: a number (which may be
/// infinite, as a block's budget is), or a JSON value.
pub enum Field {
    Num(f64),
    Json(Value),
}

/// An object with its fields in order, for answers JSON can't hold.
pub struct Fields(pub Vec<(&'static str, Field)>);

impl ToNapiValue for Fields {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let mut out = ptr::null_mut();
        // SAFETY: env is the call's; napi writes to the local `out`.
        check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
        for (k, f) in &val.0 {
            // SAFETY: env is the call's.
            let key = unsafe { string(env, k)? };
            let value = match f {
                Field::Num(n) => {
                    let mut v = ptr::null_mut();
                    // SAFETY: env is the call's; napi writes to the local `v`.
                    check(unsafe { sys::napi_create_double(env, *n, &raw mut v) }, "make a number")?;
                    v
                }
                // SAFETY: env is the call's.
                Field::Json(j) => unsafe { to_js(env, j, 0)? },
            };
            // SAFETY: env, `out`, `key` and `value` are the call's.
            check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill an object")?;
        }
        Ok(out)
    }
}

impl napi::bindgen_prelude::TypeName for Fields {
    fn type_name() -> &'static str {
        "Object"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Object
    }
}

/// A JS value to build, for answers JSON can't hold: undefined, NaN and
/// the infinities, bytes, and objects with such fields, in order.
pub enum Jv {
    Undef,
    Num(f64),
    Bool(bool),
    Json(Value),
    Bytes(Vec<u8>),
    Obj(Vec<(&'static str, Jv)>),
    Arr(Vec<Jv>),
    /// A JS value made in this call, handed straight back.
    Raw(sys::napi_value),
}

impl From<Option<Value>> for Jv {
    fn from(v: Option<Value>) -> Self {
        v.map_or(Jv::Undef, Jv::Json)
    }
}

unsafe fn build(env: sys::napi_env, v: &Jv) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    match v {
        // SAFETY: env is the call's; napi writes to the local `out`.
        Jv::Undef => check(unsafe { sys::napi_get_undefined(env, &raw mut out) }, "make undefined")?,
        // SAFETY: env is the call's; napi writes to the local `out`.
        Jv::Num(n) => check(unsafe { sys::napi_create_double(env, *n, &raw mut out) }, "make a number")?,
        // SAFETY: env is the call's; napi writes to the local `out`.
        Jv::Bool(b) => check(unsafe { sys::napi_get_boolean(env, *b, &raw mut out) }, "make a boolean")?,
        // SAFETY: env is the call's.
        Jv::Json(j) => out = unsafe { to_js(env, j, 0)? },
        Jv::Bytes(b) => {
            let mut data = ptr::null_mut();
            // SAFETY: env is the call's, napi copies b.len() bytes from `b`,
            // and it writes to the locals `data` and `out`.
            check(unsafe { sys::napi_create_buffer_copy(env, b.len(), b.as_ptr().cast(), &raw mut data, &raw mut out) }, "make a buffer")?;
        }
        Jv::Obj(fields) => {
            // SAFETY: env is the call's; napi writes to the local `out`.
            check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
            for (k, f) in fields {
                // SAFETY: env is the call's.
                let key = unsafe { string(env, k)? };
                // SAFETY: env is the call's.
                let value = unsafe { build(env, f)? };
                // SAFETY: env, `out`, `key` and `value` are the call's.
                check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill an object")?;
            }
        }
        Jv::Raw(v) => out = *v,
        Jv::Arr(items) => {
            // SAFETY: env is the call's; napi writes to the local `out`.
            check(unsafe { sys::napi_create_array_with_length(env, items.len(), &raw mut out) }, "make an array")?;
            for (i, x) in items.iter().enumerate() {
                // SAFETY: env is the call's.
                let item = unsafe { build(env, x)? };
                // SAFETY: env, `out` and `item` are the call's.
                check(unsafe { sys::napi_set_element(env, out, i as u32, item) }, "fill an array")?;
            }
        }
    }
    Ok(out)
}

impl ToNapiValue for Jv {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        // SAFETY: env is the call's.
        unsafe { build(env, &val) }
    }
}

impl napi::bindgen_prelude::TypeName for Jv {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

/// A JS value coming in that may be undefined (None), as `typeof` tells
/// it from null.
pub struct MaybeJson(pub Option<Value>);

impl FromNapiValue for MaybeJson {
    unsafe fn from_napi_value(env: sys::napi_env, v: sys::napi_value) -> Result<Self> {
        // SAFETY: env and `v` are the call's.
        if unsafe { type_of(env, v)? } == sys::ValueType::napi_undefined {
            return Ok(MaybeJson(None));
        }
        // SAFETY: env and `v` are the call's.
        Ok(MaybeJson(Some(unsafe { from_js(env, v, 0)? })))
    }
}

impl napi::bindgen_prelude::TypeName for MaybeJson {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

impl napi::bindgen_prelude::ValidateNapiValue for MaybeJson {}
