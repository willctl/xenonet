//! What the map reads of Codex besides its rollouts' lines: the account
//! it's signed in to (auth.json and its ID token), where a thread's rollout
//! is, and the program XenoNet runs its own turns with.

use serde_json::{json, Map, Value};
use xenonet_adapter::dirs;
use xenonet_adapter::platform::{SessionFiles, Signin};
use xenonet_adapter::reader::VendorCheck;
use xenonet_os::paths;
use xenonet_sources::lines::{opt_prop, Js};
use xenonet_text::text::{display, to_number, trim, truthy};

/// `typeof v === "string" && v.trim() ? v.trim() : null`
fn str_of(v: Js<'_>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !trim(s).is_empty() => Some(trim(s).to_string()),
        _ => None,
    }
}

fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

fn tpl(v: Js<'_>) -> String {
    v.map_or_else(|| "undefined".into(), display)
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::String)
}

// Buffer.from(s, "base64url"): both alphabets, other characters passed
// over, and the end at the first "=".
fn base64url(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    out
}

/// The claims in Codex's ID token (~/.codex/auth.json), decoded without
/// verifying; null when there are none to read. Nothing else in the file
/// is read.
pub fn id_token_claims(auth: Js<'_>) -> Value {
    if !truthy(auth) || !matches!(auth, Some(Value::Object(_) | Value::Array(_))) {
        return Value::Null;
    }
    let token = tpl(opt_prop(opt_prop(auth, "tokens"), "id_token"));
    let Some(part) = token.split('.').nth(1) else { return Value::Null };
    xenonet_text::json::parse(&String::from_utf8_lossy(&base64url(part))).unwrap_or(Value::Null)
}

/// Codex's sign-in, from auth.json and its ID token's claims (as the lenient
/// reader left them). An API key sign-in says nothing about who.
pub fn chatgpt_account(auth: Js<'_>, claims: Js<'_>) -> Value {
    if !truthy(auth) || !matches!(auth, Some(Value::Object(_) | Value::Array(_))) {
        return Value::Null;
    }
    let empty = Value::Object(Map::new());
    let oa = nn(opt_prop(claims, "https://api.openai.com/auth")).unwrap_or(&empty);
    let oa = Some(oa);
    let tokens = opt_prop(auth, "tokens");
    let workspace = str_of(opt_prop(oa, "chatgpt_account_id")).or_else(|| str_of(opt_prop(tokens, "account_id")));
    let mode = nn(opt_prop(auth, "auth_mode"));
    if let Some(w) = workspace {
        if mode.is_none() || mode.and_then(Value::as_str) == Some("chatgpt") {
            let since = to_number(opt_prop(claims, "auth_time")) * 1000.0;
            let since = if since == 0.0 || since.is_nan() { Value::Null } else { xenonet_text::json::number(since) };
            return json!({ "id": format!("chatgpt:{w}"), "kind": "chatgpt", "handle": opt_str(str_of(opt_prop(claims, "email"))), "detail": { "plan": opt_str(str_of(opt_prop(oa, "chatgpt_plan_type"))) }, "since": since });
        }
    }
    if truthy(opt_prop(auth, "OPENAI_API_KEY")) || mode.and_then(Value::as_str) == Some("apikey") {
        return json!({ "id": "openai:api-key", "kind": "openai", "handle": null, "detail": { "plan": null } });
    }
    Value::Null
}

// auth.json, with its ID token's claims checked leniently against their
// schema.
fn read_account(auth: &Value, vendor: &dyn VendorCheck) -> Value {
    let claims = id_token_claims(Some(auth));
    let claims = if claims.is_null() { claims } else { vendor.check("openai/codex/auth-claims", claims, None, None).unwrap_or(Value::Null) };
    chatgpt_account(Some(auth), Some(&claims))
}

/// Where Codex keeps its sign-in: auth.json in its home. A thread runs
/// under it when it names OpenAI as its provider, or none; accounts were
/// once saved as `openai`.
pub fn signin() -> Signin {
    Signin { setting: "codexHome", file: Some("auth.json"), read: read_account, aliases: &["openai"], provider: Some("openai"), owners: false }
}

/// The codex XenoNet runs its own turns with: the one the ChatGPT app
/// bundles, else codex on the PATH.
pub fn program(env: &dirs::Env, platform: &str, home: &str, look: &mut dyn dirs::Look) -> String {
    dirs::bundled_codex_binaries(env, platform, home, look).into_iter().next().unwrap_or_else(|| "codex".into())
}

// The first UUID in a text, either case, in lowercase.
fn uuid_in(text: &str) -> Option<String> {
    re!(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").find(&text.to_ascii_lowercase()).map(|m| m.as_str().to_string())
}

// /^rollout-.*\.jsonl(\.zst)?$/.
fn is_rollout(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("rollout-") else { return false };
    let middle = rest.strip_suffix(".jsonl").or_else(|| rest.strip_suffix(".jsonl.zst"));
    middle.is_some_and(|m| !m.contains(['\n', '\r', '\u{2028}', '\u{2029}']))
}

/// Rollouts are found by their thread id, from a listing of the rollout
/// folders made once and again when an id isn't in it (at most every 10
/// seconds).
#[derive(Default)]
pub struct Files {
    rollouts: Vec<(String, String)>,
    listed_at: f64,
}

impl Files {
    /// Lists the rollout folders again.
    fn list(&mut self, dirs: &[String], now: f64) {
        self.listed_at = now;
        for root in dirs {
            for file in paths::list_files(root, &mut |name| is_rollout(name), 4) {
                if let Some(id) = uuid_in(&xenonet_os::nodepath::basename(&file)) {
                    match self.rollouts.iter_mut().find(|(k, _)| *k == id) {
                        Some(r) => r.1 = file,
                        None => self.rollouts.push((id, file)),
                    }
                }
            }
        }
    }
}

impl SessionFiles for Files {
    fn files(&mut self, dirs: &[String], raw: &str, now: f64) -> Vec<(String, Option<String>)> {
        if !self.rollouts.iter().any(|(k, _)| *k == raw) && now - self.listed_at > 10e3 {
            self.list(dirs, now);
        }
        self.rollouts.iter().find(|(k, _)| *k == raw).map(|(_, f)| vec![(f.clone(), None)]).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_its_sign_in() {
        let claims = json!({ "email": "n@x", "auth_time": 1_700_000_000, "https://api.openai.com/auth": { "chatgpt_account_id": "ws", "chatgpt_plan_type": "plus" } });
        let token = format!("e30.{}.sig", "eyJhIjoxfQ");
        assert_eq!(id_token_claims(Some(&json!({ "tokens": { "id_token": token } }))), json!({ "a": 1 }));
        let auth = json!({ "auth_mode": "chatgpt", "tokens": {} });
        assert_eq!(chatgpt_account(Some(&auth), Some(&claims))["since"], json!(1_700_000_000_000u64));
        assert_eq!(chatgpt_account(Some(&json!({ "auth_mode": "apikey" })), None)["id"], "openai:api-key");
        assert_eq!(base64url("e30=xyz"), b"{}");
        assert_eq!(base64url("e3$0"), b"{}");
    }

    #[test]
    fn finds_a_rollout_by_its_thread() {
        assert!(is_rollout("rollout-2025-01-01T00-00-00-0199abcd-0000-7000-8000-000000000001.jsonl") && is_rollout("rollout-x.jsonl.zst") && !is_rollout("x.jsonl"));
        assert_eq!(uuid_in("rollout-2025-0199ABCD-0000-7000-8000-000000000001.jsonl").as_deref(), Some("0199abcd-0000-7000-8000-000000000001"));
        assert_eq!(uuid_in("no id"), None);
    }
}
