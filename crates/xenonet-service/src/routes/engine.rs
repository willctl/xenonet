//! The engine's HTTP API (server/routes/engine.js), on XenoNet's own port
//! with its protections. Agents reach it through the MCP server, which says
//! who's calling in x-xenonet-agent; Codex's hooks post to /api/hook/codex
//! (through xenonet hook); the console reads /api/engine/state, the
//! xenonet command /api/engine/agents and /api/engine/who; and
//! /api/engine/connect previews, adds and removes XenoNet in Claude Code's
//! and Codex's settings. Every request in an agent's name is checked
//! against the process it comes from, and connecting agents only goes
//! through when no agent is asking.
//!
//! Registered actors call the same operations as `actor:<slug>`, with their
//! token, checked as their own routes are, and held to what you allowed
//! them: sending needs message-agents (60 messages an hour, 6 of them to a
//! repo or to all), claiming needs edit-repos. An actor's focus is its
//! status, an event on the feed, so each counts against its events.

use crate::actor_caller::{rates, retry_after, ActorCaller};
use crate::borrowed::{attest, platform, Sender};
use crate::cores::{ok, reason, Actors, Channels, Connect, CoreError, Engine, Verify};
use crate::ctx::{fail, pending_reply};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use xenonet_actors::tokens::token_in_url;
use xenonet_base::text::{js_number, truthy};
use xenonet_engine::delivery::selector::parse_selector;
use xenonet_engine::engine::is_agent_id;
use xenonet_engine::sql::Arg;

const ACTOR_BODY: usize = 16 << 10;

/// What the engine's routes are given (engineRoutes's arguments).
pub struct EngineRoutes<'a> {
    pub engine: Arc<dyn Engine>,
    pub connect: Arc<dyn Connect>,
    pub verify: Arc<dyn Verify>,
    pub caller: Option<&'a ActorCaller>,
    pub actors: Option<Arc<dyn Actors>>,
    pub channels: Option<Arc<dyn Channels>>,
    pub home: &'a str,
    pub now: f64,
}

/// A peers scope: repo, machine, or a selector.
fn scope_of(s: &Value) -> String {
    match s.as_str() {
        Some("machine") => "machine".into(),
        Some(t) if parse_selector(&Arg::text(t)).is_some() => t.to_string(),
        _ => "repo".into(),
    }
}

const OPS: [&str; 9] = ["whoami", "peers", "who-has", "send", "inbox", "claim", "release", "focus", "channel"];

/// The capability each operation needs from an actor.
fn needs(op: &str) -> Option<&'static str> {
    match op {
        "send" => Some("message-agents"),
        "claim" => Some("edit-repos"),
        _ => None,
    }
}

fn get(b: &Value, k: &str) -> Value {
    b.get(k).cloned().unwrap_or(Value::Null)
}

/// Like `{ a, b }` in JS: the keys whose value isn't undefined (absent).
fn pick(b: &Value, keys: &[(&str, &str)]) -> Value {
    let mut out = serde_json::Map::new();
    for (to, from) in keys {
        if let Some(v) = b.get(*from) {
            out.insert(to.to_string(), v.clone());
        }
    }
    Value::Object(out)
}

impl EngineRoutes<'_> {
    async fn op(&self, name: &str, me: &str, b: &Value, extra: Option<Value>) -> Result<Value, CoreError> {
        let e = &self.engine;
        match name {
            "whoami" => e.whoami(me).await,
            "peers" => e.peers(me, &scope_of(&get(b, "scope"))).await,
            "who-has" => e.who_has(Some(me), &js_string_or_empty(&get(b, "path"))).await,
            "send" => e.send(me, pick(b, &[("to", "to"), ("text", "text"), ("replyTo", "replyTo")]), extra).await,
            "inbox" => {
                let secs = js_number(b.get("waitSeconds")).min(120.0);
                let secs = if secs.is_nan() || secs <= 0.0 { 0.0 } else { secs };
                e.inbox(me, Duration::from_secs_f64(secs)).await
            }
            "claim" => e.claim(me, pick(b, &[("paths", "paths"), ("note", "note"), ("minutes", "minutes")])).await,
            "release" => {
                let paths = match b.get("paths") {
                    Some(Value::Array(a)) if !a.is_empty() => Value::Array(a.clone()),
                    _ => Value::Null,
                };
                e.release(me, paths).await
            }
            "focus" => e.set_focus(me, get(b, "text")).await,
            // A Claude Code session's channel waiting for messages.
            "channel" => {
                let Some(channels) = self.channels.as_ref().filter(|_| me.starts_with("claude:")) else {
                    return Ok(json!({ "items": [], "off": true }));
                };
                let secs = js_number(b.get("waitSeconds"));
                let secs = if secs.is_nan() || secs == 0.0 { 25.0 } else { secs }.min(50.0);
                let items = channels.poll(me, Duration::from_secs_f64(secs.max(0.0))).await?;
                Ok(json!({ "items": items }))
            }
            _ => Err(CoreError::new("not found")),
        }
    }

    // An operation called by an actor, with its token: who it is, checked;
    // what it may do; then the operation as actor:<slug>.
    async fn as_actor(&self, req: &mut Req, name: &str) -> Reply {
        let Some(caller) = self.caller else { return fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &json!({})) };
        let auth = match caller.authenticate(req, false).await {
            Ok(a) => a,
            Err(r) => return fail(r.status, &r.code, r.error, &r.headers, &r.extra),
        };
        let verdict = match caller.check(req, &auth, Value::Null).await {
            Ok(v) => v,
            Err(r) => return fail(r.status, &r.code, r.error, &[], &json!({})),
        };
        let id = auth["id"].as_str().unwrap_or("").to_string();
        let slug = auth["slug"].as_str().unwrap_or("").to_string();
        let body = match req.json(ACTOR_BODY).await {
            Ok(b) => b,
            Err(e) if e.status == 413 => return fail(413, "too-big", "that body is over the size this route takes", &[], &json!({})),
            Err(_) => return fail(400, "bad-request", "that body isn't JSON", &[], &json!({})),
        };
        if let Some(need) = needs(name) {
            if !self.actors.as_ref().is_some_and(|a| a.can(&id, need)) {
                return fail(403, "not-allowed", format!("{name} needs the {need} capability, which this actor wasn't allowed"), &[], &json!({}));
            }
        }
        if name == "send" {
            let wide = matches!(body.get("to").and_then(Value::as_str), Some("repo" | "project" | "all"));
            let mut wait = if wide { caller.take(&format!("a:{slug}|wide"), rates::WIDE, 1.0) } else { 0.0 };
            if wait == 0.0 {
                wait = caller.take(&format!("a:{slug}|messages"), rates::MESSAGES, 1.0);
            }
            if wait != 0.0 {
                let words = if wide { "an actor sends at most 6 messages an hour to a repo or to all" } else { "an actor sends at most 60 messages an hour" };
                return fail(429, "too-many", words, &[retry_after(wait)], &json!({}));
            }
        }
        if name == "focus" {
            let wait = caller.take(&format!("a:{slug}|events"), rates::EVENTS, 1.0);
            if wait != 0.0 {
                return fail(
                    429,
                    "too-many",
                    "an actor's focus is its status, an event on the feed, and 60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes",
                    &[retry_after(wait)],
                    &json!({}),
                );
            }
        }
        let extra = (name == "send").then(|| {
            let info = self.actors.as_ref().and_then(|a| a.info(&id));
            let name = info.as_ref().and_then(|i| i.get("name")).and_then(Value::as_str).map_or_else(|| slug.clone(), str::to_string);
            let product = info.as_ref().and_then(|i| i.get("product")).and_then(|p| p.get("id")).and_then(Value::as_str).map(str::to_string);
            json!({ "envelope": attest(Sender { id: &id, kind: "actor", name: Some(name), product }, "token", &verdict, self.home, self.now, platform()) })
        });
        match self.op(name, &id, &body, extra).await {
            Ok(v) => Reply::json(200, v),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("bad-request"), e.message, &[], &json!({})),
        }
    }

    pub async fn route(&self, req: &mut Req) -> Reply {
        match self.route_inner(req).await {
            Ok(r) => r,
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(400, e.message),
        }
    }

    async fn route_inner(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let route = req.path.clone();
        if token_in_url(req.params(), &route) {
            return Ok(Reply::coded(400, "bad-request", "a token goes in the Authorization header, never in a URL"));
        }
        if req.method == "POST" && route == "/api/hook/codex" {
            let body = req.json_body().await.unwrap_or_else(|_| json!({}));
            if truthy(body.get("session_id")) {
                let id = format!("codex:{}", xenonet_base::text::js_string(&body["session_id"]));
                let verdict = self.verify.check(req.peer, &id, true).await;
                if !ok(&verdict) {
                    return Ok(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
                }
            }
            let event = body.get("hook_event_name").and_then(Value::as_str).map(str::to_string);
            let out = self.engine.hook("codex", event.as_deref(), &body).await?;
            return Ok(match out {
                Some(v) if truthy(Some(&v)) => Reply::json(200, v),
                _ => Reply::empty(204),
            });
        }
        if req.method == "GET" && route == "/api/engine/state" {
            return Ok(Reply::json(200, self.engine.snapshot().await?));
        }
        if req.method == "GET" && route == "/api/engine/agents" {
            return Ok(Reply::json(200, self.engine.agents().await?));
        }
        if req.method == "GET" && route == "/api/engine/who" {
            let file = req.param("path").unwrap_or("").to_string();
            if !is_absolute(&file) {
                return Ok(Reply::error(400, "give the file's absolute path"));
            }
            return Ok(Reply::json(200, self.engine.who_has(None, &file).await?));
        }
        if route == "/api/engine/connect" {
            if req.method == "GET" {
                return Ok(Reply::json(200, self.connect.preview().await?));
            }
            // Changing Claude Code's and Codex's settings is yours to do, not
            // an agent's, and never an actor's.
            if req.header("authorization").is_some() {
                return Ok(Reply::coded(403, "yours", "connecting Claude Code and Codex is yours to do, and an actor can't do it"));
            }
            let you = self.verify.person(req.peer).await;
            if !ok(&you) {
                return Ok(Reply::error(403, reason(&you)));
            }
            if req.method == "POST" {
                let apps = req.json_body().await?.get("apps").cloned().unwrap_or(Value::Null);
                return Ok(Reply::json(200, self.connect.apply(apps).await?));
            }
            if req.method == "DELETE" {
                return Ok(Reply::json(200, self.connect.remove().await?));
            }
        }
        let name = route.strip_prefix("/api/engine/").unwrap_or("").to_string();
        if req.method != "POST" || !OPS.contains(&name.as_str()) {
            return Ok(Reply::error(404, "not found"));
        }
        if req.header("authorization").is_some() {
            return Ok(if name == "channel" { Reply::error(404, "not found") } else { self.as_actor(req, &name).await });
        }
        let Some(me) = req.header("x-xenonet-agent").filter(|m| is_agent_id(&Arg::text(*m))).map(str::to_string) else {
            return Ok(Reply::error(400, "say which session is calling, in x-xenonet-agent (claude:<session id> or codex:<thread id>)"));
        };
        let verdict = self.verify.check(req.peer, &me, false).await;
        if !ok(&verdict) {
            return Ok(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
        }
        let body = req.json_body().await?;
        // The agent's own process, where XenoNet could see it; else what the
        // MCP server says.
        let pid = verdict.get("root").and_then(|r| r.get("pid")).filter(|p| !p.is_null()).cloned().unwrap_or_else(|| xenonet_base::json::number(js_number(body.get("pid"))));
        let cwd = body.get("cwd").filter(|c| c.is_string()).cloned().unwrap_or(Value::Null);
        self.engine.touch(&me, json!({ "app": me.split(':').next().unwrap_or(""), "cwd": cwd, "via": "mcp", "pid": pid }))?;
        let extra = if name == "send" {
            let title = self.engine.view(&me).await.ok().flatten().and_then(|v| v.get("title").cloned()).and_then(|t| t.as_str().map(str::to_string));
            Some(json!({ "envelope": attest(Sender { id: &me, kind: "agent", name: title, product: None }, "session", &verdict, self.home, self.now, platform()) }))
        } else {
            None
        };
        Ok(Reply::json(200, self.op(&name, &me, &body, extra).await?))
    }
}

fn js_string_or_empty(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        v => xenonet_base::text::js_string(v),
    }
}

/// path.isAbsolute, as Node has it on this platform.
fn is_absolute(p: &str) -> bool {
    if cfg!(windows) {
        p.starts_with('/')
            || p.starts_with('\\')
            || Path::new(p).is_absolute()
            || (p.len() >= 2 && p.as_bytes()[1] == b':' && (p.len() == 2 || matches!(p.as_bytes()[2], b'/' | b'\\')))
    } else {
        p.starts_with('/')
    }
}
