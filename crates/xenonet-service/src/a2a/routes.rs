//! The window's side of A2A, `/api/a2a`, the owner's alone: GET
//! says whether A2A agents may reach XenoNet, the card's address and the
//! agents that called; POST turns it on or off (`{ on }`), or sets an
//! agent's default target (`{ actor, target }`, target null to clear it).

use crate::ctx::{fail, Ctx};
use crate::http::{Reply, Req};
use serde_json::Value;
use std::sync::Arc;

const SMALL_BODY: usize = 1 << 10;

pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Reply {
    let Some(a2a) = ctx.a2a.get().cloned() else { return fail(503, "off", "A2A isn't here: actors are off", &[], &Value::Null) };
    if let Some(r) = ctx.from_you(req).await {
        return fail(403, "yours", r, &[], &Value::Null);
    }
    let address = ctx.gateway.get().and_then(|g| g.url().as_str().map(str::to_string));
    if req.method == "GET" {
        return Reply::json(200, a2a.status(address.as_deref()));
    }
    let body = match req.json(SMALL_BODY).await {
        Ok(b) if b.is_object() => b,
        Ok(_) => return fail(400, "bad-request", "send an object: { on } or { actor, target }", &[], &Value::Null),
        Err(e) => return fail(e.status, "bad-request", e.message, &[], &Value::Null),
    };
    if let Some(on) = body.get("on") {
        let Some(on) = on.as_bool() else { return fail(400, "bad-request", "on is true or false", &[], &Value::Null) };
        if let Err(e) = a2a.set_enabled(on) {
            return fail(500, "a2a", e, &[], &Value::Null);
        }
    }
    if let Some(actor) = body.get("actor") {
        let Some(slug) = actor.as_str().map(|s| s.trim_start_matches("actor:")).filter(|s| !s.is_empty() && s.len() <= 64) else {
            return fail(400, "bad-request", "actor is an actor's slug", &[], &Value::Null);
        };
        let target = match body.get("target") {
            None | Some(Value::Null) => None,
            Some(Value::String(t)) => Some(t.as_str()),
            Some(_) => return fail(400, "bad-request", "target is a session's id, repo:<name>, a selector, or null", &[], &Value::Null),
        };
        if let Err(e) = a2a.set_default_target(&format!("actor:{slug}"), target) {
            return fail(400, "bad-request", e, &[], &Value::Null);
        }
    }
    Reply::json(200, a2a.status(address.as_deref()))
}
