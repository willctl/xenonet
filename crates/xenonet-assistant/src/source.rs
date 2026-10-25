//! core/assistant/source.js: what the assistant reads from, XenoNet's own
//! state as the service holds it in memory, never the map with other
//! hosts' in it. A view is what projection.rs takes: each source's health,
//! the places, the sessions with what the tailer saw of each, the
//! collisions and the actors. One is made at most every second.
//!
//! The state and the actors' registry are other modules' (core/state.js,
//! core/actors/): the twin hands their contents over, and answers what
//! the view asks of them (a session's place, a product, whether an actor
//! is registered).

use serde_json::{json, Map, Value};
use xenonet_app::jsv::truthy;

pub const VIEW_MS: f64 = 1000.0;

fn or_null(v: Option<&Value>) -> Value {
    v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null)
}

/// How an app's source stands, from its session list's gatherer and the
/// tailer (their health(), or None): ready while both work, degraded when
/// one has a problem, unavailable when the list failed or hasn't run yet.
pub fn source_of(g: Option<&Value>, l: Option<&Value>, t: f64) -> Value {
    let num = xenonet_base::json::number;
    let Some(g) = g.filter(|g| truthy(Some(g))) else { return json!({ "state": "unavailable", "observedAt": null, "coverage": "unknown", "reason": "not_started" }) };
    let state = g.get("state").and_then(Value::as_str);
    let last_ok = g.get("lastOk").filter(|v| !v.is_null());
    let Some(last_ok) = last_ok.filter(|_| state != Some("idle")) else {
        return json!({ "state": "unavailable", "observedAt": null, "coverage": "unknown", "reason": "not_started" });
    };
    if matches!(state, Some("failed" | "stopped")) {
        return json!({ "state": "unavailable", "observedAt": last_ok, "coverage": "unknown", "reason": if state == Some("stopped") { "stopped" } else { "read_failed" } });
    }
    let tailing = l.and_then(|l| l.get("state")).and_then(Value::as_str) == Some("running");
    let degraded = state == Some("degraded") || !tailing;
    let live_at = if tailing { Some(num(t)) } else { l.and_then(|l| l.get("lastOk")).filter(|v| !v.is_null()).cloned() };
    let observed = match live_at {
        None => last_ok.clone(),
        Some(at) => {
            let (a, b) = (xenonet_base::text::js_number(Some(last_ok)), xenonet_base::text::js_number(Some(&at)));
            num(if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) })
        }
    };
    json!({ "state": if degraded { "degraded" } else { "ready" }, "observedAt": observed, "coverage": if degraded { "partial" } else { "complete" }, "reason": if degraded { "read_failed" } else { "ok" } })
}

/// What the view asks of the modules around it.
pub trait Around {
    /// state.homeOf(id): the place a session lives in.
    fn home_of(&mut self, id: &Value) -> Value;
    /// productOf(id)?.id ?? null, from core/actors/products.js.
    fn product_of(&mut self, id: Option<&Value>) -> Value;
    /// actors.isRegistered(id).
    fn registered(&mut self, id: &Value) -> bool;
}

fn targets(a: &Value) -> Value {
    Value::Array(
        a.get("targets").filter(|t| !t.is_null()).and_then(Value::as_array).into_iter().flatten().map(|x| json!({ "id": x.get("id"), "relation": x.get("relation") })).collect(),
    )
}

fn via(a: &Value) -> Value {
    a.get("via").and_then(Value::as_array).cloned().map_or_else(|| json!([]), Value::Array)
}

/// A view at `t`, from the map's nodes (state.index's values), the live
/// state (state.liveSnapshot()), the registry's actors and the tailed
/// apps' sources.
#[expect(clippy::too_many_arguments, reason = "the JS's view and its inputs, which xenonet-node passes one by one")]
pub fn view(t: f64, nodes: &[Value], live: &Value, actors: &[Value], has_actors: bool, claude: Value, codex: Value, grok: Value, around: &mut dyn Around) -> Value {
    let mut live_by_id: Map<String, Value> = Map::new();
    for s in live.get("sessions").and_then(Value::as_array).into_iter().flatten() {
        live_by_id.insert(s.get("id").map(xenonet_base::text::js_string).unwrap_or_default(), s.clone());
    }
    let mut places: Vec<(Value, Value)> = Vec::new();
    let mut sessions = Vec::new();
    for n in nodes {
        let kind = n.get("kind").and_then(Value::as_str);
        let host = truthy(n.get("host"));
        if matches!(kind, Some("project" | "zone")) && !host {
            let id = n.get("id").cloned().unwrap_or(Value::Null);
            let entry = json!({ "name": or_null(n.get("name")), "kind": kind });
            match places.iter_mut().find(|(k, _)| xenonet_app::jsv::strict_eq(Some(k), Some(&id))) {
                Some(slot) => slot.1 = entry,
                None => places.push((id, entry)),
            }
        }
        if kind != Some("session") || host {
            continue;
        }
        let id = n.get("id").cloned().unwrap_or(Value::Null);
        let l = live_by_id.get(&xenonet_base::text::js_string(&id));
        let live = l.map_or(Value::Null, |l| {
            let mut m = Map::new();
            if let Some(s) = l.get("status") {
                m.insert("status".into(), s.clone());
            }
            let need = l.get("needsYou").filter(|v| truthy(Some(v)));
            m.insert("needsYou".into(), need.map_or(Value::Null, |n| json!({ "kind": n.get("kind"), "since": or_null(n.get("since")) })));
            let stuck = l.get("stuck").filter(|v| truthy(Some(v)));
            m.insert("stuck".into(), stuck.map_or(Value::Null, |s| json!({ "reasons": s.get("reasons").cloned().unwrap_or_else(|| json!([])) })));
            for k in ["turnStartedAt", "lastTurnEndAt", "interruptedAt", "lastEventAt"] {
                m.insert(k.into(), or_null(l.get(k)));
            }
            Value::Object(m)
        });
        let place = around.home_of(&id);
        let mut s = Map::new();
        s.insert("id".into(), id);
        if let Some(app) = n.get("app") {
            s.insert("app".into(), app.clone());
        }
        s.insert("place".into(), place);
        s.insert("title".into(), or_null(n.get("title")));
        s.insert("updatedAt".into(), or_null(n.get("updatedAt")));
        s.insert("archived".into(), json!(truthy(n.get("archived"))));
        s.insert("live".into(), live);
        sessions.push(Value::Object(s));
    }
    let actors: Vec<Value> = actors
        .iter()
        .map(|a| {
            let id = a.get("id").cloned().unwrap_or(Value::Null);
            let product = around.product_of(a.get("product").and_then(|p| p.get("id")));
            let reachable = has_actors && around.registered(&id) && a.get("presence").and_then(Value::as_str) != Some("offline");
            actor_entry(a, product, reachable)
        })
        .collect();
    let collisions: Vec<Value> = live
        .get("collisions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| json!({ "a": c.get("a"), "b": c.get("b"), "files": files_of(c.get("files")), "ts": or_null(c.get("ts")) }))
        .collect();
    json!({
        "at": xenonet_base::json::number(t),
        "fixture": false,
        "sources": { "claude": claude, "codex": codex, "grok": grok },
        "places": places.into_iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "sessions": sessions,
        "collisions": collisions,
        "actors": actors,
    })
}

// `files?.length ?? 0`.
fn files_of(f: Option<&Value>) -> Value {
    match f {
        Some(Value::Array(a)) => json!(a.len()),
        Some(Value::String(s)) => json!(xenonet_app::jstext::utf16_len(s)),
        Some(Value::Object(o)) => o.get("length").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(0)),
        _ => json!(0),
    }
}

fn actor_entry(a: &Value, product: Value, reachable: bool) -> Value {
    let mut m = Map::new();
    for k in ["id", "kind"] {
        if let Some(v) = a.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    m.insert("productId".into(), product);
    for k in ["name", "presence"] {
        if let Some(v) = a.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    m.insert("via".into(), via(a));
    m.insert("remote".into(), or_null(a.get("remote")));
    m.insert("reachable".into(), json!(reachable));
    m.insert("targets".into(), targets(a));
    Value::Object(m)
}

/// An actor as the assistant shows itself, from the registry's snapshot.
pub fn actor(snapshot: &Value, product: Value) -> Value {
    actor_entry(snapshot, product, false)
}
