//! Actors' grants (docs/design/direction.md, "Authorization"), for the
//! cores outside the engine (the remote actors' connections and reads):
//! one kind for every actor, a bot or a cloud assistant, minted from this
//! start's token root, issued or widened with the person there, and
//! decided on this machine's ground. A page cursor is the grant narrowed to
//! one read and its filters, for five minutes.

use crate::ground::{ground, EngineGround};
use crate::runtime::Runtime;
use serde_json::Value;
use std::sync::Arc;
use xenonet_api::cores::Grants;
use xenonet_authz::{decide, Act, Action, Door, Principal, ReadScope, Request, Resource, Token, ACTOR_PREFIX};
use xenonet_id::codec::sha256;

pub struct GrantsDriver(pub Arc<Runtime>);

/// The actions these words name; one that names none is refused.
pub fn actions(words: &[String]) -> Result<Vec<Action>, String> {
    words.iter().map(|w| Action::parse(w).ok_or_else(|| format!("{w} isn't a right"))).collect()
}

/// The hash a grant's scopes are bound by in the person's presence: their
/// JSON, as the caller writes it.
pub fn scopes_hash(scopes: &Value) -> [u8; 32] {
    sha256(scopes.to_string().as_bytes())
}

/// An act of the actor `slug` here, now.
fn request(g: &EngineGround, slug: &str, act: Act) -> Option<Request> {
    let own = g.facts.machine.clone();
    let slug = xenonet_authz::ActorSlug::new(slug).ok()?;
    Some(Request::new(Principal::Actor { slug, machine: own.clone() }, Resource::Machine(own), act, Door::Local, g.facts.now, None))
}

// The act a right is checked with, for the rights that act on this
// machine itself (a claim is decided in its project, by the engine).
fn act_of(action: Action) -> Option<Act> {
    Some(match action {
        Action::Read => Act::Read { scope: ReadScope::Map, filters: None },
        Action::Message => Act::Message,
        Action::Handoff => Act::Handoff,
        _ => return None,
    })
}

/// A token's text, as `slug`'s grant from this machine: refused as
/// `revoked` or `invalid`.
fn parse(g: &EngineGround, slug: &str, text: &str) -> Result<Token, &'static str> {
    let token = match Token::parse(text, ACTOR_PREFIX, g, g.facts.now) {
        Ok(t) => t,
        Err(e) if e.code == "revoked" => return Err("revoked"),
        Err(_) => return Err("invalid"),
    };
    let b = token.bearer();
    if b.slug.as_str() != slug || b.machine != g.facts.machine {
        return Err("invalid");
    }
    Ok(token)
}

impl Grants for GrantsDriver {
    fn present(&self, slug: &str, rights: &[String], scopes: &Value, reason: &str) -> Result<(), String> {
        // Only the window's routes pair or widen a connection.
        crate::ground::present(&self.0, true, slug, actions(rights)?, scopes_hash(scopes), reason)
    }

    fn mint(&self, slug: &str, rights: &[String], until: Option<f64>) -> Result<(String, String), String> {
        let token = crate::ground::mint(&self.0, slug, actions(rights)?, None, until.map(|u| u as i64), None)?;
        Ok((token.encode(ACTOR_PREFIX).map_err(|e| e.message)?, token.id()))
    }

    fn check(&self, slug: &str, text: &str, action: &str) -> Result<String, &'static str> {
        let g = ground(&self.0, None).ok_or("invalid")?;
        let token = parse(&g, slug, text)?;
        let act = Action::parse(action).and_then(act_of).ok_or("invalid")?;
        let r = request(&g, slug, act).ok_or("invalid")?;
        match decide(Some(&token), &r, &g, None) {
            d if d.is_allow() => Ok(token.id()),
            d if d.reason().is_some_and(|r| r.code() == "revoked") => Err("revoked"),
            d if d.reason().is_some_and(|r| r.code() == "right") => Err("no-right"),
            _ => Err("invalid"),
        }
    }

    fn forward(&self, text: &str, alpn: &str) -> Result<(String, Vec<String>), &'static str> {
        let g = ground(&self.0, None).ok_or("no-grant")?;
        forward_grant(&g, g.facts.now, text, alpn)
    }

    fn cursor(&self, text: &str, bound: &str, after: Option<&str>) -> Result<String, String> {
        let g = ground(&self.0, None).ok_or("this machine's governance isn't plugged in")?;
        let token = Token::parse(text, ACTOR_PREFIX, &g, g.facts.now).map_err(|e| e.message)?;
        let cursor = token.cursor(ReadScope::Map, &sha256(bound.as_bytes()), g.facts.now, after).map_err(|e| e.message)?;
        cursor.encode(ACTOR_PREFIX).map_err(|e| e.message)
    }

    fn page(&self, slug: &str, text: &str, bound: &str) -> Result<Option<String>, &'static str> {
        const BAD: &str = "INVALID_ARGUMENT";
        const OLD: &str = "CURSOR_EXPIRED";
        let g = ground(&self.0, None).ok_or(BAD)?;
        let cursor = match parse(&g, slug, text) {
            Ok(t) => t,
            Err("revoked") => return Err(OLD),
            Err(_) => return Err(BAD),
        };
        let (after, until) = cursor.page().ok_or(BAD)?;
        if until / 1000 < g.facts.now / 1000 {
            return Err(OLD);
        }
        let r = request(&g, slug, Act::Read { scope: ReadScope::Map, filters: Some(sha256(bound.as_bytes())) }).ok_or(BAD)?;
        match decide(Some(&cursor), &r, &g, None) {
            d if d.is_allow() => Ok((!after.is_empty()).then_some(after)),
            d if d.reason().is_some_and(|r| r.code() == "revoked") => Err(OLD),
            _ => Err(BAD),
        }
    }
}

/// A grant to forward `alpn` (`Grants::forward`): this machine's own
/// actor's token, in its time and not revoked by `g`, whose `forward(alpn)`
/// names exactly that ALPN; its slug and revocation ids.
pub fn forward_grant(g: &dyn xenonet_authz::Ground, now: i64, text: &str, alpn: &str) -> Result<(String, Vec<String>), &'static str> {
    let token = match Token::parse(text, ACTOR_PREFIX, g, now) {
        Ok(t) => t,
        Err(e) if e.code == "revoked" => return Err("revoked"),
        Err(_) => return Err("no-grant"),
    };
    // This machine's own actor's, and its forward this ALPN alone: a grant
    // forwards the one it names, never another.
    if &token.bearer().machine != g.own_machine() {
        return Err("no-grant");
    }
    if token.until() < now {
        return Err("expired");
    }
    match token.forward() {
        Some(a) if a == alpn => {}
        Some(_) => return Err("other-alpn"),
        None => return Err("no-grant"),
    }
    let ids = Token::ids_of(text, ACTOR_PREFIX).unwrap_or_else(|| vec![token.id()]);
    Ok((token.bearer().slug.as_str().to_string(), ids))
}

/// Whether `a` lets an actor Allow nothing more than `b`.
fn within(a: &xenonet_authz::AllowScope, b: &xenonet_authz::AllowScope) -> bool {
    let sessions = match (&a.sessions, &b.sessions) {
        (_, xenonet_authz::Set::All) => true,
        (xenonet_authz::Set::All, _) => false,
        (xenonet_authz::Set::Only(x), xenonet_authz::Set::Only(y)) => x.iter().all(|s| y.contains(s)),
    };
    sessions && a.kinds.iter().all(|k| b.kinds.contains(k)) && a.until <= b.until
}

/// The allow scope this machine's actor `slug` holds now, as its row keeps
/// it, or None: no token, no row, or one that doesn't read.
pub fn allows_of(w: &Runtime, slug: &str) -> Option<Value> {
    let row = w.store.with(|s| s.actor_row(slug)).ok().flatten()?;
    let token = row.text("token_id")?.to_string();
    let scope = w.store.with(|s| s.scope(&token)).ok().flatten()?;
    let v = serde_json::from_str::<Value>(scope.text("allows")?).ok()?;
    crate::ground::allow_scope_of(&v).map(|_| v)
}

/// What this machine's actor `slug` may Allow of the prompts it answers:
/// `scope` (`{ sessions: "all" | [session ids], kinds: [read, edit,
/// command, web, tool], until }`), or none, when its right to answer is a
/// Deny's alone. Kept in its grant's scopes row. Issuing or widening it is
/// widening the grant, so it takes the person's presence for the grant's
/// rights and this scope, from the window; narrowing or ending it doesn't.
pub fn set_allows(w: &Runtime, at_window: bool, slug: &str, scope: Option<&Value>, reason: &str) -> Result<(), String> {
    let row = w.store.with(|s| s.actor_row(slug)).map_err(|e| e.to_string())?.ok_or_else(|| format!("XenoNet has no actor {slug}"))?;
    let token = row.text("token_id").ok_or_else(|| format!("actor:{slug} has no token to scope: give it one first"))?.to_string();
    let words: Vec<String> = row.text("token_rights").and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    let rights = xenonet_actors::tokens::rights(&words);
    if !rights.contains(&Action::Answer) {
        return Err(format!("actor:{slug} has no right to answer prompts: allow it the answer-prompts capability first"));
    }
    let new = match scope {
        None => None,
        Some(v) => Some(crate::ground::allow_scope_of(v).ok_or("an allow scope is { sessions: \"all\" or [session ids], kinds: [read, edit, command, web, tool], until }")?),
    };
    let had = w
        .store
        .with(|s| s.scope(&token))
        .ok()
        .flatten()
        .and_then(|r| r.text("allows").and_then(|t| serde_json::from_str::<Value>(t).ok()))
        .and_then(|v| crate::ground::allow_scope_of(&v));
    let widens = new.as_ref().is_some_and(|n| had.as_ref().is_none_or(|h| !within(n, h)));
    if widens {
        let scopes = serde_json::json!({ "allows": scope });
        crate::ground::present(w, at_window, slug, rights, scopes_hash(&scopes), reason)?;
    }
    let now = xenonet_text::time::now_ms();
    let allows = scope.map_or(Value::Null, |v| Value::String(v.to_string()));
    w.store.with(|s| s.put_scope(&serde_json::json!({ "token": token, "actor": format!("actor:{slug}"), "allows": allows, "at": now }))).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use xenonet_authz::{ActorGrant, ActorSlug, Binding, Consent, Device, MachineId, NetworkId, RootCertificate, Schema, Settings, StandingGrant, TokenRoot};
    use xenonet_id::codec::hex;
    use xenonet_id::sign::{KeyHome, SeedSigner, Signer, VerifyingKey};

    const NOW: i64 = 1_800_000_000_000;

    // A ground that knows one machine, its one root, and what's revoked.
    struct One {
        machine: MachineId,
        key: VerifyingKey,
        cert: RootCertificate,
        revoked: BTreeSet<String>,
    }

    impl xenonet_authz::Ground for One {
        fn own_machine(&self) -> &MachineId {
            &self.machine
        }
        fn role(&self, _: &NetworkId, _: &MachineId) -> Option<xenonet_authz::Role> {
            None
        }
        fn settings(&self, _: Option<&NetworkId>) -> Settings {
            Settings::default()
        }
        fn own_person(&self) -> Option<xenonet_authz::PersonId> {
            None
        }
        fn device(&self, _: &xenonet_authz::PersonId, _: &xenonet_authz::DeviceId) -> Option<Device> {
            None
        }
        fn revoked(&self, id: &[u8]) -> bool {
            self.revoked.contains(&hex(id))
        }
        fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
            (self.cert.key_id() == key_id).then(|| self.cert.clone()).into_iter().collect()
        }
        fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
            (machine == &self.machine).then(|| self.key.clone())
        }
        fn consent(&self, _: &xenonet_authz::SessionId) -> Consent {
            Consent::default()
        }
        fn device_of(&self, _: &MachineId) -> Option<xenonet_authz::DeviceId> {
            None
        }
        fn delegations(&self) -> Vec<StandingGrant> {
            Vec::new()
        }
        fn allow_scope(&self, _: &[u8]) -> Option<xenonet_authz::AllowScope> {
            None
        }
        fn ask(&self, _: &xenonet_authz::AskId) -> Option<xenonet_authz::Asked> {
            None
        }
        fn governance_synced_at(&self) -> Option<i64> {
            None
        }
    }

    fn grant(root: &TokenRoot, forward: Option<&str>, until: i64) -> (String, String) {
        let g = ActorGrant { actor: ActorSlug::new("lith").unwrap(), network: None, rights: vec![Action::Read], until };
        let t = root.mint_actor_with(&g, &Schema::XENONET, &Binding::new(None, forward).unwrap()).unwrap();
        (t.encode(ACTOR_PREFIX).unwrap(), t.id())
    }

    #[test]
    fn a_forward_needs_this_machines_grant_naming_that_alpn_in_its_time_and_not_revoked() {
        let signer = SeedSigner::from_seed(&[3; 32], KeyHome::File).unwrap();
        let machine = MachineId::new("m1").unwrap();
        let root = TokenRoot::new(machine.clone(), &signer, NOW - 1000, NOW + 3_600_000).unwrap();
        let mut g = One { machine, key: signer.public(), cert: root.certificate().clone(), revoked: BTreeSet::new() };
        let (sync, id) = grant(&root, Some("xenolith/sync/2"), NOW + 60_000);
        assert_eq!(forward_grant(&g, NOW, &sync, "xenolith/sync/2"), Ok(("lith".to_string(), vec![id.clone()])));
        assert_eq!(forward_grant(&g, NOW, &sync, "xenonet/link/2"), Err("other-alpn"));
        let (none, _) = grant(&root, None, NOW + 60_000);
        assert_eq!(forward_grant(&g, NOW, &none, "xenolith/sync/2"), Err("no-grant"), "a grant without a forward forwards nothing");
        assert_eq!(forward_grant(&g, NOW, "xenonet_actor_bm90IGEgdG9rZW4", "xenolith/sync/2"), Err("no-grant"));
        let (short, _) = grant(&root, Some("xenolith/sync/2"), NOW + 1);
        assert_eq!(forward_grant(&g, NOW + 2, &short, "xenolith/sync/2"), Err("expired"));
        g.revoked.insert(id);
        assert_eq!(forward_grant(&g, NOW, &sync, "xenolith/sync/2"), Err("revoked"));
        // Another machine's grant forwards nothing here.
        let other = SeedSigner::from_seed(&[4; 32], KeyHome::File).unwrap();
        let theirs = TokenRoot::new(MachineId::new("m2").unwrap(), &other, NOW - 1000, NOW + 3_600_000).unwrap();
        let (t, _) = grant(&theirs, Some("xenolith/sync/2"), NOW + 60_000);
        assert_eq!(forward_grant(&g, NOW, &t, "xenolith/sync/2"), Err("no-grant"));
    }
}
