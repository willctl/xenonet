//! core/assistant/fixtures.js: made-up data for remote actors, what a
//! grant reads until you've verified who can call its endpoint, and what
//! `xenonet mcp assistant --fixture` serves with no XenoNet at all. A view
//! in source.rs's shape, on a six-minute cycle of two-minute phases.
//! Nothing in it comes from this computer.
//!
//! Places here are [id, value] pairs, where the JS had a Map.

use crate::crypto::sha256;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;

pub const PHASE_MS: f64 = 2.0 * 60e3;

fn session(n: u64) -> String {
    format!("0190fa00-0000-7000-8000-{n:012}")
}

// The made-up sessions' apps and the sources, as fixture.json has them:
// data, so the code names no app.
fn data() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("fixture.json")).unwrap_or(Value::Null))
}

// A made-up session's app and id, by its part in the fixture.
fn part(role: &str) -> (Value, String) {
    let s = &data()["sessions"][role];
    let app = s["app"].as_str().unwrap_or_default();
    (json!(app), format!("{app}:{}", session(s["n"].as_u64().unwrap_or_default())))
}

pub const ATLAS: &str = "fixture:atlas-api";
pub const LEDGER: &str = "fixture:ledger";
pub const ATLAS_PROJECT: &str = "prj_0190fa00-0000-4000-8000-000000000001";
pub const LEDGER_PROJECT: &str = "prj_0190fa00-0000-4000-8000-000000000002";
pub const WATCHER: &str = "actor:fixture-watcher";

/// FIXTURE, as the JS exports it.
pub fn fixture() -> Value {
    json!({
        "places": { "atlas": ATLAS, "ledger": LEDGER },
        "projects": { "atlas": ATLAS_PROJECT, "ledger": LEDGER_PROJECT },
        "sessions": data()["sessions"].as_object().into_iter().flatten().map(|(role, _)| (role.clone(), json!(part(role).1))).collect::<Map<String, Value>>(),
        "watcher": WATCHER,
    })
}

/// The phase at `t`, 0, 1 or 2, and when it began.
pub fn phase_at(t: f64) -> (i64, f64) {
    let n = (t / PHASE_MS).floor();
    ((n.rem_euclid(3.0)) as i64, n * PHASE_MS)
}

pub fn fixture_view(t: f64) -> Value {
    let (phase, since) = phase_at(t);
    let num = xenonet_text::json::number;
    // The work session, the second app's, and the quiet one, in
    // fixture.json's order.
    let mut parts = data()["sessions"].as_object().into_iter().flatten().map(|(role, _)| part(role));
    let ((work_app, work), (codex_app, codex), (quiet_app, quiet)) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    let work_live = json!({
        "status": if phase == 0 { "tool" } else { "idle" },
        "needsYou": match phase { 1 => json!({ "kind": "question", "since": num(since + 5e3) }), 2 => json!({ "kind": "review", "since": num(since + 5e3) }), _ => Value::Null },
        "stuck": null,
        "turnStartedAt": if phase == 0 { num(since) } else { Value::Null },
        "lastTurnEndAt": if phase == 0 { Value::Null } else { num(since + 5e3) },
        "interruptedAt": null,
        "lastEventAt": num(since + 5e3),
    });
    let codex_live = json!({
        "status": if phase == 0 { "idle" } else { "thinking" },
        "needsYou": null,
        "stuck": if phase == 2 { json!({ "reasons": ["4 failed tool calls in a row"] }) } else { Value::Null },
        "turnStartedAt": if phase == 0 { Value::Null } else { num(since) },
        "lastTurnEndAt": null,
        "interruptedAt": null,
        "lastEventAt": num(since + 20e3),
    });
    let ready = json!({ "state": "ready", "observedAt": num(t), "coverage": "complete", "reason": "ok" });
    json!({
        "at": num(t),
        "fixture": true,
        "sources": data()["sources"].as_array().into_iter().flatten().map(|app| (xenonet_text::text::display(app), ready.clone())).collect::<Map<String, Value>>(),
        "places": [[ATLAS, { "name": "atlas-api (made up)" }], [LEDGER, { "name": "ledger (made up)" }]],
        "sessions": [
            { "id": work, "app": work_app, "place": ATLAS, "title": "Add partial refunds (made up)", "updatedAt": num(since + 5e3), "archived": false, "live": work_live },
            { "id": codex, "app": codex_app, "place": ATLAS, "title": "Fix the flaky orders test (made up)", "updatedAt": num(since + 20e3), "archived": false, "live": codex_live },
            { "id": quiet, "app": quiet_app, "place": LEDGER, "title": "Tidy the ledger exports (made up)", "updatedAt": num(t - 3.0 * 3600e3), "archived": false, "live": null },
        ],
        "collisions": if phase == 1 { json!([{ "a": work, "b": codex, "files": 2, "ts": num(since + 30e3) }]) } else { json!([]) },
        "actors": [{ "id": WATCHER, "kind": "agent", "productId": null, "name": "Fixture watcher (made up)", "presence": "idle", "via": ["observed"], "remote": null, "reachable": false, "targets": [{ "id": ATLAS, "relation": "watches" }] }],
    })
}

/// A grant's policy as it reads the fixture: both made-up projects, its
/// own labels switch and the rest as it is.
pub fn fixture_policy(policy: &Value) -> Value {
    let mut m: Map<String, Value> = policy.as_object().cloned().unwrap_or_default();
    m.insert("projects".into(), json!([ATLAS_PROJECT, LEDGER_PROJECT]));
    m.insert("places".into(), json!([[ATLAS_PROJECT, ATLAS], [LEDGER_PROJECT, LEDGER]]));
    m.insert("dataMode".into(), json!("fixture"));
    Value::Object(m)
}

/// Made-up stats for a fixture session.
pub fn fixture_stats(id: &str) -> Value {
    let seed = sha256(id.as_bytes());
    let n = |i: usize, scale: u32| (u32::from(seed[i]) * 256 + u32::from(seed[i + 1])) % scale;
    let turns: Vec<Value> = (0..=n(10, 9)).map(|_| json!({})).collect();
    json!({ "totals": { "input": 2000 + n(0, 9000), "cacheWrite": n(2, 4000), "cacheRead": 40000 + n(4, 90000), "output": 1500 + n(6, 6000) }, "tools": [{ "calls": 3 + n(8, 40) }], "turns": turns })
}

/// The policy `xenonet mcp assistant --fixture` reads with: no grant, no
/// device of this computer's, and nothing paired.
pub fn standalone_policy(labels: bool) -> Value {
    fixture_policy(&json!({
        "grantId": "gr_000000000000000000000000",
        "revision": 1,
        "actorId": "actor:fixture-client",
        "connectionId": "cn_000000000000000000000000",
        "deviceId": "dev_00000000-0000-4000-8000-000000000000",
        "deviceAlias": "fixture device",
        "productId": null,
        "accountLink": "unverified",
        "labels": labels,
        "lookbackMs": xenonet_text::json::number(30.0 * 24.0 * 3600e3),
        "transportScope": "other_approved_scope",
        "audienceLabel": "fixture",
        "permissions": ["read"],
    }))
}
