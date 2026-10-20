//! A message's envelope, from the binary's copy of attest (the service's
//! borrowed.rs, which its routes and the actor gateway use) and from the
//! engine's (core/engine/attest.js's twin): for every way a sender can
//! prove who it is and every verdict on its process, both say the same,
//! and how strong it is follows the one table.

use serde_json::{json, Value};
use xenonet_engine::attest::attest;
use xenonet_engine::sql::Arg;
use xenonet_service::borrowed::{self, Sender};

fn arg(v: &Value) -> Arg {
    match v {
        Value::Null => Arg::Null,
        Value::Bool(b) => Arg::Bool(*b),
        Value::Number(n) => Arg::num(n.as_f64().unwrap_or(f64::NAN)),
        Value::String(s) => Arg::text(s.clone()),
        Value::Array(a) => Arg::List(a.iter().map(arg).collect()),
        Value::Object(o) => Arg::Obj(o.iter().map(|(k, v)| (k.clone(), arg(v))).collect()),
    }
}

/// What XenoNet saw of the process, and how strong each way of proving
/// who sent it is with that.
fn table(method: &str, seen: &str) -> (&'static str, &'static str) {
    match (method, seen) {
        ("window", _) => ("n/a", "strong"),
        ("gateway", _) => ("n/a", "weak"),
        ("session", "signed") => ("signed", "strong"),
        ("session", "checked") => ("checked", "partial"),
        ("token" | "grant", "signed") => ("signed", "partial"),
        ("token" | "grant", "checked") => ("checked", "partial"),
        (_, "signed") => ("signed", "weak"),
        (_, "checked") => ("checked", "weak"),
        _ => ("unchecked", "weak"),
    }
}

#[test]
fn every_way_of_proving_who_sent_it_is_as_strong_in_the_binary_as_in_the_engine() {
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
                    let sender = json!({ "id": id, "kind": kind, "name": name, "product": product });
                    let from_engine = attest(&arg(&sender), &Arg::text(method), &arg(verdict), &Arg::text("/home/nova"), &Arg::num(5.0), platform).unwrap().to_json();
                    let at = format!("{kind} {method} {seen} {verdict} {platform}");
                    for k in ["schema", "at", "sender", "auth", "client", "transport", "line"] {
                        assert_eq!(from_bin[k], from_engine[k], "{k}: {at}");
                    }
                    let (process, strength) = table(method, seen);
                    assert_eq!(from_bin["auth"], json!({ "method": method, "process": process, "strength": strength }), "{at}");
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
