//! core/integrations/index.js: the integrations together, as the server
//! reaches them: their settings (whether Pi's sessions are read), the
//! history of decisions and sandbox jobs, Pi's records, and the demo
//! instance's made-up decider, OpenShell and records.

use crate::common::{clean, fail, query, Fail};
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_os::files;
use xenonet_os::nodepath::NATIVE;

/// Whether settings.json says to read Pi's sessions.
pub fn pi_on(settings_file: Option<&str>) -> bool {
    settings_file.and_then(|f| files::read_text(f).ok()).and_then(|t| xenonet_text::json::parse(&t).ok()).is_some_and(|s| s.get("piOn") == Some(&json!(true)))
}

/// Keeps whether to read Pi's sessions, readable by you alone.
pub fn set_pi(settings_file: Option<&str>, on: &Value) -> Result<Value, Fail> {
    let Some(on) = on.as_bool() else { return fail("Choose whether to read Pi sessions.") };
    if let Some(file) = settings_file {
        let tmp = format!("{file}.tmp");
        files::write_file(&tmp, xenonet_text::json::stringify(&json!({ "piOn": on }), 0).as_bytes(), Some(0o600))?;
        files::rename(&tmp, file)?;
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
    let existing = records.iter().find(|r| xenonet_text::value::same(r.get("sourceId"), Some(&source_id)));
    Ok(match existing {
        Some(r) => json!({ "update": r["id"], "fields": fields }),
        None => {
            fields.insert("sourceId".into(), source_id);
            json!({ "add": fields })
        }
    })
}

// The demo's made-up data, as demo.json has it: data, so the code names
// no app.
fn demo() -> &'static Value {
    static D: OnceLock<Value> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("demo.json")).unwrap_or(Value::Null))
}

pub fn demo_decision() -> Value {
    demo()["decision"].clone()
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
        return Ok(xenonet_text::json::stringify(&out, 0));
    }
    if has("get") && has("json") {
        return Ok(xenonet_text::json::stringify(&json!({ "phase": "Ready" }), 0));
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
    demo()["records"].clone()
}

/// A demo sandbox job's record, its review seeded.
pub fn demo_sandbox(name: &str, options: &Value, review_id: &Value) -> Value {
    let retrieval = if options.get("unavailable") == Some(&json!(true)) {
        "unavailable"
    } else if options.get("problems").is_some_and(|p| xenonet_text::text::truthy(Some(p))) {
        "incomplete"
    } else {
        "retrieved"
    };
    let mut job = demo()["sandbox"].clone();
    job["name"] = json!(format!("demo-{name}"));
    job["reviewId"] = review_id.clone();
    job["retrieval"] = json!(retrieval);
    job
}
