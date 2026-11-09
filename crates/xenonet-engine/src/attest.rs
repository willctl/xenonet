//! How XenoNet knows who sent a message (docs/guide/agents.md, Messages):
//! an envelope, message/envelope/v1, that XenoNet writes from what its own
//! checks found when the message came in, kept apart from what the sender
//! said. The sender's words can't write into it: it's built from the
//! route's verdict, never from the request's body.
//!
//!   sender     { id, kind: agent|actor|you, name, product }
//!   auth       how it proved who it is: session, token, window, grant or
//!              gateway
//!   process    what XenoNet saw of the program that sent it: signed,
//!              checked, unchecked, or n/a
//!   strength   strong, partial or weak: how far the checks go
//!   client     the program it came from, when XenoNet saw it (home as ~)
//!   line       the same in a sentence, for people and agents
//!
//! It says what XenoNet checked and names what it couldn't; it's never an
//! approval of anything the message asks.

use crate::js::template;
use crate::peer::Thrown;
use serde_json::{json, Value};
use xenonet_base::text::truthy;

pub const SCHEMA: &str = "message/envelope/v1";

/// Who sent a message, as the route knows it.
#[derive(Clone, Copy, Debug)]
pub struct Sender<'a> {
    pub id: &'a str,
    /// agent, actor or you.
    pub kind: &'a str,
    pub name: Option<&'a str>,
    pub product: Option<&'a str>,
}

fn process_of(verdict: &Value, method: &str) -> &'static str {
    if method == "window" || method == "gateway" {
        return "n/a";
    }
    if !truthy(verdict.get("checked")) {
        return "unchecked";
    }
    if truthy(verdict.get("sure")) {
        "signed"
    } else {
        "checked"
    }
}

fn strength_of(method: &str, process: &str) -> &'static str {
    match method {
        "window" => "strong",
        "session" => match process {
            "signed" => "strong",
            "checked" => "partial",
            _ => "weak",
        },
        "token" | "grant" if matches!(process, "checked" | "signed") => "partial",
        _ => "weak",
    }
}

fn uuid() -> Result<String, Thrown> {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = xenonet_base::jsstr::hex(&b);
    Ok(format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32]))
}

/// The envelope for a message from `sender`, authenticated by `method`,
/// with the route's `verdict` (verify): { checked, sure, chain: [{ exe }] }.
/// It fails only when the system has no random numbers for the id.
pub fn attest(sender: Sender<'_>, method: &str, verdict: &Value, home: &str, now: f64, platform: &str) -> Result<Value, Thrown> {
    let process = process_of(verdict, method);
    let strength = strength_of(method, process);
    let exe = verdict.get("chain").and_then(Value::as_array).and_then(|l| l.first()).and_then(|f| f.get("exe")).filter(|e| truthy(Some(e)));
    let client = match exe {
        Some(exe) => {
            let text = template(Some(exe));
            let shown = if home.is_empty() { text.clone() } else { xenonet_base::paths::tilde(&text, home) };
            json!({ "exe": shown, "name": xenonet_base::nodepath::basename(&text) })
        }
        None => Value::Null,
    };
    let transport = match method {
        "grant" => "tunnel",
        "gateway" => "gateway",
        _ => "this machine",
    };
    let mut env = json!({
        "schema": SCHEMA,
        "mid": format!("msg_{}", uuid()?),
        "at": xenonet_base::json::number(now),
        "sender": { "id": sender.id, "kind": sender.kind, "name": sender.name, "product": sender.product },
        "auth": { "method": method, "process": process, "strength": strength },
        "client": client,
        "transport": transport,
    });
    let line = verified_line(&env, platform);
    env["line"] = Value::String(line);
    Ok(env)
}

/// How each method proves who sent it, in words.
fn how(method: &str) -> String {
    match method {
        "session" => "its own session's process".into(),
        "token" => "its token".into(),
        "window" => "XenoNet's own window, with its key".into(),
        "grant" => "its remote connection's credential".into(),
        "gateway" => "its gateway token alone".into(),
        other => crate::js::proto_member(other).unwrap_or_else(|| other.to_string()),
    }
}

/// "verified: …": what XenoNet checked, in a sentence, and what it
/// couldn't.
pub fn verified_line(env: &Value, platform: &str) -> String {
    let sender = &env["sender"];
    let kind = sender.get("kind").and_then(Value::as_str);
    let who = if kind == Some("you") {
        "you, at XenoNet's own window".to_string()
    } else {
        let what = if kind == Some("actor") { "the actor" } else { "the session" };
        let name = sender.get("name").filter(|n| !n.is_null()).or_else(|| sender.get("id"));
        format!("{what} \"{}\" ({})", template(name), template(sender.get("id")))
    };
    let auth = &env["auth"];
    let method = template(auth.get("method"));
    let how = how(&method);
    let process = match auth.get("process").and_then(Value::as_str) {
        Some("signed") => "; XenoNet checked its process and the app's signature".to_string(),
        Some("checked") => {
            let client = &env["client"];
            let name = if truthy(Some(client)) { format!(" ({})", template(client.get("name"))) } else { String::new() };
            format!("; XenoNet checked the process it came from{name}")
        }
        Some("unchecked") => {
            let os = match platform {
                "win32" => "Windows",
                "darwin" => "this Mac",
                _ => "Linux",
            };
            let word = if method == "token" { "the token alone" } else { "its word" };
            format!("; XenoNet can't check which process sent it on {os} yet, so it's {word}")
        }
        _ => String::new(),
    };
    let place = if method == "gateway" { "from outside this machine, through XenoNet's actor gateway" } else { "on this machine" };
    format!("verified: {who}, by {how}{process}, {place}. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.")
}

/// A message's own words as they're shown beneath XenoNet's lines: each of
/// its lines quoted, with what could pass for a line of XenoNet's own
/// taken out (line breaks of other kinds, controls, and characters that
/// reorder text).
pub fn quoted(text: &str) -> String {
    let lines = crate::re!(r"\r\n?|[\u0085\u2028\u2029\u000b\u000c]").replace_all(text, "\n");
    let clean = crate::re!(r"[\u0000-\u0008\u000e-\u001f\u007f-\u0084\u0086-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]").replace_all(&lines, "");
    clean.split('\n').map(|l| format!("  > {l}")).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_session_is_strong() {
        let verdict = json!({ "checked": true, "sure": true, "chain": [{ "exe": "/Users/me/bin/claude" }] });
        let sender = Sender { id: "claude:a", kind: "agent", name: Some("Docs"), product: None };
        let env = attest(sender, "session", &verdict, "/Users/me", 1.0, "darwin").unwrap();
        assert_eq!(env["auth"]["strength"], "strong");
        assert_eq!(env["client"]["exe"], "~/bin/claude");
        assert!(env["line"].as_str().unwrap().starts_with("verified: the session \"Docs\" (claude:a), by its own session's process; XenoNet checked its process"));
    }

    #[test]
    fn quoting_takes_out_what_could_pass_for_xenonets_own_lines() {
        assert_eq!(quoted("a\r\nb\u{2028}c\u{202e}d"), "  > a\n  > b\n  > cd");
    }
}
