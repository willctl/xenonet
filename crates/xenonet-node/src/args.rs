//! JS values as SQL arguments, rows as node:sqlite makes them, and its
//! errors (xenonet_engine::sql): undefined, a BigInt and bytes kept apart
//! from null, a number and a list; rows with no prototype, as
//! node:sqlite's are; errors with node:sqlite's codes.

use napi::bindgen_prelude::*;
use napi::sys;
use std::ptr;
use xenonet_engine::sql::{Arg, Cell, Row, SqlError};

fn check(status: sys::napi_status, what: &str) -> Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Error::new(Status::GenericFailure, format!("couldn't {what} ({status})")))
    }
}

pub(crate) unsafe fn arg_of(env: sys::napi_env, v: sys::napi_value, depth: usize) -> Result<Arg> {
    use sys::ValueType::{napi_bigint, napi_boolean, napi_null, napi_number, napi_object, napi_string, napi_undefined};
    if depth > 64 {
        return Ok(Arg::Other);
    }
    let t = unsafe { crate::js::type_of(env, v)? };
    Ok(match t {
        _ if t == napi_undefined => Arg::Undef,
        _ if t == napi_null => Arg::Null,
        _ if t == napi_boolean => Arg::Bool(unsafe { bool::from_napi_value(env, v)? }),
        _ if t == napi_number => Arg::Num(unsafe { f64::from_napi_value(env, v)? }),
        _ if t == napi_string => Arg::Text(unsafe { String::from_napi_value(env, v)? }),
        _ if t == napi_bigint => {
            let mut n = 0i64;
            let mut lossless = false;
            check(unsafe { sys::napi_get_value_bigint_int64(env, v, &raw mut n, &raw mut lossless) }, "read a BigInt")?;
            if lossless {
                Arg::Big(n)
            } else {
                Arg::Other
            }
        }
        _ if t == napi_object => {
            let mut is = false;
            check(unsafe { sys::napi_is_array(env, v, &raw mut is) }, "tell an array")?;
            if is {
                let mut len = 0u32;
                check(unsafe { sys::napi_get_array_length(env, v, &raw mut len) }, "read an array's length")?;
                let mut out = Vec::with_capacity(len as usize);
                for i in 0..len {
                    let mut item = ptr::null_mut();
                    check(unsafe { sys::napi_get_element(env, v, i, &raw mut item) }, "read an array")?;
                    out.push(unsafe { arg_of(env, item, depth + 1)? });
                }
                return Ok(Arg::List(out));
            }
            check(unsafe { sys::napi_is_typedarray(env, v, &raw mut is) }, "tell bytes")?;
            if is {
                let mut kind = 0;
                let mut len = 0usize;
                let mut data = ptr::null_mut();
                let mut buf = ptr::null_mut();
                let mut offset = 0usize;
                check(unsafe { sys::napi_get_typedarray_info(env, v, &raw mut kind, &raw mut len, &raw mut data, &raw mut buf, &raw mut offset) }, "read bytes")?;
                if kind == sys::TypedarrayType::uint8_array || kind == sys::TypedarrayType::uint8_clamped_array || kind == sys::TypedarrayType::int8_array {
                    let bytes = if len == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(data as *const u8, len) }.to_vec() };
                    return Ok(Arg::Bytes(bytes));
                }
                return Ok(Arg::Other);
            }
            check(unsafe { sys::napi_is_date(env, v, &raw mut is) }, "tell a date")?;
            if is {
                return Ok(Arg::Other);
            }
            // A Map as its [key, value] pairs, a Set as its values.
            if let Some(items) = unsafe { collection(env, v)? } {
                return unsafe { arg_of(env, items, depth + 1) };
            }
            let mut keys = ptr::null_mut();
            check(
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
            check(unsafe { sys::napi_get_array_length(env, keys, &raw mut n) }, "count an object's keys")?;
            let mut fields = Vec::with_capacity(n as usize);
            for i in 0..n {
                let mut key = ptr::null_mut();
                check(unsafe { sys::napi_get_element(env, keys, i, &raw mut key) }, "read a key")?;
                let mut item = ptr::null_mut();
                check(unsafe { sys::napi_get_property(env, v, key, &raw mut item) }, "read a property")?;
                fields.push((unsafe { String::from_napi_value(env, key)? }, unsafe { arg_of(env, item, depth + 1)? }));
            }
            Arg::Obj(fields)
        }
        _ => Arg::Other,
    })
}

// A Map or a Set, as Array.from makes it, or None for anything else.
unsafe fn collection(env: sys::napi_env, v: sys::napi_value) -> Result<Option<sys::napi_value>> {
    let mut global = ptr::null_mut();
    check(unsafe { sys::napi_get_global(env, &raw mut global) }, "get the global")?;
    for name in [c"Map", c"Set"] {
        let mut ctor = ptr::null_mut();
        check(unsafe { sys::napi_get_named_property(env, global, name.as_ptr(), &raw mut ctor) }, "get a constructor")?;
        let mut is = false;
        check(unsafe { sys::napi_instanceof(env, v, ctor, &raw mut is) }, "tell a collection")?;
        if is {
            let mut array = ptr::null_mut();
            check(unsafe { sys::napi_get_named_property(env, global, c"Array".as_ptr(), &raw mut array) }, "get Array")?;
            let mut from = ptr::null_mut();
            check(unsafe { sys::napi_get_named_property(env, array, c"from".as_ptr(), &raw mut from) }, "get Array.from")?;
            let mut out = ptr::null_mut();
            check(unsafe { sys::napi_call_function(env, array, from, 1, &raw const v, &raw mut out) }, "list a collection")?;
            return Ok(Some(out));
        }
    }
    Ok(None)
}

/// A JS value as it came.
pub struct JsArg(pub Arg);

impl FromNapiValue for JsArg {
    unsafe fn from_napi_value(env: sys::napi_env, v: sys::napi_value) -> Result<Self> {
        Ok(JsArg(unsafe { arg_of(env, v, 0)? }))
    }
}

impl TypeName for JsArg {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

impl ValidateNapiValue for JsArg {}

/// A call's arguments, as a JS array of them.
pub struct JsArgs(pub Vec<Arg>);

impl FromNapiValue for JsArgs {
    unsafe fn from_napi_value(env: sys::napi_env, v: sys::napi_value) -> Result<Self> {
        match unsafe { arg_of(env, v, 0)? } {
            Arg::List(a) => Ok(JsArgs(a)),
            Arg::Undef | Arg::Null => Ok(JsArgs(Vec::new())),
            other => Ok(JsArgs(vec![other])),
        }
    }
}

impl TypeName for JsArgs {
    fn type_name() -> &'static str {
        "unknown[]"
    }
    fn value_type() -> ValueType {
        ValueType::Object
    }
}

impl ValidateNapiValue for JsArgs {}

unsafe fn object_create(env: sys::napi_env) -> Result<(sys::napi_value, sys::napi_value)> {
    let mut global = ptr::null_mut();
    check(unsafe { sys::napi_get_global(env, &raw mut global) }, "get the global")?;
    let mut object = ptr::null_mut();
    check(unsafe { sys::napi_get_named_property(env, global, c"Object".as_ptr(), &raw mut object) }, "get Object")?;
    let mut create = ptr::null_mut();
    check(unsafe { sys::napi_get_named_property(env, object, c"create".as_ptr(), &raw mut create) }, "get Object.create")?;
    Ok((object, create))
}

unsafe fn cell_value(env: sys::napi_env, c: &Cell) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    match c {
        Cell::Null => check(unsafe { sys::napi_get_null(env, &raw mut out) }, "make null")?,
        Cell::Int(i) => check(unsafe { sys::napi_create_double(env, *i as f64, &raw mut out) }, "make a number")?,
        Cell::Real(f) => check(unsafe { sys::napi_create_double(env, *f, &raw mut out) }, "make a number")?,
        Cell::Text(s) => check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &raw mut out) }, "make a string")?,
        Cell::Blob(b) => {
            let mut data = ptr::null_mut();
            let mut buffer = ptr::null_mut();
            check(unsafe { sys::napi_create_arraybuffer(env, b.len(), &raw mut data, &raw mut buffer) }, "make bytes")?;
            if !b.is_empty() {
                unsafe { ptr::copy_nonoverlapping(b.as_ptr(), data.cast::<u8>(), b.len()) };
            }
            check(unsafe { sys::napi_create_typedarray(env, sys::TypedarrayType::uint8_array, b.len(), buffer, 0, &raw mut out) }, "make a Uint8Array")?;
        }
    }
    Ok(out)
}

unsafe fn row_value(env: sys::napi_env, row: &Row, object: sys::napi_value, create: sys::napi_value) -> Result<sys::napi_value> {
    let mut null = ptr::null_mut();
    check(unsafe { sys::napi_get_null(env, &raw mut null) }, "make null")?;
    let mut out = ptr::null_mut();
    check(unsafe { sys::napi_call_function(env, object, create, 1, &raw const null, &raw mut out) }, "make a row")?;
    for (k, c) in &row.0 {
        let mut key = ptr::null_mut();
        check(unsafe { sys::napi_create_string_utf8(env, k.as_ptr().cast(), k.len() as isize, &raw mut key) }, "make a key")?;
        let value = unsafe { cell_value(env, c)? };
        check(unsafe { sys::napi_set_property(env, out, key, value) }, "fill a row")?;
    }
    Ok(out)
}

/// What a store call answers.
pub enum Out {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Json(serde_json::Value),
    Row(Option<Row>),
    /// A statement's get(): undefined for no row.
    First(Option<Row>),
    Rows(Vec<Row>),
    Changes(xenonet_engine::sql::Changes),
}

impl ToNapiValue for Out {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let mut out = ptr::null_mut();
        match val {
            Out::Undef => check(unsafe { sys::napi_get_undefined(env, &raw mut out) }, "make undefined")?,
            Out::Null | Out::Row(None) => check(unsafe { sys::napi_get_null(env, &raw mut out) }, "make null")?,
            Out::First(None) => check(unsafe { sys::napi_get_undefined(env, &raw mut out) }, "make undefined")?,
            Out::Bool(b) => check(unsafe { sys::napi_get_boolean(env, b, &raw mut out) }, "make a boolean")?,
            Out::Num(n) => check(unsafe { sys::napi_create_double(env, n, &raw mut out) }, "make a number")?,
            Out::Json(v) => out = unsafe { crate::js::to_js(env, &v, 0)? },
            Out::Row(Some(r)) | Out::First(Some(r)) => {
                let (object, create) = unsafe { object_create(env)? };
                out = unsafe { row_value(env, &r, object, create)? };
            }
            Out::Rows(rows) => {
                let (object, create) = unsafe { object_create(env)? };
                check(unsafe { sys::napi_create_array_with_length(env, rows.len(), &raw mut out) }, "make an array")?;
                for (i, r) in rows.iter().enumerate() {
                    let item = unsafe { row_value(env, r, object, create)? };
                    check(unsafe { sys::napi_set_element(env, out, i as u32, item) }, "fill an array")?;
                }
            }
            Out::Changes(c) => {
                check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
                for (k, v) in [(c"lastInsertRowid", c.last_insert_rowid as f64), (c"changes", c.changes as f64)] {
                    let mut value = ptr::null_mut();
                    check(unsafe { sys::napi_create_double(env, v, &raw mut value) }, "make a number")?;
                    check(unsafe { sys::napi_set_named_property(env, out, k.as_ptr(), value) }, "fill an object")?;
                }
            }
        }
        Ok(out)
    }
}

impl TypeName for Out {
    fn type_name() -> &'static str {
        "unknown"
    }
    fn value_type() -> ValueType {
        ValueType::Unknown
    }
}

/// Throws a SQL error as node:sqlite throws it, with its code (and
/// SQLite's errcode and errstr), and says so to napi-rs.
pub fn throw_sql(env: &Env, e: &SqlError) -> Error {
    let env = env.raw();
    unsafe {
        let mut msg = ptr::null_mut();
        let m = e.message();
        sys::napi_create_string_utf8(env, m.as_ptr().cast(), m.len() as isize, &raw mut msg);
        let mut code = ptr::null_mut();
        if let Some(c) = e.code() {
            sys::napi_create_string_utf8(env, c.as_ptr().cast(), c.len() as isize, &raw mut code);
        }
        let mut err = ptr::null_mut();
        match e {
            SqlError::Bind(_) => sys::napi_create_type_error(env, code, msg, &raw mut err),
            SqlError::Range(_) => sys::napi_create_range_error(env, code, msg, &raw mut err),
            _ => sys::napi_create_error(env, code, msg, &raw mut err),
        };
        if let SqlError::Sqlite { errcode, errstr, .. } = e {
            let mut n = ptr::null_mut();
            sys::napi_create_int32(env, *errcode, &raw mut n);
            sys::napi_set_named_property(env, err, c"errcode".as_ptr(), n);
            let mut s = ptr::null_mut();
            sys::napi_create_string_utf8(env, errstr.as_ptr().cast(), errstr.len() as isize, &raw mut s);
            sys::napi_set_named_property(env, err, c"errstr".as_ptr(), s);
        }
        sys::napi_throw(env, err);
    }
    Error::new(Status::PendingException, e.message().to_string())
}

pub(crate) unsafe fn arg_to_js(env: sys::napi_env, a: &Arg) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    match a {
        Arg::Undef | Arg::Other => check(unsafe { sys::napi_get_undefined(env, &raw mut out) }, "make undefined")?,
        Arg::Null => check(unsafe { sys::napi_get_null(env, &raw mut out) }, "make null")?,
        Arg::Bool(b) => check(unsafe { sys::napi_get_boolean(env, *b, &raw mut out) }, "make a boolean")?,
        Arg::Num(n) => check(unsafe { sys::napi_create_double(env, *n, &raw mut out) }, "make a number")?,
        Arg::Big(n) => check(unsafe { sys::napi_create_bigint_int64(env, *n, &raw mut out) }, "make a BigInt")?,
        Arg::Text(s) => check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &raw mut out) }, "make a string")?,
        Arg::Bytes(b) => out = unsafe { cell_value(env, &Cell::Blob(b.clone()))? },
        Arg::List(items) => {
            check(unsafe { sys::napi_create_array_with_length(env, items.len(), &raw mut out) }, "make an array")?;
            for (i, x) in items.iter().enumerate() {
                let v = unsafe { arg_to_js(env, x)? };
                check(unsafe { sys::napi_set_element(env, out, i as u32, v) }, "fill an array")?;
            }
        }
        Arg::Obj(fields) => {
            check(unsafe { sys::napi_create_object(env, &raw mut out) }, "make an object")?;
            for (k, x) in fields {
                let mut key = ptr::null_mut();
                check(unsafe { sys::napi_create_string_utf8(env, k.as_ptr().cast(), k.len() as isize, &raw mut key) }, "make a key")?;
                let v = unsafe { arg_to_js(env, x)? };
                check(unsafe { sys::napi_set_property(env, out, key, v) }, "fill an object")?;
            }
        }
    }
    Ok(out)
}

impl ToNapiValue for JsArg {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        unsafe { arg_to_js(env, &val.0) }
    }
}

/// A JSON value as the JS value it reads as.
pub fn value_to_arg(v: &serde_json::Value) -> Arg {
    use serde_json::Value as V;
    match v {
        V::Null => Arg::Null,
        V::Bool(b) => Arg::Bool(*b),
        V::Number(n) => Arg::Num(n.as_f64().unwrap_or(f64::NAN)),
        V::String(s) => Arg::Text(s.clone()),
        V::Array(a) => Arg::List(a.iter().map(value_to_arg).collect()),
        V::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), value_to_arg(v))).collect()),
    }
}
