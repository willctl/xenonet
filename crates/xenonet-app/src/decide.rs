//! core/decide.js: decisions XenoNet hands to a decision model (Jev,
//! TypeSafe's model on OpenRouter, by default), which answers typed
//! questions with how sure it is. Nothing goes unless you ask: each kind of
//! decision builds exactly what's sent from shapes, never transcripts. The
//! key is OpenRouter's, kept in the credential store and sent only in the
//! request's header. Each decision goes in XenoNet's log with its model,
//! time and cost, never what was asked.
//!
//! The waiting (for the credential store and for OpenRouter) is the
//! caller's: this builds the request and reads the answer.

use crate::jstext::{slice16, url_host};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_base::text::js_number;
use xenonet_base::units::to_fixed;

pub const DECISIONS_URL: &str = "https://openrouter.ai/api/alpha/decisions";
pub const DEFAULT_MODEL: &str = "~typesafe/jev-latest";
pub const KEY_TARGET: &str = "XenoNet/OpenRouter/decisions";
/// An OpenRouter key.
pub const KEY: &str = "^sk-or-[A-Za-z0-9_-]{20,200}$";
pub const KEY_COMMENT: &str = "OpenRouter key XenoNet uses for decision models";
pub const TIMEOUT_MS: f64 = 30000.0;

fn key_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(KEY).unwrap())
}

/// A decision that couldn't be made, as an HTTP answer would say it.
#[derive(Clone, Debug, PartialEq)]
pub struct DecisionError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

fn fail(status: u16, code: &'static str, message: impl Into<String>) -> DecisionError {
    DecisionError { status, code, message: message.into() }
}

/// A line for XenoNet's log: its level, code and message.
#[derive(Clone, Debug, PartialEq)]
pub struct LogLine {
    pub level: &'static str,
    pub code: String,
    pub message: String,
}

/// Where decisions go and which model makes them: XENONET_DECISIONS_URL and
/// XENONET_DECISION_MODEL when they're set and not empty.
pub fn settings(url: Option<&str>, model: Option<&str>) -> (String, String) {
    (url.filter(|u| !u.is_empty()).unwrap_or(DECISIONS_URL).into(), model.filter(|m| !m.is_empty()).unwrap_or(DEFAULT_MODEL).into())
}

/// A key as given, trimmed, if it's an OpenRouter one.
pub fn check_key(k: &str) -> Result<String, DecisionError> {
    let k = xenonet_base::text::js_trim(k);
    if !key_re().is_match(k) {
        return Err(fail(400, "bad-request", "that isn't an OpenRouter key (sk-or-…)"));
    }
    Ok(k.into())
}

/// Why there's no decision without a key.
pub fn no_key() -> DecisionError {
    fail(409, "no-key", "XenoNet has no OpenRouter key for decisions yet")
}

/// The request: its headers and body. Every question in it is answered
/// together; zero data retention is asked of the provider, and no user or
/// session ids go.
pub fn request(model: &str, key: &str, state: Option<&Value>, questions: Option<&Value>) -> (Vec<(&'static str, String)>, String) {
    let headers = vec![("authorization", format!("Bearer {key}")), ("content-type", "application/json".into()), ("x-title", "XenoNet".into())];
    let mut body = Map::new();
    body.insert("model".into(), json!(model));
    // JSON.stringify leaves out a key whose value is undefined.
    if let Some(s) = state {
        body.insert("state".into(), s.clone());
    }
    if let Some(q) = questions {
        body.insert("questions".into(), q.clone());
    }
    body.insert("provider".into(), json!({ "zdr": true }));
    (headers, xenonet_base::json::stringify(&Value::Object(body), 0))
}

/// OpenRouter couldn't be reached: what's logged and the error.
pub fn unreachable(url: &str, kind: &str, timed_out: bool, message: &str) -> (LogLine, DecisionError) {
    let why = if timed_out { "it took too long" } else { message };
    let host = url_host(url).unwrap_or_default();
    (
        LogLine { level: "warn", code: "decide.unreachable".into(), message: format!("couldn't reach {host} for a {kind} decision: {why}") },
        fail(502, "unreachable", format!("couldn't reach OpenRouter: {why}")),
    )
}

/// OpenRouter said no: what's logged and the error. A key it refuses is
/// the person's to fix (409); anything else is upstream's (502).
pub fn refused(status: u16, body: Option<&Value>, kind: &str) -> (LogLine, DecisionError) {
    let why = match body.and_then(|b| b.get("error")).and_then(|e| e.get("message")) {
        Some(Value::String(m)) => slice16(m, 200),
        _ => format!("HTTP {status}"),
    };
    let to = if status == 401 || status == 403 { 409 } else { 502 };
    (
        LogLine { level: "warn", code: "decide.refused".into(), message: format!("OpenRouter answered {status} to a {kind} decision: {why}") },
        fail(to, if status == 401 { "bad-key" } else { "refused" }, format!("OpenRouter said: {why}")),
    )
}

/// OpenRouter answered: what's logged and the decision.
pub fn answered(body: Option<&Value>, model: &str, kind: &str, questions: usize, ms: f64) -> (LogLine, Value) {
    let field = |k: &str| body.and_then(|b| b.get(k)).filter(|v| !v.is_null());
    let cost = js_number(body.and_then(|b| b.get("usage")).and_then(|u| u.get("cost")));
    let cost = if cost.is_nan() || cost == 0.0 { 0.0 } else { cost };
    let answered_by = field("model").cloned().unwrap_or_else(|| json!(model));
    let by = match &answered_by {
        Value::String(s) => s.clone(),
        v => xenonet_base::text::js_string(v),
    };
    let plural = if questions == 1 { "" } else { "s" };
    let line = LogLine {
        level: "info",
        code: format!("decide.{kind}"),
        message: format!("{by} answered {questions} question{plural} in {} ms, ${}", xenonet_base::json::number_string(ms), to_fixed(cost, 5)),
    };
    let mut out = Map::new();
    out.insert("answers".into(), field("answers").cloned().unwrap_or_else(|| json!({})));
    out.insert("model".into(), answered_by);
    out.insert("provider".into(), field("provider").cloned().unwrap_or(Value::Null));
    out.insert("cost".into(), xenonet_base::json::number(cost));
    out.insert("ms".into(), xenonet_base::json::number(ms));
    (line, Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_requests_and_answers() {
        assert_eq!(check_key(" sk-or-v1-CANARY0123456789abcdefABCDEF ").unwrap(), "sk-or-v1-CANARY0123456789abcdefABCDEF");
        assert_eq!(check_key("sk-ant-not-openrouter-0000000000").unwrap_err().code, "bad-request");
        let (headers, body) = request("m", "sk-or-k", Some(&json!({ "folders": ["a"] })), Some(&json!({})));
        assert_eq!(headers[0].1, "Bearer sk-or-k");
        assert_eq!(body, r#"{"model":"m","state":{"folders":["a"]},"questions":{},"provider":{"zdr":true}}"#);
        let (line, d) = answered(Some(&json!({ "model": "jev-1", "answers": { "f0": 1 }, "usage": { "cost": 0.00004 } })), "m", "projects", 1, 12.0);
        assert_eq!(line.message, "jev-1 answered 1 question in 12 ms, $0.00004");
        assert_eq!(d["provider"], Value::Null);
        let (_, e) = refused(401, Some(&json!({ "error": { "message": "No auth credentials found" } })), "x");
        assert_eq!((e.status, e.code, e.message.as_str()), (409, "bad-key", "OpenRouter said: No auth credentials found"));
        let (line, e) = unreachable(DECISIONS_URL, "x", true, "");
        assert_eq!(line.message, "couldn't reach openrouter.ai for a x decision: it took too long");
        assert_eq!(e.code, "unreachable");
    }
}
