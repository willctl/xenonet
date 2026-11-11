//! The person's intents (docs/design/goals.md): work asked for at one of
//! their devices, worked on the machine it names under one of their
//! delegations. Rust-only, with no JS twin.
//!
//! - `GET /api/intents`: the intents this machine keeps, and what a new one
//!   may name (machines, delegations, projects). Yours, at the window or
//!   your terminal.
//! - `POST /api/intents` (`{ goal, target, project, app?, delegation?,
//!   hours? }`) and `POST /api/intents/<id>/cancel`: the window's alone,
//!   where your device signs them (rules.rs); anywhere else they'd need
//!   your presence, which XenoNet asks for only in its window.
//! - `POST /api/intents/<id>/note` (`{ text }`) and `.../done` (`{ outcome,
//!   failed? }`): from the session working it (x-xenonet-agent, checked
//!   against its process) or an actor whose grant names it (its token),
//!   decided by the engine. What the xenonet MCP tools intent_note and
//!   intent_done call.

use crate::cores::{CoreError, Reporter};
use crate::ctx::{fail, pending_reply, Answer, Ctx};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use xenonet_api::bearer::bearer_of;

const BODY: usize = 16 << 10;

fn reply(r: Result<Value, CoreError>) -> Reply {
    match r {
        Ok(v) => Reply::json(200, v),
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) => Reply::coded(e.status.unwrap_or(400), if e.status == Some(403) { "refused" } else { "intent" }, e.message),
    }
}

/// The core's answer, on a blocking thread: making one signs with your
/// device, which may wait on Touch ID.
async fn blocking(f: impl FnOnce() -> Result<Value, CoreError> + Send + 'static) -> Reply {
    match tokio::task::spawn_blocking(f).await {
        Ok(r) => reply(r),
        Err(e) => Reply::error(500, e.to_string()),
    }
}

/// `/api/intents…`, or None for any other path.
pub async fn route(ctx: &Ctx, req: &mut Req) -> Option<Answer> {
    let path = req.path.clone();
    let rest = if path == "/api/intents" { "" } else { path.strip_prefix("/api/intents/")? };
    let intents = ctx.cores.intents.clone();
    let at_window = req.by.window;
    let reply = match (req.method.as_str(), rest.split_once('/')) {
        ("GET", None) if rest.is_empty() => blocking(move || intents.list()).await,
        ("POST", None) if rest.is_empty() => match req.json(BODY).await {
            Ok(body) => blocking(move || intents.make(body, at_window)).await,
            Err(e) => Reply::error(e.status, e.message),
        },
        ("POST", Some((id, "cancel"))) => {
            let id = id.to_string();
            blocking(move || intents.cancel(&id, at_window)).await
        }
        ("POST", Some((id, step @ ("note" | "done")))) => {
            let by = match reporter(ctx, req).await {
                Ok(by) => by,
                Err(r) => return Some(Ok(r)),
            };
            let body = match req.json(BODY).await {
                Ok(b) => b,
                Err(e) => return Some(Ok(Reply::error(e.status, e.message))),
            };
            let (id, step) = (id.to_string(), step.to_string());
            blocking(move || intents.report(by, &id, &step, body)).await
        }
        _ => Reply::error(404, "not found"),
    };
    Some(Ok(reply))
}

/// Who reports: an actor of this machine by its token, or the session in
/// x-xenonet-agent as rules.rs checked it against its process.
async fn reporter(ctx: &Ctx, req: &Req) -> Result<Reporter, Reply> {
    if req.header("authorization").is_some() {
        let Some(caller) = ctx.caller.as_ref() else { return Err(fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &json!({}))) };
        let auth = caller.authenticate(req, false).await.map_err(|r| fail(r.status, &r.code, r.error, &r.headers, &r.extra))?;
        caller.check(req, &auth, Value::Null).await.map_err(|r| fail(r.status, &r.code, r.error, &[], &json!({})))?;
        let token = req.header("authorization").and_then(bearer_of).map(str::to_string);
        return token.map(|token| Reporter::Actor { token }).ok_or_else(|| Reply::coded(401, "no-token", "this route takes an actor's token, as Authorization: Bearer <token>"));
    }
    let Some((id, verdict)) = req.by.session.clone() else {
        return Err(Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id>)"));
    };
    Ok(Reporter::Session { id, attestation: crate::routes::rules::attestation_of(&verdict) })
}
