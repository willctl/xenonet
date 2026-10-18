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

/// The deps object of a call into Rust.
pub struct SyncCalls {
    env: sys::napi_env,
    deps: sys::napi_value,
}

impl SyncCalls {
    pub fn new(env: &Env, deps: &Object) -> Self {
        SyncCalls { env: env.raw(), deps: deps.raw() }
    }

    unsafe fn at(&self, path: &str) -> peer::Result<sys::napi_value> {
        let mut v = self.deps;
        for k in path.split('.').filter(|k| !k.is_empty()) {
            let mut t = 0;
            check(unsafe { sys::napi_typeof(self.env, v, &mut t) }, "tell a value's type")?;
            if t != sys::ValueType::napi_object && t != sys::ValueType::napi_function {
                return Err(Thrown::coded(format!("Cannot read properties of {} (reading '{k}')", if t == sys::ValueType::napi_null { "null" } else { "undefined" }), "TypeError"));
            }
            let key = std::ffi::CString::new(k).unwrap();
            let mut next = ptr::null_mut();
            check(unsafe { sys::napi_get_named_property(self.env, v, key.as_ptr(), &mut next) }, "read a property")?;
            v = next;
        }
        Ok(v)
    }

    unsafe fn caught(&self) -> Thrown {
        let mut e = ptr::null_mut();
        unsafe { sys::napi_get_and_clear_last_exception(self.env, &mut e) };
        let message = unsafe { prop_text(self.env, e, c"message") }.unwrap_or_else(|| unsafe { text_of(self.env, e) });
        let code = unsafe { arg_of(self.env, prop(self.env, e, c"code"), 0) }.ok().filter(|c| !c.is_nullish()).map(|c| c.to_json());
        let mut r = ptr::null_mut();
        unsafe { sys::napi_create_reference(self.env, e, 1, &mut r) };
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
    unsafe { sys::napi_typeof(env, v, &mut t) };
    if t == sys::ValueType::napi_object || t == sys::ValueType::napi_function {
        unsafe { sys::napi_get_named_property(env, v, k.as_ptr(), &mut out) };
    }
    if out.is_null() {
        unsafe { sys::napi_get_undefined(env, &mut out) };
    }
    out
}

unsafe fn prop_text(env: sys::napi_env, v: sys::napi_value, k: &std::ffi::CStr) -> Option<String> {
    match unsafe { arg_of(env, prop(env, v, k), 0) }.ok()? {
        Arg::Text(s) => Some(s),
        _ => None,
    }
}

unsafe fn text_of(env: sys::napi_env, v: sys::napi_value) -> String {
    let mut s = ptr::null_mut();
    unsafe { sys::napi_coerce_to_string(env, v, &mut s) };
    unsafe { String::from_napi_value(env, s) }.unwrap_or_default()
}

impl peer::Calls for SyncCalls {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> peer::Result<Arg> {
        unsafe {
            let target = self.at(path)?;
            let key = std::ffi::CString::new(method).unwrap();
            let mut t = 0;
            check(sys::napi_typeof(self.env, target, &mut t), "tell a value's type")?;
            if t != sys::ValueType::napi_object && t != sys::ValueType::napi_function {
                return Err(Thrown::coded(
                    format!("Cannot read properties of {} (reading '{method}')", if t == sys::ValueType::napi_null { "null" } else { "undefined" }),
                    "TypeError",
                ));
            }
            let mut f = ptr::null_mut();
            check(sys::napi_get_named_property(self.env, target, key.as_ptr(), &mut f), "read a method")?;
            check(sys::napi_typeof(self.env, f, &mut t), "tell a method")?;
            if t != sys::ValueType::napi_function {
                return Err(Thrown::new(format!("{path}.{method} is not a function")));
            }
            let mut argv = Vec::with_capacity(args.len());
            for a in &args {
                argv.push(arg_to_js(self.env, a).map_err(|e| Thrown::new(e.reason.clone()))?);
            }
            let mut out = ptr::null_mut();
            let status = sys::napi_call_function(self.env, target, f, argv.len(), argv.as_ptr(), &mut out);
            if status == sys::Status::napi_pending_exception {
                return Err(self.caught());
            }
            check(status, "call a method")?;
            arg_of(self.env, out, 0).map_err(|e| Thrown::new(e.reason.clone()))
        }
    }

    fn get(&mut self, path: &str) -> peer::Result<Arg> {
        unsafe {
            let v = self.at(path)?;
            arg_of(self.env, v, 0).map_err(|e| Thrown::new(e.reason.clone()))
        }
    }

    fn kind(&mut self, path: &str) -> peer::Result<String> {
        unsafe {
            let v = match self.at(path) {
                Ok(v) => v,
                // Reading through undefined or null is undefined here, as
                // optional chaining reads it.
                Err(_) => return Ok("undefined".into()),
            };
            let mut t = 0;
            check(sys::napi_typeof(self.env, v, &mut t), "tell a value's type")?;
            use sys::ValueType::*;
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
            unsafe {
                sys::napi_get_reference_value(env.raw(), r, &mut v);
                sys::napi_throw(env.raw(), v);
            }
            out = Some(Error::new(Status::PendingException, t.message.clone()));
        }
        unsafe { sys::napi_delete_reference(env.raw(), r) };
    }
    out.unwrap_or_else(|| {
        unsafe {
            let mut msg = ptr::null_mut();
            sys::napi_create_string_utf8(env.raw(), t.message.as_ptr().cast(), t.message.len() as isize, &mut msg);
            let mut err = ptr::null_mut();
            sys::napi_create_error(env.raw(), ptr::null_mut(), msg, &mut err);
            if let Some(c) = &t.code {
                if let Ok(code) = crate::js::to_js(env.raw(), c, 0) {
                    sys::napi_set_named_property(env.raw(), err, c"code".as_ptr(), code);
                }
            }
            sys::napi_throw(env.raw(), err);
        }
        Error::new(Status::PendingException, t.message)
    })
}

/// The values kept for the call under way let go, once it's done.
pub fn settle(env: &Env) {
    for (_, r) in THROWN.with(|k| std::mem::take(&mut *k.borrow_mut())) {
        unsafe { sys::napi_delete_reference(env.raw(), r) };
    }
}
