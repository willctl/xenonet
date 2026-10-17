//! Small pure pieces of other slices' cores that the routes import
//! directly, copied here until those crates export them (then the routes
//! use theirs, and these go). Each says which JS it copies.

use serde_json::{json, Value};
use xenonet_base::ids;
use xenonet_base::paths::{base_name, tilde};

/// isAgentId (core/engine/engine.js): a session's own id, claude:… or codex:…
/// or any app XenoNet reads (SESSION_LOCAL).
pub fn is_agent_id(id: Option<&str>) -> bool {
    id.is_some_and(ids::is_session_id)
}

/// isActorAgent (core/engine/engine.js): an actor of this machine, never you.
pub fn is_actor_agent(id: &str) -> bool {
    matches!(ids::parse_actor_id(id), Some((None, _))) && id != ids::YOU
}

const KEYS: [&str; 6] = ["project", "identity", "app", "machine", "status", "by"];

/// parseSelector (core/delivery/selector.js): the terms, or None for text
/// that isn't a selector.
pub fn parse_selector(text: &str) -> Option<Vec<(String, String)>> {
    let s = xenonet_base::text::js_trim(text);
    if s.is_empty() {
        return None;
    }
    if s == "all" {
        return Some(Vec::new());
    }
    let mut terms = Vec::new();
    for p in s.split(|c: char| c == '+' || xenonet_base::text::is_js_space(c)).filter(|p| !p.is_empty()) {
        let (k, v) = p.split_once(':')?;
        let key = k.to_ascii_lowercase();
        // [^\s+]+: the value has no spaces or + (split took those) and isn't empty.
        if !KEYS.contains(&key.as_str()) || v.is_empty() {
            return None;
        }
        terms.push((key, v.to_lowercase()));
    }
    Some(terms)
}

/// fromOpenCodeId (core/delivery/opencode.js): ses_… or opencode:ses_… as
/// XenoNet's opencode:ses-…; any other id as it was, trimmed.
pub fn from_opencode_id(id: &Value) -> String {
    let s = match id {
        Value::Null => String::new(),
        v => xenonet_base::text::js_string(v),
    };
    let s = xenonet_base::text::js_trim(&s).to_string();
    let rest = s.strip_prefix("opencode:").unwrap_or(&s);
    match rest.strip_prefix("ses_") {
        Some(tail) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_alphanumeric()) => format!("opencode:ses-{tail}"),
        _ => s,
    }
}

/// crypto.randomUUID(): a version 4 UUID.
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn process_of(verdict: &Value, method: &str) -> &'static str {
    if method == "window" || method == "gateway" {
        return "n/a";
    }
    if !xenonet_base::text::truthy(verdict.get("checked")) {
        return "unchecked";
    }
    if xenonet_base::text::truthy(verdict.get("sure")) {
        "signed"
    } else {
        "checked"
    }
}

fn strength_of(method: &str, proc: &str) -> &'static str {
    match method {
        "window" => "strong",
        "session" => match proc {
            "signed" => "strong",
            "checked" => "partial",
            _ => "weak",
        },
        "token" | "grant" if proc == "checked" || proc == "signed" => "partial",
        _ => "weak",
    }
}

/// Who sent a message, for attest().
pub struct Sender<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub name: Option<String>,
    pub product: Option<String>,
}

/// attest (core/engine/attest.js): the envelope for a message from
/// `sender`, authenticated by `method`, with the route's verdict.
pub fn attest(sender: Sender, method: &str, verdict: &Value, home: &str, now: f64, platform: &str) -> Value {
    let proc = process_of(verdict, method);
    let exe = verdict.get("chain").and_then(|c| c.get(0)).and_then(|p| p.get("exe")).and_then(Value::as_str);
    let mut env = json!({
        "schema": "message/envelope/v1",
        "mid": format!("msg_{}", random_uuid()),
        "at": xenonet_base::json::number(now),
        "sender": { "id": sender.id, "kind": sender.kind, "name": sender.name, "product": sender.product },
        "auth": { "method": method, "process": proc, "strength": strength_of(method, proc) },
        "client": exe.map(|e| json!({ "exe": tilde(e, home), "name": base_name(e) })),
        "transport": match method { "grant" => "tunnel", "gateway" => "gateway", _ => "this machine" },
    });
    env["line"] = Value::String(verified_line(&env, platform));
    env
}

fn how(method: &str) -> String {
    match method {
        "session" => "its own session's process".into(),
        "token" => "its token".into(),
        "window" => "XenoNet's own window, with its key".into(),
        "grant" => "its remote connection's credential".into(),
        "gateway" => "its gateway token alone".into(),
        m => m.into(),
    }
}

/// verifiedLine (core/engine/attest.js).
pub fn verified_line(env: &Value, platform: &str) -> String {
    let s = &env["sender"];
    let str_of = |v: &Value| match v {
        Value::String(x) => Some(x.clone()),
        Value::Null => None,
        other => Some(xenonet_base::text::js_string(other)),
    };
    let id = str_of(&s["id"]).unwrap_or_default();
    let who = if s["kind"] == "you" {
        "you, at XenoNet's own window".to_string()
    } else {
        format!("{} \"{}\" ({id})", if s["kind"] == "actor" { "the actor" } else { "the session" }, str_of(&s["name"]).unwrap_or_else(|| id.clone()))
    };
    let method = env["auth"]["method"].as_str().unwrap_or("");
    let proc = match env["auth"]["process"].as_str().unwrap_or("") {
        "signed" => "; XenoNet checked its process and the app's signature".to_string(),
        "checked" => format!("; XenoNet checked the process it came from{}", env["client"]["name"].as_str().map(|n| format!(" ({n})")).unwrap_or_default()),
        "unchecked" => format!(
            "; XenoNet can't check which process sent it on {} yet, so it's {}",
            match platform {
                "win32" => "Windows",
                "darwin" => "this Mac",
                _ => "Linux",
            },
            if method == "token" { "the token alone" } else { "its word" }
        ),
        _ => String::new(),
    };
    let place = if method == "gateway" { "from outside this machine, through XenoNet's actor gateway" } else { "on this machine" };
    format!("verified: {who}, by {}{proc}, {place}. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.", how(method))
}

/// process.platform's name for this one.
pub fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_as_the_js_reads_them() {
        assert_eq!(parse_selector("all"), Some(vec![]));
        assert_eq!(parse_selector("project:XenoNet status:idle"), Some(vec![("project".into(), "xenonet".into()), ("status".into(), "idle".into())]));
        assert_eq!(parse_selector("app:opencode+by:hermes").map(|t| t.len()), Some(2));
        assert_eq!(parse_selector("repo"), None);
        assert_eq!(parse_selector("claude:abc"), None);
        assert_eq!(parse_selector("project:"), None);
        assert_eq!(parse_selector("  "), None);
    }

    #[test]
    fn opencode_ids() {
        assert_eq!(from_opencode_id(&json!("ses_abc123")), "opencode:ses-abc123");
        assert_eq!(from_opencode_id(&json!(" opencode:ses_X ")), "opencode:ses-X");
        assert_eq!(from_opencode_id(&json!("claude:abc")), "claude:abc");
        assert_eq!(from_opencode_id(&Value::Null), "");
    }

    #[test]
    fn agents_and_actors() {
        assert!(is_agent_id(Some("claude:11111111-2222-3333-4444-555555555555")));
        assert!(!is_agent_id(Some("someone")));
        assert!(!is_agent_id(None));
        assert!(is_actor_agent("actor:muse"));
        assert!(!is_actor_agent("actor:you"));
        assert!(!is_actor_agent("@box:actor:muse"));
    }

    #[test]
    fn envelopes_say_how_xenonet_knows() {
        let env = attest(Sender { id: "actor:you", kind: "you", name: Some("you".into()), product: None }, "window", &Value::Null, "/home/nova", 5.0, "darwin");
        assert_eq!(env["auth"], json!({ "method": "window", "process": "n/a", "strength": "strong" }));
        assert_eq!(env["line"], "verified: you, at XenoNet's own window, by XenoNet's own window, with its key, on this machine. This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing.");
        let v = json!({ "checked": true, "chain": [{ "exe": "/home/nova/bin/claude" }] });
        let env = attest(Sender { id: "claude:x", kind: "agent", name: None, product: None }, "session", &v, "/home/nova", 5.0, "linux");
        assert_eq!(env["client"], json!({ "exe": "~/bin/claude", "name": "claude" }));
        assert!(env["line"].as_str().unwrap().contains("the session \"claude:x\" (claude:x), by its own session's process; XenoNet checked the process it came from (claude)"));
        assert_eq!(env["mid"].as_str().unwrap().len(), 40);
    }
}
