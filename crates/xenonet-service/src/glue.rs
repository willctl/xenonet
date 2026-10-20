//! What server/index.js does when its cores say something: each event to
//! the pages, and the notifications the desktop app shows. A core in Rust
//! is given `Glue::event` to call with its events, as the JS cores' emitters
//! are listened to.

use crate::ctx::Ctx;
use crate::events::Local;
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_base::sync::Locked;

/// The events the cores send, by where they come from.
pub enum From {
    /// XenoNetState: pulse, activity, refusal, tokens, graph, live.
    State,
    /// The engine: message.
    Engine,
    /// The delivery router: delivery, error.
    Delivery,
    /// Actors: actors, event.
    Actors,
    /// Registration: requests, asked, asked-gateway.
    Registration,
    /// Wakes: mail (a message for an actor nothing can wake).
    Wakes,
    /// The hub: graph, live, activity, tokens, pulse, agent-message, hosts.
    Hub,
    /// The assistant's tunnels: changed.
    Tunnels,
    /// The doctor: result, problems.
    Doctor,
}

fn first_upper(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The first `n` UTF-16 units of a string, as JS's slice(0, n) takes them.
fn cut(s: &str, n: usize) -> String {
    xenonet_base::log::js_slice(s, n).to_string()
}

pub fn event(ctx: &Arc<Ctx>, from: From, name: &str, data: Value) {
    let cores = &ctx.cores;
    match (from, name) {
        (From::State, "pulse" | "activity" | "tokens") => ctx.broadcast(name, &data),
        (From::State, "refusal") => refusal(ctx, data),
        (From::State, "graph") => {
            // The map goes out with other hosts' in it, with hosts on.
            match &cores.hub {
                Some(h) => ctx.events.broadcast("graph", &h.graph(data.clone()), Local::Own(&data)),
                None => ctx.broadcast("graph", &data),
            }
            if let Some(a) = &cores.actors {
                a.refresh();
            }
        }
        (From::State, "live") => match &cores.hub {
            Some(h) => ctx.events.broadcast("live", &h.live(data.clone()), Local::Own(&data)),
            None => ctx.broadcast("live", &data),
        },
        (From::Engine, "message") => {
            ctx.broadcast("agent-message", &data);
            // A message an actor sent becomes its said event, never its text.
            if let Some(a) = &cores.actors {
                a.said(&data);
            }
        }
        (From::Delivery, "delivery") => ctx.broadcast("delivery", &data),
        (From::Delivery, "error") => eprintln!("[xenonet] delivery: {}", xenonet_base::text::js_string(&data["message"])),
        // A message for an actor nothing can wake: a notification for you.
        (From::Delivery, "notify") => {
            if ctx.config.demo {
                return;
            }
            let (agent, title, from) = (js(&data["agent"]), js(&data["title"]), js(&data["from"]));
            ctx.notify_desktop(json!({ "type": "notify", "key": format!("delivery:{agent}"), "id": agent, "title": "A message wasn't picked up", "subtitle": title, "body": format!("{from}'s message to {title} is still waiting: the session hasn't started a turn in a minute.") }));
        }
        (From::Actors, "actors") => ctx.broadcast("actors", &data),
        (From::Actors, "event") => ctx.broadcast("actor-event", &data),
        (From::Registration, "requests") => ctx.broadcast("actor-requests", &data),
        // A registration waiting for you: a notification, which opens the
        // Actors panel. The demo stays quiet.
        (From::Registration, "asked") => {
            if !ctx.config.demo {
                ctx.notify_desktop(json!({ "type": "notify", "key": "actor-request", "command": "actors", "title": "An actor asks to join XenoNet", "subtitle": format!("\"{}\" waits for you to allow it", js(&data["name"])), "body": "Allow or deny it in XenoNet's Actors panel." }));
            }
        }
        // A session asked for an actor's gateway token: whose, never a token.
        (From::Registration, "asked-gateway") => {
            if !ctx.config.demo {
                ctx.notify_desktop(json!({ "type": "notify", "key": format!("gateway-ask:{}", js(&data["actor"])), "command": "actors", "title": "A session asks for a gateway token", "subtitle": format!("For {}'s app", js(&data["name"])), "body": "Allow it in XenoNet's Actors panel. The token is made there and shown masked." }));
            }
        }
        // How many messages wait, never who sent them or what they say.
        (From::Wakes, "mail") => {
            if ctx.config.demo {
                return;
            }
            let name = js(&data["name"]);
            let unread = data["unread"].as_f64().unwrap_or(0.0);
            let n = if unread == 1.0 { "a message".to_string() } else { format!("{} messages", xenonet_base::json::number_string(unread)) };
            ctx.notify_desktop(json!({ "type": "notify", "key": format!("actor-mail:{}", js(&data["actor"])), "command": "actors", "title": format!("{name} has {n} waiting"), "subtitle": format!("Nothing can wake {name}, so it reads them when it next checks"), "body": "Ask it to check its XenoNet inbox, or set up a wake in the Actors panel." }));
        }
        (From::Hub, _) => ctx.events.broadcast(name, &data, Local::None),
        // The window reads how they stand with its key; this says only that
        // it changed.
        (From::Tunnels, "changed") => ctx.broadcast("assistant", &json!({ "at": xenonet_base::json::number(ctx.now()) })),
        (From::Doctor, "result") => ctx.broadcast("doctor", &data),
        (From::Doctor, "problems") => {
            let fresh = data.as_array().cloned().unwrap_or_default();
            notify_doctor(ctx, &fresh);
            // Each new problem goes in the log too, so a failed check is
            // still there to read after the next run passes.
            for p in &fresh {
                let level = if p["status"] == "fail" { "error" } else { "warn" };
                let msg = format!("{}: {}", js(&p["title"]), js(&p["detail"]));
                ctx.log(level, &format!("doctor.{}", js(&p["id"])), &msg, None, p["fix"].as_str());
            }
        }
        _ => {}
    }
}

fn js(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        v => xenonet_base::text::js_string(v),
    }
}

/// New problems the doctor found, as a notification that opens it.
pub fn notify_doctor(ctx: &Ctx, fresh: &[Value]) {
    if ctx.config.demo || fresh.is_empty() {
        return;
    }
    let worst = fresh.iter().find(|r| r["status"] == "fail").unwrap_or(&fresh[0]);
    let title = if fresh.len() == 1 { "XenoNet found a problem".to_string() } else { format!("XenoNet found {} problems", fresh.len()) };
    let detail = first_upper(&js(&worst["detail"]));
    ctx.notify_desktop(json!({
        "type": "notify",
        "key": "doctor",
        "command": "doctor",
        "title": title,
        "subtitle": cut(&format!("{}: {detail}", js(&worst["group"])), 120),
        "body": worst.get("fix").filter(|f| !f.is_null()).cloned().unwrap_or(json!("Open XenoNet's doctor for the details.")),
    }));
}

// Every provider refusal XenoNet sees, kept; a new one seen live is a line
// in the log and, a few an hour at most, a notification.
fn refusal(ctx: &Arc<Ctx>, ev: Value) {
    let rec = match ctx.cores.refusals.record(&ev["signal"], &ev) {
        Ok(Some(r)) => r,
        Ok(None) => return,
        Err(e) if e.is_pending() => return,
        Err(e) => {
            ctx.log("warn", "refusals.record", &format!("couldn't keep a refusal: {}", e.message), None, None);
            return;
        }
    };
    if !xenonet_base::text::truthy(ev.get("live")) {
        return;
    }
    let session = js(&ev["sessionId"]);
    let category = rec["signal"]["category"].as_str().map(|c| format!(" ({c})")).unwrap_or_default();
    ctx.log("info", "refusals.seen", &format!("{} {}{category} in {session}", js(&rec["provider"]), js(&rec["kind"])), None, None);
    ctx.broadcast("refusal", &json!({ "id": rec["id"], "at": rec["at"], "provider": rec["provider"], "kind": rec["kind"], "category": rec["signal"]["category"], "session": ev["sessionId"], "context": xenonet_base::text::truthy(rec.get("context")) }));
    let t = ctx.now();
    let mut notes = ctx.refusal_notes.locked();
    notes.at.retain(|x| t - x < 3600e3);
    let last = notes.by_session.get(&session).copied().unwrap_or(0.0);
    if ctx.config.demo || rec["kind"] != "refusal" || notes.at.len() >= 6 || t - last < 10.0 * 60e3 {
        return;
    }
    notes.at.push(t);
    notes.by_session.insert(session.clone(), t);
    drop(notes);
    let node = ctx.cores.state.node(&session).ok().flatten();
    let title = node.as_ref().and_then(|n| n.get("title")).and_then(Value::as_str).map_or_else(|| session.clone(), str::to_string);
    let provider = if rec["provider"] == "openai" { "OpenAI" } else { "Anthropic" };
    let body = rec["signal"]["explanation"].as_str().or_else(|| rec["signal"]["category"].as_str()).unwrap_or("XenoNet kept a record of it.");
    ctx.notify_desktop(json!({ "type": "notify", "key": format!("refusal:{session}"), "id": session, "title": format!("{provider} refused a turn"), "subtitle": cut(&title, 80), "body": cut(body, 160) }));
}
