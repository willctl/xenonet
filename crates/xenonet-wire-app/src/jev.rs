//! JevRouter's route() (the npm library jevrouter, dist/router.js), as
//! core/integrations/routing.js uses it: one routing decision over a few
//! candidates, single-stage, with XenoNet's policy, no input to validate
//! and "suggest" as the actor's only permission. The decision comes from
//! the decision model (decide.rs) and is checked here as the library checks
//! it: a Choice answer whose probabilities name the candidates, the
//! highest-probability safe candidate selected, and nothing selected below
//! the policy's confidence.

use serde_json::{json, Map, Value};

/// XenoNet's policy for JevRouter (core/integrations/routing.js).
pub fn policy() -> Value {
    json!({ "min_confidence": 0.75, "require_verified_candidates": true, "allowed_risk_levels": ["low"], "confirmation_risk_levels": ["low"], "single_stage_max_candidates": 32 })
}

const QUESTION: &str = "Which single capability should handle this request?";
pub const PROVIDER: &str = "XenoNet OpenRouter";

/// A JevProviderError: its code and message.
#[derive(Clone, Debug, PartialEq)]
pub struct JevError {
    pub code: &'static str,
    pub message: String,
}

impl JevError {
    pub fn http(message: impl Into<String>) -> Self {
        JevError { code: "jev_http_error", message: message.into() }
    }
    fn malformed(message: impl Into<String>) -> Self {
        JevError { code: "jev_malformed_response", message: message.into() }
    }
}

// stableJson: keys sorted at every level, as localeCompare sorts them.
fn stable(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(stable).collect()),
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| xenonet_app::jstext::locale_compare(a, b));
            Value::Object(keys.into_iter().map(|k| (k.clone(), stable(&m[k]))).collect())
        }
        other => other.clone(),
    }
}

fn sha256(v: &Value) -> String {
    format!("sha256:{}", xenonet_integrations::b64::sha256_hex(xenonet_base::json::stringify(&stable(v), 0).as_bytes()))
}

fn request_id(prefix: &str) -> String {
    format!("{prefix}_{}", xenonet_service::borrowed::random_uuid())
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().map(xenonet_base::text::js_string).collect()).unwrap_or_default()
}

fn candidate_view(c: &Value, probability: Option<f64>, confidence: Option<f64>, stage: Option<&str>, actor_permissions: &[String], policy: &Value) -> Value {
    let risk = c.get("risk").and_then(|r| r.get("level")).and_then(Value::as_str).unwrap_or("low");
    let available = c.get("availability").and_then(|a| a.get("available")) != Some(&json!(false));
    let verification = c.get("verification").and_then(|v| v.get("status")).and_then(Value::as_str).unwrap_or("unknown");
    let verified = verification == "verified";
    let permissions = strs(c.get("permissions"));
    let required = strs(policy.get("required_permissions"));
    let missing_policy: Vec<String> = required.iter().filter(|p| !permissions.contains(p)).cloned().collect();
    let missing_actor: Vec<String> = permissions.iter().filter(|p| !actor_permissions.contains(p)).cloned().collect();
    let allowed_risk = strs(policy.get("allowed_risk_levels")).iter().any(|r| r == risk);
    let mut reasons: Vec<String> = Vec::new();
    if !available && policy.get("allow_unavailable_fallback") != Some(&json!(true)) {
        reasons.push(c.get("availability").and_then(|a| a.get("reason")).and_then(Value::as_str).unwrap_or("capability_unavailable").to_string());
    }
    if !missing_policy.is_empty() {
        reasons.push(format!("manifest_missing_permissions:{}", missing_policy.join(",")));
    }
    if !missing_actor.is_empty() {
        reasons.push(format!("actor_missing_permissions:{}", missing_actor.join(",")));
    }
    if !allowed_risk {
        reasons.push(format!("risk_not_allowed:{risk}"));
    }
    if policy.get("require_verified_candidates") == Some(&json!(true)) && !verified {
        reasons.push(format!("capability_not_verified:{verification}"));
    }
    let confirmation = c.get("policy").and_then(|p| p.get("requires_confirmation")) == Some(&json!(true)) || strs(policy.get("confirmation_risk_levels")).iter().any(|r| r == risk);
    let num = |n: Option<f64>| n.map_or(Value::Null, xenonet_base::json::number);
    json!({
        "id": c.get("id"),
        "type": c.get("type"),
        "name": c.get("name"),
        "jev_probability": num(probability),
        "jev_confidence": num(confidence),
        "jev_stage": stage,
        "router_rank": null,
        "router": {
            "available": available,
            "verified": verified,
            "verification_status": verification,
            "verification_source": c.get("verification").and_then(|v| v.get("source")).cloned().unwrap_or(Value::Null),
            "allowed": reasons.is_empty(),
            "risk_level": risk,
            "requires_confirmation": confirmation,
            "filtered": !reasons.is_empty(),
            "filter_reason": if reasons.is_empty() { Value::Null } else { json!(reasons.join(";")) },
        },
    })
}

/// getChoiceAnswer(raw, "tool"): the choice, its probabilities and the
/// confidence.
fn choice_answer(raw: &Value) -> Result<(String, Map<String, Value>, f64), JevError> {
    let Some(a) = raw.get("answers").and_then(|a| a.get("tool")).filter(|a| a.is_object()) else { return Err(JevError::malformed("Jev response is missing answers.tool")) };
    let (Some("choice"), Some(choice), Some(probs)) =
        (a.get("type").and_then(Value::as_str), a.get("choice").and_then(Value::as_str), a.get("probabilities").and_then(Value::as_object))
    else {
        return Err(JevError::malformed("answers.tool is not a Choice answer"));
    };
    for (k, p) in probs {
        if !p.as_f64().is_some_and(|p| p.is_finite() && (0.0..=1.0).contains(&p)) {
            return Err(JevError::malformed(format!("Invalid probability for {k}")));
        }
    }
    let confidence = match a.get("confidence") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        _ => probs.values().filter_map(Value::as_f64).fold(0.0, f64::max),
    };
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(JevError::malformed("Invalid confidence"));
    }
    Ok((choice.to_string(), probs.clone(), confidence))
}

fn base(ordered: &[Value], policy: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("request_id".into(), json!(request_id("req")));
    m.insert("decision_id".into(), json!(request_id("dec")));
    m.insert("mode".into(), json!("decision_only"));
    m.insert("execution".into(), json!({ "enabled": false, "status": "not_started" }));
    m.insert("provenance".into(), json!({ "jev_provider": PROVIDER, "candidate_snapshot_hash": sha256(&Value::Array(ordered.to_vec())), "policy_hash": sha256(policy) }));
    m
}

fn no_decision(mut b: Map<String, Value>, ordered: &[Value], actor: &[String], policy: &Value, e: &JevError, raw: Value) -> Value {
    b.insert("status".into(), json!("no_decision"));
    b.insert("decision".into(), json!({ "kind": "choice", "question": QUESTION, "selected": null, "jev_choice": null, "candidates": ordered.iter().map(|c| candidate_view(c, None, None, None, actor, policy)).collect::<Vec<_>>() }));
    b.insert("fallback".into(), json!({ "type": "provider_error", "reason": e.message }));
    b.insert("raw_jev".into(), raw);
    b.insert("error".into(), json!({ "code": e.code, "message": e.message }));
    Value::Object(b)
}

/// route({ request, actor_permissions }, candidates), the provider's raw
/// answer (or what it threw) given.
pub fn route(candidates: &[Value], actor_permissions: &[String], raw: Result<Value, JevError>) -> Value {
    let policy = policy();
    let mut ordered = candidates.to_vec();
    ordered.sort_by(|a, b| xenonet_app::jstext::locale_compare(&xenonet_base::text::js_string(&a["id"]), &xenonet_base::text::js_string(&b["id"])));
    let mut b = base(&ordered, &policy);
    if ordered.is_empty() {
        b.insert("status".into(), json!("no_decision"));
        b.insert("decision".into(), json!({ "kind": "choice", "question": QUESTION, "selected": null, "jev_choice": null, "candidates": [] }));
        b.insert("fallback".into(), json!({ "type": "no_safe_candidate", "reason": "No capability candidates were supplied" }));
        b.insert("raw_jev".into(), Value::Null);
        return Value::Object(b);
    }
    let raw = match raw {
        Ok(r) => r,
        Err(e) => return no_decision(b, &ordered, actor_permissions, &policy, &e, Value::Null),
    };
    let (choice, probs, confidence) = match choice_answer(&raw) {
        Ok(a) => a,
        Err(e) => return no_decision(b, &ordered, actor_permissions, &policy, &e, Value::Null),
    };
    let id_of = |c: &Value| xenonet_base::text::js_string(&c["id"]);
    // finalize()
    if !ordered.iter().any(|c| id_of(c) == choice) {
        return no_decision(b, &ordered, actor_permissions, &policy, &JevError::malformed(format!("Jev selected unknown capability {choice}")), raw);
    }
    let missing: Vec<String> = ordered.iter().map(id_of).filter(|id| !probs.contains_key(id)).collect();
    if !missing.is_empty() {
        return no_decision(b, &ordered, actor_permissions, &policy, &JevError::malformed(format!("Jev response omitted probabilities for {}", missing.join(", "))), raw);
    }
    let mut all: Vec<Value> =
        ordered.iter().map(|c| candidate_view(c, probs.get(&id_of(c)).and_then(Value::as_f64), Some(confidence), Some("single"), actor_permissions, &policy)).collect();
    let p = |c: &Value| c["jev_probability"].as_f64().unwrap_or(-1.0);
    let by_rank = |a: &Value, b: &Value| p(b).total_cmp(&p(a)).then_with(|| xenonet_app::jstext::locale_compare(&id_of(a), &id_of(b)));
    let mut ranked = all.clone();
    ranked.sort_by(by_rank);
    for c in all.iter_mut() {
        let id = id_of(c);
        c["router_rank"] = ranked.iter().position(|r| id_of(r) == id).map_or(Value::Null, |i| json!(i + 1));
    }
    let mut safe: Vec<&Value> = all.iter().filter(|c| c["router"]["filtered"] != json!(true)).collect();
    safe.sort_by(|a, b| by_rank(a, b));
    let top = safe.first().copied();
    let threshold = if top.is_some() { policy["min_confidence"].as_f64().unwrap_or(0.55).clamp(0.0, 1.0) } else { 1.0 };
    let mut selected = top.map(id_of);
    let mut status = if top.is_some_and(|t| t["router"]["requires_confirmation"] == json!(true)) { "needs_confirmation" } else { "selected" };
    let mut fallback = json!({ "type": null, "reason": null });
    match top {
        None => {
            status = "no_decision";
            fallback = json!({ "type": "no_safe_candidate", "reason": "All candidates were filtered by availability, permission, or policy" });
        }
        Some(_) if confidence < threshold => {
            selected = None;
            status = "no_decision";
            let fixed = |n: f64| xenonet_base::units::to_fixed(n, 3);
            fallback = json!({ "type": "low_confidence", "reason": format!("Jev confidence {} is below {}", fixed(confidence), fixed(threshold)) });
        }
        Some(t) if id_of(t) != choice => {
            fallback = json!({ "type": "manual_review", "reason": format!("Jev selected {choice}, which was filtered; router selected the highest-probability safe candidate") });
        }
        _ => {}
    }
    b.insert("status".into(), json!(status));
    b.insert("decision".into(), json!({ "kind": "choice", "question": QUESTION, "selected": selected, "jev_choice": choice, "candidates": all }));
    b.insert("fallback".into(), fallback);
    b.insert("raw_jev".into(), raw);
    Value::Object(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<Value> {
        xenonet_integrations::routing::inventory(&[json!({ "kind": "session", "app": "codex" })]).as_array().cloned().unwrap()
    }

    fn answer(choice: &str, probs: Value, confidence: f64) -> Value {
        json!({ "answers": { "tool": { "type": "choice", "choice": choice, "probabilities": probs, "confidence": confidence } }, "model": "demo/jev" })
    }

    #[test]
    fn the_highest_safe_candidate_with_enough_confidence() {
        let suggest = vec!["suggest".to_string()];
        let r = route(&candidates(), &suggest, Ok(answer("codex", json!({ "codex": 0.8, "claude": 0.1, "review": 0.1 }), 0.9)));
        assert_eq!((r["status"].clone(), r["decision"]["selected"].clone()), (json!("needs_confirmation"), json!("codex")));
        // As the JS library answers the same (checked side by side).
        let seen: Vec<Value> = r["decision"]["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| json!([c["id"], c["router_rank"], c["router"]["filtered"], c["router"]["filter_reason"], c["router"]["requires_confirmation"]]))
            .collect();
        assert_eq!(seen, vec![json!(["claude", 2, true, "capability_unavailable", true]), json!(["codex", 1, false, null, true]), json!(["review", 3, false, null, true])]);
        let low = route(&candidates(), &suggest, Ok(answer("codex", json!({ "codex": 0.6, "claude": 0.2, "review": 0.2 }), 0.6)));
        assert_eq!((low["status"].clone(), low["fallback"]["type"].clone(), low["decision"]["selected"].clone()), (json!("no_decision"), json!("low_confidence"), Value::Null));
        assert_eq!(low["fallback"]["reason"], "Jev confidence 0.600 is below 0.750");
        let failed = route(&candidates(), &suggest, Err(JevError::http("no key")));
        assert_eq!((failed["fallback"]["type"].clone(), failed["error"]["code"].clone()), (json!("provider_error"), json!("jev_http_error")));
        let missing = route(&candidates(), &suggest, Ok(answer("codex", json!({ "codex": 1.0 }), 0.9)));
        assert_eq!(missing["error"]["code"], "jev_malformed_response");
        // Claude Code isn't on the map, so it's filtered, and the router
        // takes the next safe one.
        let filtered = route(&candidates(), &suggest, Ok(answer("claude", json!({ "codex": 0.3, "claude": 0.6, "review": 0.1 }), 0.9)));
        assert_eq!((filtered["decision"]["selected"].clone(), filtered["fallback"]["type"].clone()), (json!("codex"), json!("manual_review")));
    }
}
