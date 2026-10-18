//! A remote actor's account links (docs/design/cloud-assistants.md, section
//! 3): how sure XenoNet is of the account an actor works under, kept apart
//! from authenticating it. core/actors/account-links.js.
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

use serde_json::Value;
use sha2::{Digest, Sha256};
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::{Arg, Cell, Row, SqlError};
use xenonet_engine::store::{js_text, Shared};

/// A link refused: a code a program tests, and a sentence.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkFail {
    Link { code: &'static str, message: String },
    Thrown(Thrown),
}

impl From<SqlError> for LinkFail {
    fn from(e: SqlError) -> Self {
        LinkFail::Thrown(e.into())
    }
}

impl From<Thrown> for LinkFail {
    fn from(t: Thrown) -> Self {
        LinkFail::Thrown(t)
    }
}

pub type Result<T> = std::result::Result<T, LinkFail>;

fn refuse<T>(code: &'static str, message: &str) -> Result<T> {
    Err(LinkFail::Link { code, message: message.into() })
}

/// `<prefix>_` and 24 random hex digits.
pub fn new_id(prefix: &str) -> std::result::Result<String, Thrown> {
    let mut b = [0u8; 12];
    getrandom::fill(&mut b).map_err(|e| Thrown::new(format!("no random bytes: {e}")))?;
    Ok(format!("{prefix}_{}", crate::tokens::hex(&b)))
}

/// parts.join("\0"), hashed: null and undefined join as nothing.
fn hash_of(parts: &[Arg]) -> String {
    let joined: Vec<String> = parts.iter().map(|p| if p.is_nullish() { String::new() } else { js_text(p) }).collect();
    crate::tokens::hex(&Sha256::digest(joined.join("\0").as_bytes()))
}

/// `a === b` for a column's value and a value given.
fn same(cell: &Cell, v: &Arg) -> bool {
    match (cell, v) {
        (Cell::Null, Arg::Null) => true,
        (Cell::Text(a), Arg::Text(b)) => a == b,
        (c, Arg::Num(n)) => c.as_f64() == Some(*n),
        _ => false,
    }
}

/// Set#has for a set of identities.
fn has(known: &[Arg], v: &str) -> bool {
    known.iter().any(|k| k.as_str() == Some(v))
}

fn live(l: &Row, now: f64) -> bool {
    match l.get("expires_at") {
        Cell::Null => true,
        c => c.as_f64().is_some_and(|e| e > now),
    }
}

fn row(fields: Vec<(&str, Arg)>) -> Arg {
    Arg::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// Your word that actor `slug` works as `identity` (one of the identities
/// XenoNet knows, `known`), from XenoNet's window. The link kept.
pub fn assert_link(store: &Shared, slug: &Arg, identity: &Arg, provider: &Arg, known: Option<&[Arg]>, now: &Arg) -> Result<Arg> {
    let ok = identity.as_str().is_some_and(|i| known.is_some_and(|k| has(k, i)));
    if !ok {
        return refuse("unknown-identity", "pick one of the identities XenoNet knows");
    }
    let link = row(vec![
        ("link_id", Arg::text(new_id("al")?)),
        ("actor_slug", slug.clone()),
        ("identity_id", identity.clone()),
        ("issuer", Arg::Null),
        ("provider", provider.clone()),
        ("workspace", Arg::Null),
        ("proof_status", Arg::text("owner_asserted")),
        ("proof_method", Arg::text("owner_selected")),
        ("subject_binding", Arg::Null),
        ("checked_at", now.clone()),
        ("expires_at", Arg::Null),
        ("created_at", now.clone()),
    ]);
    store.with(|s| s.add_account_link(&link))?;
    Ok(link)
}

/// What an adapter verified, kept. It names an identity XenoNet knows only
/// when the assertion maps to exactly that one: an OpenAI workspace to
/// chatgpt:<workspace>. A verified link under another workspace than the
/// one before means the account changed: the old links lapse, and what was
/// approved under them is yours to approve again. (identity, switched).
pub fn record_verified(store: &Shared, slug: &Arg, facts: &Arg, known: &[Arg], now: f64) -> Result<(Option<String>, bool)> {
    if facts.field("state").as_str() != Some("verified") {
        return refuse("unverified", "only a verified assertion is kept as verified");
    }
    let workspace = facts.field("workspace");
    let identity = if facts.field("provider").as_str() == Some("openai") && workspace.truthy() {
        let id = format!("chatgpt:{}", js_text(&workspace));
        has(known, &id).then_some(id)
    } else {
        None
    };
    let before: Vec<Row> = store.with(|s| s.account_links(slug.clone()))?.into_iter().filter(|l| l.text("proof_status") == Some("verified") && live(l, now)).collect();
    let issuer = facts.field("issuer");
    let workspace = workspace.or(Arg::Null);
    let switched = before.iter().any(|l| !same(l.get("issuer"), &issuer) || !same(l.get("workspace"), &workspace));
    let link = row(vec![
        ("link_id", Arg::text(new_id("al")?)),
        ("actor_slug", slug.clone()),
        ("identity_id", Arg::opt_text(identity.as_deref())),
        ("issuer", issuer.clone()),
        ("provider", facts.field("provider").or(Arg::text("unknown"))),
        ("workspace", workspace),
        ("proof_status", Arg::text("verified")),
        ("proof_method", facts.field("method").or(Arg::text("provider_assertion"))),
        ("subject_binding", Arg::text(hash_of(&[issuer, facts.field("subject")]))),
        ("checked_at", Arg::num(now)),
        ("expires_at", facts.field("expiresAt").or(Arg::Null)),
        ("created_at", Arg::num(now)),
    ]);
    store.transaction(|| -> Result<()> {
        if switched {
            for l in &before {
                store.with(|s| s.expire_account_link(l.get("link_id").arg(), Arg::num(now)))?;
            }
        }
        store.with(|s| s.add_account_link(&link))?;
        Ok(())
    })?;
    Ok((identity, switched))
}

/// How sure XenoNet is of an actor's account now: verified while an
/// unexpired verified link stands, else owner_asserted while one of yours
/// does, else unverified. With the identity it names, if any.
pub fn link_state(store: &Shared, slug: &Arg, now: f64) -> std::result::Result<(&'static str, Value), SqlError> {
    let links: Vec<Row> = store.with(|s| s.account_links(slug.clone()))?.into_iter().filter(|l| live(l, now)).collect();
    for state in ["verified", "owner_asserted"] {
        if let Some(l) = links.iter().rev().find(|l| l.text("proof_status") == Some(state)) {
            return Ok((state, l.get("identity_id").to_json()));
        }
    }
    Ok(("unverified", Value::Null))
}

/// An actor's id at its provider, once its adapter verified it. The same
/// id under the same namespace for another actor is refused: that's for
/// you to look at, and XenoNet never merges actors by it, nor by a name,
/// an email or a product. The ref kept, or the one there was.
pub fn add_external_ref(store: &Shared, slug: &Arg, adapter_id: &Arg, facts: &Arg, now: &Arg) -> Result<Value> {
    if facts.field("state").as_str() != Some("verified") || !facts.field("externalActorId").truthy() {
        return refuse("unverified", "an external id is kept only once its provider proved it");
    }
    let key = vec![
        ("adapter_id", adapter_id.clone()),
        ("issuer", facts.field("issuer")),
        ("account_binding", facts.field("accountBinding")),
        ("external_actor_id", facts.field("externalActorId")),
    ];
    let had = store.with(|s| s.external_ref(&row(key.clone())))?;
    if let Some(had) = had {
        if !same(had.get("actor_slug"), slug) {
            let message = format!("that provider id already names actor:{}; review both in XenoNet's window", js_text(&had.get("actor_slug").arg()));
            return Err(LinkFail::Link { code: "conflict", message });
        }
        return Ok(had.to_json());
    }
    let mut fields = vec![("external_ref_id", Arg::text(new_id("xr")?)), ("actor_slug", slug.clone())];
    fields.extend(key);
    fields.push(("verified_at", now.clone()));
    let r = row(fields);
    store.with(|s| s.add_external_ref(&r))?;
    Ok(r.to_json())
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_engine::store::EngineStore;

    fn store() -> Shared {
        let s = Shared::new(EngineStore::open(":memory:", 1000.0).unwrap());
        s.with(|s| {
            xenonet_engine::sql::exec(s.db().unwrap(), "INSERT INTO actors (slug, kind, name, via, capabilities, created_at) VALUES ('dots', 'agent', 'Dots', '[]', '[]', 0)")
        })
        .unwrap();
        s
    }

    #[test]
    fn your_word_stands_until_a_verified_link_does() {
        let s = store();
        let slug = Arg::text("dots");
        assert_eq!(link_state(&s, &slug, 1.0).unwrap(), ("unverified", Value::Null));
        let known = [Arg::text("chatgpt:w")];
        assert!(matches!(assert_link(&s, &slug, &Arg::text("other"), &Arg::text("openai"), Some(&known), &Arg::num(1.0)), Err(LinkFail::Link { code: "unknown-identity", .. })));
        assert_link(&s, &slug, &Arg::text("chatgpt:w"), &Arg::text("openai"), Some(&known), &Arg::num(1.0)).unwrap();
        assert_eq!(link_state(&s, &slug, 2.0).unwrap(), ("owner_asserted", Value::String("chatgpt:w".into())));
        let facts = Arg::Obj(vec![
            ("state".into(), Arg::text("verified")),
            ("issuer".into(), Arg::text("i")),
            ("provider".into(), Arg::text("openai")),
            ("workspace".into(), Arg::text("w")),
            ("subject".into(), Arg::text("u")),
        ]);
        assert_eq!(record_verified(&s, &slug, &facts, &known, 3.0).unwrap(), (Some("chatgpt:w".into()), false));
        assert_eq!(link_state(&s, &slug, 4.0).unwrap().0, "verified");
    }

    #[test]
    fn hashes_join_null_as_nothing() {
        assert_eq!(hash_of(&[Arg::Null, Arg::text("a")]), hash_of(&[Arg::text(""), Arg::text("a")]));
    }
}
