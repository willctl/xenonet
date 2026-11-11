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
// machine itself.
fn act_of(action: Action) -> Option<Act> {
    Some(match action {
        Action::Read => Act::Read { scope: ReadScope::Map, filters: None },
        Action::Message => Act::Message,
        Action::Handoff => Act::Handoff,
        Action::Claim => Act::Claim,
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
