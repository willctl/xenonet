//! A twin's collaborators, called from Rust on JS's thread
//! (xenonet_engine::peer): `deps.<path>.<method>(...args)` on the JS object
//! the twin hands over, looked up when it's called. What a call throws is
//! kept for the call into Rust to throw on as it was.

use crate::args::{arg_of, arg_to_js};
use napi::bindgen_prelude::*;
use napi::sys;
use std::cell::RefCell;
use std::ptr;
use xenonet_engine::peer::{self, Thrown};
use xenonet_engine::sql::Arg;

thread_local! {
    // What JS threw during the call under way, by handle.
    static THROWN: RefCell<Vec<(u64, sys::napi_ref)>> = const { RefCell::new(Vec::new()) };
    static NEXT: RefCell<u64> = const { RefCell::new(0) };
}

fn check(status: sys::napi_status, what: &str) -> peer::Result<()> {
    if status == sys::Status::napi_ok {
        Ok(())
    } else {
        Err(Thrown::new(format!("couldn't {what} ({status})")))
    }
}

// napi takes a property's name NUL-terminated. The paths and methods are
// the crates' own names, so one holding a NUL is a mistake to report, not
// a property to look up.
fn key_of(name: &str) -> peer::Result<std::ffi::CString> {
    std::ffi::CString::new(name).map_err(|_| Thrown::new(format!("couldn't read {name:?}: it holds a NUL")))
}

/// The deps object of a call into Rust. It's made from the env and deps
/// napi-rs passes into an export, and lives only while that export runs,
/// so its handles are valid for every call made through it.
pub struct SyncCalls {
    env: sys::napi_env,
    deps: sys::napi_value,
}

impl SyncCalls {
    pub fn new(env: &Env, deps: &Object<'_>) -> Self {
        SyncCalls { env: env.raw(), deps: deps.raw() }
    }

    unsafe fn at(&self, path: &str) -> peer::Result<sys::napi_value> {
        let mut v = self.deps;
        for k in path.split('.').filter(|k| !k.is_empty()) {
            let mut t = 0;
            // SAFETY: self.env is the export's (see SyncCalls), and `v` is
            // deps or a property read from it in this call; napi writes to
            // the local `t`.
            check(unsafe { sys::napi_typeof(self.env, v, &raw mut t) }, "tell a value's type")?;
            if t != sys::ValueType::napi_object && t != sys::ValueType::napi_function {
                return Err(Thrown::coded(format!("Cannot read properties of {} (reading '{k}')", if t == sys::ValueType::napi_null { "null" } else { "undefined" }), "TypeError"));
            }
            let key = key_of(k)?;
            let mut next = ptr::null_mut();
            // SAFETY: as above, and `key` is NUL-terminated; napi writes to
            // the local `next`.
            check(unsafe { sys::napi_get_named_property(self.env, v, key.as_ptr(), &raw mut next) }, "read a property")?;
            v = next;
        }
        Ok(v)
    }

    unsafe fn caught(&self) -> Thrown {
        let mut e = ptr::null_mut();
        // SAFETY: self.env is the export's (see SyncCalls), and a JS call
        // made through it just threw; napi writes what it threw to the
        // local `e`.
        unsafe { sys::napi_get_and_clear_last_exception(self.env, &raw mut e) };
        // SAFETY: self.env and `e` are the call's, and the name is
        // NUL-terminated.
        let message = unsafe { prop_text(self.env, e, c"message") }.unwrap_or_else(|| unsafe { text_of(self.env, e) });
        // SAFETY: as above.
        let code = unsafe { arg_of(self.env, prop(self.env, e, c"code"), 0) }.ok().filter(|c| !c.is_nullish()).map(|c| c.to_json());
        let mut r = ptr::null_mut();
        // SAFETY: self.env and `e` are the call's; napi writes the new
        // reference to the local `r`, which rethrow or settle deletes.
        unsafe { sys::napi_create_reference(self.env, e, 1, &raw mut r) };
        let h = NEXT.with(|n| {
            *n.borrow_mut() += 1;
            *n.borrow()
        });
        THROWN.with(|t| t.borrow_mut().push((h, r)));
        Thrown { message, code: code.map(Box::new), handle: Some(h), stdout: None }
    }
}

unsafe fn prop(env: sys::napi_env, v: sys::napi_value, k: &std::ffi::CStr) -> sys::napi_value {
    let mut out = ptr::null_mut();
    let mut t = 0;
    // SAFETY: env and `v` are the call's; napi writes to the local `t`.
    unsafe { sys::napi_typeof(env, v, &raw mut t) };
    if t == sys::ValueType::napi_object || t == sys::ValueType::napi_function {
        // SAFETY: env and `v` are the call's, and `k` is NUL-terminated;
        // napi writes to the local `out`.
        unsafe { sys::napi_get_named_property(env, v, k.as_ptr(), &raw mut out) };
    }
    if out.is_null() {
        // SAFETY: env is the call's; napi writes to the local `out`.
        unsafe { sys::napi_get_undefined(env, &raw mut out) };
    }
    out
}

unsafe fn prop_text(env: sys::napi_env, v: sys::napi_value, k: &std::ffi::CStr) -> Option<String> {
    // SAFETY: env and `v` are the call's, and `k` is NUL-terminated.
    match unsafe { arg_of(env, prop(env, v, k), 0) }.ok()? {
        Arg::Text(s) => Some(s),
        _ => None,
    }
}

unsafe fn text_of(env: sys::napi_env, v: sys::napi_value) -> String {
    let mut s = ptr::null_mut();
    // SAFETY: env and `v` are the call's; napi writes to the local `s`.
    unsafe { sys::napi_coerce_to_string(env, v, &raw mut s) };
    // SAFETY: env and `s` are the call's.
    unsafe { String::from_napi_value(env, s) }.unwrap_or_default()
}

impl peer::Calls for SyncCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> peer::Result<Arg> {
        // SAFETY: self.env and self.deps are the export's (see SyncCalls),
        // and every other handle here is read or made from them in this
        // call. `key` is NUL-terminated, napi writes to the locals `t`, `f`
        // and `out`, and argv's pointer and length are the handles it holds.
        unsafe {
            let target = self.at(path)?;
            let key = key_of(method)?;
            let mut t = 0;
            check(sys::napi_typeof(self.env, target, &raw mut t), "tell a value's type")?;
            if t != sys::ValueType::napi_object && t != sys::ValueType::napi_function {
                return Err(Thrown::coded(
                    format!("Cannot read properties of {} (reading '{method}')", if t == sys::ValueType::napi_null { "null" } else { "undefined" }),
                    "TypeError",
                ));
            }
            let mut f = ptr::null_mut();
            check(sys::napi_get_named_property(self.env, target, key.as_ptr(), &raw mut f), "read a method")?;
            check(sys::napi_typeof(self.env, f, &raw mut t), "tell a method")?;
            if t != sys::ValueType::napi_function {
                return Err(Thrown::new(format!("{path}.{method} is not a function")));
            }
            let mut argv = Vec::with_capacity(args.len());
            for a in &args {
                argv.push(arg_to_js(self.env, a).map_err(|e| Thrown::new(e.reason.clone()))?);
            }
            let mut out = ptr::null_mut();
            let status = sys::napi_call_function(self.env, target, f, argv.len(), argv.as_ptr(), &raw mut out);
            if status == sys::Status::napi_pending_exception {
                return Err(self.caught());
            }
            check(status, "call a method")?;
            arg_of(self.env, out, 0).map_err(|e| Thrown::new(e.reason.clone()))
        }
    }

    fn get(&mut self, path: &str) -> peer::Result<Arg> {
        // SAFETY: self.env and self.deps are the export's (see SyncCalls),
        // and `v` is read from deps in this call.
        unsafe {
            let v = self.at(path)?;
            arg_of(self.env, v, 0).map_err(|e| Thrown::new(e.reason.clone()))
        }
    }

    fn kind(&mut self, path: &str) -> peer::Result<String> {
        // SAFETY: as in get; napi writes to the local `t`.
        unsafe {
            // Reading through undefined or null is undefined here, as
            // optional chaining reads it.
            let Ok(v) = self.at(path) else { return Ok("undefined".into()) };
            let mut t = 0;
            check(sys::napi_typeof(self.env, v, &raw mut t), "tell a value's type")?;
            use sys::ValueType::{napi_bigint, napi_boolean, napi_function, napi_null, napi_number, napi_string, napi_symbol, napi_undefined};
            Ok(match t {
                _ if t == napi_undefined => "undefined",
                _ if t == napi_null => "null",
                _ if t == napi_boolean => "boolean",
                _ if t == napi_number => "number",
                _ if t == napi_string => "string",
                _ if t == napi_symbol => "symbol",
                _ if t == napi_function => "function",
                _ if t == napi_bigint => "bigint",
                _ => "object",
            }
            .into())
        }
    }
}

/// What a call into Rust that failed throws: the very value a JS call
/// threw when that's what it failed with, else an Error with its words
/// (and code). The values kept for this call go.
pub fn rethrow(env: &Env, t: Thrown) -> Error {
    let kept = THROWN.with(|k| std::mem::take(&mut *k.borrow_mut()));
    let mut out = None;
    for (h, r) in kept {
        if Some(h) == t.handle && out.is_none() {
            let mut v = ptr::null_mut();
            // SAFETY: env is the export's, as every Env here is, and `r` is
            // a reference caught() made that nothing has deleted yet; napi
            // writes its value to the local `v`, which is then thrown.
            unsafe {
                sys::napi_get_reference_value(env.raw(), r, &raw mut v);
                sys::napi_throw(env.raw(), v);
            }
            out = Some(Error::new(Status::PendingException, t.message.clone()));
        }
        // SAFETY: env is the export's, and `r` is a reference caught() made,
        // taken out of THROWN above, so it's deleted once.
        unsafe { sys::napi_delete_reference(env.raw(), r) };
    }
    out.unwrap_or_else(|| {
        let code: Vec<_> = t.code.iter().map(|c| (c"code", (**c).clone())).collect();
        throw_with(env, &t.message, &code)
    })
}

/// Throws `new Error(message)` with `fields` set on it, as the JS modules
/// throw their own errors (a LinkError, an ActorError, …), and says so to
/// napi-rs. A field JSON can't make is left off.
pub fn throw_with(env: &Env, message: &str, fields: &[(&std::ffi::CStr, serde_json::Value)]) -> Error {
    let raw = env.raw();
    // SAFETY: raw is the env of the export under way, as every Env here
    // is. msg, err and each js are handles made in it just above their
    // use, message's pointer and length are its UTF-8 bytes, and each key
    // is a NUL-terminated C string.
    unsafe {
        let mut msg = ptr::null_mut();
        sys::napi_create_string_utf8(raw, message.as_ptr().cast(), message.len() as isize, &raw mut msg);
        let mut err = ptr::null_mut();
        sys::napi_create_error(raw, ptr::null_mut(), msg, &raw mut err);
        for (k, v) in fields {
            if let Ok(js) = crate::js::to_js(raw, v, 0) {
                sys::napi_set_named_property(raw, err, k.as_ptr(), js);
            }
        }
        sys::napi_throw(raw, err);
    }
    Error::new(Status::PendingException, message.to_string())
}

/// The values kept for the call under way let go, once it's done.
pub fn settle(env: &Env) {
    for (_, r) in THROWN.with(|k| std::mem::take(&mut *k.borrow_mut())) {
        // SAFETY: env is the export's, as every Env here is, and `r` is a
        // reference caught() made, taken out of THROWN, so it's deleted once.
        unsafe { sys::napi_delete_reference(env.raw(), r) };
    }
}
