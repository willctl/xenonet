//! Sessions XenoNet starts, and its runs (docs/guide/network.md): Rust-only,
//! with no JS twin, as the network is.
//!
//! - `POST /api/start`: a new session here or on another machine of the
//!   network (`{ machine?, folder?, app?, mode?, prompt }`). From you
//!   (the window's key, or your terminal), from a session (x-xenonet-agent,
//!   checked against its process), or from an actor on this machine with
//!   its token, whose right to start (the spawn-sessions capability) the
//!   engine decides. Never on the actor gateway, so nothing outside this
//!   machine reaches it but through the network. Whether this machine or
//!   that one takes it is its start setting's, in the network's chain.
//! - `GET`, `POST` and `DELETE /api/runs/sign-in`: whether XenoNet's runs
//!   have a Claude Code sign-in, a token from `claude setup-token` for them
//!   (kept in your credential store, never given back), and forgetting it.
//!   Yours alone.
//! - `POST /api/runs/permission`: a run's permission prompt, from the
//!   xenonet MCP server's `permission` tool (x-xenonet-agent): held until
//!   you answer, and refused for a session XenoNet didn't start.
//! - `GET /api/runs/asks`, `POST /api/runs/answer`: the prompts waiting,
//!   here and from other machines, and an answer: yours from the window (an
//!   Allow asks you once, a Deny not at all), or an actor's on this
//!   machine with its token and the answer-prompts capability (a Deny, or
//!   an Allow inside its grant's allow scope).
//! - `GET /api/runs/forks`: the copies of sessions XenoNet forked to answer
//!   a message while the session was open elsewhere, each with the session
//!   it came from, which the console links them to.

use crate::ctx::{fail, pending_reply, Answer, Ctx};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_api::bearer::bearer_of;

/// Why a call as you is refused away from the window: it's signed by your
/// device, which asks you (Touch ID on a Mac), so it's done from
/// XenoNet's window. Every machine that takes it checks the signature.
pub const AS_YOU: &str = "as you, this runs only from XenoNet's window, where your device signs it";

const BODY: usize = 64 << 10;

/// `/api/start` and `/api/runs/…`, or None for any other path.
pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Option<Answer> {
    // The copies XenoNet forked, each with its session: for the console.
    if req.method == "GET" && req.path == "/api/runs/forks" {
        return Some(Ok(match ctx.cores.engine.forks().await {
            Ok(v) => Reply::json(200, v),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(500, e.message),
        }));
    }
    // The sign-in XenoNet's runs use: yours alone, from the window or your
    // terminal, never an agent's or an actor's. The token goes in and is
    // never given back.
    if req.path == "/api/runs/sign-in" {
        return Some(Ok(sign_in(ctx, req).await));
    }
    match (req.method.as_str(), req.path.as_str()) {
        ("POST", "/api/runs/permission") => return Some(Ok(permission(ctx, req).await)),
        ("GET", "/api/runs/asks") => return Some(Ok(asks(ctx, req).await)),
        ("POST", "/api/runs/answer") => return Some(Ok(answer(ctx, req).await)),
        _ => {}
    }
    if req.path != "/api/start" {
        return Some(Ok(Reply::error(404, "not found")));
    }
    if req.method != "POST" {
        return Some(Ok(Reply::error(404, "not found")));
    }
    Some(Ok(start(ctx, req).await))
}

/// Who asks for a start, as this machine checked them: { kind, id, title },
/// and whether it came from the window (so a start as you may be signed by
/// your device there). Or the refusal.
async fn who(ctx: &Ctx, req: &mut Req) -> Result<(Value, bool), Reply> {
    let cores = &ctx.cores;
    if req.header("authorization").is_some() {
        let Some(caller) = ctx.caller.as_ref() else { return Err(fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &json!({}))) };
        let auth = caller.authenticate(req, false).await.map_err(|r| fail(r.status, &r.code, r.error, &r.headers, &r.extra))?;
        caller.check(req, &auth, Value::Null).await.map_err(|r| fail(r.status, &r.code, r.error, &[], &json!({})))?;
        let id = auth["id"].as_str().unwrap_or("").to_string();
        let name = cores.actors.as_ref().and_then(|a| a.info(&id)).and_then(|i| i.get("name").and_then(Value::as_str).map(str::to_string));
        // The engine decides the start with the token, for the run it
        // starts, and keeps it with nothing.
        let token = req.header("authorization").and_then(bearer_of).map(str::to_string);
        return Ok((json!({ "kind": "actor", "id": id, "title": name, "token": token }), false));
    }
    // A session, as rules.rs checked it against its process.
    if let Some(me) = req.header("x-xenonet-agent").map(str::to_string) {
        let Some((me, _)) = req.by.session.clone().filter(|(id, _)| *id == me && cores.apps.is_session_id(id)) else {
            return Err(Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id> or codex:<thread id>)"));
        };
        let title = cores.engine.view(&me).await.ok().flatten().and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string));
        return Ok((json!({ "kind": "session", "id": me, "title": title }), false));
    }
    // It's yours (rules.rs). As you, a start is signed by your device,
    // which asks you: from the window alone. Never prompted for from a
    // terminal or an agent.
    if !req.by.window {
        return Err(Reply::coded(403, "weak", AS_YOU));
    }
    Ok((json!({ "kind": "you", "id": xenonet_model::ids::YOU, "title": "you" }), true))
}

fn core_reply(r: Result<Value, crate::cores::CoreError>) -> Reply {
    match r {
        Ok(v) => Reply::json(200, v),
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) => Reply::coded(e.status.unwrap_or(400), if e.status == Some(403) { "not-a-run" } else { "refused" }, e.message),
    }
}

/// A run's permission prompt, from the xenonet MCP server's `permission`
/// tool in its session: x-xenonet-agent, checked against its process.
/// Held until you answer.
async fn permission(ctx: &Ctx, req: &mut Req) -> Reply {
    if req.header("authorization").is_some() {
        return Reply::coded(403, "not-a-run", "an actor's own prompts aren't asked here");
    }
    // The session, as rules.rs checked it against its process.
    let Some((me, _)) = req.by.session.clone() else {
        return Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id>)");
    };
    let body = match req.json(BODY).await {
        Ok(b) => b,
        Err(e) => return Reply::error(e.status, e.message),
    };
    let title = ctx.cores.engine.view(&me).await.ok().flatten().and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string));
    core_reply(ctx.cores.engine.permission(&me, title, body).await)
}

/// An actor of this machine, by its token, allowed to answer prompts (the
/// answer-prompts capability): its token, or the refusal.
async fn answering_actor(ctx: &Ctx, req: &Req) -> Result<String, Reply> {
    let Some(caller) = ctx.caller.as_ref() else { return Err(fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &json!({}))) };
    let auth = caller.authenticate(req, false).await.map_err(|r| fail(r.status, &r.code, r.error, &r.headers, &r.extra))?;
    caller.check(req, &auth, Value::Null).await.map_err(|r| fail(r.status, &r.code, r.error, &[], &json!({})))?;
    if !auth["capabilities"].as_array().is_some_and(|c| c.iter().any(|w| w == "answer-prompts")) {
        return Err(Reply::coded(403, "no-right", "answering prompts needs the answer-prompts capability, which this actor wasn't allowed"));
    }
    req.header("authorization")
        .and_then(bearer_of)
        .map(str::to_string)
        .ok_or_else(|| Reply::coded(401, "no-token", "this route takes an actor's token, as Authorization: Bearer <token>"))
}

/// The permission prompts waiting, for the window, or an actor that
/// answers them.
async fn asks(ctx: &Ctx, req: &Req) -> Reply {
    // Else it's the window's (rules.rs).
    if req.header("authorization").is_some() {
        if let Err(r) = answering_actor(ctx, req).await {
            return r;
        }
    }
    core_reply(ctx.cores.engine.asks().await)
}

/// An Allow or Deny: yours from the window, where an Allow asks for your
/// presence (once) and a Deny for nothing; or an actor's, by its token.
/// Decided here for an ask here, or sent to the machine whose ask it is,
/// which decides it (the engine does both).
async fn answer(ctx: &Ctx, req: &mut Req) -> Reply {
    // Else it's the window's (rules.rs).
    let token = if req.header("authorization").is_some() {
        match answering_actor(ctx, req).await {
            Ok(t) => Some(t),
            Err(r) => return r,
        }
    } else {
        None
    };
    let body = match req.json(4 << 10).await {
        Ok(b) => b,
        Err(e) => return Reply::error(e.status, e.message),
    };
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (id, decision) = (text("ask"), text("decision"));
    let answered = match &token {
        Some(t) => ctx.cores.engine.answer_as(t, &id, &decision).await,
        None => ctx.cores.engine.answer(&id, &decision).await,
    };
    let r = match answered {
        Ok(r) => r,
        Err(e) => return core_reply(Err(e)),
    };
    Reply::json(200, json!({ "ok": true, "ask": id, "decision": decision, "host": r.get("host").cloned().unwrap_or(Value::Null) }))
}

async fn sign_in(ctx: &Ctx, req: &mut Req) -> Reply {
    let (action, token) = match req.method.as_str() {
        "GET" => ("status", String::new()),
        "DELETE" => ("forget", String::new()),
        "POST" => match req.json(4 << 10).await {
            Ok(b) => ("sign-in", b.get("token").and_then(Value::as_str).unwrap_or("").to_string()),
            Err(e) => return Reply::error(e.status, e.message),
        },
        _ => return Reply::error(404, "not found"),
    };
    match ctx.cores.engine.runs_sign_in(action, &token).await {
        Ok(v) => Reply::json(200, v),
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) => Reply::coded(e.status.unwrap_or(400), "refused", e.message),
    }
}

async fn start(ctx: &Ctx, req: &mut Req) -> Reply {
    let (by, strong) = match who(ctx, req).await {
        Ok(w) => w,
        Err(r) => return r,
    };
    let body = match req.json(BODY).await {
        Ok(b) => b,
        Err(e) => return Reply::error(e.status, e.message),
    };
    match ctx.cores.engine.start(by, body, strong).await {
        Ok(v) => Reply::json(200, v),
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) => Reply::coded(e.status.unwrap_or(400), if e.status == Some(409) { "refused" } else { "bad-request" }, e.message),
    }
}
