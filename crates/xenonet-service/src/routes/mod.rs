//! The API, in the order server/index.js's api() dispatches it: the version
//! check, the window's message to an actor, then each route module and
//! index.js's own routes. The order matters: it's part of the contract,
//! since a route that answers first is the one that answers.

pub mod actors;
pub mod assistant;
pub mod delivery;
pub mod drift;
pub mod engine;
pub mod environments;
pub mod hosts;
pub mod integrations;
pub mod mcp;
pub mod refusals;
pub mod themes;
pub mod workspace;

use crate::borrowed::{attest, is_actor_agent, platform, Sender};
use crate::cores::settings::{MAX_BRIEF, NEEDS_INPUT_URL};
use crate::cores::{ok, CoreError, Kind};
use crate::ctx::{pending_reply, Answer, Ctx};
use crate::events::Local;
use crate::http::{decode_uri_component, Reply, Req};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use xenonet_base::ids::YOU;
use xenonet_base::log::{log_dir, read_logs, summarize, LEVELS};
use xenonet_base::sync::Locked;
use xenonet_base::text::{js_number, js_string, truthy};

pub const SERVICE_API: (u32, u32) = (1, 1);
pub const CONSOLE_API: (u32, u32) = (1, 1);
pub const CAPABILITIES: [&str; 4] = ["console", "terminals", "terminal-reattach", "assistant-1"];

pub fn api_range((min, max): (u32, u32)) -> Value {
    json!({ "min": min, "max": max })
}

fn num_param(req: &Req, k: &str) -> f64 {
    js_number(req.param(k).map(|s| Value::String(s.to_string())).as_ref())
}

/// `Number(q.get(k) ?? fallback)`: the fallback only when it's absent.
fn num_or(req: &Req, k: &str, fallback: f64) -> f64 {
    req.param(k).map_or(fallback, |s| js_number(Some(&Value::String(s.to_string()))))
}

pub async fn api(ctx: &Arc<Ctx>, mut req: Req) -> Answer {
    let route = req.path.clone();
    let req = &mut req;
    let method = req.method.clone();
    let m = method.as_str();
    if let Some(v) = req.header("x-xenonet-api-version") {
        if !v.is_empty() && v != "1" {
            return Ok(Reply::json(426, json!({ "error": "This XenoNet Service does not support the requested API version.", "api": api_range(SERVICE_API) })));
        }
    }
    let cores = &ctx.cores;

    // You, writing to an actor from XenoNet's own window: only with the
    // window's key, never with a token, and only to an actor that reads
    // its inbox.
    if m == "POST" && route == "/api/actors/you/message" {
        if !ctx.window_only(req) {
            return Ok(Reply::coded(403, "yours", "this is for XenoNet's own window"));
        }
        let w = cores.verify.window(req.peer).await;
        if !ok(&w) {
            return Ok(Reply::send(403, &json!({ "error": w.get("reason").cloned().unwrap_or(Value::Null), "code": "yours" }), &[]));
        }
        let body = req.json(16 << 10).await.unwrap_or(Value::Null);
        let to = body.get("to").and_then(Value::as_str).unwrap_or("").to_string();
        if !is_actor_agent(&to) {
            return Ok(Reply::coded(400, "bad-request", "write to an actor here (actor:<slug>); sessions get messages from other sessions and actors"));
        }
        let envelope = attest(Sender { id: YOU, kind: "you", name: Some("you".into()), product: None }, "window", &Value::Null, &ctx.config.home, ctx.now(), platform());
        return Ok(match cores.engine.send(YOU, json!({ "to": to, "text": body.get("text") }), Some(json!({ "envelope": envelope }))).await {
            Ok(v) => Reply::json(200, v),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::coded(400, "bad-request", e.message),
        });
    }
    if route.starts_with("/api/engine/") || route == "/api/hook/codex" {
        let r = engine::EngineRoutes {
            engine: cores.engine.clone(),
            connect: cores.connect.clone(),
            verify: cores.verify.clone(),
            caller: ctx.caller.as_ref(),
            actors: cores.actors.clone(),
            channels: Some(cores.channels.clone()),
            home: &ctx.config.home,
            now: ctx.now(),
        };
        return Ok(r.route(req).await);
    }
    if route == "/api/delivery" || route.starts_with("/api/delivery/") {
        let r = delivery::DeliveryRoutes { policy: cores.policy.clone(), router: cores.router.clone(), engine: cores.engine.clone(), verify: cores.verify.clone() };
        return Ok(r.route(req).await);
    }
    if route == "/api/actors" || route.starts_with("/api/actors/") {
        let taken = std::mem::replace(req, Req::fake("GET", "/", &[], None));
        return actors::ActorRoutes { ctx }.route(taken).await;
    }
    if route == "/api/assistant" || route.starts_with("/api/assistant/") {
        let taken = std::mem::replace(req, Req::fake("GET", "/", &[], None));
        return assistant::AssistantRoutes { ctx }.route(taken).await;
    }
    if let Some(r) = (hosts::HostRoutes { hosts: cores.hosts.clone(), hub: cores.hub.clone(), verify: cores.verify.clone(), home: &ctx.config.home }).route(req).await {
        return Ok(r);
    }

    if m == "GET" && route == "/api/health" {
        // What it has: a source not read yet (its core not in Rust) is none.
        let components = ctx.components.locked().is_some();
        return Ok(Reply::json(
            200,
            json!({
                "ok": true,
                "name": ctx.config.name,
                "version": ctx.version,
                "api": api_range(SERVICE_API),
                "consoleApi": api_range(CONSOLE_API),
                "capabilities": if components { json!(CAPABILITIES) } else { json!([]) },
                "demo": ctx.config.demo,
                "instance": ctx.config.instance,
                "pid": std::process::id(),
                "sources": cores.state.sources().unwrap_or_else(|_| json!([])),
                "blocks": cores.state.blocks().unwrap_or_else(|_| json!([])),
            }),
        ));
    }
    // The page's uncaught errors, which it sends here: in the log as
    // page.uncaught, scrubbed and held to ten seconds apart like any.
    if m == "POST" && route == "/api/logs/page" {
        let body = req.json(8192).await.ok();
        if let Some(message) = body.as_ref().and_then(|b| b.get("message")).and_then(Value::as_str) {
            let stack = body.as_ref().and_then(|b| b.get("stack")).and_then(Value::as_str).filter(|s| !s.is_empty());
            let err = stack.map(|s| xenonet_base::log::ErrInfo { message: Some(message.to_string()), stack: Some(xenonet_base::log::js_slice(s, 3000).to_string()), code: None });
            ctx.log("error", "page.uncaught", xenonet_base::log::js_slice(message, 1000), err.as_ref(), None);
        }
        return Ok(Reply::json(200, json!({ "ok": true })));
    }
    if m == "GET" && route == "/api/logs" {
        let now = ctx.now();
        let entries = if ctx.config.demo {
            Vec::new()
        } else {
            let since = num_param(req, "since");
            let since = if since.is_nan() || since == 0.0 { now - 24.0 * 3600e3 } else { since };
            let level = req.param("level").filter(|l| LEVELS.contains(l)).unwrap_or("warn").to_string();
            let limit = num_param(req, "limit");
            let limit = (if limit.is_nan() || limit == 0.0 { 200.0 } else { limit }).min(1000.0);
            let component = req.param("component").filter(|c| !c.is_empty()).map(str::to_string);
            let code = req.param("code").filter(|c| !c.is_empty()).map(str::to_string);
            read_logs(Path::new(&log_dir(&ctx.config.state_dir)), since, &level, component.as_deref(), code.as_deref(), limit.max(0.0) as usize)
        };
        let at = xenonet_base::json::number(now);
        return Ok(Reply::json(200, if req.param("summary") == Some("1") { json!({ "at": at, "summary": summarize(&entries) }) } else { json!({ "at": at, "entries": entries }) }));
    }

    // What OpenRouter's key has spent, and what each actor's work cost today.
    if m == "GET" && route == "/api/spend" {
        return Ok(Reply::json(200, json!({ "openrouter": cores.openrouter.status().await?, "actors": cores.spend.actor_spend()? })));
    }
    if m == "GET" && route == "/api/spend/actors" {
        return Ok(if cores.actors.is_some() { Reply::json(200, cores.spend.actor_spend()?) } else { Reply::error(404, "this XenoNet has no actors") });
    }
    if m == "GET" && route == "/api/system" {
        return Ok(Reply::json(200, cores.system.read().await?));
    }

    if let Some(r) = themes::route(ctx, req).await? {
        return Ok(r);
    }
    if let Some(r) = mcp::route(ctx, req).await? {
        return Ok(r);
    }
    if let Some(r) = environments::route(ctx, req).await? {
        return Ok(r);
    }
    if route == "/api/themes" && m == "GET" {
        let read = cores.themes.read(&ctx.themes_dir)?;
        let current = current_theme(ctx)?;
        let themes: Vec<Value> = read["themes"]
            .as_object()
            .map(|t| t.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .map(|t| {
                let mut out = json!({ "id": t["id"], "name": t["name"], "about": t.get("about").filter(|a| !a.is_null()).cloned().unwrap_or(json!("")), "builtIn": t.get("builtIn").cloned().unwrap_or(Value::Null) });
                if truthy(t.get("replacesBuiltIn")) {
                    out["replacesBuiltIn"] = json!(true);
                }
                let c = &t["colors"];
                out["swatch"] = json!([c["bg"], c["accent"], c["claude"], c["codex"], c["attention"]]);
                out
            })
            .collect();
        return Ok(Reply::json(200, json!({ "current": current["id"], "folder": ctx.themes_dir, "problems": read["problems"], "themes": themes })));
    }
    if route == "/api/theme" && m == "GET" {
        return Ok(Reply::json(200, current_theme(ctx)?));
    }
    if route == "/api/theme" && m == "POST" {
        if let Some(r) = ctx.yours(req).await {
            return Ok(r);
        }
        let id = req.json_body().await?.get("id").cloned().unwrap_or(Value::Null);
        let read = cores.themes.read(&ctx.themes_dir)?;
        let Some(id) = id.as_str().filter(|i| read["themes"].get(*i).is_some()).map(str::to_string) else {
            let shown = if id.is_null() { "undefined".into() } else { js_string(&id) };
            return Ok(Reply::error(404, format!("there's no theme called {shown}")));
        };
        cores.store.set_theme(&id)?;
        let theme = current_theme(ctx)?;
        ctx.broadcast("theme", &theme);
        return Ok(Reply::json(200, theme));
    }
    if route == "/api/themes/copy" && m == "POST" {
        let body = req.json_body().await?;
        let from = match body.get("from").filter(|f| !f.is_null()) {
            Some(f) => js_string(f),
            None => js_string(&current_theme(ctx)?["id"]),
        };
        let id = body.get("id").filter(|i| !i.is_null()).map(js_string).unwrap_or_default();
        return Ok(match cores.themes.copy(&ctx.themes_dir, &from, &id, body.get("name").unwrap_or(&Value::Null)) {
            Ok(file) => Reply::json(200, json!({ "file": file })),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(409, e.message),
        });
    }

    // The xenonet command on your PATH, run by this app's runtime.
    if route == "/api/command" {
        if m == "GET" {
            return Ok(Reply::json(200, cores.command.state()?));
        }
        if m == "POST" {
            let entry = ctx.config.cli_entry().to_string_lossy().into_owned();
            return Ok(match cores.command.install(&entry, ctx.config.launcher_exec.as_deref()) {
                Ok(v) => Reply::json(200, v),
                Err(e) if e.is_pending() => pending_reply(&e),
                Err(e) => Reply::error(409, e.message),
            });
        }
    }

    if route == "/api/drift" || route.starts_with("/api/drift/") {
        return drift::route(ctx, req).await;
    }

    // The doctor's last results (running it first if it hasn't run), or a
    // new run; only yours counts as by hand.
    if route == "/api/doctor" {
        if m == "GET" {
            return Ok(Reply::json(
                200,
                match cores.doctor.last() {
                    Some(l) => l,
                    None => cores.doctor.check(false, false).await?,
                },
            ));
        }
        if m == "POST" {
            let by_hand = ok(&cores.verify.person(req.peer).await);
            return Ok(Reply::json(200, cores.doctor.check(false, by_hand).await?));
        }
    }

    if route == "/api/spotify" || route.starts_with("/api/spotify/") {
        if let Some(r) = spotify(ctx, req).await? {
            return Ok(r);
        }
    }

    // The decision model: GET how it stands; POST /key {key}, with the
    // window's key, keeps it in your credential store; DELETE /key forgets it.
    if route == "/api/decide" || route == "/api/decide/key" {
        let decider = &cores.decider;
        let out: Result<Option<Reply>, CoreError> = async {
            if m == "GET" && route == "/api/decide" {
                return Ok(Some(Reply::json(200, decider.status().await?)));
            }
            if route == "/api/decide/key" && m == "POST" {
                if !ctx.window_only(req) {
                    return Ok(Some(Reply::coded(403, "yours", "this is for XenoNet's own window")));
                }
                let key = req.json(2048).await?.get("key").cloned().unwrap_or(Value::Null);
                let stored = decider.store_key(&key).await?;
                cores.openrouter.forget();
                return Ok(Some(Reply::json(200, stored)));
            }
            if route == "/api/decide/key" && m == "DELETE" {
                if let Some(r) = ctx.yours(req).await {
                    return Ok(Some(r));
                }
                cores.openrouter.forget();
                return Ok(Some(Reply::json(200, decider.forget_key().await?)));
            }
            Ok(None)
        }
        .await;
        match out {
            Ok(Some(r)) => return Ok(r),
            Ok(None) => {}
            Err(e) if e.is(Kind::Decision) => return Ok(Reply::send(e.status.unwrap_or(400), &json!({ "error": e.message, "code": e.code }), &[])),
            Err(e) => return Err(e),
        }
    }

    if let Some(r) = refusals::route(ctx, req).await? {
        return Ok(r);
    }
    if let Some(r) = integrations::route(ctx, req).await {
        return Ok(r);
    }

    // What XenoNet can tell someone helping you, safe to share: GET the
    // bundle, as JSON and Markdown; POST /ask {app, model} makes a folder
    // for an agent to look at it with you and says the command that starts
    // it. Starting an agent is yours to do.
    if route == "/api/diagnose" && m == "GET" {
        let b = cores.diagnostics.bundle().await?;
        let md = cores.diagnostics.markdown(&b)?;
        return Ok(Reply::json(200, json!({ "bundle": b, "markdown": md })));
    }
    if route == "/api/diagnose/ask" && m == "POST" {
        if ctx.config.demo {
            return Ok(Reply::coded(409, "demo", "this is the demo: ask about the XenoNet with your own sessions"));
        }
        if let Some(r) = ctx.yours(req).await {
            return Ok(r);
        }
        let body = req.json(4096).await?;
        // Codex, unless you ask for Claude Code.
        let app = xenonet_base::apps::asked_app(body.get("app").and_then(Value::as_str).unwrap_or_default(), "codex");
        let model = body.get("model").and_then(Value::as_str).filter(|s| model_ok(s)).map(str::to_string);
        let b = cores.diagnostics.bundle().await?;
        let dir = cores.diagnostics.write_look(&b)?;
        let cmd = cores.diagnostics.ask_command(app, model.as_deref(), &dir)?;
        // As one line any shell takes: the words in double quotes.
        let words: Vec<String> = cmd["args"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|a| {
                let a = js_string(a);
                if !a.is_empty() && a.bytes().all(|c| c.is_ascii_alphanumeric() || b"._:=/-".contains(&c)) {
                    a
                } else {
                    format!("\"{}\"", a.replace('"', ""))
                }
            })
            .collect();
        let line = std::iter::once(app.to_string()).chain(words).collect::<Vec<_>>().join(" ");
        return Ok(Reply::json(200, json!({ "app": app, "model": model, "dir": dir, "folder": xenonet_base::paths::tilde(&dir, &ctx.config.home), "command": line })));
    }

    // With hosts on, other hosts' maps and live state are in them, except
    // for ?hosts=0, this machine's own.
    let merged = cores.hub.as_ref().filter(|_| req.param("hosts") != Some("0"));
    if m == "GET" && route == "/api/graph" {
        let graph = match cores.state.graph()? {
            Some(g) => g,
            None => cores.state.refresh().await?,
        };
        return Ok(Reply::json(
            200,
            match merged {
                Some(hub) => hub.graph(graph),
                None => graph,
            },
        ));
    }
    if m == "GET" && route == "/api/live" {
        let live = cores.state.live_snapshot()?;
        return Ok(Reply::json(
            200,
            match merged {
                Some(hub) => hub.live(live),
                None => live,
            },
        ));
    }
    // Event history (last 24h) for replay and digests.
    if m == "GET" && route == "/api/history" {
        let now = ctx.now();
        let since = num_or(req, "since", now - 24.0 * 3600e3);
        return Ok(Reply::json(200, json!({ "events": cores.state.history_since(since)?, "now": xenonet_base::json::number(now) })));
    }
    if m == "GET" && route == "/api/search" {
        let q = req.param("q").unwrap_or("").to_string();
        let results = cores.state.search_transcripts(&q).await?;
        return Ok(Reply::json(200, json!({ "results": results, "status": cores.state.search_status()? })));
    }
    if m == "POST" && route == "/api/ack" {
        let body = req.json_body().await?;
        let id = match body.get("sessionId").and_then(Value::as_str) {
            Some(s) if cores.state.node(s)?.is_some() => s,
            _ => return Ok(Reply::error(400, "XenoNet doesn't know that session")),
        };
        cores.store.ack(id)?;
        ctx.broadcast("live", &cores.state.live_snapshot()?);
        return Ok(Reply::json(200, json!({ "ok": true })));
    }
    if m == "GET" && route == "/api/collision" {
        let (a, b) = (req.param("a").map(str::to_string), req.param("b").map(str::to_string));
        let side = |id: &Option<String>| -> Result<Value, CoreError> {
            let node = match id {
                Some(i) => cores.state.node(i)?,
                None => None,
            };
            Ok(json!({ "id": id, "title": cores.state.title_of(id.as_deref())?, "branch": node.and_then(|n| n.get("branch").cloned()).unwrap_or(Value::Null) }))
        };
        return Ok(Reply::json(200, json!({ "a": side(&a)?, "b": side(&b)?, "files": cores.state.collision_diffs(a.as_deref(), b.as_deref())? })));
    }
    if route == "/api/handoff" {
        if m == "GET" {
            // Claude Code, unless you ask for Codex.
            let target = xenonet_base::apps::asked_app(req.param("target").unwrap_or_default(), "claude");
            let node = req.param("nodeId").map(str::to_string);
            return Ok(match cores.state.handoff(node.as_deref(), target).await {
                Ok(v) => {
                    let mut out = json!({ "target": target });
                    if let Value::Object(more) = v {
                        for (k, v) in more {
                            out[k] = v;
                        }
                    }
                    Reply::json(200, out)
                }
                Err(e) if e.is_pending() => pending_reply(&e),
                Err(e) => Reply::error(400, e.message),
            });
        }
        if m == "POST" {
            let body = req.json_body().await?;
            let node = match body.get("nodeId").and_then(Value::as_str) {
                Some(id) => cores.state.node(id)?,
                None => None,
            };
            let target = body.get("target").and_then(Value::as_str).filter(|t| ["claude", "codex"].contains(t));
            let brief = body.get("brief").and_then(Value::as_str).filter(|b| !xenonet_base::text::is_blank(b) && b.encode_utf16().count() <= MAX_BRIEF);
            let (Some(node), Some(target), Some(brief)) = (node.filter(|n| n["kind"] == "session"), target, brief) else {
                return Ok(Reply::error(400, "bad handoff"));
            };
            if ctx.config.demo {
                return Ok(Reply::error(409, "Demo sessions exist only in XenoNet, so they can't be handed off."));
            }
            let folder = node.get("worktree").filter(|w| truthy(Some(w))).or_else(|| node.get("cwd")).cloned().unwrap_or(Value::Null);
            let url = cores.opener.handoff_url(target, &folder, brief)?;
            ctx.open(&url).await?;
            return Ok(Reply::json(200, json!({ "ok": true, "target": target })));
        }
    }
    if m == "GET" && route == "/api/digest" {
        let since = num_or(req, "since", ctx.now() - 2.0 * 3600e3);
        return Ok(Reply::json(200, cores.state.digest(since)?));
    }
    if m == "GET" && route == "/api/daylog" {
        return Ok(Reply::json(200, cores.state.day_log()?));
    }
    if m == "GET" && route == "/api/constellation" {
        return Ok(Reply::json(200, cores.state.constellation(req.param("names") == Some("1"))?));
    }
    if m == "GET" && route == "/api/constellation/labels" {
        return Ok(Reply::json(200, cores.state.export_labels()?));
    }
    if m == "POST" && route == "/api/narrate" {
        let body = req.json_body().await?;
        let kind = if body.get("kind").and_then(Value::as_str) == Some("day") { "day" } else { "away" };
        let since = js_number(body.get("since"));
        let since = if since.is_nan() || since == 0.0 { ctx.now() - 2.0 * 3600e3 } else { since };
        return Ok(match cores.state.narrate(kind, since).await {
            Ok(text) => Reply::json(200, json!({ "text": text })),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(502, e.message),
        });
    }
    if route == "/api/layout" {
        if m == "GET" {
            return Ok(Reply::json(200, cores.store.layout()?));
        }
        if m == "POST" {
            let positions = req.json_body().await?.get("positions").cloned().unwrap_or(Value::Null);
            cores.store.set_layout(&positions)?;
            return Ok(Reply::json(200, json!({ "ok": true })));
        }
        if m == "DELETE" {
            cores.store.clear_layout()?;
            let c = ctx.clone();
            tokio::spawn(async move {
                let _ = c.cores.state.refresh().await;
            });
            return Ok(Reply::json(200, json!({ "ok": true })));
        }
    }

    // The projects folder itself: which it is, checking another, changing it.
    if route.starts_with("/api/workspace/") {
        return Ok(workspace::route(ctx, req).await);
    }

    // How the projects folder is read: the last scan, your rules, Codex's picks.
    if route.starts_with("/api/projects") {
        match projects(ctx, req).await {
            Ok(Some(r)) => return Ok(r),
            Ok(None) => {}
            Err(e) if e.is_pending() => return Ok(pending_reply(&e)),
            Err(e) => return Ok(Reply::error(e.status.unwrap_or(400), e.message)),
        }
    }

    // Identities: what was found, the names you gave them, ones you hid, and
    // the sessions and projects you pinned to one.
    if route.starts_with("/api/identities") {
        match identities(ctx, req).await {
            Ok(Some(r)) => return Ok(r),
            Ok(None) => {}
            Err(e) if e.is_pending() => return Ok(pending_reply(&e)),
            Err(e) => return Ok(Reply::error(e.status.unwrap_or(400), e.message)),
        }
    }

    // With hosts on, live hosts' alerts count in the tray too; the actors
    // waiting on you and how many programs ask to be actors come with it, a
    // count, never who asked, since this route has no key.
    if m == "GET" && route == "/api/summary" {
        let summary = cores.state.summary()?;
        let mut out = match &cores.hub {
            Some(h) => h.summary(summary),
            None => summary,
        };
        if let Value::Object(waits) = waits_on_you(ctx)? {
            for (k, v) in waits {
                out[k] = v;
            }
        }
        return Ok(Reply::json(200, out));
    }

    if m == "GET" && route == "/api/events" {
        let own_only = cores.hub.is_some() && req.param("hosts") == Some("0");
        let rx = ctx.events.subscribe(own_only);
        return Ok(Reply::stream(200, &[("content-type", "text/event-stream"), ("cache-control", "no-store"), ("connection", "keep-alive")], rx));
    }

    // Sessions by id or title words, newest first, for the xenonet command;
    // with cwd, only those that ran in that folder (or one inside it).
    if m == "GET" && route == "/api/sessions" {
        return Ok(Reply::json(200, sessions(ctx, req)?));
    }

    // A session's models, tokens, costs and timing.
    if let Some(raw) = route.strip_prefix("/api/session/").and_then(|r| r.strip_suffix("/stats")).filter(|r| !r.is_empty() && !r.contains('/')) {
        if m == "GET" {
            let Some(id) = decode_uri_component(raw) else { return Err(CoreError::new("URI malformed")) };
            let Some(node) = cores.state.node(&id)?.filter(|n| n["kind"] == "session") else {
                return Ok(Reply::error(404, "XenoNet doesn't know that session"));
            };
            let nid = js_string(&node["id"]);
            let result = match cores.state.session_stats(&nid).await {
                Ok(r) => r,
                Err(e) if e.is_pending() => return Ok(pending_reply(&e)),
                Err(e) => Some(json!({ "error": e.message })),
            };
            let Some(mut result) = result else { return Ok(Reply::error(404, "XenoNet can't find this session's transcript")) };
            if truthy(result.get("error")) {
                return Ok(Reply::json(502, result));
            }
            for k in ["title", "cwd", "branch", "model"] {
                result[k] = node.get(k).cloned().unwrap_or(Value::Null);
            }
            return Ok(Reply::json(200, result));
        }
    }
    if let Some(raw) = route.strip_prefix("/api/session/").and_then(|r| r.strip_suffix("/transcript")).filter(|r| !r.is_empty() && !r.contains('/')) {
        if m == "GET" {
            let Some(id) = decode_uri_component(raw) else { return Err(CoreError::new("URI malformed")) };
            let Some(node) = cores.state.node(&id)?.filter(|n| n["kind"] == "session") else {
                return Ok(Reply::error(404, "XenoNet doesn't know that session"));
            };
            let cursor = req.param("cursor").map(str::to_string);
            let limit = num_or(req, "limit", 120.0);
            return Ok(match cores.state.read_transcript(&node, cursor.as_deref(), limit).await {
                Ok(page) => Reply::json(200, page),
                Err(e) if e.is_pending() => pending_reply(&e),
                Err(e) => Reply::error(502, e.message),
            });
        }
    }

    // A session's new name, saved in its own app: it changes another app's
    // files, so it's yours to do.
    if m == "POST" && route == "/api/session/rename" {
        if let Some(r) = ctx.yours(req).await {
            return Ok(r);
        }
        let body = req.json_body().await?;
        return Ok(match cores.state.rename_session(body.get("nodeId").cloned().unwrap_or(Value::Null), body.get("title").cloned().unwrap_or(Value::Null)).await {
            Ok(v) => Reply::json(200, v),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(e.status.unwrap_or(502), e.message),
        });
    }

    if m == "POST" && route == "/api/open" {
        let body = req.json_body().await?;
        let action = body.get("action").cloned().unwrap_or(Value::Null);
        if action == "needs-input" {
            ctx.open(NEEDS_INPUT_URL).await?;
            return Ok(Reply::json(200, json!({ "ok": true, "target": NEEDS_INPUT_URL })));
        }
        let node = match body.get("nodeId").and_then(Value::as_str) {
            Some(id) => cores.state.node(id)?,
            None => None,
        };
        let available = |n: &Value| n.get("actions").and_then(Value::as_array).is_some_and(|a| a.iter().any(|x| x["id"] == action && !truthy(x.get("client"))));
        let Some(node) = node.filter(available) else {
            return Ok(Reply::error(400, "that action isn't available here"));
        };
        let Some(target) = cores.opener.url_for(&node, action.as_str().unwrap_or(""))? else {
            return Ok(Reply::error(400, "there's nothing to open for this item"));
        };
        if ctx.config.demo && node["kind"] == "session" && (target.starts_with("claude:") || target.starts_with("codex:")) {
            return Ok(Reply::error(409, "Demo sessions exist only in XenoNet, so they can't open in an app."));
        }
        ctx.open(&target).await?;
        return Ok(Reply::json(200, json!({ "ok": true, "target": target })));
    }

    // Claude Code's HTTP hooks: events transcripts don't record, like
    // permission prompts and notifications.
    if m == "POST" && route == "/api/hook/claude" {
        let body = req.json_body().await.unwrap_or_else(|_| json!({}));
        // XenoNet's plugin for Claude Code posts the same events as
        // connect's own hooks, marked via=plugin. While connect's hook for
        // an event is in Claude Code's settings, that one counts and the
        // plugin's stands by, so each event arrives once.
        if req.param("via") == Some("plugin") {
            let event = body.get("hook_event_name").map(js_string).unwrap_or_default();
            if cores.connect.hook_connected(&event).await.unwrap_or(false) {
                return Ok(Reply::empty(204));
            }
        }
        if let Some(sid) = body.get("session_id").and_then(Value::as_str).filter(|s| is_uuid_like(s)) {
            let id = format!("claude:{sid}");
            let verdict = cores.verify.check(req.peer, &id, true).await;
            if !ok(&verdict) {
                return Ok(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
            }
            cores.state.note_hook(&id, &body)?;
            if body.get("hook_event_name").and_then(Value::as_str) == Some("SessionStart") {
                let c = ctx.clone();
                tokio::spawn(async move {
                    let _ = c.cores.state.refresh().await;
                });
            }
            // What the engine wants the agent to hear.
            let event = body.get("hook_event_name").and_then(Value::as_str).map(str::to_string);
            if let Ok(Some(out)) = cores.engine.hook("claude", event.as_deref(), &body).await {
                if truthy(Some(&out)) {
                    return Ok(Reply::json(200, out));
                }
            }
        }
        return Ok(Reply::empty(204));
    }

    // Grok Build's hooks (xenonet hook grok): its events in Claude Code's
    // format, with its own names for the session and the tool (sessionId,
    // toolName, toolInput, stopHookActive), read as Claude Code's. They
    // tell the tailer when a turn ends, which its lines don't.
    if m == "POST" && route == "/api/hook/grok" {
        let raw = req.json_body().await.unwrap_or_else(|_| json!({}));
        let mut body = match &raw {
            Value::Object(o) => o.clone(),
            _ => serde_json::Map::new(),
        };
        for (ours, theirs) in [("session_id", "sessionId"), ("tool_name", "toolName"), ("tool_input", "toolInput"), ("stop_hook_active", "stopHookActive")] {
            if !body.contains_key(ours) {
                if let Some(v) = raw.get(theirs) {
                    body.insert(ours.into(), v.clone());
                }
            }
        }
        let body = Value::Object(body);
        if let Some(sid) = body.get("session_id").and_then(Value::as_str).filter(|s| is_uuid_like(s)) {
            let id = format!("grok:{sid}");
            let verdict = cores.verify.check(req.peer, &id, true).await;
            if !ok(&verdict) {
                return Ok(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
            }
            cores.state.note_hook(&id, &body)?;
            if body.get("hook_event_name").and_then(Value::as_str) == Some("SessionStart") {
                let c = ctx.clone();
                tokio::spawn(async move {
                    let _ = c.cores.state.refresh().await;
                });
            }
            let event = body.get("hook_event_name").and_then(Value::as_str).map(str::to_string);
            if let Ok(Some(out)) = cores.engine.hook("grok", event.as_deref(), &body).await {
                if truthy(Some(&out)) {
                    return Ok(Reply::json(200, out));
                }
            }
        }
        return Ok(Reply::empty(204));
    }

    // A session's Stop hook that listens after its turns: it waits for a
    // message up to the window the session started with, and hands it
    // over. If the session stops waiting first, the message stays unread.
    if m == "POST" && route == "/api/hook/claude/listen" {
        let body = req.json_body().await.unwrap_or_else(|_| json!({}));
        let sid = body.get("session_id").and_then(Value::as_str).filter(|s| is_uuid_like(s));
        if let (Some("Stop"), Some(sid)) = (body.get("hook_event_name").and_then(Value::as_str), sid) {
            let me = format!("claude:{sid}");
            let verdict = cores.verify.check(req.peer, &me, true).await;
            if !ok(&verdict) {
                return Ok(Reply::send(403, &json!({ "error": verdict.get("reason").cloned().unwrap_or(Value::Null) }), &[]));
            }
            let seconds = num_param(req, "seconds");
            let seconds = (if seconds.is_nan() || seconds == 0.0 { 300.0 } else { seconds }).clamp(5.0, 3600.0);
            // The hook going away (this answer dropped) tells the engine to
            // stop waiting; a message it found by then stays unread for the
            // session's next turn.
            let (gone_tx, gone_rx) = tokio::sync::watch::channel(false);
            let (tx, rx) = tokio::sync::oneshot::channel::<Value>();
            let engine = cores.engine.clone();
            let who = me.clone();
            tokio::spawn(async move {
                if let Ok(Some(out)) = engine.listen(&who, Duration::from_secs_f64(seconds), gone_rx).await {
                    if let Err(out) = tx.send(out) {
                        let _ = engine.mark_unseen(&who, &out["ids"]);
                    }
                }
            });
            let _gone = Gone(gone_tx);
            if let Ok(out) = rx.await {
                return Ok(Reply::json(200, json!({ "decision": out["decision"], "reason": out["reason"] })));
            }
        }
        return Ok(Reply::empty(204));
    }

    // Dev only: rare states to check the visuals by.
    if ctx.config.dev && m == "POST" && route == "/api/dev/simulate" {
        let body = req.json_body().await?;
        let kind = body.get("kind").cloned().unwrap_or(Value::Null);
        let done = simulate(ctx, &kind, &body)?;
        if let Some(e) = done.as_ref().and_then(|d| d.get("error")).filter(|e| truthy(Some(e))) {
            return Ok(Reply::send(400, &json!({ "error": e }), &[]));
        }
        return Ok(match done {
            Some(d) => Reply::json(200, d),
            None => Reply::error(400, "unknown simulation or session"),
        });
    }

    // XenoNet Dev's control channel, relayed to the connected Desktop over
    // the component service's socket. JSON only, which a cross-site form
    // can't send.
    if m == "POST" && route == "/api/dev/control" {
        let components = ctx.components.locked().clone();
        let Some(components) = components.filter(|_| ctx.config.control) else {
            return Ok(Reply::error(503, "this server has no window to control: it isn't running inside XenoNet Dev (npm run app, or npm run drive -- launch)"));
        };
        let json_type = req.header("content-type").is_some_and(|t| {
            let t = t.as_bytes();
            t.len() >= 16 && t[..16].eq_ignore_ascii_case(b"application/json") && t.get(16).is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
        });
        if !json_type {
            return Ok(Reply::error(415, "send JSON (content-type: application/json)"));
        }
        let body = req.json(1 << 16).await?;
        let timeout = js_number(body.get("timeout"));
        let ms = 30e3 + (if timeout.is_nan() { 0.0 } else { timeout }).min(120e3);
        return Ok(match tokio::time::timeout(Duration::from_millis(ms.max(0.0) as u64), components.console(body)).await {
            Err(_) => Reply::error(504, format!("XenoNet Dev didn't answer in {}s", xenonet_base::json::number_string(ms / 1000.0))),
            Ok(Err(e)) => Reply::error(400, e),
            // Always JSON: a result can be a string.
            Ok(Ok(result)) => Reply::raw(200, xenonet_base::json::stringify(&result, 0), &[]),
        });
    }

    // Dev only: save a frame of the WebGL canvas for visual checks.
    if ctx.config.dev && m == "POST" && route == "/api/dev/capture" {
        let body = req.json(64 << 20).await?;
        let data = body.get("dataUrl").and_then(Value::as_str).and_then(|d| d.strip_prefix("data:image/png;base64,")).filter(|d| !d.is_empty());
        let Some(data) = data else { return Ok(Reply::error(400, "expected a PNG data URL")) };
        let dir = ctx.config.root.join(".xenonet");
        std::fs::create_dir_all(&dir).map_err(|e| CoreError::new(e.to_string()))?;
        let name: String = req.param("name").unwrap_or("capture").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let name = if name.is_empty() { "capture".to_string() } else { name };
        let file = dir.join(format!("{name}.png"));
        std::fs::write(&file, base64_decode(data)).map_err(|e| CoreError::new(e.to_string()))?;
        return Ok(Reply::json(200, json!({ "ok": true, "file": file.to_string_lossy() })));
    }

    Ok(Reply::error(404, "not found"))
}

/// Tells the engine the listen hook went, when its answer is dropped.
struct Gone(tokio::sync::watch::Sender<bool>);

impl Drop for Gone {
    fn drop(&mut self) {
        let _ = self.0.send(true);
    }
}

/// ^[0-9a-f-]{36}$, any case.
fn is_uuid_like(s: &str) -> bool {
    s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// ^[A-Za-z0-9._:\/-]{1,80}$
fn model_ok(s: &str) -> bool {
    (1..=80).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
}

fn base64_decode(s: &str) -> Vec<u8> {
    let val = |c: u8| -> Option<u32> {
        Some(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        }))
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in s.bytes() {
        let Some(v) = val(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The theme in use, or the default.
pub fn current_theme(ctx: &Ctx) -> Result<Value, CoreError> {
    let read = ctx.cores.themes.read(&ctx.themes_dir)?;
    let id = ctx.cores.store.theme()?;
    let themes = &read["themes"];
    Ok(id.as_str().and_then(|i| themes.get(i)).or_else(|| themes.get("xenonet")).cloned().unwrap_or(Value::Null))
}

/// What waits on you apart from sessions: the actors waiting on you, and
/// how many programs ask to be actors.
fn waits_on_you(ctx: &Ctx) -> Result<Value, CoreError> {
    let list = match &ctx.cores.actors {
        Some(a) => a.list()?["actors"].as_array().cloned().unwrap_or_default(),
        None => Vec::new(),
    };
    let waiting: Vec<Value> = list
        .iter()
        .filter(|a| truthy(a.get("needsYou")) && a["kind"] != "human")
        .map(|a| json!({ "id": a["id"], "name": a["name"], "kind": a["needsYou"]["kind"], "text": a["needsYou"]["text"], "since": a["needsYou"].get("since").cloned().unwrap_or(Value::Null) }))
        .collect();
    Ok(json!({ "actorsNeedYou": waiting, "requests": ctx.cores.registration.as_ref().map_or(0, |r| r.waiting()) }))
}

fn sessions(ctx: &Ctx, req: &Req) -> Result<Value, CoreError> {
    let q = xenonet_base::text::js_trim(req.param("q").unwrap_or("")).to_lowercase();
    let words: Vec<&str> = q.split(xenonet_base::text::is_js_space).filter(|w| !w.is_empty()).collect();
    let cwd = req.param("cwd").map(str::to_string);
    let sep = if cfg!(windows) { '\\' } else { '/' };
    let under = |dir: &Value| -> bool {
        let Some(d) = dir.as_str().filter(|d| !d.is_empty()) else { return false };
        let c = cwd.as_deref().unwrap_or("");
        d == c || d.starts_with(&format!("{c}{sep}"))
    };
    let lower = |v: &Value| v.as_str().map(str::to_lowercase);
    let mut found: Vec<Value> = ctx
        .cores
        .state
        .nodes()?
        .into_iter()
        .filter(|n| n["kind"] == "session")
        .filter(|n| {
            q.is_empty() || lower(&n["id"]).is_some_and(|i| i.starts_with(&q)) || lower(&n["rawId"]).is_some_and(|i| i.starts_with(&q)) || {
                let text = format!(
                    "{} {}",
                    n.get("title").filter(|t| !t.is_null()).map(js_string).unwrap_or_default(),
                    n.get("cwd").filter(|t| !t.is_null()).map(js_string).unwrap_or_default()
                )
                .to_lowercase();
                words.iter().all(|w| text.contains(w))
            }
        })
        .filter(|n| cwd.is_none() || under(&n["cwd"]) || under(&n["worktree"]))
        .collect();
    found.sort_by(|a, b| b["updatedAt"].as_f64().unwrap_or(0.0).total_cmp(&a["updatedAt"].as_f64().unwrap_or(0.0)));
    let limit = num_param(req, "limit");
    let limit = (if limit.is_nan() || limit == 0.0 { 20.0 } else { limit }).min(200.0);
    let limit = if limit < 0.0 { found.len().saturating_sub((-limit) as usize) } else { limit as usize };
    Ok(Value::Array(
        found
            .into_iter()
            .take(limit)
            .map(|n| {
                let or_null = |k: &str| n.get(k).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
                json!({ "id": n["id"], "app": n.get("app"), "title": n.get("title"), "cwd": or_null("cwd"), "branch": or_null("branch"), "updatedAt": or_null("updatedAt"), "activity": or_null("activity") })
            })
            .collect(),
    ))
}

async fn projects(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let state = &ctx.cores.state;
    let route = req.path.clone();
    let m = req.method.clone();
    match (m.as_str(), route.as_str()) {
        ("GET", "/api/projects") => Ok(Some(Reply::json(200, state.project_detection().await?))),
        ("POST", "/api/projects/rule") => {
            if let Some(r) = ctx.yours(req).await {
                return Ok(Some(r));
            }
            let body = req.json_body().await?;
            let as_ = body.get("as").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
            Ok(Some(Reply::json(200, state.set_project_rule(body.get("path").cloned().unwrap_or(Value::Null), as_).await?)))
        }
        // The rules for the folder in use; with ?all=1, every folder's.
        ("DELETE", "/api/projects/rules") => {
            if let Some(r) = ctx.yours(req).await {
                return Ok(Some(r));
            }
            Ok(Some(Reply::json(200, state.clear_project_rules(req.param("all") == Some("1")).await?)))
        }
        ("POST", "/api/projects/rescan") => Ok(Some(Reply::json(200, state.rescan_projects().await?))),
        // The Codex pass: GET shows exactly what would be sent, POST sends
        // that (it must name the same fingerprint), DELETE forgets the picks.
        ("GET", "/api/projects/codex") => {
            let mut out = state.codex_outline().await?;
            out["last"] = state.project_detection().await?.get("codex").cloned().unwrap_or(Value::Null);
            out["demo"] = json!(ctx.config.demo);
            out["decider"] = ctx.cores.decider.status().await?;
            Ok(Some(Reply::json(200, out)))
        }
        ("POST", "/api/projects/codex") => {
            if let Some(r) = ctx.yours(req).await {
                return Ok(Some(r));
            }
            let body = req.json_body().await?;
            let fingerprint = body.get("fingerprint").cloned().unwrap_or(Value::Null);
            if !truthy(Some(&fingerprint)) {
                return Ok(Some(Reply::error(400, "Look at what will be sent first")));
            }
            Ok(Some(Reply::json(200, state.ask_codex_about_projects(fingerprint, body.get("via").and_then(Value::as_str) == Some("decider")).await?)))
        }
        ("DELETE", "/api/projects/codex") => {
            if let Some(r) = ctx.yours(req).await {
                return Ok(Some(r));
            }
            Ok(Some(Reply::json(200, state.forget_codex_picks().await?)))
        }
        _ => Ok(None),
    }
}

async fn identities(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let state = &ctx.cores.state;
    let route = req.path.clone();
    let m = req.method.clone();
    if m == "GET" && route == "/api/identities" {
        return Ok(Some(Reply::json(200, state.identity_detection()?)));
    }
    if m == "POST" && ["/api/identities/name", "/api/identities/hidden", "/api/identities/pin"].contains(&route.as_str()) {
        if let Some(r) = ctx.yours(req).await {
            return Ok(Some(r));
        }
    }
    let get = |b: &Value, k: &str| b.get(k).cloned().unwrap_or(Value::Null);
    match (m.as_str(), route.as_str()) {
        ("POST", "/api/identities/name") => {
            let b = req.json_body().await?;
            Ok(Some(Reply::json(200, state.name_identity(get(&b, "id"), get(&b, "name")).await?)))
        }
        ("POST", "/api/identities/hidden") => {
            let b = req.json_body().await?;
            Ok(Some(Reply::json(200, state.hide_identity(get(&b, "id"), get(&b, "hidden")).await?)))
        }
        ("POST", "/api/identities/pin") => {
            let b = req.json_body().await?;
            Ok(Some(Reply::json(200, state.pin_identity(get(&b, "nodeId"), get(&b, "identity")).await?)))
        }
        ("POST", "/api/identities/rescan") => Ok(Some(Reply::json(200, state.rescan_identities().await?))),
        _ => Ok(None),
    }
}

/// Spotify, as a remote for the app you use: GET /api/spotify, the
/// window's POST /app, /connect, /player, /settings, your /disconnect, and
/// /callback, Spotify sending you back.
async fn spotify(ctx: &Arc<Ctx>, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let s = &ctx.cores.spotify;
    let route = req.path.clone();
    let m = req.method.clone();
    let out: Result<Option<Reply>, CoreError> = async {
        if m == "GET" && route == "/api/spotify" {
            let mut st = s.status().await?;
            let now = if truthy(st.get("connected")) { s.player().await.unwrap_or(Value::Null) } else { Value::Null };
            st["now"] = now;
            return Ok(Some(Reply::json(200, st)));
        }
        if m == "GET" && route == "/api/spotify/callback" {
            let page = |title: &str, text: &str| Reply::raw(200, format!("<!doctype html><meta charset=\"utf-8\"><title>XenoNet</title><body style=\"font:15px system-ui;background:#05070d;color:#e8f4ff;display:grid;place-items:center;height:100vh;margin:0\"><div><h2>{title}</h2><p>{text}</p></div></body>"), &[("content-type", "text/html; charset=utf-8")]);
            let (code, state, error) = (req.param("code").map(str::to_string), req.param("state").map(str::to_string), req.param("error").map(str::to_string));
            return Ok(Some(match s.callback(code.as_deref(), state.as_deref(), error.as_deref()).await {
                Ok(()) => {
                    crate::serve::poll_spotify_soon(ctx, 0);
                    page("Spotify is connected", "You can close this tab and go back to XenoNet.")
                }
                Err(e) if e.is_pending() => pending_reply(&e),
                Err(e) => page("Spotify isn't connected", &e.message.replace(['<', '>', '&'], "")),
            }));
        }
        if m == "POST" && route == "/api/spotify/disconnect" {
            if let Some(r) = ctx.yours(req).await {
                return Ok(Some(r));
            }
            return Ok(Some(Reply::json(200, s.disconnect().await?)));
        }
        if m == "POST" {
            if !ctx.window_only(req) {
                return Ok(Some(Reply::coded(403, "yours", "this is for XenoNet's own window")));
            }
            let body = req.json(2048).await.ok().filter(|b| !b.is_null()).unwrap_or_else(|| json!({}));
            match route.as_str() {
                "/api/spotify/app" => return Ok(Some(Reply::json(200, s.set_app(body.get("clientId").unwrap_or(&Value::Null))?))),
                "/api/spotify/connect" => return Ok(Some(Reply::json(200, s.begin()?))),
                "/api/spotify/settings" => {
                    let device = if body.get("device").and_then(Value::as_str) == Some("current") {
                        s.player().await.ok().and_then(|p| p.get("device").and_then(|d| d.get("id")).cloned()).unwrap_or(Value::Null)
                    } else {
                        body.get("device").cloned().unwrap_or(Value::Null)
                    };
                    return Ok(Some(Reply::json(200, s.set(&device, body.get("duck").unwrap_or(&Value::Null))?)));
                }
                "/api/spotify/player" => {
                    let op = body.get("op").filter(|o| !o.is_null()).map(js_string).unwrap_or_default();
                    s.command(&op, body.get("value").unwrap_or(&Value::Null)).await?;
                    ctx.music.locked().last.clear();
                    crate::serve::poll_spotify_soon(ctx, 400);
                    return Ok(Some(Reply::json(200, json!({ "ok": true }))));
                }
                _ => {}
            }
        }
        Ok(None)
    }
    .await;
    match out {
        Err(e) if e.is(Kind::Spotify) => Ok(Some(Reply::send(e.status.unwrap_or(400), &json!({ "error": e.message, "code": e.code }), &[]))),
        other => other,
    }
}

/// Dev only: puts sessions into states that are otherwise rare, so the
/// visuals can be checked (server/index.js's simulate). What it did, or
/// None for an unknown simulation or session.
fn simulate(ctx: &Arc<Ctx>, kind: &Value, body: &Value) -> Result<Option<Value>, CoreError> {
    let cores = &ctx.cores;
    let kind_s = match kind {
        Value::String(s) => s.clone(),
        Value::Null => "undefined".into(),
        v => js_string(v),
    };
    let str_arg = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    let (a, b) = (str_arg("a"), str_arg("b"));
    if kind_s == "actor-request" {
        return match &cores.registration {
            Some(r) => r.simulate(&kind_s, a),
            None => Ok(None),
        };
    }
    // A read through an actor's made-up tunnel, as a remote actor's read
    // reaches the page.
    if kind_s == "actor-read" {
        let Some(actors) = cores.actors.as_ref().filter(|a| a.is_demo()) else { return Ok(None) };
        let a = a.or_else(|| actors.ids().into_iter().find(|id| actors.remote_of(id).is_some_and(|r| truthy(r.get("active")))));
        let Some(a) = a.filter(|a| actors.remote_of(a).is_some()) else { return Ok(None) };
        let reach: Vec<Value> = match b {
            Some(b) => vec![json!(b)],
            None => {
                let mut seen = Vec::new();
                for t in actors.targets_of(&a) {
                    if !seen.contains(&t["id"]) {
                        seen.push(t["id"].clone());
                    }
                }
                seen.into_iter().take(4).collect()
            }
        };
        let transport = actors.remote_of(&a).and_then(|r| r.get("transports").and_then(|t| t.get(0)).cloned()).unwrap_or(Value::Null);
        ctx.broadcast("remote-read", &json!({ "actor": a, "transport": transport, "reach": reach, "at": xenonet_base::json::number(ctx.now()) }));
        return Ok(Some(json!({ "ok": true, "kind": kind_s, "a": a, "reach": reach })));
    }
    if kind_s.starts_with("actor-") {
        return match &cores.actors {
            Some(x) => x.simulate(&kind_s, a, b),
            None => Ok(None),
        };
    }
    if kind_s.starts_with("host-") {
        return match &cores.hub {
            Some(h) => h.simulate(&kind_s, body),
            None => Ok(None),
        };
    }
    if kind_s == "doctor" {
        // The doctor finding a problem, reported as a real run would be; the
        // next real run (or clear) puts back what's true.
        let problem =
            json!({ "id": "codex.auth", "group": "Codex", "title": "Codex is signed in", "status": "fail", "detail": "not signed in", "fix": "Run codex login.", "ms": 1 });
        let mut results: Vec<Value> =
            cores.doctor.last().and_then(|l| l["results"].as_array().cloned()).unwrap_or_default().into_iter().filter(|r| r["id"] != problem["id"]).collect();
        results.push(problem.clone());
        let results = Value::Array(results);
        let now = xenonet_base::json::number(ctx.now());
        cores.doctor.report(json!({ "at": now, "ms": 1, "results": results, "summary": cores.doctor.summarize(&results)? }))?;
        crate::serve::notify_doctor(ctx, &[problem]);
        return Ok(Some(json!({ "ok": true, "kind": kind_s })));
    }
    if kind_s == "clear" {
        let c = ctx.clone();
        tokio::spawn(async move {
            let _ = c.cores.doctor.check(true, false).await;
        });
        // An actor waiting on you stops waiting too.
        if let Some(x) = &cores.actors {
            let _ = x.simulate("clear", None, None);
        }
    }
    let Some(done) = cores.state.simulate(&kind_s, a, b)? else { return Ok(None) };
    ctx.events.broadcast("live", &cores.state.live_snapshot()?, Local::Same);
    Ok(Some(done))
}
