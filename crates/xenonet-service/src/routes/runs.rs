//! Sessions XenoNet starts, and its runs (docs/guide/network.md): Rust-only,
//! with no JS twin, as the network is.
//!
//! - `POST /api/start`: a new session here or on another machine of the
//!   network (`{ machine?, folder?, app?, mode?, prompt }`). From you
//!   (the window's key, or your terminal), from a session (x-xenonet-agent,
//!   checked against its process), or from an actor on this machine with
//!   its token and the spawn-sessions capability. Never on the actor
//!   gateway, so nothing outside this machine reaches it but through the
//!   network. Whether this machine or that one takes it is its start
//!   setting's (core/delivery/reach.js).
//! - `GET`, `POST` and `DELETE /api/runs/sign-in`: whether XenoNet's runs
//!   have a Claude Code sign-in, a token from `claude setup-token` for them
//!   (kept in your credential store, never given back), and forgetting it.
//!   Yours alone.
//! - `POST /api/runs/permission`: a run's permission prompt, from the
//!   xenonet MCP server's `permission` tool (x-xenonet-agent): held until
//!   you answer, and refused for a session XenoNet didn't start.
//! - `GET /api/runs/asks`, `POST /api/runs/answer`: the prompts waiting,
//!   here and from other machines, and your Allow or Deny, from the window.
//! - `GET /api/runs/forks`: the copies of sessions XenoNet forked to answer
//!   a message while the session was open elsewhere, each with the session
//!   it came from, which the console links them to.

use crate::cores::ok;
use crate::ctx::{fail, pending_reply, Answer, Ctx};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_engine::engine::is_agent_id;
use xenonet_engine::sql::Arg;

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
/// and whether you were checked strongly. Or the refusal.
async fn who(ctx: &Ctx, req: &mut Req) -> Result<(Value, bool), Reply> {
    let cores = &ctx.cores;
    if req.header("authorization").is_some() {
        let Some(caller) = ctx.caller.as_ref() else { return Err(fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &json!({}))) };
        let auth = caller.authenticate(req, false).await.map_err(|r| fail(r.status, &r.code, r.error, &r.headers, &r.extra))?;
        caller.check(req, &auth, Value::Null).await.map_err(|r| fail(r.status, &r.code, r.error, &[], &json!({})))?;
        let id = auth["id"].as_str().unwrap_or("").to_string();
        if !cores.actors.as_ref().is_some_and(|a| a.can(&id, "spawn-sessions")) {
            return Err(fail(403, "not-allowed", "starting a session needs the spawn-sessions capability, which this actor wasn't allowed", &[], &json!({})));
        }
        let name = cores.actors.as_ref().and_then(|a| a.info(&id)).and_then(|i| i.get("name").and_then(Value::as_str).map(str::to_string));
        return Ok((json!({ "kind": "actor", "id": id, "title": name }), false));
    }
    if let Some(me) = req.header("x-xenonet-agent").map(str::to_string) {
        if !is_agent_id(&Arg::text(me.as_str())) {
            return Err(Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id> or codex:<thread id>)"));
        }
        let verdict = cores.verify.check(req.peer, &me, false).await;
        if !ok(&verdict) {
            return Err(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
        }
        let title = cores.engine.view(&me).await.ok().flatten().and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string));
        return Ok((json!({ "kind": "session", "id": me, "title": title }), false));
    }
    if let Some(why) = ctx.from_you(req).await {
        return Err(Reply::coded(403, "yours", why));
    }
    // Strong from the window's key, or from a terminal where process checks
    // tell you from an agent: macOS.
    let strong = req.header("x-xenonet-window").is_some() || cfg!(target_os = "macos");
    Ok((json!({ "kind": "you", "id": xenonet_base::ids::YOU, "title": "you" }), strong))
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
    let Some(me) = req.header("x-xenonet-agent").filter(|m| is_agent_id(&Arg::text(*m))).map(str::to_string) else {
        return Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id>)");
    };
    let verdict = ctx.cores.verify.check(req.peer, &me, false).await;
    if !ok(&verdict) {
        return Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]);
    }
    let body = match req.json(BODY).await {
        Ok(b) => b,
        Err(e) => return Reply::error(e.status, e.message),
    };
    let title = ctx.cores.engine.view(&me).await.ok().flatten().and_then(|v| v.get("title").and_then(Value::as_str).map(str::to_string));
    core_reply(ctx.cores.engine.permission(&me, title, body).await)
}

/// The permission prompts waiting, for the window.
async fn asks(ctx: &Ctx, req: &Req) -> Reply {
    if let Some(why) = ctx.from_window(req).await {
        return Reply::coded(403, "yours", why);
    }
    core_reply(ctx.cores.engine.asks().await)
}

/// Your Allow or Deny, from the window: one here is answered at once; one
/// from another machine goes back there as an `operator` frame's `answer`,
/// which only a window host sends.
async fn answer(ctx: &Ctx, req: &mut Req) -> Reply {
    if let Some(why) = ctx.from_window(req).await {
        return Reply::coded(403, "yours", why);
    }
    let body = match req.json(4 << 10).await {
        Ok(b) => b,
        Err(e) => return Reply::error(e.status, e.message),
    };
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (id, decision) = (text("ask"), text("decision"));
    let r = match ctx.cores.engine.answer(&id, &decision).await {
        Ok(r) => r,
        Err(e) => return core_reply(Err(e)),
    };
    if let Some(host) = r.get("host").and_then(Value::as_str) {
        let Some(svc) = ctx.net.get() else { return Reply::coded(503, "no-network", "the network isn't available in this XenoNet") };
        if let Err(e) = svc.node().operator(host, "answer", json!({ "ask": id, "decision": decision }), "window", true) {
            return Reply::coded(e.status, e.code, e.message);
        }
    }
    Reply::json(200, json!({ "ok": true, "ask": id, "decision": decision, "host": r.get("host").cloned().unwrap_or(Value::Null) }))
}

async fn sign_in(ctx: &Ctx, req: &mut Req) -> Reply {
    if let Some(why) = ctx.from_you(req).await {
        return Reply::coded(403, "yours", why);
    }
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
