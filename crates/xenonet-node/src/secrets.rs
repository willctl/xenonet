//! xenonet-actors' secret store for the twin of core/secret-store.js: Rust
//! checks targets and secrets, says what to run and reads what it said;
//! the twin runs the store's program (`run`).

use crate::js::Jv;
use crate::machine::{start, Asker, Machine};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_actors::secret_store::{self, Command, MemorySecrets, Ran, SecretStore};
use xenonet_base::text::js_string;

struct Bridged<'a>(&'a Asker);

impl secret_store::Run for Bridged<'_> {
    fn run(&mut self, c: Command) -> std::result::Result<Ran, String> {
        let mut q = json!({ "op": "run", "file": c.file, "args": c.args });
        if let Some(i) = c.input {
            q["input"] = Value::String(i);
        }
        match self.0.ask(q) {
            Ok(r) => Ok(Ran {
                code: r.get("code").and_then(Value::as_f64).unwrap_or(f64::NAN),
                stdout: match r.get("stdout") {
                    None => "undefined".into(),
                    Some(v) => js_string(v),
                },
            }),
            Err(e) => Err(e.get("message").map(js_string).unwrap_or_default()),
        }
    }
}

#[napi(js_name = "secretPatterns")]
pub fn secret_patterns() -> Vec<&'static str> {
    vec![secret_store::TARGET, secret_store::SECRET]
}

#[napi(js_name = "isSecretTarget")]
pub fn is_secret_target(s: String) -> bool {
    secret_store::is_target(&s)
}

#[napi]
pub struct SecretStoreCore {
    inner: Arc<SecretStore>,
}

#[napi]
impl SecretStoreCore {
    /// `on_path`: where secret-tool is, asked only where it's the store.
    #[napi(constructor)]
    pub fn new(platform: String, on_path: Function<'_, (), Option<String>>) -> Result<Self> {
        let tool = if platform == "darwin" || platform == "win32" { None } else { on_path.call(())? };
        Ok(SecretStoreCore { inner: Arc::new(SecretStore::new(&platform, tool)) })
    }

    #[napi(getter)]
    pub fn kind(&self) -> &'static str {
        self.inner.kind()
    }

    #[napi]
    pub fn available(&self) -> bool {
        self.inner.available()
    }

    #[napi]
    pub fn has(&self, target: String) -> Machine {
        let s = self.inner.clone();
        start(move |a| Ok(Jv::Bool(s.has(&mut Bridged(a), &target)?)))
    }

    #[napi]
    pub fn get(&self, target: String) -> Machine {
        let s = self.inner.clone();
        start(move |a| Ok(Jv::Json(s.get(&mut Bridged(a), &target)?.map_or(Value::Null, Value::String))))
    }

    #[napi]
    pub fn set(&self, target: String, secret: String, comment: String) -> Machine {
        let s = self.inner.clone();
        start(move |a| Ok(Jv::Bool(s.set(&mut Bridged(a), &target, &secret, &comment)?)))
    }

    #[napi]
    pub fn remove(&self, target: String) -> Machine {
        let s = self.inner.clone();
        start(move |a| Ok(Jv::Bool(s.remove(&mut Bridged(a), &target)?)))
    }
}

#[napi]
pub struct MemorySecretsCore {
    inner: Mutex<MemorySecrets>,
}

#[napi]
// On the impl, not the methods: napi-derive copies a method's attributes
// onto the two-argument wrapper it generates for it, and an expectation
// there goes unfulfilled.
#[expect(clippy::unused_self, reason = "kind is a getter on the JS class, as it was in the JS, though it reads a constant")]
impl MemorySecretsCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        MemorySecretsCore { inner: Mutex::new(MemorySecrets::default()) }
    }

    #[napi(getter)]
    pub fn kind(&self) -> &'static str {
        MemorySecrets::KIND
    }

    #[napi]
    pub fn has(&self, target: String) -> bool {
        self.inner.lock().unwrap().has(&target)
    }

    #[napi]
    pub fn get(&self, target: String) -> Option<String> {
        self.inner.lock().unwrap().get(&target)
    }

    #[napi]
    pub fn set(&self, target: String, secret: String) -> Result<bool> {
        self.inner.lock().unwrap().set(&target, &secret).map_err(|e| Error::new(Status::GenericFailure, e))
    }

    #[napi]
    pub fn remove(&self, target: String) -> bool {
        self.inner.lock().unwrap().remove(&target)
    }
}
