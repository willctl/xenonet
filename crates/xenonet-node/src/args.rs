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
    use sys::ValueType::*;
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
            check(unsafe { sys::napi_get_value_bigint_int64(env, v, &mut n, &mut lossless) }, "read a BigInt")?;
            if lossless {
                Arg::Big(n)
            } else {
                Arg::Other
            }
        }
        _ if t == napi_object => {
            let mut is = false;
            check(unsafe { sys::napi_is_array(env, v, &mut is) }, "tell an array")?;
            if is {
                let mut len = 0u32;
                check(unsafe { sys::napi_get_array_length(env, v, &mut len) }, "read an array's length")?;
                let mut out = Vec::with_capacity(len as usize);
                for i in 0..len {
                    let mut item = ptr::null_mut();
                    check(unsafe { sys::napi_get_element(env, v, i, &mut item) }, "read an array")?;
                    out.push(unsafe { arg_of(env, item, depth + 1)? });
                }
                return Ok(Arg::List(out));
            }
            check(unsafe { sys::napi_is_typedarray(env, v, &mut is) }, "tell bytes")?;
            if is {
                let mut kind = 0;
                let mut len = 0usize;
                let mut data = ptr::null_mut();
                let mut buf = ptr::null_mut();
                let mut offset = 0usize;
                check(unsafe { sys::napi_get_typedarray_info(env, v, &mut kind, &mut len, &mut data, &mut buf, &mut offset) }, "read bytes")?;
                if kind == sys::TypedarrayType::uint8_array || kind == sys::TypedarrayType::uint8_clamped_array || kind == sys::TypedarrayType::int8_array {
                    let bytes = if len == 0 { Vec::new() } else { unsafe { std::slice::from_raw_parts(data as *const u8, len) }.to_vec() };
                    return Ok(Arg::Bytes(bytes));
                }
                return Ok(Arg::Other);
            }
            check(unsafe { sys::napi_is_date(env, v, &mut is) }, "tell a date")?;
            if is {
                return Ok(Arg::Other);
            }
            let mut keys = ptr::null_mut();
            check(
                unsafe { sys::napi_get_all_property_names(env, v, sys::KeyCollectionMode::own_only, sys::KeyFilter::enumerable | sys::KeyFilter::skip_symbols, sys::KeyConversion::numbers_to_strings, &mut keys) },
                "list an object's keys",
            )?;
            let mut n = 0u32;
            check(unsafe { sys::napi_get_array_length(env, keys, &mut n) }, "count an object's keys")?;
            let mut fields = Vec::with_capacity(n as usize);
            for i in 0..n {
                let mut key = ptr::null_mut();
                check(unsafe { sys::napi_get_element(env, keys, i, &mut key) }, "read a key")?;
                let mut item = ptr::null_mut();
                check(unsafe { sys::napi_get_property(env, v, key, &mut item) }, "read a property")?;
                fields.push((unsafe { String::from_napi_value(env, key)? }, unsafe { arg_of(env, item, depth + 1)? }));
            }
            Arg::Obj(fields)
        }
        _ => Arg::Other,
    })
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
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
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
    fn value_type() -> napi::ValueType {
        napi::ValueType::Object
    }
}

impl ValidateNapiValue for JsArgs {}

unsafe fn object_create(env: sys::napi_env) -> Result<(sys::napi_value, sys::napi_value)> {
    let mut global = ptr::null_mut();
    check(unsafe { sys::napi_get_global(env, &mut global) }, "get the global")?;
    let mut object = ptr::null_mut();
    check(unsafe { sys::napi_get_named_property(env, global, c"Object".as_ptr(), &mut object) }, "get Object")?;
    let mut create = ptr::null_mut();
    check(unsafe { sys::napi_get_named_property(env, object, c"create".as_ptr(), &mut create) }, "get Object.create")?;
    Ok((object, create))
}

unsafe fn cell_value(env: sys::napi_env, c: &Cell) -> Result<sys::napi_value> {
    let mut out = ptr::null_mut();
    match c {
        Cell::Null => check(unsafe { sys::napi_get_null(env, &mut out) }, "make null")?,
        Cell::Int(i) => check(unsafe { sys::napi_create_double(env, *i as f64, &mut out) }, "make a number")?,
        Cell::Real(f) => check(unsafe { sys::napi_create_double(env, *f, &mut out) }, "make a number")?,
        Cell::Text(s) => check(unsafe { sys::napi_create_string_utf8(env, s.as_ptr().cast(), s.len() as isize, &mut out) }, "make a string")?,
        Cell::Blob(b) => {
            let mut data = ptr::null_mut();
            let mut buffer = ptr::null_mut();
            check(unsafe { sys::napi_create_arraybuffer(env, b.len(), &mut data, &mut buffer) }, "make bytes")?;
            if !b.is_empty() {
                unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), data as *mut u8, b.len()) };
            }
            check(unsafe { sys::napi_create_typedarray(env, sys::TypedarrayType::uint8_array, b.len(), buffer, 0, &mut out) }, "make a Uint8Array")?;
        }
    }
    Ok(out)
}

unsafe fn row_value(env: sys::napi_env, row: &Row, object: sys::napi_value, create: sys::napi_value) -> Result<sys::napi_value> {
    let mut null = ptr::null_mut();
    check(unsafe { sys::napi_get_null(env, &mut null) }, "make null")?;
    let mut out = ptr::null_mut();
    check(unsafe { sys::napi_call_function(env, object, create, 1, &null, &mut out) }, "make a row")?;
    for (k, c) in &row.0 {
        let mut key = ptr::null_mut();
        check(unsafe { sys::napi_create_string_utf8(env, k.as_ptr().cast(), k.len() as isize, &mut key) }, "make a key")?;
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
            Out::Undef => check(unsafe { sys::napi_get_undefined(env, &mut out) }, "make undefined")?,
            Out::Null | Out::Row(None) => check(unsafe { sys::napi_get_null(env, &mut out) }, "make null")?,
            Out::First(None) => check(unsafe { sys::napi_get_undefined(env, &mut out) }, "make undefined")?,
            Out::Bool(b) => check(unsafe { sys::napi_get_boolean(env, b, &mut out) }, "make a boolean")?,
            Out::Num(n) => check(unsafe { sys::napi_create_double(env, n, &mut out) }, "make a number")?,
            Out::Json(v) => out = unsafe { crate::js::to_js(env, &v, 0)? },
            Out::Row(Some(r)) | Out::First(Some(r)) => {
                let (object, create) = unsafe { object_create(env)? };
                out = unsafe { row_value(env, &r, object, create)? };
            }
            Out::Rows(rows) => {
                let (object, create) = unsafe { object_create(env)? };
                check(unsafe { sys::napi_create_array_with_length(env, rows.len(), &mut out) }, "make an array")?;
                for (i, r) in rows.iter().enumerate() {
                    let item = unsafe { row_value(env, r, object, create)? };
                    check(unsafe { sys::napi_set_element(env, out, i as u32, item) }, "fill an array")?;
                }
            }
            Out::Changes(c) => {
                check(unsafe { sys::napi_create_object(env, &mut out) }, "make an object")?;
                for (k, v) in [("lastInsertRowid", c.last_insert_rowid as f64), ("changes", c.changes as f64)] {
                    let mut value = ptr::null_mut();
                    check(unsafe { sys::napi_create_double(env, v, &mut value) }, "make a number")?;
                    let key = std::ffi::CString::new(k).unwrap();
                    check(unsafe { sys::napi_set_named_property(env, out, key.as_ptr(), value) }, "fill an object")?;
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
    fn value_type() -> napi::ValueType {
        napi::ValueType::Unknown
    }
}

/// Throws a SQL error as node:sqlite throws it, with its code (and
/// SQLite's errcode and errstr), and says so to napi-rs.
pub fn throw_sql(env: &Env, e: &SqlError) -> Error {
    let env = env.raw();
    unsafe {
        let mut msg = ptr::null_mut();
        let m = e.message();
        sys::napi_create_string_utf8(env, m.as_ptr().cast(), m.len() as isize, &mut msg);
        let mut code = ptr::null_mut();
        if let Some(c) = e.code() {
            sys::napi_create_string_utf8(env, c.as_ptr().cast(), c.len() as isize, &mut code);
        }
        let mut err = ptr::null_mut();
        match e {
            SqlError::Bind(_) => sys::napi_create_type_error(env, code, msg, &mut err),
            SqlError::Range(_) => sys::napi_create_range_error(env, code, msg, &mut err),
            _ => sys::napi_create_error(env, code, msg, &mut err),
        };
        if let SqlError::Sqlite { errcode, errstr, .. } = e {
            let mut n = ptr::null_mut();
            sys::napi_create_int32(env, *errcode, &mut n);
            sys::napi_set_named_property(env, err, c"errcode".as_ptr(), n);
            let mut s = ptr::null_mut();
            sys::napi_create_string_utf8(env, errstr.as_ptr().cast(), errstr.len() as isize, &mut s);
            sys::napi_set_named_property(env, err, c"errstr".as_ptr(), s);
        }
        sys::napi_throw(env, err);
    }
    Error::new(Status::PendingException, e.message().to_string())
}
