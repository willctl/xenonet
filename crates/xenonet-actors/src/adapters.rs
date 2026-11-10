//! Remote provider adapters (core/actors/adapters.js;
//! docs/design/cloud-assistants.md, section 3): the part of a remote
//! actor's connection that belongs to its product. Common code
//! (connections) owns consent, grants, projects, the actor's record and the
//! log; an adapter only says which transports its product reaches XenoNet
//! through, and checks what its provider can prove.
//!
//! None of the transports here passes XenoNet a provider assertion it can
//! check, so every account link they make is unverified, or your word
//! (owner_asserted). A product's name sent by a caller never joins an
//! actor; only your choice in XenoNet's window does.

use serde_json::{json, Value};
use std::sync::LazyLock;
use xenonet_adapter::products::PAIRINGS;

pub struct Transport {
    pub id: &'static str,
    pub name: &'static str,
    pub provider: &'static str,
}

pub const TRANSPORTS: [Transport; 2] = [
    Transport { id: "openai-secure-mcp-tunnel", name: "OpenAI Secure MCP Tunnel", provider: "openai" },
    Transport { id: "local-stdio", name: "A local MCP client, over stdio", provider: "local" },
];

pub struct Adapter {
    /// Its key among the adapters.
    pub key: &'static str,
    pub id: &'static str,
    pub product: Option<&'static str>,
    pub provider: &'static str,
    pub transports: &'static [&'static str],
    /// "experimental": its route is meant for real use and untested end to
    /// end; "fixture": only its common code is tested.
    pub support: &'static str,
    /// Why neither the account nor the actor can be verified through it.
    pub unavailable: &'static str,
    pub capabilities: &'static [&'static str],
}

/// The adapters: one for each product you may pair, as the products
/// table has its pairing, in its order, then the one for a program of
/// your own.
pub static ADAPTERS: LazyLock<Vec<Adapter>> = LazyLock::new(|| {
    PAIRINGS
        .iter()
        .map(|p| Adapter {
            key: p.product,
            id: p.adapter,
            product: Some(p.product),
            provider: p.provider,
            transports: p.transports,
            support: p.support,
            unavailable: p.unavailable,
            capabilities: p.capabilities,
        })
        .chain([Adapter {
            key: "custom",
            id: "custom",
            product: None,
            provider: "local",
            transports: &["openai-secure-mcp-tunnel", "local-stdio"],
            support: "experimental",
            unavailable: "no-provider",
            capabilities: &["read"],
        }])
        .collect()
});

/// The adapter for a product (by its id), or for a program of your own
/// (None), among adapters by their products (`products`, with whether a
/// custom one is there): its index, or None for a product that can't be
/// paired.
pub fn adapter_for(product: Option<&Value>, products: &[Option<Value>], custom: Option<usize>) -> Option<usize> {
    match product {
        None | Some(Value::Null) => custom,
        Some(p) => products.iter().position(|q| q.as_ref() == Some(p)),
    }
}

/// What an adapter's verification said, checked: a verified answer must
/// name its issuer and subject (or its account binding and the actor's
/// id), or it's taken as unavailable. Adapters are code XenoNet ships, but
/// a mistake in one must not read as proof. None: the answer stands as it
/// is.
pub fn verified(out: Option<&Value>, failed: bool) -> Option<Value> {
    if failed {
        return Some(json!({ "state": "unavailable", "reason": "check-failed" }));
    }
    let o = out.filter(|o| o.get("state").and_then(Value::as_str) == Some("verified"));
    let Some(o) = o else {
        let reason = out.and_then(|o| o.get("reason")).and_then(Value::as_str).unwrap_or("unverified");
        return Some(json!({ "state": "unavailable", "reason": reason }));
    };
    let fields: &[&str] = if o.get("externalActorId").is_some() { &["issuer", "accountBinding", "externalActorId"] } else { &["issuer", "subject"] };
    if !fields.iter().all(|k| o.get(*k).and_then(Value::as_str).is_some_and(|s| !s.is_empty())) {
        return Some(json!({ "state": "unavailable", "reason": "incomplete" }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_adapters_are_the_pairings_then_your_own() {
        let keys: Vec<&str> = ADAPTERS.iter().map(|a| a.key).collect();
        assert_eq!(keys, ["dots", "grok-bot", "hermes-agent", "custom"]);
        let dots = &ADAPTERS[0];
        assert_eq!((dots.id, dots.provider, dots.transports.len(), dots.support, dots.unavailable), ("openai-dots", "openai", 2, "experimental", "transport-has-no-assertion"));
        assert_eq!((ADAPTERS[2].provider, ADAPTERS[3].product), ("nous", None));
    }
}
