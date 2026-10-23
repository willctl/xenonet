//! The tray app's API (docs/design/always-on.md, "The tray app"; contracts
//! section 14):
//!
//!   GET  /api/tray/v1/status         a tray/status/v1
//!   GET  /api/tray/v1/events         the status again whenever it changes
//!                                    (`status`), and a tray/event/v1 when
//!                                    something starts or stops waiting on
//!                                    you (`alert`)
//!   POST /api/tray/v1/open-console   { id? }: Desktop's window, at that
//!                                    session or actor, when a Desktop is
//!                                    connected
//!
//! Every request needs the tray's key (x-xenonet-tray), which only the
//! tray app the component socket can tell is XenoNet's gets, and must be
//! yours (verify.person), so nothing works from under an agent even with
//! the key. An actor's token never does.

use crate::ctx::{Answer, Ctx};
use crate::http::{Reply, Req};
use bytes::Bytes;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_base::sync::Locked;
use xenonet_base::text::truthy;
use xenonet_engine::engine::{is_actor_agent, is_agent_id};
use xenonet_engine::sql::Arg;
use xenonet_state::needs_you;

/// The most items a status lists (core/schema/tray.js's TRAY_ITEMS).
pub const ITEMS: usize = 20;
/// How often the event stream looks for a change.
const LOOK: Duration = Duration::from_secs(2);
/// A comment line this often keeps the stream open through idle proxies.
const PING_LOOKS: u32 = 12;

/// A text cut to `max` UTF-16 units, as the schema counts them, with an
/// ellipsis when it was longer.
fn clip(s: &str, max: usize) -> String {
    if s.encode_utf16().count() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut units = 0;
    for c in s.chars() {
        units += c.len_utf16();
        if units > max - 1 {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn text_or_null(v: Option<&Value>, max: usize) -> Value {
    match v {
        Some(Value::String(s)) => json!(clip(s, max)),
        _ => Value::Null,
    }
}

/// An item of the Needs you queue, as the tray has it: its title and the
/// words the console says after it.
fn item(it: &Value) -> Value {
    let mut m = Map::new();
    m.insert("key".into(), json!(clip(it.get("key").and_then(Value::as_str).unwrap_or(""), 300)));
    m.insert("kind".into(), it.get("kind").cloned().unwrap_or(Value::Null));
    m.insert("id".into(), text_or_null(it.get("id"), 300));
    m.insert("title".into(), json!(clip(&it.get("title").map(xenonet_base::text::js_string).unwrap_or_default(), 200)));
    m.insert("words".into(), json!(needs_you::waits_words(it)));
    m.insert("app".into(), text_or_null(it.get("app"), 32));
    if let Some(h) = it.get("host").and_then(Value::as_str).filter(|h| xenonet_base::ids::is_host_id(h)) {
        m.insert("host".into(), json!(h));
    }
    m.insert("text".into(), text_or_null(it.get("text"), 500));
    m.insert("since".into(), it.get("since").filter(|s| s.is_number()).cloned().unwrap_or(Value::Null));
    if truthy(it.get("stale")) {
        m.insert("stale".into(), Value::Bool(true));
    }
    if it.get("kind").and_then(Value::as_str) == Some("requests") {
        m.insert("count".into(), it.get("count").cloned().unwrap_or(json!(0)));
    }
    Value::Object(m)
}

/// This machine's place in its network: its name, this machine's id, the
/// link to the gateway, and how many of the others are online.
fn network(ctx: &Ctx) -> Value {
    let off = json!({ "name": null, "host": null, "link": "off", "hosts": 0, "online": 0 });
    let Some(n) = ctx.net.get() else { return off };
    let s = n.state();
    if !s["network"].is_object() {
        return off;
    }
    let link = s["gateway"]["link"].as_str().filter(|l| ["off", "connecting", "live", "offline", "refused", "removed"].contains(l)).unwrap_or("offline");
    let others: Vec<&Value> = s["hosts"].as_array().map(|h| h.iter().filter(|x| x["self"] != json!(true)).collect()).unwrap_or_default();
    json!({
        "name": text_or_null(s["network"].get("name"), 64),
        "host": s["self"]["id"].as_str().filter(|h| xenonet_base::ids::is_host_id(h)),
        "link": link,
        "hosts": others.len(),
        "online": others.iter().filter(|x| x["online"] == json!(true)).count(),
    })
}

/// The actor gateway, as the tray shows it.
fn gateway(ctx: &Ctx) -> Value {
    match ctx.gateway.get() {
        Some(g) => {
            let mut b = g.brief();
            b["url"] = text_or_null(b.get("url"), 300);
            b["problem"] = text_or_null(b.get("problem"), 500);
            b
        }
        None => json!({ "on": false, "state": "off", "url": null, "problem": null }),
    }
}

/// The tray's status, and the whole Needs you queue it lists the first
/// ITEMS of.
fn look(ctx: &Ctx) -> Result<(Value, Vec<Value>), crate::cores::CoreError> {
    let summary = super::summary(ctx)?;
    let queue: Vec<Value> = needs_you::queue_of_summary(Some(&summary)).iter().map(item).collect();
    let desktop = ctx.components.locked().clone().is_some_and(|c| c.info()["desktop"] == json!(true));
    let status = json!({
        "schema": "tray/status/v1",
        "running": true,
        "version": clip(&ctx.version, 40),
        "instance": ctx.config.instance.as_deref().map(|i| clip(i, 40)),
        "url": format!("http://127.0.0.1:{}/", ctx.config.port),
        "desktop": desktop,
        "working": summary.get("working").filter(|w| w.is_u64()).cloned().unwrap_or(json!(0)),
        "needsYou": { "count": xenonet_base::json::number(needs_you::count_of(&queue)), "items": queue.iter().take(ITEMS).cloned().collect::<Vec<_>>() },
        "gateway": gateway(ctx),
        "network": network(ctx),
        "at": xenonet_base::json::number(ctx.now().floor()),
    });
    Ok((status, queue))
}

/// GET /api/tray/v1/status's answer.
pub fn status(ctx: &Ctx) -> Result<Value, crate::cores::CoreError> {
    look(ctx).map(|(s, _)| s)
}

fn alert(kind: &str, item: &Value, at: f64) -> Bytes {
    crate::events::frame("alert", &json!({ "schema": "tray/event/v1", "kind": kind, "item": item, "at": xenonet_base::json::number(at.floor()) }))
}

/// The alerts between two looks at the queue: what started waiting (not a
/// stale wait on a machine that isn't live), then what stopped.
pub fn alerts(before: &[Value], after: &[Value], at: f64) -> Vec<Bytes> {
    let key = |v: &Value| v["key"].as_str().unwrap_or("").to_string();
    let had: Vec<String> = before.iter().map(key).collect();
    let has: Vec<String> = after.iter().map(key).collect();
    let started = after.iter().filter(|it| !had.contains(&key(it)) && !truthy(it.get("stale"))).map(|it| alert("needs-you", it, at));
    let stopped = before.iter().filter(|it| !has.contains(&key(it))).map(|it| alert("resolved", it, at));
    started.chain(stopped).collect()
}

/// The event stream: the status at once, then every LOOK whatever changed.
/// Nothing is alerted for what already waited when the tray connected.
fn events(ctx: &Arc<Ctx>) -> mpsc::UnboundedReceiver<Bytes> {
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = tx.send(Bytes::from_static(b"retry: 3000\n\n"));
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(LOOK);
        let mut last: Option<(String, Vec<Value>)> = None;
        let mut looks = 0u32;
        loop {
            every.tick().await;
            if tx.is_closed() {
                return;
            }
            looks += 1;
            if looks.is_multiple_of(PING_LOOKS) && tx.send(Bytes::from_static(b": ping\n\n")).is_err() {
                return;
            }
            let Ok((status, queue)) = look(&ctx) else { continue };
            let mut same = status.clone();
            same["at"] = Value::Null;
            let shape = xenonet_base::json::stringify(&same, 0);
            if let Some((was, before)) = &last {
                for a in alerts(before, &queue, ctx.now()) {
                    if tx.send(a).is_err() {
                        return;
                    }
                }
                if *was == shape {
                    last = Some((shape, queue));
                    continue;
                }
            }
            if tx.send(crate::events::frame("status", &status)).is_err() {
                return;
            }
            last = Some((shape, queue));
        }
    });
    rx
}

/// A session's or an actor's id, here or on another machine of the network.
fn selectable(id: &str) -> bool {
    let local = |id: &str| is_agent_id(&Arg::text(id)) || is_actor_agent(&Arg::text(id));
    id.len() <= 300 && (local(id) || xenonet_base::ids::parse_qualified(id).is_some_and(|(h, l)| h.is_some() && local(l)))
}

fn refused(error: &str) -> Reply {
    Reply::send(403, &json!({ "error": error, "code": "tray" }), &[])
}

pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Answer {
    // The tray's key and nothing else: an actor's token never makes a
    // request the tray's.
    if req.header("authorization").is_some() {
        return Ok(refused("this is for XenoNet's tray app, and an actor can't use it"));
    }
    if !ctx.tray_key.matches(req.header("x-xenonet-tray")) {
        return Ok(refused("this is for XenoNet's tray app, and the request didn't come with its key"));
    }
    if let Some(r) = ctx.yours(req).await {
        return Ok(r);
    }
    let method = req.method.clone();
    match (method.as_str(), req.path.as_str()) {
        ("GET", "/api/tray/v1/status") => Ok(Reply::json(200, status(ctx)?)),
        ("GET", "/api/tray/v1/events") => {
            Ok(Reply::stream(200, &[("content-type", "text/event-stream"), ("cache-control", "no-store"), ("connection", "keep-alive")], events(ctx)))
        }
        ("POST", "/api/tray/v1/open-console") => {
            let body = req.json(4 << 10).await.unwrap_or(Value::Null);
            let id = match body.get("id") {
                None | Some(Value::Null) => None,
                Some(Value::String(id)) if selectable(id) => Some(id.clone()),
                Some(_) => return Ok(Reply::coded(400, "bad-request", "open the console at a session or an actor by its id")),
            };
            let components = ctx.components.locked().clone();
            let shown = match components {
                Some(c) => c.show(id.as_deref()).await,
                None => false,
            };
            let url = format!("http://127.0.0.1:{}/", ctx.config.port);
            Ok(Reply::json(200, json!({ "ok": true, "opened": if shown { json!("desktop") } else { Value::Null }, "url": url })))
        }
        _ => Ok(Reply::coded(404, "not-found", "XenoNet's tray API has no such route")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_schema::check::{compile, Checker, Data};

    const STATUS: &str = include_str!("../../../../schemas/tray/status/v1.json");
    const EVENT: &str = include_str!("../../../../schemas/tray/event/v1.json");

    fn checker() -> Checker {
        let docs: Vec<Value> = [STATUS, EVENT].iter().map(|t| xenonet_base::json::parse(t).unwrap()).collect();
        compile(&docs).unwrap()
    }

    fn fits(c: &Checker, root: usize, v: &Value) -> bool {
        c.check(c.root(root), &Data::from(v), true).ok()
    }

    #[test]
    fn the_fixtures_the_tray_reads_are_what_the_schemas_say() {
        let c = checker();
        let status = xenonet_base::json::parse(include_str!("../../../../test/fixtures/tray/status.json")).unwrap();
        let event = xenonet_base::json::parse(include_str!("../../../../test/fixtures/tray/event.json")).unwrap();
        assert!(fits(&c, 0, &status));
        assert!(fits(&c, 1, &event));
        assert!(!fits(&c, 0, &event), "an alert isn't a status");
    }

    #[test]
    fn items_say_what_waits_in_the_consoles_words_and_fit_the_schema() {
        let q = needs_you::queue_of_summary(Some(&json!({
            "needsYou": [
                { "id": "claude:0190d3e0-0000-7000-8000-000000000001", "app": "claude", "title": "x".repeat(400), "kind": "question", "text": "Per key?", "since": 2000 },
                { "id": "@studio:codex:a", "app": "codex", "title": "Move", "kind": "permission", "since": 1000, "host": "studio", "stale": true },
            ],
            "actorsNeedYou": [{ "id": "actor:wren", "name": "Wren", "kind": "review", "text": "", "since": 500 }],
            "requests": 2,
        })));
        let items: Vec<Value> = q.iter().map(item).collect();
        let words: Vec<(&str, &str)> = items.iter().map(|i| (i["kind"].as_str().unwrap(), i["words"].as_str().unwrap())).collect();
        assert_eq!(words, [("actor", "is ready for review"), ("question", "asks"), ("requests", "ask to join XenoNet"), ("permission", "needs permission")]);
        assert_eq!(items[1]["title"].as_str().unwrap().encode_utf16().count(), 200, "a long title is cut to fit");
        assert_eq!((items[2]["count"].clone(), items[3]["host"].clone(), items[3]["stale"].clone()), (json!(2), json!("studio"), json!(true)));
        let status = json!({
            "schema": "tray/status/v1", "running": true, "version": "0.4.1", "instance": "demo", "url": "http://127.0.0.1:48321/",
            "desktop": false, "working": 1, "needsYou": { "count": 5, "items": items },
            "gateway": { "on": false, "state": "off", "url": null, "problem": null },
            "network": { "name": null, "host": null, "link": "off", "hosts": 0, "online": 0 }, "at": 1,
        });
        assert!(fits(&checker(), 0, &status));
    }

    #[test]
    fn alerts_say_what_started_and_stopped_waiting_but_not_a_stale_wait() {
        let it =
            |key: &str, stale: bool| json!({ "key": key, "kind": "question", "id": null, "title": key, "words": "asks", "app": null, "text": null, "since": null, "stale": stale });
        let before = [it("a", false), it("b", false)];
        let after = [it("b", false), it("c", false), it("d", true)];
        let said: Vec<String> = alerts(&before, &after, 7.0).iter().map(|b| String::from_utf8_lossy(b).into_owned()).collect();
        assert_eq!(said.len(), 2);
        assert!(said[0].starts_with("event: alert\ndata: {") && said[0].contains("\"kind\":\"needs-you\"") && said[0].contains("\"key\":\"c\""), "{}", said[0]);
        assert!(said[1].contains("\"kind\":\"resolved\"") && said[1].contains("\"key\":\"a\""), "{}", said[1]);
        let event = xenonet_base::json::parse(said[0].trim_start_matches("event: alert\ndata: ").trim()).unwrap();
        assert!(fits(&checker(), 1, &event));
    }

    #[test]
    fn the_console_opens_at_a_session_or_an_actor_and_nothing_else() {
        assert!(selectable("claude:0190d3e0-0000-7000-8000-000000000001"));
        assert!(selectable("actor:wren"));
        assert!(selectable("@studio:actor:wren"));
        for bad in ["", "project:/x", "https://example.com", "@studio:", "javascript:alert(1)"] {
            assert!(!selectable(bad), "{bad}");
        }
    }

    #[test]
    fn clipping_counts_as_the_schema_does() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("😀😀😀", 4), "😀…");
    }
}
