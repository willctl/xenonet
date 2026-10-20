//! core/integrations/index.js: the integrations together, as the server
//! reaches them: their settings (whether Pi's sessions are read), the
//! history of decisions and sandbox jobs, Pi's records, and the demo
//! instance's made-up decider, OpenShell and records.

use crate::common::{clean, fail, query, Fail};
use serde_json::{json, Map, Value};
use xenonet_app::fsx;
use xenonet_app::nodepath::NATIVE;

/// Whether settings.json says to read Pi's sessions.
pub fn pi_on(settings_file: Option<&str>) -> bool {
    settings_file.and_then(|f| fsx::read_text(f).ok()).and_then(|t| xenonet_base::json::parse(&t).ok()).is_some_and(|s| s.get("piOn") == Some(&json!(true)))
}

/// Keeps whether to read Pi's sessions, readable by you alone.
pub fn set_pi(settings_file: Option<&str>, on: &Value) -> Result<Value, Fail> {
    let Some(on) = on.as_bool() else { return fail("Choose whether to read Pi sessions.") };
    if let Some(file) = settings_file {
        let tmp = format!("{file}.tmp");
        fsx::write_file(&tmp, xenonet_base::json::stringify(&json!({ "piOn": on }), 0).as_bytes(), Some(0o600))?;
        fsx::rename(&tmp, file)?;
    }
    Ok(json!({ "piOn": on }))
}

pub fn pi_root(home: &str) -> String {
    NATIVE.join(&[home, ".pi", "agent", "sessions"])
}

pub fn pi_root_shown(home: &str) -> String {
    clean(Some(&json!(pi_root(home))), 1024)
}

/// The decisions and sandbox jobs, queried.
pub fn history(records: &[Value], params: &Value) -> Value {
    let kept: Vec<Value> = records.iter().filter(|r| matches!(r.get("kind").and_then(Value::as_str), Some("decision" | "sandbox"))).cloned().collect();
    query(&kept, params)
}

/// Pi's records: the exports you imported and the sessions read now.
pub fn pi(records: &[Value], scanned: &Value, enabled: bool, params: &Value) -> Value {
    let mut all: Vec<Value> = records.iter().filter(|r| r.get("kind").and_then(Value::as_str) == Some("pi-export")).cloned().collect();
    all.extend(scanned.get("records").and_then(Value::as_array).cloned().unwrap_or_default());
    let mut out: Map<String, Value> = query(&all, params).as_object().cloned().unwrap_or_default();
    out.insert("enabled".into(), json!(enabled));
    out.insert("warnings".into(), scanned.get("warnings").cloned().unwrap_or_else(|| json!([])));
    Value::Object(out)
}

/// An export's metadata as importPi keeps it: { update: id, fields } for
/// one imported before, or { add: record }.
pub fn import_plan(records: &[Value], value: &Value) -> Result<Value, Fail> {
    let mut fields: Map<String, Value> = crate::pi::export_metadata(value)?.as_object().cloned().unwrap_or_default();
    let source_id = fields.shift_remove("id").unwrap_or(Value::Null);
    let existing = records.iter().find(|r| xenonet_app::jsv::strict_eq(r.get("sourceId"), Some(&source_id)));
    Ok(match existing {
        Some(r) => json!({ "update": r["id"], "fields": fields }),
        None => {
            fields.insert("sourceId".into(), source_id);
            json!({ "add": fields })
        }
    })
}

pub fn demo_decision() -> Value {
    json!({ "answers": { "tool": { "type": "choice", "choice": "codex", "confidence": 0.93, "probabilities": { "codex": 0.8, "claude": 0.15, "review": 0.05 } } }, "model": "demo/jev", "cost": 0, "ms": 12 })
}

/// What the demo's made-up OpenShell prints for these args, or the error
/// its offline gateway gives.
pub fn demo_run(args: &[Value]) -> Result<String, String> {
    let has = |s: &str| args.iter().any(|a| a.as_str() == Some(s));
    if has("-e") {
        if has("demo-unavailable") {
            return Err("Demo gateway is offline.".into());
        }
        let problems = if has("demo-partial") { json!([{ "path": "image.png", "reason": "Binary file" }]) } else { json!([]) };
        let out = json!({ "files": [{ "path": "hello.txt", "content": crate::b64::encode(b"Hello from the sandbox.\n") }], "omissions": [], "problems": problems });
        return Ok(xenonet_base::json::stringify(&out, 0));
    }
    if has("get") && has("json") {
        return Ok(xenonet_base::json::stringify(&json!({ "phase": "Ready" }), 0));
    }
    if has("policy") {
        return Ok("Demo policy: /workspace writable; network limited to the selected provider.".into());
    }
    if has("logs") {
        return Ok("Demo execution-policy-denial: unlisted.example was blocked. This is a runtime policy event, not a model refusal.".into());
    }
    Ok(if has("--version") { "OpenShell demo".into() } else { r#"{"sandboxes":[]}"#.into() })
}

/// The demo's records: a decision, an export, and a sandbox job for each
/// way a review can come out (each with the review seeded from `options`).
pub fn demo_records() -> Value {
    json!({
        "decision": { "kind": "decision", "status": "needs_confirmation", "selected": "codex", "model": "demo/jev", "taskHash": "demo", "cost": 0, "ms": 12, "reason": null, "candidates": [{ "id": "codex", "name": "Codex", "probability": 0.8, "filtered": false, "confirmation": true }], "outcome": "useful" },
        "export": { "kind": "pi-export", "sourceId": "pi-export:demo", "externalId": "demo-pi-run", "status": "completed", "model": "demo-model", "provider": "demo", "tokens": 420, "cost": 0, "artifacts": 2, "checkpoints": 1, "diagnostics": 0 },
        "sandboxes": [
            ["changes", { "before": { "hello.txt": "Hello.\n", "removed.txt": "Old example.\n" }, "after": { "hello.txt": "Hello from the sandbox.\n", "new.txt": "New example.\n" } }],
            ["no-changes", { "before": { "hello.txt": "Hello.\n" }, "after": { "hello.txt": "Hello.\n" } }],
            ["source-drift", { "before": { "hello.txt": "Hello.\n" }, "after": { "hello.txt": "Hello from the sandbox.\n" }, "sourceState": "changed" }],
            ["partial", { "before": { "hello.txt": "Hello.\n" }, "after": { "hello.txt": "Hello from the sandbox.\n" }, "problems": [{ "path": "image.png", "reason": "Binary file" }] }],
            ["unavailable", { "before": { "hello.txt": "Hello.\n" }, "after": {}, "unavailable": true }],
        ],
    })
}

/// A demo sandbox job's record, its review seeded.
pub fn demo_sandbox(name: &str, options: &Value, review_id: &Value) -> Value {
    let retrieval = if options.get("unavailable") == Some(&json!(true)) {
        "unavailable"
    } else if options.get("problems").is_some_and(|p| xenonet_app::jsv::truthy(Some(p))) {
        "incomplete"
    } else {
        "retrieved"
    };
    json!({ "kind": "sandbox", "status": "created", "name": format!("demo-{name}"), "agent": "pi", "image": "demo/pi", "profile": { "backend": "native", "distro": "" }, "reviewId": review_id, "execution": "unknown", "completedSteps": 5, "launchSteps": 5, "retrieval": retrieval })
}
