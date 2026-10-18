//! core/integrations/routing.js: a suggestion of which agent should take a
//! task, made by a model through OpenRouter. The preview sends nothing;
//! routing sends only the task and the candidates' descriptions, and the
//! record keeps the task's hash, the scores and the outcome, never the
//! task. Nothing starts an agent. JevRouter (an npm library) does the
//! routing itself, in the twin.

use crate::common::{clean, fail, failed, has_credential, text, Fail};
use serde_json::{json, Map, Value};
use xenonet_app::jsv::truthy;
use xenonet_base::text::js_number;

/// The agents a task could go to, from the sessions on the map.
pub fn inventory(nodes: &[Value]) -> Value {
    let has = |app: &str| nodes.iter().any(|n| n.get("kind").and_then(Value::as_str) == Some("session") && n.get("app").and_then(Value::as_str) == Some(app));
    let list = [
        ("codex", "Codex", "Coding, repository inspection and tool-assisted research in Codex.", has("codex")),
        ("claude", "Claude Code", "Coding and repository work in Claude Code.", has("claude")),
        ("review", "Human review", "Keep the task for human review when requirements, permissions or evidence are unclear.", true),
    ];
    Value::Array(
        list.iter()
            .map(|(id, name, description, available)| {
                json!({
                    "id": id, "name": name, "description": description,
                    "type": if *id == "review" { "skill" } else { "subagent" },
                    "permissions": ["suggest"], "risk": { "level": "low" }, "availability": { "available": available },
                    "verification": { "status": "verified", "source": "XenoNet observed sessions" }, "policy": { "requires_confirmation": true },
                })
            })
            .collect(),
    )
}

/// What goes to the decider: the task and each candidate's description.
pub fn payload(request: &str, candidates: &[Value]) -> Value {
    let criteria: Map<String, Value> = candidates
        .iter()
        .map(|c| (xenonet_base::text::js_string(&c["id"]), json!({ "description": c.get("description"), "available": c.get("availability").and_then(|a| a.get("available")) })))
        .collect();
    json!({ "state": { "request": request }, "questions": { "tool": { "type": "choice", "instructions": "Which single candidate should handle this task? Prefer human review if none fits. This is a suggestion only.", "criteria": criteria } } })
}

/// The task a preview takes, or why not.
pub fn request_of(body: Option<&Value>) -> Result<String, Fail> {
    let request = text(body.and_then(|b| b.get("request")), "Task", 2000)?;
    if has_credential(&request) {
        return fail("Remove credentials from the task before sending it.");
    }
    Ok(request)
}

pub const NOTE: &str =
    "Only this task and the listed candidate descriptions go to OpenRouter. The saved record contains a task hash, scores and outcome, never task text. No agent starts.";

/// The preview's outgoing request, as shown before anything goes.
pub fn outgoing(model: &Value, request: &str, candidates: &[Value]) -> Value {
    let mut m = Map::new();
    m.insert("model".into(), model.clone());
    for (k, v) in payload(request, candidates).as_object().into_iter().flatten() {
        m.insert(k.clone(), v.clone());
    }
    m.insert("provider".into(), json!({ "zdr": true }));
    Value::Object(m)
}

fn prob(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|p| p.is_finite() && (0.0..=1.0).contains(p))
}

/// Whether the decider's answer is a probability distribution over the
/// candidates, with a choice among them.
pub fn answer_ok(answers: &Value, candidates: &[Value]) -> bool {
    let Some(a) = answers.get("tool").filter(|a| truthy(Some(a))) else { return false };
    if prob(a.get("confidence")).is_none() {
        return false;
    }
    if !candidates.iter().any(|c| xenonet_app::jsv::strict_eq(c.get("id"), a.get("choice"))) {
        return false;
    }
    let Some(probs) = a.get("probabilities").and_then(Value::as_object).filter(|p| p.len() == candidates.len()) else { return false };
    if candidates.iter().any(|c| prob(probs.get(&xenonet_base::text::js_string(&c["id"]))).is_none()) {
        return false;
    }
    let sum: f64 = probs.values().map(|p| js_number(Some(p))).sum();
    (sum - 1.0).abs() <= 0.02
}

/// The record a routing decision leaves: its hash, scores and outcome.
pub fn decision_record(result: &Value, request: &str, metrics: &Value) -> Value {
    let n = |k: &str| {
        let v = js_number(metrics.get(k));
        if v.is_nan() || v == 0.0 {
            0.0
        } else {
            v
        }
    };
    let decision = result.get("decision");
    let candidates: Vec<Value> = decision
        .and_then(|d| d.get("candidates"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let r = c.get("router");
            fields(&[
                ("id", c.get("id").cloned()),
                ("name", c.get("name").cloned()),
                ("probability", c.get("jev_probability").cloned()),
                ("filtered", r.and_then(|r| r.get("filtered")).cloned()),
                ("reason", Some(json!(clean(r.and_then(|r| r.get("filter_reason")), 160)))),
                ("confirmation", r.and_then(|r| r.get("requires_confirmation")).cloned()),
            ])
        })
        .collect();
    fields(&[
        ("kind", Some(json!("decision"))),
        ("status", result.get("status").cloned()),
        ("selected", decision.and_then(|d| d.get("selected")).cloned()),
        ("taskHash", Some(json!(crate::b64::sha256_hex(request.as_bytes())))),
        ("model", Some(json!(clean(metrics.get("model"), 160)))),
        ("cost", Some(xenonet_base::json::number(n("cost")))),
        ("ms", Some(xenonet_base::json::number(n("ms")))),
        ("reason", Some(result.get("fallback").and_then(|f| f.get("type")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null))),
        ("candidates", Some(Value::Array(candidates))),
        ("outcome", Some(json!("unreviewed"))),
    ])
}

// An object as JS writes one, with what's undefined (None) left out.
pub fn fields(pairs: &[(&str, Option<Value>)]) -> Value {
    Value::Object(pairs.iter().filter_map(|(k, v)| Some(((*k).to_string(), v.clone()?))).collect())
}

/// A decision's outcome, as you say it was.
pub fn outcome_ok(outcome: &Value) -> Result<(), Fail> {
    match outcome.as_str() {
        Some("accepted" | "rejected" | "useful" | "unhelpful" | "unreviewed") => Ok(()),
        _ => fail("Choose a valid decision outcome."),
    }
}

pub fn no_decision<T>() -> Result<T, Fail> {
    failed("No such decision.", "missing", 404)
}
