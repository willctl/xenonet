//! core/claude-usage.js: Claude plan usage, as Anthropic answers the way
//! Claude Code's /usage asks (GET api.anthropic.com/api/oauth/usage with
//! Claude Code's own sign-in), and as Claude desktop's samples have it.
//!
//! Here, what each answer means: the sign-in a credentials text holds, the
//! windows an answer gives, why there's no reading (no sign-in, an expired
//! one, a pause Anthropic asked for, a refusal, another status), when to
//! ask again, and the samples a desktop file holds. Reading the keychain or
//! the file, the request itself and the vendor schemas' checks are the
//! caller's.

use crate::lines::opt_prop;
use serde_json::{json, Map, Value};
use xenonet_base::json::{self, number};
use xenonet_base::text::{js_number, js_string, truthy};
use xenonet_base::time::parse_iso;

/// Where usage is asked.
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Asked at most this often; a reading this old says nothing about the
/// windows now.
pub const USAGE_EVERY_MS: f64 = 5.0 * 60e3;
/// The keychain item Claude Code keeps its sign-in in, on macOS.
pub const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
/// How long to leave Anthropic when it asks for a pause without saying.
const PAUSE_MS: f64 = 15.0 * 60e3;

/// Claude Code's sign-in in a credentials text (the keychain item's or
/// .credentials.json's): its token and when it expires. Other tokens the
/// same text holds (MCP servers') are left alone.
pub fn token_of(text: Option<&str>) -> Option<(String, Option<f64>)> {
    let v = json::parse(text.unwrap_or("null")).ok()?;
    let oauth = opt_prop(Some(&v), "claudeAiOauth");
    let token = opt_prop(oauth, "accessToken").and_then(Value::as_str).filter(|t| !t.is_empty())?;
    // Number(expiresAt) || null: none said, or none that's a time.
    let expires = js_number(opt_prop(oauth, "expiresAt"));
    Some((token.to_string(), (expires != 0.0 && !expires.is_nan()).then_some(expires)))
}

/// A window's use in percent and when it resets: (null, null) when it
/// says no number.
fn window(w: Option<&Value>) -> (Value, Value) {
    let pct = js_number(opt_prop(w, "utilization"));
    if !pct.is_finite() {
        return (Value::Null, Value::Null);
    }
    let resets = opt_prop(w, "resets_at");
    let at = if truthy(resets) {
        // Date.parse(String(resets_at)).
        parse_iso(&js_string(resets.unwrap())).unwrap_or(f64::NAN)
    } else {
        f64::NAN
    };
    (number(pct), if at.is_finite() { number(at) } else { Value::Null })
}

/// The windows XenoNet shows, as percentages, with when each resets: {
/// fiveHour, sevenDay, resets: { fiveHour, sevenDay }, observedAt, source },
/// or None when the answer has neither.
pub fn parse_usage(body: Option<&Value>, now: f64) -> Option<Value> {
    let (five, five_at) = window(opt_prop(body, "five_hour"));
    let (seven, seven_at) = window(opt_prop(body, "seven_day"));
    if five.is_null() && seven.is_null() {
        return None;
    }
    Some(json!({ "fiveHour": five, "sevenDay": seven, "resets": { "fiveHour": five_at, "sevenDay": seven_at }, "observedAt": number(now), "source": "anthropic" }))
}

fn reason(text: &str) -> Value {
    json!({ "reason": text })
}

/// Why a sign-in can't be used now ({ reason }), or None when it can: none
/// found, or one that has expired (Claude Code refreshes it on its next
/// request; XenoNet never does).
pub fn sign_in_problem(cred: Option<&Value>, now: f64) -> Option<Value> {
    if !truthy(cred) {
        return Some(reason("no Claude Code sign-in found"));
    }
    let expires = opt_prop(cred, "expiresAt");
    if truthy(expires) && js_number(expires) <= now {
        return Some(reason("Claude Code's sign-in has expired; it refreshes on its next request"));
    }
    None
}

/// Why the request got no answer: `name` and `message` are the error's.
pub fn unreachable(name: Option<&Value>, message: Option<&Value>) -> Value {
    let why = if name.and_then(Value::as_str) == Some("TimeoutError") { "timed out".to_string() } else { message.map_or_else(|| "undefined".to_string(), js_string) };
    reason(&format!("couldn't reach Anthropic ({why})"))
}

/// What an answer's status says, before its body: a pause Anthropic asked
/// for (429, 403), with how long (its retry-after, else 15 minutes); a
/// refused sign-in (401); another status that isn't a success. None to read
/// the body.
pub fn status_problem(status: Option<&Value>, ok: bool, retry_after: Option<&Value>) -> Option<Value> {
    let code = status.and_then(Value::as_f64);
    let shown = status.map_or_else(|| "undefined".to_string(), js_string);
    if code == Some(429.0) || code == Some(403.0) {
        let after = js_number(retry_after);
        let wait = if after.is_finite() && after > 0.0 { after * 1000.0 } else { PAUSE_MS };
        return Some(json!({ "reason": format!("Anthropic asked to wait ({shown})"), "retryAfterMs": number(wait) }));
    }
    if code == Some(401.0) {
        return Some(reason("Claude Code's sign-in was refused; it refreshes on its next request"));
    }
    if !ok {
        return Some(reason(&format!("Anthropic answered {shown}")));
    }
    None
}

/// A reading from an answer's body (as its schema let it through): {
/// usage }, or { reason } when it has no windows.
pub fn reading(body: Option<&Value>, now: f64) -> Value {
    match parse_usage(body, now) {
        Some(u) => json!({ "usage": u }),
        None => reason("Anthropic's answer had no usage windows"),
    }
}

/// When to ask again after a reading: the cadence after one with usage
/// (None), else what Anthropic asked for, when it said.
pub fn next_in(r: Option<&Value>) -> Option<Value> {
    if truthy(opt_prop(r, "usage")) {
        return Some(Value::Null);
    }
    opt_prop(r, "retryAfterMs").cloned()
}

/// The version a desktop file says its format is, which picks the schema.
pub fn desktop_version(raw: Option<&Value>) -> Option<String> {
    opt_prop(raw, "version").filter(|v| !v.is_null()).map(js_string)
}

/// Why a desktop file's samples can't be read: a file with nothing to read
/// (`Plain`), or one JS would have thrown a TypeError on (`Type`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopError {
    Plain(String),
    Type(String),
}

/// The samples in Claude desktop's plan-usage-history.json ({t, u: {fh,
/// sd}}: five-hour and seven-day percentages; undocumented), as XenoNet's
/// samples, those without usage left out. `raw` is the file as parsed,
/// `checked` what its schema let through; a file with nothing to read (not
/// an object, or samples of another sort) is an error, so the samples there
/// were are kept.
pub fn desktop_samples(raw: Option<&Value>, checked: Option<&Value>) -> Result<Value, DesktopError> {
    let none = || DesktopError::Plain("plan-usage-history.json has no samples XenoNet can read".into());
    if !truthy(checked) {
        return Err(none());
    }
    let raw_samples = match raw {
        None => return Err(DesktopError::Type("Cannot read properties of undefined (reading 'samples')".into())),
        Some(Value::Null) => return Err(DesktopError::Type("Cannot read properties of null (reading 'samples')".into())),
        r => opt_prop(r, "samples"),
    };
    let samples = opt_prop(checked, "samples");
    if raw_samples.is_some() && samples.is_none() {
        return Err(none());
    }
    let list = match samples {
        None | Some(Value::Null) => &[][..],
        Some(Value::Array(a)) => a,
        Some(_) => return Err(DesktopError::Type("(d.samples ?? []).filter is not a function".into())),
    };
    let mut out = Vec::new();
    for x in list {
        let u = match x {
            Value::Null => return Err(DesktopError::Type("Cannot read properties of null (reading 'u')".into())),
            x => opt_prop(Some(x), "u"),
        };
        if !truthy(u) {
            continue;
        }
        let pct = |k: &str| opt_prop(u, k).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
        let mut m = Map::new();
        // A sample without a time keeps none (JS's would hold undefined).
        if let Some(t) = opt_prop(Some(x), "t") {
            m.insert("ts".into(), t.clone());
        }
        m.insert("fiveHour".into(), pct("fh"));
        m.insert("sevenDay".into(), pct("sd"));
        m.insert("resets".into(), Value::Null);
        m.insert("source".into(), Value::String("claude-desktop".into()));
        out.push(Value::Object(m));
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_ins() {
        let creds = r#"{"mcpOAuth":{"s":{"accessToken":"mcp"}},"claudeAiOauth":{"accessToken":"claude-token","expiresAt":1800000000000}}"#;
        assert_eq!(token_of(Some(creds)), Some(("claude-token".into(), Some(1.8e12))));
        assert_eq!(token_of(Some(r#"{"claudeAiOauth":{"accessToken":"t","expiresAt":"soon"}}"#)), Some(("t".into(), None)));
        assert_eq!(token_of(Some("not json")), None);
        assert_eq!(token_of(None), None);
        assert_eq!(token_of(Some(r#"{"claudeAiOauth":{"accessToken":""}}"#)), None);
        assert_eq!(sign_in_problem(None, 0.0).unwrap()["reason"], "no Claude Code sign-in found");
        assert!(sign_in_problem(Some(&json!({ "token": "t", "expiresAt": 5 })), 10.0).unwrap()["reason"].as_str().unwrap().contains("expired"));
        assert_eq!(sign_in_problem(Some(&json!({ "token": "t", "expiresAt": null })), 10.0), None);
    }

    #[test]
    fn answers() {
        let now = 1_800_000_000_000.0;
        let u = parse_usage(
            Some(&json!({ "five_hour": { "utilization": 10, "resets_at": "2026-10-03T14:00:00+00:00" }, "seven_day": { "utilization": 6.5, "resets_at": null } })),
            now,
        )
        .unwrap();
        assert_eq!(u["fiveHour"], 10);
        assert_eq!(u["sevenDay"], 6.5);
        assert_eq!(u["resets"]["fiveHour"].as_f64(), parse_iso("2026-10-03T14:00:00Z"));
        assert_eq!(u["resets"]["sevenDay"], Value::Null);
        assert_eq!(parse_usage(Some(&json!({ "extra_usage": {} })), now), None);
        assert_eq!(parse_usage(Some(&json!({ "five_hour": { "utilization": null } })), now).unwrap()["fiveHour"], 0);
        assert_eq!(status_problem(Some(&json!(429)), false, Some(&json!("120"))).unwrap()["retryAfterMs"], 120000);
        assert_eq!(status_problem(Some(&json!(403)), false, None).unwrap()["retryAfterMs"], 900000);
        assert!(status_problem(Some(&json!(401)), false, None).unwrap()["reason"].as_str().unwrap().contains("refused"));
        assert_eq!(status_problem(Some(&json!(500)), false, None).unwrap()["reason"], "Anthropic answered 500");
        assert_eq!(status_problem(Some(&json!(200)), true, None), None);
        assert_eq!(unreachable(Some(&json!("TimeoutError")), None)["reason"], "couldn't reach Anthropic (timed out)");
        assert_eq!(unreachable(Some(&json!("Error")), Some(&json!("getaddrinfo ENOTFOUND")))["reason"], "couldn't reach Anthropic (getaddrinfo ENOTFOUND)");
        assert_eq!(next_in(Some(&json!({ "usage": {} }))), Some(Value::Null));
        assert_eq!(next_in(Some(&json!({ "reason": "x", "retryAfterMs": 5 }))), Some(json!(5)));
        assert_eq!(next_in(Some(&json!({ "reason": "x" }))), None);
    }

    #[test]
    fn desktop() {
        let raw = json!({ "version": 2, "samples": [{ "t": 5, "u": { "fh": 14, "sd": 7 } }, { "t": 6 }] });
        assert_eq!(desktop_version(Some(&raw)).as_deref(), Some("2"));
        assert_eq!(desktop_samples(Some(&raw), Some(&raw)).unwrap(), json!([{ "ts": 5, "fiveHour": 14, "sevenDay": 7, "resets": null, "source": "claude-desktop" }]));
        assert!(matches!(desktop_samples(Some(&Value::Null), None), Err(DesktopError::Plain(_))));
        assert!(matches!(desktop_samples(Some(&json!({ "samples": 1 })), Some(&json!({}))), Err(DesktopError::Plain(_))));
        assert_eq!(desktop_samples(Some(&json!({})), Some(&json!({}))).unwrap(), json!([]));
    }
}
