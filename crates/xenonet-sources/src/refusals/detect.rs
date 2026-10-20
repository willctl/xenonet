//! core/refusals/detect.js: a provider's refusal, as a transcript line shows
//! it (docs/guide/refusals.md), read from Claude Code's and Codex's own
//! lines the way they mark it, never guessed from what a model wrote.
//!
//! Claude Code writes an assistant line that carries the API's stop_reason
//! "refusal" (a synthetic one, isApiErrorMessage, or a model's own reply),
//! and newer versions write system lines for a refusal nothing took over
//! from (model_refusal_no_fallback) and one another model answered instead
//! (model_refusal_fallback); a reply may also carry a fallback block whose
//! trigger was a refusal. Codex writes an error event whose
//! codex_error_info names a policy, and a model_reroute when it moved the
//! turn to another model.
//!
//! A signal is { provider, kind, source, category, explanation, text,
//! requestId, messageId, uuid, parentUuid, model, fallbackModel, code, at },
//! in that order; kind is refusal, refusal-recovered, reroute or
//! possible-refusal (an invalid_prompt, which isn't always a policy). A
//! credit token the API sends with a fallback is never kept.

use crate::lines::opt_prop;
use serde_json::{Map, Value};
use xenonet_base::jsstr;
use xenonet_base::text::{join, js_string, truthy};
use xenonet_base::time::parse_iso;

// /policy|cyber|bio|misalignment|safety|content_filter/i, without the u
// flag: ASCII letters only, any case.
fn is_policy(code: &str) -> bool {
    let c = code.to_ascii_lowercase();
    ["policy", "cyber", "bio", "misalignment", "safety", "content_filter"].iter().any(|w| c.contains(w))
}

/// `typeof v === "string" && v ? v.slice(0, max) : null`.
fn words(v: Option<&Value>, max: usize) -> Value {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Value::String(jsstr::head(s, max)),
        _ => Value::Null,
    }
}

/// What a message's content says: its strings and text blocks, joined by
/// line breaks, at most 4000 units, or null for none.
fn text_of(content: Option<&Value>) -> Value {
    // A content that isn't a list is a list of one; a missing one reads
    // as "".
    let one = [content.cloned().unwrap_or(Value::Null)];
    let list: &[Value] = match content {
        Some(Value::Array(a)) => a,
        _ => &one,
    };
    let parts: Vec<Option<&Value>> = list
        .iter()
        .map(|b| match b {
            Value::String(_) => Some(b),
            b if opt_prop(Some(b), "type").and_then(Value::as_str) == Some("text") => opt_prop(Some(b), "text"),
            _ => None,
        })
        .filter(|v| truthy(*v))
        .collect();
    let text = jsstr::head(&join(parts.into_iter(), "\n"), 4000);
    if text.is_empty() {
        Value::Null
    } else {
        Value::String(text)
    }
}

/// `typeof t === "string" ? Date.parse(t) || null : null`.
fn time_of(t: Option<&Value>) -> Value {
    match t {
        Some(Value::String(s)) => match parse_iso(s) {
            Some(ms) if ms != 0.0 => xenonet_base::json::number(ms),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

fn or_null(v: Option<&Value>) -> Value {
    v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null)
}

/// Every field a signal has, in order, as `base` makes them.
fn base(provider: &str, raw: &Value) -> Map<String, Value> {
    let r = Some(raw);
    let mut m = Map::new();
    for (k, v) in [
        ("provider", Value::String(provider.into())),
        ("kind", Value::String("refusal".into())),
        ("source", Value::Null),
        ("category", Value::Null),
        ("explanation", Value::Null),
        ("text", Value::Null),
        ("requestId", Value::Null),
        ("messageId", Value::Null),
        ("uuid", or_null(opt_prop(r, "uuid"))),
        ("parentUuid", or_null(opt_prop(r, "parentUuid"))),
        ("model", Value::Null),
        ("fallbackModel", Value::Null),
        ("code", Value::Null),
        ("at", time_of(opt_prop(r, "timestamp"))),
    ] {
        m.insert(k.into(), v);
    }
    m
}

fn signal(provider: &str, raw: &Value, fields: Vec<(&str, Value)>) -> Value {
    let mut m = base(provider, raw);
    for (k, v) in fields {
        m.insert(k.into(), v);
    }
    Value::Object(m)
}

fn s(v: &str) -> Value {
    Value::String(v.into())
}

fn is_object(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(_) | Value::Array(_)))
}

/// A Claude Code line's refusal, or None.
pub fn claude_signal(raw: Option<&Value>) -> Option<Value> {
    claude_refusal(raw)
}

/// A Codex rollout line's refusal (or reroute), or None.
pub fn codex_signal(raw: Option<&Value>) -> Option<Value> {
    codex_refusal(raw)
}

/// A Claude Code line's refusal, as the transcripts' mapper reads it
/// (line_events.rs).
pub fn claude_refusal(raw: Option<&Value>) -> Option<Value> {
    let raw = raw.filter(|r| truthy(Some(r)) && is_object(Some(r)))?;
    let r = Some(raw);
    let get = |k: &str| opt_prop(r, k);
    let m = get("message").filter(|m| truthy(Some(m)) && is_object(Some(m)));
    let kind = get("type").and_then(Value::as_str);
    if kind == Some("assistant") && m.is_some() {
        let mget = |k: &str| opt_prop(m, k);
        let model = mget("model").filter(|v| truthy(Some(v)) && v.as_str() != Some("<synthetic>")).cloned().unwrap_or(Value::Null);
        if mget("stop_reason").and_then(Value::as_str) == Some("refusal") {
            let details = mget("stop_details");
            return Some(signal(
                "anthropic",
                raw,
                vec![
                    ("source", s(if truthy(get("isApiErrorMessage")) { "api-error" } else { "stop-reason" })),
                    ("category", words(opt_prop(details, "category"), 80)),
                    ("explanation", words(opt_prop(details, "explanation"), 2000)),
                    ("text", text_of(mget("content"))),
                    ("requestId", words(get("requestId"), 120)),
                    ("messageId", words(mget("id"), 120)),
                    ("model", model),
                    ("code", words(get("error"), 80)),
                ],
            ));
        }
        let blocks: &[Value] = match mget("content") {
            Some(Value::Array(a)) => a,
            _ => &[],
        };
        let fallback = blocks.iter().find(|b| {
            opt_prop(Some(b), "type").and_then(Value::as_str) == Some("fallback") && opt_prop(opt_prop(Some(b), "trigger"), "type").and_then(Value::as_str) == Some("refusal")
        });
        if let Some(f) = fallback {
            let f = Some(f);
            return Some(signal(
                "anthropic",
                raw,
                vec![
                    ("kind", s("refusal-recovered")),
                    ("source", s("fallback-block")),
                    ("category", words(opt_prop(opt_prop(f, "trigger"), "category"), 80)),
                    ("requestId", words(get("requestId"), 120)),
                    ("messageId", words(mget("id"), 120)),
                    ("model", words(opt_prop(f, "original_model"), 120)),
                    ("fallbackModel", model),
                ],
            ));
        }
        return None;
    }
    let subtype = get("subtype").and_then(Value::as_str);
    if kind == Some("system") && matches!(subtype, Some("model_refusal_no_fallback" | "model_refusal_fallback")) {
        let recovered = subtype == Some("model_refusal_fallback");
        let model = get("originalModel").filter(|v| !v.is_null()).or(get("model"));
        return Some(signal(
            "anthropic",
            raw,
            vec![
                ("kind", s(if recovered { "refusal-recovered" } else { "refusal" })),
                ("source", s(subtype.unwrap_or_default())),
                ("category", words(get("apiRefusalCategory"), 80)),
                ("explanation", words(get("apiRefusalExplanation"), 2000)),
                ("text", words(get("content"), 2000)),
                ("requestId", words(get("requestId"), 120)),
                ("model", words(model, 120)),
                ("fallbackModel", if recovered { words(get("fallbackModel"), 120) } else { Value::Null }),
            ],
        ));
    }
    None
}

/// The first key `Object.keys` lists of an object or array.
fn first_key(v: &Value) -> Option<String> {
    match v {
        Value::Object(m) => {
            if m.keys().any(|k| jsstr::is_array_index(k)) {
                jsstr::js_key_order(m.clone()).keys().next().cloned()
            } else {
                m.keys().next().cloned()
            }
        }
        Value::Array(a) => (!a.is_empty()).then(|| "0".to_string()),
        _ => None,
    }
}

/// A Codex rollout line's refusal (or reroute), as the transcripts' mapper
/// reads it.
pub fn codex_refusal(raw: Option<&Value>) -> Option<Value> {
    let raw = raw.filter(|r| truthy(Some(r)) && is_object(Some(r)))?;
    let r = Some(raw);
    if opt_prop(r, "type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let empty = Value::Object(Map::new());
    let p = Some(opt_prop(r, "payload").filter(|p| !p.is_null()).unwrap_or(&empty));
    let get = |k: &str| opt_prop(p, k);
    match get("type").and_then(Value::as_str) {
        Some("error") => {
            let info = get("codex_error_info");
            let code = match info {
                Some(Value::String(c)) => Some(c.clone()),
                Some(v) if truthy(Some(v)) && is_object(Some(v)) => first_key(v),
                _ => None,
            };
            let code = code.filter(|c| !c.is_empty())?;
            let detail = match info {
                Some(v) if truthy(Some(v)) && is_object(Some(v)) => match v {
                    Value::Object(m) => m.get(&code),
                    Value::Array(a) => code.parse::<usize>().ok().and_then(|i| a.get(i)),
                    _ => None,
                },
                _ => None,
            };
            let kind = if is_policy(&code) {
                "refusal"
            } else if code == "invalid_prompt" {
                "possible-refusal"
            } else {
                return None;
            };
            let explanation = opt_prop(detail, "detailed_explanation").filter(|v| !v.is_null()).or(opt_prop(detail, "explanation"));
            Some(signal(
                "openai",
                raw,
                vec![
                    ("kind", s(kind)),
                    ("source", s("error-event")),
                    ("category", Value::String(code.clone())),
                    ("explanation", words(explanation, 2000)),
                    ("text", words(get("message"), 2000)),
                    ("code", Value::String(code)),
                ],
            ))
        }
        Some("model_reroute") => Some(signal(
            "openai",
            raw,
            vec![
                ("kind", s("reroute")),
                ("source", s("model-reroute")),
                ("category", words(get("reason"), 80)),
                ("model", words(get("from_model"), 120)),
                ("fallbackModel", words(get("to_model"), 120)),
            ],
        )),
        _ => None,
    }
}

/// A value in a template literal: `${v}`.
fn tpl(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(v) => js_string(v),
    }
}

/// What makes two sightings of a refusal the same one: the request's id
/// when there is one, else where it was written.
pub fn key_of(sig: &Value, session_id: &str) -> String {
    let get = |k: &str| opt_prop(Some(sig), k);
    let nn = |k: &str| get(k).filter(|v| !v.is_null());
    let id = nn("requestId").or(nn("messageId")).or(nn("uuid")).map_or_else(|| format!("{session_id}@{}", tpl(get("at"))), js_string);
    format!("{}|{}|{id}", tpl(get("provider")), tpl(get("kind")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_refusals() {
        let refused = json!({ "type": "assistant", "uuid": "a3", "parentUuid": "u3", "isApiErrorMessage": true, "error": "invalid_request", "requestId": "req_1", "timestamp": "2026-10-07T12:00:05.000Z", "message": { "id": "msg_3", "model": "<synthetic>", "content": [{ "type": "text", "text": "API Error: flagged." }], "stop_reason": "refusal", "stop_details": { "category": "cyber", "explanation": "Looks like an attack.", "fallback_credit_token": "SECRET" } } });
        let sig = claude_signal(Some(&refused)).unwrap();
        assert_eq!(sig["source"], "api-error");
        assert_eq!(sig["category"], "cyber");
        assert_eq!(sig["model"], Value::Null);
        assert_eq!(sig["code"], "invalid_request");
        assert_eq!(sig["text"], "API Error: flagged.");
        assert_eq!(sig["at"].as_f64(), parse_iso("2026-10-07T12:00:05.000Z"));
        assert!(!sig.to_string().contains("SECRET"));
        let keys: Vec<&String> = sig.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["provider", "kind", "source", "category", "explanation", "text", "requestId", "messageId", "uuid", "parentUuid", "model", "fallbackModel", "code", "at"]);
        let system = json!({ "type": "system", "subtype": "model_refusal_no_fallback", "requestId": "req_1" });
        assert_eq!(key_of(&claude_signal(Some(&system)).unwrap(), "claude:s"), key_of(&sig, "claude:s"));
        let fallback = json!({ "type": "assistant", "message": { "model": "claude-b", "content": [{ "type": "fallback", "trigger": { "type": "refusal", "category": "bio" }, "original_model": "claude-a" }] } });
        let f = claude_signal(Some(&fallback)).unwrap();
        assert_eq!((f["kind"].as_str(), f["model"].as_str(), f["fallbackModel"].as_str()), (Some("refusal-recovered"), Some("claude-a"), Some("claude-b")));
        assert_eq!(claude_signal(Some(&json!({ "type": "assistant", "message": { "stop_reason": "end_turn" } }))), None);
        assert_eq!(claude_signal(Some(&Value::Null)), None);
        assert_eq!(claude_signal(None), None);
    }

    #[test]
    fn codex_refusals() {
        let ev = |p: Value| json!({ "type": "event_msg", "timestamp": "2026-10-07T12:00:00.000Z", "payload": p });
        let cyber = codex_signal(Some(&ev(json!({ "type": "error", "message": "flagged", "codex_error_info": "cyber_policy" })))).unwrap();
        assert_eq!((cyber["kind"].as_str(), cyber["category"].as_str(), cyber["text"].as_str()), (Some("refusal"), Some("cyber_policy"), Some("flagged")));
        let mis = codex_signal(Some(&ev(json!({ "type": "error", "codex_error_info": { "misalignment_policy_violation": { "detailed_explanation": "Why." } } })))).unwrap();
        assert_eq!(mis["explanation"], "Why.");
        assert_eq!(codex_signal(Some(&ev(json!({ "type": "error", "codex_error_info": "invalid_prompt" })))).unwrap()["kind"], "possible-refusal");
        assert_eq!(codex_signal(Some(&ev(json!({ "type": "error", "codex_error_info": "usage_limit_exceeded" })))), None);
        let reroute = codex_signal(Some(&ev(json!({ "type": "model_reroute", "from_model": "a", "to_model": "b", "reason": "risk" })))).unwrap();
        assert_eq!((reroute["model"].as_str(), reroute["fallbackModel"].as_str()), (Some("a"), Some("b")));
        assert_eq!(key_of(&json!({ "provider": "openai", "kind": "refusal", "at": null }), "codex:x"), "openai|refusal|codex:x@null");
    }
}
