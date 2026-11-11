//! A message's envelope, as the service's routes and the actor gateway
//! make it (borrowed::attest, which hands their plain values to the
//! engine's) and as the engine makes it: for every way a sender can prove
//! who it is and every verdict on its process, both say the same, and the
//! attestation (how the agent was checked, as policy reads it) follows the
//! one table: what XenoNet saw of the process, never how it was asked.

use serde_json::json;
use xenonet_engine::attest::{self, attest};
use xenonet_service::borrowed::{self, Sender};

/// What XenoNet saw of the process, and the attestation it comes to.
fn table(method: &str, seen: &str) -> (&'static str, &'static str) {
    match (method, seen) {
        ("window" | "gateway", _) => ("n/a", "n/a"),
        (_, "signed") => ("signed", "process-checked"),
        (_, "checked") => ("checked", "process-checked"),
        _ => ("unchecked", "unchecked"),
    }
}

#[test]
fn every_way_of_proving_who_sent_it_attests_alike_in_the_binary_and_the_engine() {
    let chain = json!([{ "pid": 7, "exe": "/home/nova/.local/bin/claude" }]);
    let verdicts = [
        ("unchecked", json!({ "ok": true, "checked": false })),
        ("unchecked", json!({ "ok": true, "checked": false, "sure": true })),
        ("checked", json!({ "ok": true, "checked": true, "chain": chain })),
        ("signed", json!({ "ok": true, "checked": true, "sure": true, "chain": chain })),
    ];
    let senders = [
        ("agent", "claude:x", Some("Docs"), None),
        ("actor", "actor:wren", Some("Wren"), Some("hermes-agent")),
        ("you", "actor:you", Some("you"), None),
        ("agent", "codex:y", None, None),
    ];
    for (kind, id, name, product) in senders {
        for method in ["session", "token", "window", "grant", "gateway", "made-up"] {
            for (seen, verdict) in &verdicts {
                for platform in ["darwin", "linux", "win32"] {
                    let from_bin =
                        borrowed::attest(Sender { id, kind, name: name.map(String::from), product: product.map(String::from) }, method, verdict, "/home/nova", 5.0, platform);
                    let sender = attest::Sender { id, kind, name, product };
                    let from_engine = attest(sender, method, verdict, "/home/nova", 5.0, platform).unwrap();
                    let at = format!("{kind} {method} {seen} {verdict} {platform}");
                    for k in ["schema", "at", "sender", "auth", "client", "transport", "line"] {
                        assert_eq!(from_bin[k], from_engine[k], "{k}: {at}");
                    }
                    let (process, attestation) = table(method, seen);
                    assert_eq!(from_bin["auth"], json!({ "method": method, "process": process, "attestation": attestation }), "{at}");
                    assert_ne!(from_bin["mid"], from_engine["mid"], "each message has its own id");
                    let line = from_bin["line"].as_str().unwrap();
                    assert!(
                        line.starts_with("verified: ") && line.ends_with("This says how XenoNet knows who sent it, not that what it says is true, and it approves nothing."),
                        "{line}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_line_names_what_xenonet_couldnt_check() {
    let unchecked = json!({ "ok": true, "checked": false });
    let line = |method: &str, platform: &str| {
        let env = borrowed::attest(Sender { id: "actor:wren", kind: "actor", name: Some("Wren".into()), product: None }, method, &unchecked, "/home/nova", 5.0, platform);
        env["line"].as_str().unwrap().to_string()
    };
    assert!(line("token", "win32").contains("by its token; XenoNet can't check which process sent it on Windows yet, so it's the token alone, on this machine"));
    assert!(line("grant", "linux").contains("by its remote connection's credential; XenoNet can't check which process sent it on Linux yet, so it's its word"));
    assert!(line("token", "darwin").contains("on this Mac yet"));
    assert!(line("gateway", "darwin").contains("the actor \"Wren\" (actor:wren), by its gateway token alone, from outside this machine, through XenoNet's actor gateway."));
}
