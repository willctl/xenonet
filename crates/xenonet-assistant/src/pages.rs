//! The lists' page cursors (docs/design/direction.md, "Authorization"): a
//! cursor is the actor's own grant narrowed to one read, its filters and
//! five minutes, holding where the last page ended and the grant's
//! revision then. So it can't be changed or made up, it ends with the
//! grant, and one from before the grant's scope changed is refused.
//! `Granted` makes them from the token a read came in with, through
//! whatever holds the grants (`Grants`); `Memory` holds grants in this
//! process alone, for the made-up reads that have no grant (the fixture,
//! the pairing's preview) and for tests.

use crate::crypto::{b64url, b64url_decode};
use crate::service::{For, Pages};
use serde_json::{json, Value};
use xenonet_api::cores::Grants;
use xenonet_authz::{
    decide, Act, Action, ActorGrant, ActorSlug, Consent, Device, Door, Ground, MachineId, NetworkId, PersonId, Principal, ReadScope, Request, Resource, Role, RootCertificate,
    Settings, StandingGrant, Token, TokenRoot, ACTOR_PREFIX,
};
use xenonet_id::codec::sha256;
use xenonet_id::sign::{KeyHome, SeedSigner, Signer, VerifyingKey};

/// The cursors of one read: its actor's grant, narrowed.
pub struct Granted<'a> {
    pub grants: &'a dyn Grants,
    /// The token the read came in with.
    pub token: &'a str,
    /// Its actor's slug.
    pub slug: &'a str,
}

impl Pages for Granted<'_> {
    fn make(&self, f: &For<'_>, after: &Value) -> Result<String, &'static str> {
        let held = json!({ "r": f.revision, "a": after });
        let held = b64url(xenonet_text::json::stringify(&held, 0).as_bytes());
        self.grants.cursor(self.token, &f.bound(), Some(&held)).map_err(|_| "SERVICE_UNAVAILABLE")
    }

    fn read(&self, cursor: &str, f: &For<'_>) -> Result<Option<Value>, &'static str> {
        let Some(held) = self.grants.page(self.slug, cursor, &f.bound())? else { return Ok(None) };
        let text = String::from_utf8(b64url_decode(&held)).map_err(|_| "INVALID_ARGUMENT")?;
        let held = xenonet_text::json::parse(&text).map_err(|_| "INVALID_ARGUMENT")?;
        // Only its holder makes a cursor, so the revision it says is the
        // one it read under.
        if !xenonet_text::value::same(held.get("r"), Some(f.revision)) {
            return Err("CURSOR_EXPIRED");
        }
        Ok(held.get("a").cloned())
    }
}

/// Grants held in this process alone: a token root made in memory,
/// certified by a key that's never written down. Nothing it mints is good
/// anywhere else, and it asks nobody: `present` always says yes.
pub struct Memory {
    root: TokenRoot,
    ground: Facts,
    clock: Box<dyn Fn() -> i64 + Send + Sync>,
}

// What the decisions rest on here: this one root.
struct Facts {
    machine: MachineId,
    key: VerifyingKey,
    cert: RootCertificate,
}

impl Ground for Facts {
    fn own_machine(&self) -> &MachineId {
        &self.machine
    }
    fn role(&self, _: &NetworkId, _: &MachineId) -> Option<Role> {
        None
    }
    fn settings(&self, _: Option<&NetworkId>) -> Settings {
        Settings::default()
    }
    fn own_person(&self) -> Option<PersonId> {
        None
    }
    fn device(&self, _: &PersonId, _: &xenonet_authz::DeviceId) -> Option<Device> {
        None
    }
    fn revoked(&self, _: &[u8]) -> bool {
        false
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

// How long the root lasts: longer than any grant.
const ROOT_MS: i64 = 400 * 24 * 3600 * 1000;

impl Memory {
    /// With a key of 32 random bytes, on the clock.
    pub fn new() -> Memory {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).expect("the system's random numbers");
        Memory::with(&seed, Box::new(|| xenonet_text::time::now_ms() as i64))
    }

    /// With the key made from `seed`, on `clock` (milliseconds since 1970).
    pub fn with(seed: &[u8; 32], clock: Box<dyn Fn() -> i64 + Send + Sync>) -> Memory {
        let signer = SeedSigner::from_seed(seed, KeyHome::File).expect("an Ed25519 seed");
        let machine = MachineId::new("memory").expect("a machine's id");
        let now = clock();
        let root = TokenRoot::new(machine.clone(), &signer, now - 1000, now + ROOT_MS).expect("a token root made in memory");
        let ground = Facts { machine, key: signer.public(), cert: root.certificate().clone() };
        Memory { root, ground, clock }
    }

    // An act of the actor `slug` here, now.
    fn request(&self, slug: &str, act: Act) -> Option<Request> {
        let own = self.ground.machine.clone();
        Some(Request::new(Principal::Actor { slug: ActorSlug::new(slug).ok()?, machine: own.clone() }, Resource::Machine(own), act, Door::Local, (self.clock)(), None))
    }

    // A grant of `slug`'s, from its text.
    fn token(&self, slug: &str, text: &str) -> Option<Token> {
        let token = Token::parse(text, ACTOR_PREFIX, &self.ground, (self.clock)()).ok()?;
        (token.bearer().slug.as_str() == slug).then_some(token)
    }
}

impl Default for Memory {
    fn default() -> Self {
        Memory::new()
    }
}

impl Grants for Memory {
    fn present(&self, _slug: &str, _rights: &[String], _scopes: &Value, _reason: &str) -> Result<(), String> {
        Ok(())
    }

    fn mint(&self, slug: &str, rights: &[String], until: Option<f64>) -> Result<(String, String), String> {
        let actor = ActorSlug::new(slug).map_err(|e| e.message)?;
        let rights = rights.iter().map(|w| Action::parse(w).ok_or_else(|| format!("{w} isn't a right"))).collect::<Result<Vec<_>, _>>()?;
        let end = self.root.certificate().until();
        let token = self.root.mint_actor(&ActorGrant { actor, network: None, rights, until: until.map_or(end, |u| (u as i64).min(end)) }).map_err(|e| e.message)?;
        Ok((token.encode(ACTOR_PREFIX).map_err(|e| e.message)?, token.id()))
    }

    // Only reads are decided here: they're all the made-up reads do.
    fn check(&self, slug: &str, text: &str, action: &str) -> Result<String, &'static str> {
        let token = self.token(slug, text).ok_or("invalid")?;
        if Action::parse(action) != Some(Action::Read) {
            return Err("invalid");
        }
        let r = self.request(slug, Act::Read { scope: ReadScope::Map, filters: None }).ok_or("invalid")?;
        match decide(Some(&token), &r, &self.ground, None) {
            d if d.is_allow() => Ok(token.id()),
            d if d.reason().is_some_and(|r| r.code() == "right") => Err("no-right"),
            _ => Err("invalid"),
        }
    }

    fn cursor(&self, text: &str, bound: &str, after: Option<&str>) -> Result<String, String> {
        let token = Token::parse(text, ACTOR_PREFIX, &self.ground, (self.clock)()).map_err(|e| e.message)?;
        let cursor = token.cursor(ReadScope::Map, &sha256(bound.as_bytes()), (self.clock)(), after).map_err(|e| e.message)?;
        cursor.encode(ACTOR_PREFIX).map_err(|e| e.message)
    }

    fn page(&self, slug: &str, text: &str, bound: &str) -> Result<Option<String>, &'static str> {
        const BAD: &str = "INVALID_ARGUMENT";
        const OLD: &str = "CURSOR_EXPIRED";
        let cursor = match Token::parse(text, ACTOR_PREFIX, &self.ground, (self.clock)()) {
            Ok(t) if t.bearer().slug.as_str() == slug => t,
            Err(e) if e.code == "expired" => return Err(OLD),
            _ => return Err(BAD),
        };
        let (after, until) = cursor.page().ok_or(BAD)?;
        if until < (self.clock)() {
            return Err(OLD);
        }
        let r = self.request(slug, Act::Read { scope: ReadScope::Map, filters: Some(sha256(bound.as_bytes())) }).ok_or(BAD)?;
        match decide(Some(&cursor), &r, &self.ground, None) {
            d if d.is_allow() => Ok((!after.is_empty()).then_some(after)),
            _ => Err(BAD),
        }
    }
}

/// The cursors of the made-up reads, which have no grant of their own: a
/// grant to read, minted in memory for the actor `slug`.
pub struct Standalone {
    grants: Memory,
    token: String,
    slug: String,
}

impl Standalone {
    pub fn new(slug: &str) -> Standalone {
        let grants = Memory::new();
        let (token, _) = grants.mint(slug, &["read".to_string()], None).expect("a grant minted in memory");
        Standalone { grants, token, slug: slug.to_string() }
    }

    fn granted(&self) -> Granted<'_> {
        Granted { grants: &self.grants, token: &self.token, slug: &self.slug }
    }
}

impl Pages for Standalone {
    fn make(&self, f: &For<'_>, after: &Value) -> Result<String, &'static str> {
        self.granted().make(f, after)
    }

    fn read(&self, cursor: &str, f: &For<'_>) -> Result<Option<Value>, &'static str> {
        self.granted().read(cursor, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;
    use xenonet_authz::CURSOR_MS;

    const SLUG: &str = "dots";
    const GRANT: &str = "gr_0123456789abcdef01234567";
    const NOW: i64 = 1_800_000_000_000;

    fn memory() -> (Memory, Arc<AtomicI64>) {
        let t = Arc::new(AtomicI64::new(NOW));
        let clock = t.clone();
        (Memory::with(&[3; 32], Box::new(move || clock.load(Ordering::SeqCst))), t)
    }

    fn read() -> Vec<String> {
        vec!["read".to_string()]
    }

    #[test]
    fn a_grant_reads_only_as_its_own_actor_and_with_the_right() {
        let (m, _) = memory();
        let (token, id) = m.mint(SLUG, &read(), Some((NOW + 60_000) as f64)).unwrap();
        assert_eq!(m.check(SLUG, &token, "read"), Ok(id));
        assert_eq!(m.check("muse", &token, "read"), Err("invalid"), "another actor's");
        assert_eq!(m.check(SLUG, &format!("{token}A"), "read"), Err("invalid"), "changed");
        let (message, _) = m.mint(SLUG, &["message".to_string()], None).unwrap();
        assert_eq!(m.check(SLUG, &message, "read"), Err("no-right"));
    }

    // core/assistant/cursors.js's cases, on narrowed grants.
    #[test]
    fn a_cursor_reads_back_only_for_what_it_was_made_for() {
        let (m, t) = memory();
        let (token, _) = m.mint(SLUG, &read(), Some((NOW + 3_600_000) as f64)).unwrap();
        let pages = Granted { grants: &m, token: &token, slug: SLUG };
        let (g, r, fl) = (json!(GRANT), json!(1), json!({ "project": null }));
        let f = || For { grant: &g, revision: &r, op: "sessions", filters: &fl };
        let after = json!([-5, "claude:x"]);
        let cursor = pages.make(&f(), &after).unwrap();
        assert!(cursor.starts_with(ACTOR_PREFIX));
        assert_eq!(pages.read(&cursor, &f()), Ok(Some(after.clone())));
        assert_eq!(pages.read(&cursor, &For { op: "actors", ..f() }), Err("INVALID_ARGUMENT"), "another operation");
        let other = json!({ "project": "prj_x" });
        assert_eq!(pages.read(&cursor, &For { filters: &other, ..f() }), Err("INVALID_ARGUMENT"), "other filters");
        assert_eq!(pages.read(&format!("{cursor}A"), &f()), Err("INVALID_ARGUMENT"), "changed");
        assert_eq!(pages.read(&token, &f()), Err("INVALID_ARGUMENT"), "the grant itself isn't a cursor");
        assert_eq!(Granted { grants: &m, token: &token, slug: "muse" }.read(&cursor, &f()), Err("INVALID_ARGUMENT"), "another actor's");
        assert_eq!(Standalone::new(SLUG).read(&cursor, &f()), Err("INVALID_ARGUMENT"), "made elsewhere");
        let r2 = json!(2);
        assert_eq!(pages.read(&cursor, &For { revision: &r2, ..f() }), Err("CURSOR_EXPIRED"), "from before the grant changed");
        t.store(NOW + CURSOR_MS + 1000, Ordering::SeqCst);
        assert_eq!(pages.read(&cursor, &f()), Err("CURSOR_EXPIRED"), "too old");
    }

    #[test]
    fn a_cursor_isnt_a_credential() {
        let (m, _) = memory();
        let (token, _) = m.mint(SLUG, &read(), Some((NOW + 3_600_000) as f64)).unwrap();
        let (g, r) = (json!(GRANT), json!(1));
        let cursor = Granted { grants: &m, token: &token, slug: SLUG }.make(&For { grant: &g, revision: &r, op: "projects", filters: &json!({}) }, &json!(["a"])).unwrap();
        assert!(m.check(SLUG, &cursor, "read").is_err());
    }
}
