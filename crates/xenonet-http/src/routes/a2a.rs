//! The window's side of A2A, `/api/a2a`, the owner's alone (rules.rs): GET
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

/// The most of a delegation's body: its text and its names.
const DELEGATE_BODY: usize = 40 << 10;

/// Outbound actors, the agents XenoNet calls (rules.rs says who may call
/// which): `GET /api/a2a/outbound` lists them for the window; `POST
/// /api/a2a/outbound/card` fetches a card for the window to show; `POST
/// /api/a2a/outbound` registers one, with the person's presence; `DELETE
/// /api/a2a/outbound/<slug>` removes one; and `POST /api/a2a/delegate`
/// is a session handing one work (`{ actor, text?, task? }`). No answer
/// holds a bearer.
pub async fn outbound(ctx: &Arc<Ctx>, req: &mut Req) -> Reply {
    let Some(out) = ctx.a2a_out.get().cloned() else { return fail(503, "off", "A2A out isn't here: actors are off", &[], &Value::Null) };
    let refused = |e: String| fail(400, "refused", e, &[], &Value::Null);
    let path = req.path.clone();
    match (req.method.as_str(), path.as_str()) {
        ("GET", "/api/a2a/outbound") => Reply::json(200, out.status()),
        ("POST", "/api/a2a/outbound/card") => {
            let body = match req.json(SMALL_BODY).await {
                Ok(b) => b,
                Err(e) => return fail(e.status, "bad-request", e.message, &[], &Value::Null),
            };
            let Some(card) = body.get("card").and_then(Value::as_str) else { return fail(400, "bad-request", "send { card }: the agent's card or its address", &[], &Value::Null) };
            match out.preview(card, body.get("privateHost").and_then(Value::as_str)).await {
                Ok(v) => Reply::json(200, v),
                Err(e) => refused(e),
            }
        }
        ("POST", "/api/a2a/outbound") => {
            let body = match req.json(4 << 10).await {
                Ok(b) if b.is_object() => b,
                Ok(_) => return fail(400, "bad-request", "send an object", &[], &Value::Null),
                Err(e) => return fail(e.status, "bad-request", e.message, &[], &Value::Null),
            };
            match out.register(&body).await {
                Ok(v) => Reply::json(200, v),
                Err(e) => refused(e),
            }
        }
        ("DELETE", p) if p.starts_with("/api/a2a/outbound/") => match out.remove(&p["/api/a2a/outbound/".len()..]).await {
            Ok(true) => Reply::json(200, out.status()),
            Ok(false) => Reply::error(404, "there's no outbound actor by that name"),
            Err(e) => refused(e),
        },
        ("POST", "/api/a2a/delegate") => {
            // The session, as rules.rs checked it against its process.
            let Some((me, verdict)) = req.by.session.clone() else {
                return Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id>)");
            };
            let body = match req.json(DELEGATE_BODY).await {
                Ok(b) => b,
                Err(e) => return fail(e.status, "bad-request", e.message, &[], &Value::Null),
            };
            let Some(actor) = body.get("actor").and_then(Value::as_str) else {
                return fail(400, "bad-request", "say which outbound actor: { actor: \"actor:<slug>\" }", &[], &Value::Null);
            };
            let project = ctx.cores.engine.view(&me).await.ok().flatten().and_then(|v| v.get("project").and_then(Value::as_str).map(str::to_string));
            let by = crate::routes::rules::attestation_of(&verdict);
            let (text, task) = (body.get("text").and_then(Value::as_str), body.get("task").and_then(Value::as_str));
            match out.delegate(&me, by, project, actor, text, task).await {
                Ok(v) => Reply::json(200, v),
                Err(e) => fail(403, "refused", e, &[], &Value::Null),
            }
        }
        _ => Reply::error(404, "not found"),
    }
}
