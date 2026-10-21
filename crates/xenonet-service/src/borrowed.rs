//! What the routes, the gateway and the wire crates take from JS's
//! built-ins (Date.now(), crypto.randomUUID(), process.platform), and the
//! engine's attest for a sender they name with plain values.

use serde_json::Value;
use xenonet_engine::sql::Arg;

/// Milliseconds since the epoch, as Date.now() gives them.
pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// crypto.randomUUID(): a version 4 UUID.
pub fn random_uuid() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system's random numbers");
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = xenonet_base::jsstr::hex(&b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Who sent a message, for attest().
pub struct Sender<'a> {
    pub id: &'a str,
    pub kind: &'a str,
    pub name: Option<String>,
    pub product: Option<String>,
}

/// attest (core/engine/attest.js), the engine's: the envelope for a
/// message from `sender`, authenticated by `method`, with the route's
/// verdict.
pub fn attest(sender: Sender<'_>, method: &str, verdict: &Value, home: &str, now: f64, platform: &str) -> Value {
    let sender = Arg::Obj(vec![
        ("id".into(), Arg::text(sender.id)),
        ("kind".into(), Arg::text(sender.kind)),
        ("name".into(), Arg::opt_text(sender.name.as_deref())),
        ("product".into(), Arg::opt_text(sender.product.as_deref())),
    ]);
    // It fails only when the system has no random numbers for the id.
    let env = xenonet_engine::attest::attest(&sender, &Arg::text(method), &Arg::from(verdict), &Arg::text(home), &Arg::num(now), platform).expect("the system's random numbers");
    env.to_json()
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
    use serde_json::json;

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
