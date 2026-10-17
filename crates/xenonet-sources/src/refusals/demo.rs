//! core/refusals/demo.js: the demo's refusals, made up. Only the demo and
//! the tests use them; no one's transcripts.

use serde_json::{json, Map, Value};
use xenonet_base::json::number;

pub fn demo_refusals(now: f64) -> Vec<Value> {
    let rows = [
        ("anthropic", "refusal", "claude-sonnet", "safety", "The provider declined this request."),
        ("anthropic", "refusal-recovered", "claude-opus", "policy", "The request was answered by a fallback model."),
        ("openai", "reroute", "gpt-example", "safety", "The provider routed this request to another model."),
        ("openai", "possible-refusal", "gpt-example", "invalid_prompt", "The provider returned invalid_prompt; a refusal is not confirmed."),
    ];
    rows.iter()
        .enumerate()
        .map(|(i, &(provider, kind, model, category, explanation))| {
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
                "app": if provider == "openai" { "codex" } else { "claude-code" },
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
