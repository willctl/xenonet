//! Decisions through xenonet-authz, and the grants it checks: Biscuit tokens rooted at this
//! Xenolith's token root.
//!
//! - **The root.** Each start of the service makes a token root in memory (`TokenRoot`), and the
//!   context key (keys.rs) certifies it: this root speaks for this Xenolith, the machine whose id
//!   is the context key's, for ten years. The root's private half never touches a disk; its
//!   certificate does (`<state>/keys/token-roots.json`, owner-only), so a token outlives the start
//!   that minted it, and the stdio server, which never holds the root, can check one too. Only the
//!   service mints.
//! - **A token** is a grant of `Schema::XENOLITH`: the bearer is the subject (`actor:hermes`,
//!   `client:phone`), the rights are the ctx levels its scopes reach (`ctx-read` up to
//!   `ctx-admin`), and it expires. Its text is `xenolith_grant_` and the Biscuit in base64url.
//!   What it may select (its scopes, `ctx:read:project:atlas`) is the row access.json keeps by its
//!   revocation id, beside its label and end. access.json never holds a token, only those rows,
//!   and a revoked token's ids.
//! - **A decision** is xenonet-authz's `decide` for `Act::Ctx(level)` on the space: this machine's
//!   own processes are the machine principal, at the level access.json's `local` gives; a token's
//!   holder is the actor its token names, at the lower of its row's scopes and its subject's live
//!   grants there, and only with the token's right to that level. What decides is the receiver's
//!   own files, never a marker in the shared folder.

use crate::access::{Level, Policy, Scope};
use crate::paths::AppPaths;
use crate::spaces::SpaceId;
use crate::wiki;
use serde_json::{Value, json};
use std::path::PathBuf;
use xenonet_authz::{
    Act, ActorGrant, ActorSlug, AllowScope, AskId, Asked, Binding, Consent, Device, DeviceId, Door, Ground, MachineId, NetworkId, PersonId, Principal, Request, Resource, Role, RootCertificate,
    Schema, SessionId, Settings, SpaceLevel, StandingGrant, Token, TokenRoot,
};
use xenonet_id::codec::unb64_n;
use xenonet_id::sign::{KeyHome, SeedSigner, Signer, VerifyingKey};

/// What a token's text starts with.
pub const PREFIX: &str = "xenolith_grant_";
/// How long a token root speaks for this Xenolith, and so the longest a token lasts.
pub const ROOT_MS: i64 = 3650 * 86_400_000;

pub use xenonet_authz::TokenRoot as Root;

/// A token text's first revocation id, unverified: for the audit chain to tell attempts with one
/// token apart, never to allow anything.
pub fn revocation_of(text: &str) -> Option<String> {
    Token::ids_of(text.trim(), PREFIX)?.into_iter().next()
}

/// Whether `text` is shaped as a token (not whether it's a good one).
pub fn looks_like_token(text: &str) -> bool {
    text.strip_prefix(PREFIX).is_some_and(|b| !b.is_empty() && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'))
}

fn roots_file(paths: &AppPaths) -> PathBuf {
    crate::keys::dir(paths).join("token-roots.json")
}

/// This Xenolith as a machine to xenonet-authz: its context key's id, and the key. Before the
/// service first made the key, a name of its own and no key, so no token checks.
#[derive(Clone, Debug)]
pub struct Me {
    pub machine: MachineId,
    pub key: Option<VerifyingKey>,
}

impl Default for Me {
    fn default() -> Me {
        Me { machine: MachineId::new("xenolith").expect("a machine id"), key: None }
    }
}

impl Me {
    /// From the context key's file, without making one.
    pub fn load(paths: &AppPaths) -> Me {
        let key = || -> Option<VerifyingKey> {
            let v: Value = serde_json::from_str(&std::fs::read_to_string(crate::keys::Use::Context.file(paths)).ok()?).ok()?;
            let seed = v["seed"].as_str().and_then(unb64_n::<32>)?;
            Some(SeedSigner::from_seed(&seed, KeyHome::File).ok()?.public())
        };
        key().and_then(|k| Me::of_key(k).ok()).unwrap_or_default()
    }

    fn of_key(key: VerifyingKey) -> Result<Me, String> {
        Ok(Me { machine: MachineId::new(&key.id()).map_err(|e| e.message)?, key: Some(key) })
    }

    pub fn of(signer: &dyn Signer) -> Result<Me, String> {
        Me::of_key(signer.public())
    }
}

/// The token roots' certificates this Xenolith has made, those still in their time.
pub fn roots(paths: &AppPaths) -> Vec<RootCertificate> {
    let v: Value = std::fs::read_to_string(roots_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v["roots"].as_array().into_iter().flatten().filter_map(RootCertificate::parse).collect()
}

/// A new token root for this start, certified by the context key, its certificate kept beside
/// the others (the ones out of their time dropped).
pub fn new_root(paths: &AppPaths, context: &dyn Signer, now: i64, readers: &[String]) -> Result<TokenRoot, String> {
    let me = Me::of(context)?;
    let root = TokenRoot::new(me.machine, context, now, now + ROOT_MS).map_err(|e| e.message)?;
    let mut all: Vec<RootCertificate> = roots(paths).into_iter().filter(|c| c.until() > now).collect();
    all.push(root.certificate().clone());
    let text = format!("{}\n", serde_json::to_string_pretty(&json!({ "v": 1, "roots": all.iter().map(RootCertificate::to_json).collect::<Vec<_>>() })).unwrap_or_default());
    wiki::write_private(&roots_file(paths), &text, readers).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&roots_file(paths))))?;
    Ok(root)
}

/// The rights a token whose scopes reach up to `level` holds: that level and the ones below.
pub fn rights_up_to(level: Level) -> Vec<xenonet_authz::Action> {
    Level::ALL.iter().filter(|l| **l <= level).map(|l| space_level(*l).action()).collect()
}

/// A token for `subject` (`actor:<slug>` or `client:<id>`) with the rights `scopes` reach, until
/// `until`.
pub fn mint(root: &TokenRoot, subject: &str, scopes: &[Scope], until: i64) -> Result<(Token, String), String> {
    let top = scopes.iter().map(|s| s.level).max().ok_or("a token needs at least one scope, such as ctx:read")?;
    let grant = ActorGrant { actor: ActorSlug::new(subject).map_err(|e| e.message)?, network: None, rights: rights_up_to(top), until: until.min(root.certificate().until()) };
    let token = root.mint_actor_with(&grant, &Schema::XENOLITH, &Binding::default()).map_err(|e| e.message)?;
    let text = token.encode(PREFIX).map_err(|e| e.message)?;
    Ok((token, text))
}

/// A token from its text, checked against this Xenolith's roots and revocations (the policy's):
/// None when it isn't one of its live tokens.
pub fn parse(policy: &Policy, text: &str, now: i64) -> Option<Token> {
    let g = Facts { policy, rows: None, now };
    Token::parse_with(text.trim(), PREFIX, &Schema::XENOLITH, &g, now).ok()
}

/// A level as xenonet-authz names it.
pub fn space_level(l: Level) -> SpaceLevel {
    match l {
        Level::Read => SpaceLevel::Read,
        Level::Append => SpaceLevel::Append,
        Level::Write => SpaceLevel::Write,
        Level::Admin => SpaceLevel::Admin,
    }
}

/// The facts a decision reads: access.json with this Xenolith and its roots (the policy), and the
/// row of the token the request came with.
struct Facts<'a> {
    policy: &'a Policy,
    /// The token's row: its subject and scopes, and the space.
    rows: Option<(&'a str, &'a [Scope], &'a SpaceId)>,
    /// The request's time.
    now: i64,
}

impl Ground for Facts<'_> {
    fn own_machine(&self) -> &MachineId {
        &self.policy.me.machine
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
    fn device(&self, _: &PersonId, _: &DeviceId) -> Option<Device> {
        None
    }
    fn revoked(&self, id: &[u8]) -> bool {
        self.policy.revoked.iter().any(|r| *r == crate::text::hex(id))
    }
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        self.policy.roots.iter().filter(|c| c.key_id() == key_id).cloned().collect()
    }
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        (machine == &self.policy.me.machine).then(|| self.policy.me.key.clone()).flatten()
    }
    fn consent(&self, _: &SessionId) -> Consent {
        Consent::default()
    }
    fn device_of(&self, _: &MachineId) -> Option<DeviceId> {
        None
    }
    fn delegations(&self) -> Vec<StandingGrant> {
        vec![]
    }
    fn allow_scope(&self, _: &[u8]) -> Option<AllowScope> {
        None
    }
    fn ask(&self, _: &AskId) -> Option<Asked> {
        None
    }
    fn governance_synced_at(&self) -> Option<i64> {
        None
    }
    fn holds_space(&self, _: &xenonet_authz::SpaceId) -> bool {
        // The caller names a space of this machine's folder; one that isn't there gets the same
        // answer as one it may not use.
        true
    }
    fn space_level(&self, _: &xenonet_authz::SpaceId, principal: &Principal) -> Option<SpaceLevel> {
        let me = &self.policy.me.machine;
        match principal {
            Principal::Machine { id } if id == me => self.policy.local.map(space_level),
            Principal::Actor { slug, machine } if machine == me => {
                let (subject, scopes, space) = self.rows?;
                (slug.as_str() == subject).then(|| self.policy.row_level(subject, scopes, space, self.now)).flatten().map(space_level)
            }
            _ => None,
        }
    }
}

/// Who asks, for a decision: a token's holder, by the token and its row (subject and scopes), or
/// (None) this machine's own process.
pub type Who<'a> = Option<(&'a Token, &'a str, &'a [Scope])>;

/// Whether `who` may act at `need` in `space`, as xenonet-authz decides on this Xenolith's facts
/// (`policy`, loaded with its id and roots): Ok, or why not.
pub fn decide(policy: &Policy, who: Who<'_>, space: &SpaceId, need: Level, now: i64) -> Result<(), String> {
    let me = &policy.me.machine;
    let resource = Resource::Space(xenonet_authz::SpaceId::new(&space.to_string()).map_err(|e| e.message)?);
    let (principal, token, rows) = match who {
        None => (Principal::Machine { id: me.clone() }, None, None),
        Some((token, subject, scopes)) => {
            let slug = ActorSlug::new(subject).map_err(|e| e.message)?;
            (Principal::Actor { slug, machine: me.clone() }, Some(token), Some((subject, scopes, space)))
        }
    };
    let g = Facts { policy, rows, now };
    let request = Request::new(principal, resource, Act::Ctx(space_level(need)), Door::Local, now, None);
    match xenonet_authz::decide(token, &request, &g, None) {
        xenonet_authz::Decision::Allow(_) => Ok(()),
        d => Err(d.reason().map(|r| r.text().to_string()).unwrap_or_else(|| "held".into())),
    }
}

/// The highest level `who` reaches in `space` by `decide`, or None.
pub fn level(policy: &Policy, who: Who<'_>, space: &SpaceId, now: i64) -> Option<Level> {
    Level::ALL.iter().rev().copied().find(|l| decide(policy, who, space, *l, now).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::parse_scope;

    const T: i64 = 1_792_512_000_000;

    fn key(n: u8) -> SeedSigner {
        SeedSigner::from_seed(&[n; 32], KeyHome::File).unwrap()
    }

    fn sc(s: &str) -> Scope {
        parse_scope(s).unwrap()
    }

    fn sp(s: &str) -> SpaceId {
        SpaceId::parse(s).unwrap()
    }

    /// A policy for the Xenolith whose context key is `context`, holding `root`'s certificate.
    fn policy(context: &SeedSigner, root: Option<&TokenRoot>) -> Policy {
        Policy { me: Me::of(context).unwrap(), roots: root.map(|r| vec![r.certificate().clone()]).unwrap_or_default(), ..Policy::new() }
    }

    #[test]
    fn a_token_reaches_the_lower_of_its_rights_its_row_and_its_grant() {
        let context = key(1);
        let root = TokenRoot::new(Me::of(&context).unwrap().machine, &context, T - 1000, T + ROOT_MS).unwrap();
        let mut p = policy(&context, Some(&root));
        p.grant("client:phone", sc("ctx:write:project:atlas"), 86_400_000, "you", T).unwrap();
        // Asks read on atlas: its rights are ctx-read alone.
        let (token, text) = mint(&root, "client:phone", &[sc("ctx:read:project:atlas")], T + 86_400_000).unwrap();
        assert!(text.starts_with(PREFIX) && looks_like_token(&text));
        assert_eq!(token.rights(), &rights_up_to(Level::Read)[..]);
        let back = parse(&p, &text, T).unwrap();
        let rows = [sc("ctx:read:project:atlas")];
        let who = Some((&back, "client:phone", &rows[..]));
        assert_eq!(level(&p, who, &sp("project:atlas"), T), Some(Level::Read));
        assert_eq!(level(&p, who, &sp("project:other"), T), None, "its row selects atlas only");
        assert_eq!(level(&p, who, &sp("machine:local"), T), None);
        // A row that asks for more than the token's rights still gets only the rights.
        let wide = [sc("ctx:write:project:atlas")];
        assert_eq!(level(&p, Some((&back, "client:phone", &wide[..])), &sp("project:atlas"), T), Some(Level::Read), "the Biscuit's rights cap it");
        let why = decide(&p, Some((&back, "client:phone", &wide[..])), &sp("project:atlas"), Level::Write, T).unwrap_err();
        assert!(why.contains("no right"), "{why}");
        // Another subject's row on the same token: refused.
        assert!(decide(&p, Some((&back, "client:other", &rows[..])), &sp("project:atlas"), Level::Read, T).is_err());
        // Revoked by its id: no longer a token.
        p.revoked.push(token.id());
        assert!(parse(&p, &text, T).is_none());
        // Another Xenolith's root: unknown here.
        let other = key(2);
        let theirs = TokenRoot::new(Me::of(&other).unwrap().machine, &other, T - 1000, T + ROOT_MS).unwrap();
        let (_, foreign) = mint(&theirs, "client:phone", &[sc("ctx:read")], T + 1000).unwrap();
        assert!(parse(&policy(&context, Some(&root)), &foreign, T).is_none());
        // Before this Xenolith has a key, no token checks at all.
        assert!(parse(&Policy::new(), &text, T).is_none());
    }

    #[test]
    fn this_machine_gets_what_local_says() {
        let mut p = policy(&key(3), None);
        assert_eq!(level(&p, None, &sp("machine:local"), T), Some(Level::Admin));
        p.local = Some(Level::Read);
        assert_eq!(level(&p, None, &sp("project:atlas"), T), Some(Level::Read));
        let why = decide(&p, None, &sp("project:atlas"), Level::Append, T).unwrap_err();
        assert!(why.contains("gives that machine read"), "{why}");
        p.local = None;
        assert_eq!(level(&p, None, &sp("project:atlas"), T), None);
        // The same before the service first made the context key.
        assert_eq!(level(&Policy::new(), None, &sp("project:atlas"), T), Some(Level::Admin));
    }

    #[test]
    fn the_roots_file_keeps_each_starts_certificate_and_only_its_owner_reads_it() {
        let base = std::env::temp_dir().join(format!("xenolith-grants-{}", crate::text::random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), base.to_string_lossy().to_string())].into_iter().collect();
        let paths = AppPaths::from_env(&env);
        let context = key(4);
        let a = new_root(&paths, &context, T, &[]).unwrap();
        let b = new_root(&paths, &context, T + 1, &[]).unwrap();
        let held = roots(&paths);
        assert_eq!(held.len(), 2);
        assert!(held.contains(a.certificate()) && held.contains(b.certificate()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(roots_file(&paths)).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // A token from an earlier start still parses after a restart.
        let (_, text) = mint(&a, "actor:hermes", &[sc("ctx:append:by:hermes")], T + 1000).unwrap();
        let p = Policy { me: Me::of(&context).unwrap(), roots: roots(&paths), ..Policy::new() };
        assert!(parse(&p, &text, T + 2).is_some());
        let _ = std::fs::remove_dir_all(base);
    }
}
