//! What the routes, the gateway and the wire crates take from JS's
//! built-ins (Date.now(), crypto.randomUUID(), process.platform), and the
//! engine's attest for a sender they name with plain values.

use serde_json::Value;

/// Milliseconds since the epoch, as Date.now() gives them.
pub use xenonet_text::time::now_ms;
// crypto.randomUUID() and process.platform are xenonet-os's, which the
// drivers use too.
pub use xenonet_os::{platform, random_uuid};

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
    let sender = xenonet_engine::attest::Sender { id: sender.id, kind: sender.kind, name: sender.name.as_deref(), product: sender.product.as_deref() };
    // It fails only when the system has no random numbers for the id.
    xenonet_engine::attest::attest(sender, method, verdict, home, now, platform).expect("the system's random numbers")
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
