//! core/refusals/demo.js: the demo's refusals, made up. Only the demo and
//! the tests use them; no one's transcripts.

use serde_json::{json, Map, Value};
use xenonet_text::json::number;

// The made-up rows (provider, the app it ran in, kind, model, category,
// explanation), kept as data.
fn rows() -> &'static [Map<String, Value>] {
    static R: std::sync::OnceLock<Vec<Map<String, Value>>> = std::sync::OnceLock::new();
    R.get_or_init(|| serde_json::from_str(include_str!("demo.json")).unwrap_or_default())
}

pub fn demo_refusals(now: f64) -> Vec<Value> {
    let text = |r: &'static Map<String, Value>, k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default();
    let rows: Vec<_> = rows().iter().map(|r| (text(r, "provider"), text(r, "app"), text(r, "kind"), text(r, "model"), text(r, "category"), text(r, "explanation"))).collect();
    rows.iter()
        .enumerate()
        .map(|(i, &(provider, app, kind, model, category, explanation))| {
            let mut signal = Map::new();
            for (k, v) in [("kind", kind), ("model", model), ("category", category), ("explanation", explanation)] {
                signal.insert(k.into(), Value::String(v.into()));
            }
            if kind == "refusal-recovered" || kind == "reroute" {
                signal.insert("fallbackModel".into(), Value::String("fallback-example".into()));
            }
            json!({
                "schema": "refusal/record/v1",
                "id": format!("20261001-{:08x}", i + 1),
                "at": number(now - i as f64 * 3600e3),
                "recordedAt": number(now),
                "provider": provider,
                "app": app,
                "kind": kind,
                "session": { "id": format!("demo-session-{}", i + 1), "cwd": "~/Projects/demo", "model": model },
                "signal": signal,
                "source": { "file": format!("~/demo/transcript-{}.jsonl", i + 1) },
                "context": if i == 0 { json!({ "file": "context.jsonl.gz", "request": "request.json", "lines": 3, "bytes": 256, "truncated": false }) } else { Value::Null },
                "missing": if i == 0 { json!(["tool definitions"]) } else { json!([]) },
            })
        })
        .collect()
}
