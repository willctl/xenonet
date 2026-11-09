//! A remote actor's account links (docs/design/cloud-assistants.md, section
//! 3): how sure XenoNet is of the account an actor works under, kept apart
//! from authenticating it.
//!
//!   verified        a provider assertion its adapter validated (adapters);
//!                   its subject is kept as a hash, never as sent
//!   owner_asserted  your word, in XenoNet's window: you picked one of the
//!                   identities XenoNet knows. Shown as yours, never as proof
//!   unverified      neither: the state of every actor without a link
//!
//! A link never authenticates anything: grants do (connections). An
//! external ref names an actor in its provider's own ids, only once
//! verified, under the namespace that proved it; a ref that would name a
//! second actor is refused for you to review, never merged.
//!
//! The links and refs are rows in the engine's database, reached through
//! `Links`; each row is a JSON object of its columns.

use crate::store::{self, Tx};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use xenonet_base::text::js_string;

/// A link refused: a code a program tests, and a sentence; or what the
/// store failed with.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkFail {
    Link { code: &'static str, message: String },
    Error(store::Error),
}

impl From<store::Error> for LinkFail {
    fn from(e: store::Error) -> Self {
        LinkFail::Error(e)
    }
}

pub type Result<T> = std::result::Result<T, LinkFail>;

fn refuse<T>(code: &'static str, message: &str) -> Result<T> {
    Err(LinkFail::Link { code, message: message.into() })
}

/// The account links and external refs kept in the engine's database.
pub trait Links: Tx + Send + Sync {
    /// An actor's links, oldest first.
    fn account_links(&self, slug: &str) -> store::Result<Vec<Value>>;
    fn add_account_link(&self, row: &Value) -> store::Result<()>;
    fn expire_account_link(&self, link_id: &Value, at: f64) -> store::Result<()>;
    /// The ref under a key (adapter_id, issuer, account_binding,
    /// external_actor_id), if there is one.
    fn external_ref(&self, key: &Value) -> store::Result<Option<Value>>;
    fn add_external_ref(&self, row: &Value) -> store::Result<()>;
}

/// `<prefix>_` and 24 random hex digits.
pub fn new_id(prefix: &str) -> store::Result<String> {
    let mut b = [0u8; 12];
    getrandom::fill(&mut b).map_err(|e| store::Error::new(format!("no random bytes: {e}")))?;
    Ok(format!("{prefix}_{}", crate::tokens::hex(&b)))
}

/// parts.join("\0"), hashed: null and a part not given join as nothing.
fn hash_of(parts: &[Option<&Value>]) -> String {
    let joined: Vec<String> = parts.iter().map(|p| p.filter(|v| !v.is_null()).map(js_string).unwrap_or_default()).collect();
    crate::tokens::hex(&Sha256::digest(joined.join("\0").as_bytes()))
}

/// `a === b` for a column's value and a value given (none given is never
/// the same).
fn same(cell: Option<&Value>, v: Option<&Value>) -> bool {
    match (cell, v) {
        (None | Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::String(a)), Some(Value::String(b))) => a == b,
        (Some(c), Some(Value::Number(n))) => c.as_f64() == n.as_f64(),
        _ => false,
    }
}

/// Set#has for a set of identities.
fn has(known: &[String], v: &str) -> bool {
    known.iter().any(|k| k == v)
}

fn live(l: &Value, now: f64) -> bool {
    match l.get("expires_at") {
        None | Some(Value::Null) => true,
        Some(c) => c.as_f64().is_some_and(|e| e > now),
    }
}

fn col(r: &Value, k: &str) -> Value {
    r.get(k).cloned().unwrap_or(Value::Null)
}

/// Your word that actor `slug` works as `identity` (one of the identities
/// XenoNet knows, `known`), from XenoNet's window. The link kept.
pub fn assert_link(store: &dyn Links, slug: &str, identity: &Value, provider: &str, known: Option<&[String]>, now: f64) -> Result<Value> {
    let ok = identity.as_str().is_some_and(|i| known.is_some_and(|k| has(k, i)));
    if !ok {
        return refuse("unknown-identity", "pick one of the identities XenoNet knows");
    }
    let link = json!({
        "link_id": new_id("al")?,
        "actor_slug": slug,
        "identity_id": identity,
        "issuer": null,
        "provider": provider,
        "workspace": null,
        "proof_status": "owner_asserted",
        "proof_method": "owner_selected",
        "subject_binding": null,
        "checked_at": xenonet_base::json::number(now),
        "expires_at": null,
        "created_at": xenonet_base::json::number(now),
    });
    store.add_account_link(&link)?;
    Ok(link)
}

/// What an adapter verified, kept. It names an identity XenoNet knows only
/// when the assertion maps to exactly that one: an OpenAI workspace to
/// `chatgpt:<workspace>`. A verified link under another workspace than the
/// one before means the account changed: the old links lapse, and what was
/// approved under them is yours to approve again. (identity, switched).
pub fn record_verified(store: &dyn Links, slug: &str, facts: &Value, known: &[String], now: f64) -> Result<(Option<String>, bool)> {
    if facts.get("state").and_then(Value::as_str) != Some("verified") {
        return refuse("unverified", "only a verified assertion is kept as verified");
    }
    let workspace = facts.get("workspace").filter(|w| xenonet_base::text::truthy(Some(w)));
    let identity = if facts.get("provider").and_then(Value::as_str) == Some("openai") && workspace.is_some() {
        let id = format!("chatgpt:{}", workspace.map(js_string).unwrap_or_default());
        has(known, &id).then_some(id)
    } else {
        None
    };
    let before: Vec<Value> = store.account_links(slug)?.into_iter().filter(|l| l.get("proof_status").and_then(Value::as_str) == Some("verified") && live(l, now)).collect();
    let issuer = facts.get("issuer");
    let workspace = facts.get("workspace").filter(|w| !w.is_null());
    let switched = before.iter().any(|l| !same(l.get("issuer"), issuer) || !same(l.get("workspace"), Some(workspace.unwrap_or(&Value::Null))));
    let or_text = |k: &str, d: &str| facts.get(k).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(d));
    let link = json!({
        "link_id": new_id("al")?,
        "actor_slug": slug,
        "identity_id": identity,
        "issuer": issuer.cloned().unwrap_or(Value::Null),
        "provider": or_text("provider", "unknown"),
        "workspace": workspace.cloned().unwrap_or(Value::Null),
        "proof_status": "verified",
        "proof_method": or_text("method", "provider_assertion"),
        "subject_binding": hash_of(&[issuer, facts.get("subject")]),
        "checked_at": xenonet_base::json::number(now),
        "expires_at": col(facts, "expiresAt"),
        "created_at": xenonet_base::json::number(now),
    });
    store::transaction(store, || -> Result<()> {
        if switched {
            for l in &before {
                store.expire_account_link(&col(l, "link_id"), now)?;
            }
        }
        store.add_account_link(&link)?;
        Ok(())
    })?;
    Ok((identity, switched))
}

/// How sure XenoNet is of an actor's account now: verified while an
/// unexpired verified link stands, else owner_asserted while one of yours
/// does, else unverified. With the identity it names, if any.
pub fn link_state(store: &dyn Links, slug: &str, now: f64) -> store::Result<(&'static str, Value)> {
    let links: Vec<Value> = store.account_links(slug)?.into_iter().filter(|l| live(l, now)).collect();
    for state in ["verified", "owner_asserted"] {
        if let Some(l) = links.iter().rev().find(|l| l.get("proof_status").and_then(Value::as_str) == Some(state)) {
            return Ok((state, col(l, "identity_id")));
        }
    }
    Ok(("unverified", Value::Null))
}

/// An actor's id at its provider, once its adapter verified it. The same
/// id under the same namespace for another actor is refused: that's for
/// you to look at, and XenoNet never merges actors by it, nor by a name,
/// an email or a product. The ref kept, or the one there was.
pub fn add_external_ref(store: &dyn Links, slug: &str, adapter_id: &str, facts: &Value, now: f64) -> Result<Value> {
    if facts.get("state").and_then(Value::as_str) != Some("verified") || !xenonet_base::text::truthy(facts.get("externalActorId")) {
        return refuse("unverified", "an external id is kept only once its provider proved it");
    }
    let key = json!({
        "adapter_id": adapter_id,
        "issuer": col(facts, "issuer"),
        "account_binding": col(facts, "accountBinding"),
        "external_actor_id": col(facts, "externalActorId"),
    });
    if let Some(had) = store.external_ref(&key)? {
        if had.get("actor_slug").and_then(Value::as_str) != Some(slug) {
            let message = format!("that provider id already names actor:{}; review both in XenoNet's window", js_string(&col(&had, "actor_slug")));
            return Err(LinkFail::Link { code: "conflict", message });
        }
        return Ok(had);
    }
    let mut r = serde_json::Map::new();
    r.insert("external_ref_id".into(), json!(new_id("xr")?));
    r.insert("actor_slug".into(), json!(slug));
    if let Value::Object(k) = key {
        r.extend(k);
    }
    r.insert("verified_at".into(), xenonet_base::json::number(now));
    let r = Value::Object(r);
    store.add_external_ref(&r)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The links as rows, in memory.
    #[derive(Default)]
    struct Fake {
        links: Mutex<Vec<Value>>,
    }

    impl Tx for Fake {
        fn in_transaction(&self) -> bool {
            false
        }
        fn begin(&self) -> store::Result<()> {
            Ok(())
        }
        fn commit(&self) -> store::Result<()> {
            Ok(())
        }
        fn rollback(&self) {}
    }

    impl Links for Fake {
        fn account_links(&self, slug: &str) -> store::Result<Vec<Value>> {
            Ok(self.links.lock().unwrap().iter().filter(|l| l["actor_slug"] == slug).cloned().collect())
        }
        fn add_account_link(&self, row: &Value) -> store::Result<()> {
            self.links.lock().unwrap().push(row.clone());
            Ok(())
        }
        fn expire_account_link(&self, link_id: &Value, at: f64) -> store::Result<()> {
            for l in self.links.lock().unwrap().iter_mut().filter(|l| l["link_id"] == *link_id) {
                l["expires_at"] = xenonet_base::json::number(at);
            }
            Ok(())
        }
        fn external_ref(&self, _key: &Value) -> store::Result<Option<Value>> {
            Ok(None)
        }
        fn add_external_ref(&self, _row: &Value) -> store::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn your_word_stands_until_a_verified_link_does() {
        let s = Fake::default();
        assert_eq!(link_state(&s, "dots", 1.0).unwrap(), ("unverified", Value::Null));
        let known = ["chatgpt:w".to_string()];
        assert!(matches!(assert_link(&s, "dots", &json!("other"), "openai", Some(&known), 1.0), Err(LinkFail::Link { code: "unknown-identity", .. })));
        assert_link(&s, "dots", &json!("chatgpt:w"), "openai", Some(&known), 1.0).unwrap();
        assert_eq!(link_state(&s, "dots", 2.0).unwrap(), ("owner_asserted", Value::String("chatgpt:w".into())));
        let facts = json!({ "state": "verified", "issuer": "i", "provider": "openai", "workspace": "w", "subject": "u" });
        assert_eq!(record_verified(&s, "dots", &facts, &known, 3.0).unwrap(), (Some("chatgpt:w".into()), false));
        assert_eq!(link_state(&s, "dots", 4.0).unwrap().0, "verified");
    }

    #[test]
    fn hashes_join_null_as_nothing() {
        assert_eq!(hash_of(&[Some(&Value::Null), Some(&json!("a"))]), hash_of(&[Some(&json!("")), Some(&json!("a"))]));
    }
}
