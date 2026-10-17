//! Delivery (core/delivery/): its policy (the pause switch, actors' scopes,
//! consent, the audit log) and the router that puts messages into live
//! sessions.

use super::{pending, Pending, R};
use serde_json::Value;

const POLICY: &str = "core/delivery/policy.js";
const ROUTER: &str = "core/delivery/router.js";

pub trait Policy: Send + Sync {
    fn paused(&self) -> R<bool> {
        pending(POLICY)
    }
    fn grants(&self) -> R<Value> {
        pending(POLICY)
    }
    /// `policy.data.consent`.
    fn consent(&self) -> R<Value> {
        pending(POLICY)
    }
    fn read_audit(&self, _n: f64) -> R<Value> {
        pending(POLICY)
    }
    fn set_paused(&self, _paused: bool) -> R<Value> {
        pending(POLICY)
    }
    fn grant(&self, _actor: &str, _scope: &str, _ttl_ms: f64) -> R<Value> {
        pending(POLICY)
    }
    fn revoke(&self, _actor: &str) -> R<Value> {
        pending(POLICY)
    }
    /// `policy.allow(session, { remote, ttlMs })`.
    fn allow(&self, _session: &str, _remote: bool, _ttl_ms: f64) -> R<Value> {
        pending(POLICY)
    }
    fn disallow(&self, _session: &str) -> R<()> {
        pending(POLICY)
    }
}

pub trait Router: Send + Sync {
    /// `router.usage(app)`: a provider's plan as the wake budget reads it.
    fn usage(&self, _app: &str) -> R<Value> {
        pending(ROUTER)
    }
    /// `router.list(n)`: the latest deliveries.
    fn list(&self, _n: usize) -> R<Value> {
        pending(ROUTER)
    }
}

impl Policy for Pending {}
impl Router for Pending {}
