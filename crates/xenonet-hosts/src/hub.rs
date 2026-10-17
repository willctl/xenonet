//! core/hosts/hub.js: the console's side of hosts. A link to each host in
//! hosts.json, each host's last snapshot kept in the cache folder, and the
//! merged view the server sends in place of this machine's own: the map at
//! most once a second, the live state at most every half second, and the
//! hosts' events with their ids qualified. Nothing a host sends changes
//! anything here; hosts stay read-only.
//!
//! This has the hub's decisions; its emitter, timers and links are the
//! twin's.

use serde_json::{json, Map, Value};
use xenonet_app::jsv::truthy;
use xenonet_base::text::js_string;

pub const GRAPH_EVERY_MS: f64 = 1000.0;
pub const LIVE_EVERY_MS: f64 = 500.0;
pub const CACHE_EVERY_MS: f64 = 30e3;
pub const TEST_MS: f64 = 30e3;

/// How long to wait before the next of a kind goes out: at most once per
/// `every`, the last change always going.
pub fn soon_wait(last: f64, every: f64, now: f64) -> f64 {
    (last + every - now).max(0.0)
}

/// How long until a host's snapshot is kept again.
pub fn cache_wait(last_cache_at: Option<f64>, now: f64) -> f64 {
    (last_cache_at.unwrap_or(f64::NEG_INFINITY) + CACHE_EVERY_MS - now).max(0.0)
}

/// A link's info as the hub gives it: with whether its snapshot came from
/// the cache (and the hello that came with it while the host isn't live),
/// and a simulated clock skew.
pub fn info_of(link_info: &Value, cached: bool, cached_hello: &Value, skew_ms: f64) -> Value {
    let mut info = link_info.as_object().cloned().unwrap_or_default();
    info.insert("cached".into(), json!(cached));
    if cached && info.get("state").and_then(Value::as_str) != Some("live") && info.get("hello").is_none_or(Value::is_null) {
        info.insert("hello".into(), cached_hello.clone());
    }
    if skew_ms != 0.0 {
        let o = info.get("offsetMs").and_then(Value::as_f64).unwrap_or(0.0);
        info.insert("offsetMs".into(), xenonet_base::json::number(o + skew_ms));
    }
    Value::Object(info)
}

/// How a host stands, for GET /api/hosts and the doctor.
pub fn view(info: &Value, entry: &Value, snapshot: &Value, cached: bool, simulated: bool, doctor: &Value) -> Value {
    let hello = info.get("hello");
    let field = |v: Option<&Value>| v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
    let nodes: Vec<Value> = snapshot.get("graph").and_then(|g| g.get("nodes")).and_then(Value::as_array).cloned().unwrap_or_default();
    let id = entry.get("id").cloned().unwrap_or(Value::Null);
    let hello_id = hello.and_then(|h| h.get("id"));
    let calls = if truthy(hello_id) && hello_id != Some(&id) { hello_id.cloned().unwrap() } else { field(entry.get("name")) };
    let mut m = Map::new();
    let put = |m: &mut Map<String, Value>, k: &str, v: Option<&Value>| {
        if let Some(v) = v {
            m.insert(k.into(), v.clone());
        }
    };
    m.insert("id".into(), id.clone());
    for k in ["state", "since", "lastHeard", "offsetMs", "rttMs"] {
        put(&mut m, k, info.get(k));
    }
    m.insert("version".into(), field(hello.and_then(|h| h.get("version"))));
    m.insert("platform".into(), field(hello.and_then(|h| h.get("platform"))));
    m.insert("demo".into(), field(hello.and_then(|h| h.get("demo"))));
    m.insert("calls".into(), calls);
    put(&mut m, "error", info.get("error"));
    put(&mut m, "nextTryAt", info.get("nextTryAt"));
    m.insert("cached".into(), json!(cached));
    m.insert("simulated".into(), json!(simulated));
    m.insert("nodes".into(), json!(nodes.iter().filter(|x| x.get("id") != Some(&json!("core"))).count()));
    m.insert("sessions".into(), json!(nodes.iter().filter(|x| x.get("kind") == Some(&json!("session"))).count()));
    put(&mut m, "transport", entry.get("transport"));
    put(&mut m, "target", info.get("target"));
    put(&mut m, "port", entry.get("port"));
    put(&mut m, "command", entry.get("command"));
    put(&mut m, "sessionsSince", entry.get("since"));
    put(&mut m, "detail", entry.get("detail"));
    put(&mut m, "key", entry.get("key"));
    put(&mut m, "addedAt", entry.get("addedAt"));
    put(&mut m, "counts", info.get("counts"));
    put(&mut m, "lastDrop", info.get("lastDrop"));
    m.insert("doctor".into(), doctor.clone());
    Value::Object(m)
}

/// The page hears how each host stands, without how it's reached: the ssh
/// target's user and machine names can hold a person's name.
pub fn for_page(view: &Value) -> Value {
    let mut m = view.as_object().cloned().unwrap_or_default();
    for k in ["target", "key", "command", "counts", "lastDrop"] {
        m.shift_remove(k);
    }
    Value::Object(m)
}

/// A host's own doctor, as its state shows it: its summary's counts and
/// its problems (the first 20), and nothing else.
pub fn doctor_of(data: &Value) -> Value {
    let summary = match data.get("summary") {
        Some(s @ Value::Object(_)) => {
            let mut m = Map::new();
            for k in ["ok", "info", "warn", "fail", "skip"] {
                let v = s.get(k).and_then(Value::as_f64).filter(|n| n.fract() == 0.0 && *n >= 0.0 && n.abs() <= 9007199254740991.0).unwrap_or(0.0);
                m.insert(k.into(), xenonet_base::json::number(v));
            }
            Value::Object(m)
        }
        _ => Value::Null,
    };
    let text = |p: &Value, k: &str, max: usize| xenonet_app::jstext::slice16(&p.get(k).filter(|v| !v.is_null()).map_or_else(String::new, js_string), max);
    let problems: Vec<Value> = data
        .get("problems")
        .and_then(Value::as_array)
        .map(|ps| {
            ps.iter()
                .filter(|p| p.is_object())
                .take(20)
                .map(|p| json!({ "id": text(p, "id", 60), "title": text(p, "title", 120), "status": if p.get("status") == Some(&json!("fail")) { "fail" } else { "warn" }, "detail": text(p, "detail", 300) }))
                .collect()
        })
        .unwrap_or_default();
    json!({ "at": data.get("at").and_then(Value::as_f64).map_or(Value::Null, xenonet_base::json::number), "summary": summary, "problems": problems })
}

/// What host-skew moves a host's clock by: ms (or b) as a number, three
/// minutes when neither is given or it isn't one.
pub fn skew_of(ms: &Value, b: &Value) -> f64 {
    if ms.is_null() && b.is_null() {
        return 180e3;
    }
    let v = if ms.is_null() { b } else { ms };
    let n = xenonet_base::text::js_number(Some(v));
    if n.is_finite() {
        n
    } else {
        180e3
    }
}

/// What a probe (xenonet hosts test) found of the snapshot: its nodes, its
/// sessions and its live sessions.
pub fn probe_snapshot(graph: &Value, live: &Value) -> Value {
    let nodes: Vec<Value> = graph.get("nodes").and_then(Value::as_array).cloned().unwrap_or_default();
    json!({
        "nodes": nodes.iter().filter(|x| x.get("id") != Some(&json!("core"))).count(),
        "sessions": nodes.iter().filter(|x| x.get("kind") == Some(&json!("session"))).count(),
        "live": live.get("sessions").and_then(Value::as_array).map_or(0, Vec::len),
    })
}
