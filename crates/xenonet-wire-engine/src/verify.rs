//! The process checks (core/engine/verify.js over
//! crates/xenonet-engine/src/verify.rs): the process at the other end of a
//! request's TCP connection, found by its ports as verify.js finds it, and
//! the ones above it, checked against what the request says it is. The
//! lookups block on commands, so each check runs on a blocking thread.

use crate::js::{json, no_method, now_ms, number, text};
use crate::procs::{self, System};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::Shared;
use xenonet_engine::verify::Verifier;
use xenonet_service::cores::Verify;
use xenonet_service::http::Peer;

/// What a check reads: procs, the clock, and the process on the other end
/// of this request's socket.
pub struct VerifyCalls<'a> {
    pub sys: &'a System,
    pub peer: Option<Peer>,
}

impl Calls for VerifyCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        match (path, method) {
            ("procs", m) => procs::call(self.sys, m, &args),
            ("", "now") => Ok(Arg::Num(now_ms())),
            ("", "peerOf") => Ok(match self.peer {
                Some(p) => crate::js::arg(&procs::peer_of(self.sys, p.local.port(), p.remote.port())),
                None => Arg::Null,
            }),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path.strip_prefix("procs") {
            Some(rest) => procs::get(self.sys, rest.trim_start_matches('.')),
            None => Arg::Undef,
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path.strip_prefix("procs") {
            Some(rest) => procs::kind(rest.trim_start_matches('.')).into(),
            None if matches!(path, "now" | "peerOf") => "function".into(),
            None => "undefined".into(),
        })
    }
    fn now(&mut self) -> f64 {
        now_ms()
    }
}

/// The checks, as the routes call them.
pub struct Checks {
    pub verifier: Arc<Verifier>,
    pub sys: Arc<System>,
}

impl Checks {
    pub fn new(store: Shared, sys: Arc<System>) -> Checks {
        // The service is its own process, so no app is its parent: the
        // Desktop that connects says which process is home.
        Checks { verifier: Arc::new(Verifier::new(store, Arg::Null)), sys }
    }

    async fn run(&self, peer: Option<Peer>, f: impl FnOnce(&Verifier, &mut VerifyCalls) -> Result<Arg> + Send + 'static) -> Value {
        let (v, sys) = (self.verifier.clone(), self.sys.clone());
        let out = tokio::task::spawn_blocking(move || {
            let mut c = VerifyCalls { sys: &sys, peer };
            f(&v, &mut c)
        })
        .await;
        match out {
            Ok(Ok(a)) => json(&a),
            // A check that couldn't run refuses, never lets through.
            Ok(Err(t)) => json!({ "ok": false, "checked": true, "reason": t.message }),
            Err(e) => json!({ "ok": false, "checked": true, "reason": e.to_string() }),
        }
    }

    /// `verify.lineage(pid)`: a process and the ones above it, as pids.
    pub fn lineage(&self, pid: &Arg) -> Arg {
        let mut c = VerifyCalls { sys: &self.sys, peer: None };
        self.verifier.lineage(&mut c, pid).unwrap_or(Arg::Null)
    }
}

#[async_trait]
impl Verify for Checks {
    async fn check(&self, peer: Peer, id: &str, hook: bool) -> Value {
        let id = Arg::text(id);
        self.run(Some(peer), move |v, c| v.check(c, &id, hook)).await
    }
    async fn person(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.person(c)).await
    }
    async fn window(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.window(c)).await
    }
    async fn desktop(&self, peer: Option<Peer>) -> Value {
        let connected = peer.is_some();
        self.run(peer, move |v, c| v.desktop(c, connected)).await
    }
    async fn actor(&self, peer: Peer, slug: &str, product: Value, pid: Value) -> Value {
        let who = crate::js::obj(vec![("slug", Arg::text(slug)), ("product", crate::js::arg(&product))]);
        let pid = crate::js::arg(&pid);
        self.run(Some(peer), move |v, c| v.actor(c, &who, &pid)).await
    }
    async fn register(&self, peer: Peer, product: Value, pid: Value) -> Value {
        let (product, pid) = (crate::js::arg(&product), crate::js::arg(&pid));
        self.run(Some(peer), move |v, c| v.register(c, &product, &pid)).await
    }
    async fn asker(&self, peer: Peer) -> Value {
        self.run(Some(peer), |v, c| v.asker(c)).await
    }
    async fn peer(&self, peer: Peer) -> String {
        let pid = self.run(Some(peer), |v, c| v.peer(c)).await;
        match pid {
            Value::Number(_) => text(&crate::js::arg(&pid)),
            _ => "?".into(),
        }
    }
    fn state(&self) -> Value {
        let c = self.verifier.counts();
        json!({
            "checked": number(c.checked),
            "refused": number(c.refused),
            "unchecked": number(c.unchecked),
            "unseen": number(c.unseen),
            "lastRefusal": c.last_refusal.as_ref().map_or(Value::Null, json),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks() -> Checks {
        Checks::new(Shared::new(xenonet_engine::store::EngineStore::open(":memory:", 0.0).unwrap()), System::new())
    }

    #[tokio::test]
    async fn a_request_with_no_process_to_see_passes_unchecked_or_is_refused_as_unseen() {
        let c = checks();
        // A loopback address with no connection behind it: nothing owns it.
        let peer = Peer { remote: "127.0.0.1:9".parse().unwrap(), local: "127.0.0.1:1".parse().unwrap() };
        let v = c.check(peer, "claude:0190d3e0-0000-7000-8000-000000000001", false).await;
        // No process on the other end: let through unchecked, as the JS did.
        assert_eq!(v["ok"], true, "{v}");
        let you = c.person(peer).await;
        if c.sys.procs.supported {
            assert_eq!((you["ok"].clone(), you["unseen"].clone()), (json!(false), json!(true)), "{you}");
        }
        assert_eq!(c.peer(peer).await, "?");
        assert_eq!(c.state()["unchecked"], json!(if c.sys.procs.supported { 1 } else { 2 }));
        // An app XenoNet doesn't check passes.
        assert_eq!(c.check(peer, "nobody:x", false).await["ok"], true);
    }
}
