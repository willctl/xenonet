//! Tokens: Biscuits rooted at a machine. Biscuit signs only with a key in
//! memory, and a machine's signing key may be one that never leaves a
//! secure element, so each service start makes a fresh [`TokenRoot`] and
//! has the machine's [`Signer`] sign its [`RootCertificate`]: this root
//! speaks for this machine from `from` to `until`. Certificates are public,
//! so they're kept and travel, and tokens outlive restarts; the root's
//! private half never touches a disk.
//!
//! Tokens name machines and networks only, in this phase: an actor token
//! holds the actor's slug, its machine, the network it may act in and one
//! `right(...)` per action; an assistant token holds its grant's id, its
//! revision and `right("read")`. Both expire. Attenuation appends checks
//! (a door, an earlier end, fewer actions, a page cursor), which in Biscuit
//! can only narrow. Verifying refuses a root the ground doesn't hold, one
//! out of its time, one whose machine's key didn't sign it, a revoked
//! token, and any authority block that isn't exactly what's minted here, so
//! no root can slip a rule or a fact of the ground's into a decision.

use crate::vocab::{Act, Action, ActorSlug, GrantId, MachineId, NetworkId, ReadScope, Request};
use crate::{AuthzError, Door, Ground, R};
use biscuit_auth::builder::{date, fact, int, string, Algorithm, Term};
use biscuit_auth::error::{FailedCheck, Logic, Token as BiscuitError};
use biscuit_auth::{AuthorizerBuilder, AuthorizerLimits, Biscuit, BlockBuilder, KeyPair, PublicKey, UnverifiedBiscuit};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use xenonet_id::codec::{b64, hex, sha256, unb64};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};

/// The prefix of an actor's token.
pub const ACTOR_PREFIX: &str = "xenonet_actor_";
/// The prefix of an actor's token narrowed to the gateway.
pub const GATEWAY_PREFIX: &str = "xenonet_gateway_";
/// The prefix of an assistant's read token.
pub const ASSISTANT_PREFIX: &str = "xenonet_assistant_";
/// How long a page cursor lasts.
pub const CURSOR_MS: i64 = 5 * 60_000;

/// The longest token text read.
const TEXT_MAX: usize = 16 * 1024;

// What an authority block may hold: these facts, and this one check.
const FACTS: [&str; 7] = ["actor", "grant", "machine", "network", "right", "revision", "expires"];
const EXPIRY: &str = "check if time($time), expires($until), $time <= $until";

// Datalog that a token can't make slow: a decision is on a hot path.
fn limits() -> AuthorizerLimits {
    AuthorizerLimits { max_facts: 1000, max_iterations: 100, max_time: Duration::from_millis(50) }
}

fn refused(message: impl Into<String>) -> AuthzError {
    AuthzError::new("token", message)
}

fn biscuit_error(e: BiscuitError) -> AuthzError {
    AuthzError::new("internal", format!("the token couldn't be made: {e}"))
}

// Biscuit's dates are whole seconds; times before 1970 are 1970.
fn to_date(ms: i64) -> Term {
    date(&(UNIX_EPOCH + Duration::from_secs((ms.max(0) / 1000) as u64)))
}

fn from_date(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// A machine's word that a token root speaks for it, for a while.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCertificate {
    machine: MachineId,
    root: VerifyingKey,
    from: i64,
    until: i64,
    sig: Vec<u8>,
}

impl RootCertificate {
    fn message(machine: &MachineId, root: &VerifyingKey, from: i64, until: i64) -> Vec<u8> {
        format!("{machine}\n{}\n{}\n{from}\n{until}", root.id(), b64(root.bytes())).into_bytes()
    }

    pub fn machine(&self) -> &MachineId {
        &self.machine
    }

    pub fn root(&self) -> &VerifyingKey {
        &self.root
    }

    pub fn from(&self) -> i64 {
        self.from
    }

    pub fn until(&self) -> i64 {
        self.until
    }

    /// The hint a token carries for its root: the first four bytes of the
    /// SHA-256 of the root key.
    pub fn key_id(&self) -> u32 {
        let h = sha256(self.root.bytes());
        u32::from_be_bytes([h[0], h[1], h[2], h[3]])
    }

    /// Whether `machine_key` signed it and `at` is within its time.
    pub fn verify(&self, machine_key: &VerifyingKey, at: i64) -> R<()> {
        if !machine_key.verify(Purpose::TOKEN_ROOT, &RootCertificate::message(&self.machine, &self.root, self.from, self.until), &self.sig) {
            return Err(AuthzError::new("root", format!("that token root isn't certified by {}'s signing key", self.machine)));
        }
        if at < self.from || at > self.until {
            return Err(AuthzError::new("root", format!("that token root of {} is out of its time", self.machine)));
        }
        Ok(())
    }

    fn biscuit_key(&self) -> R<PublicKey> {
        match &self.root {
            VerifyingKey::Ed25519(k) => PublicKey::from_bytes(k, Algorithm::Ed25519).map_err(|_| AuthzError::new("root", "that token root isn't a key")),
            VerifyingKey::P256(_) => Err(AuthzError::new("root", "a token root is an Ed25519 key")),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "v": 1, "machine": self.machine.as_str(), "root": self.root.to_json(), "from": self.from, "until": self.until, "sig": b64(&self.sig) })
    }

    /// One read from its JSON; its signature isn't checked here.
    pub fn parse(v: &Value) -> Option<RootCertificate> {
        if v.get("v")?.as_i64()? != 1 {
            return None;
        }
        Some(RootCertificate {
            machine: MachineId::new(v.get("machine")?.as_str()?).ok()?,
            root: VerifyingKey::parse(v.get("root")?)?,
            from: v.get("from")?.as_i64()?,
            until: v.get("until")?.as_i64()?,
            sig: unb64(v.get("sig")?.as_str()?).filter(|s| (64..=72).contains(&s.len()))?,
        })
    }
}

/// The key a machine mints tokens with, made fresh in memory at each start.
pub struct TokenRoot {
    pair: KeyPair,
    cert: RootCertificate,
}

impl fmt::Debug for TokenRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[the token root {} of {}]", self.cert.root.id(), self.cert.machine)
    }
}

/// What an actor's token allows: its rights, where, and until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorGrant {
    pub actor: ActorSlug,
    /// The one network it may act in, or any this machine is in.
    pub network: Option<NetworkId>,
    pub rights: Vec<Action>,
    /// Milliseconds since 1970.
    pub until: i64,
}

/// An assistant's read grant, by its id and revision: what it may read
/// (projects, data, lookback) stays in the receiver's rows for the grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantGrant {
    pub grant: GrantId,
    pub revision: u64,
    pub until: i64,
}

impl TokenRoot {
    /// A new root for `machine`, certified by `signer`, the machine's
    /// signing key, from `from` to `until`.
    pub fn new(machine: MachineId, signer: &dyn Signer, from: i64, until: i64) -> R<TokenRoot> {
        let pair = KeyPair::new();
        let bytes: [u8; 32] = pair.public().to_bytes().try_into().map_err(|_| AuthzError::new("internal", "a token root isn't an Ed25519 key"))?;
        let root = VerifyingKey::Ed25519(bytes);
        let sig = signer.sign(Purpose::TOKEN_ROOT, &RootCertificate::message(&machine, &root, from, until))?;
        Ok(TokenRoot { pair, cert: RootCertificate { machine, root, from, until, sig } })
    }

    pub fn certificate(&self) -> &RootCertificate {
        &self.cert
    }

    fn mint(&self, bearer: Bearer, network: Option<NetworkId>, rights: Vec<Action>, until: i64) -> R<Token> {
        if until > self.cert.until {
            return Err(AuthzError::new("token", "a token can't outlive its root"));
        }
        let b = || -> Result<Biscuit, BiscuitError> {
            let mut b = Biscuit::builder().root_key_id(self.cert.key_id());
            b = match &bearer {
                Bearer::Actor { slug, machine } => b.fact(fact("actor", &[string(slug.as_str())]))?.fact(fact("machine", &[string(machine.as_str())]))?,
                Bearer::Assistant { grant, machine, revision } => {
                    b.fact(fact("grant", &[string(grant.as_str())]))?.fact(fact("machine", &[string(machine.as_str())]))?.fact(fact("revision", &[int(*revision as i64)]))?
                }
            };
            if let Some(n) = &network {
                b = b.fact(fact("network", &[string(n.as_str())]))?;
            }
            for r in &rights {
                b = b.fact(fact("right", &[string(r.as_str())]))?;
            }
            b.fact(fact("expires", &[to_date(until)]))?.check(EXPIRY)?.build(&self.pair)
        };
        let biscuit = b().map_err(biscuit_error)?;
        Ok(Token { biscuit, root: self.cert.clone(), bearer, network, rights, until })
    }

    /// An actor's token, with one right per action it may do.
    pub fn mint_actor(&self, grant: &ActorGrant) -> R<Token> {
        let mut rights = grant.rights.clone();
        rights.sort();
        rights.dedup();
        self.mint(Bearer::Actor { slug: grant.actor.clone(), machine: self.cert.machine.clone() }, grant.network.clone(), rights, grant.until)
    }

    /// An assistant's read token.
    pub fn mint_assistant(&self, grant: &AssistantGrant) -> R<Token> {
        if grant.revision > i64::MAX as u64 {
            return Err(AuthzError::new("token", "that revision is too large"));
        }
        self.mint(Bearer::Assistant { grant: grant.grant.clone(), machine: self.cert.machine.clone(), revision: grant.revision }, None, vec![Action::Read], grant.until)
    }
}

/// Who a token speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bearer {
    Actor { slug: ActorSlug, machine: MachineId },
    Assistant { grant: GrantId, machine: MachineId, revision: u64 },
}

/// One way to narrow a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Narrowing {
    /// Only through this door (a gateway token: `Door(Door::Gateway)`).
    Door(Door),
    /// Only until then, in milliseconds since 1970.
    Until(i64),
    /// Only these actions.
    Actions(Vec<Action>),
    /// A page cursor: only this read, with these filters (their hash),
    /// until then.
    Cursor { scope: ReadScope, filters: [u8; 32], until: i64 },
}

/// A token, verified: minted here, or parsed against the ground's
/// certified roots. Logged, it names whom it's for and never its bytes.
pub struct Token {
    biscuit: Biscuit,
    root: RootCertificate,
    bearer: Bearer,
    network: Option<NetworkId>,
    rights: Vec<Action>,
    until: i64,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.bearer {
            Bearer::Actor { slug, machine } => write!(f, "[the token of the actor {slug} on {machine}]"),
            Bearer::Assistant { grant, machine, .. } => write!(f, "[the token of the grant {grant} on {machine}]"),
        }
    }
}

// The values of one fact in a token's authority block. A query trusts
// every block unless told otherwise, so this one names the authority: a
// holder's block can't add a right, a network or a later end here.
fn values<T>(a: &mut biscuit_auth::Authorizer, name: &str) -> R<Vec<T>>
where
    T: TryFrom<Term, Error = BiscuitError>,
{
    let rows: Vec<(T,)> = a.query_all(format!("data($x) <- {name}($x) trusting authority").as_str()).map_err(|e| refused(format!("that token couldn't be read: {e}")))?;
    Ok(rows.into_iter().map(|(v,)| v).collect())
}

fn one<T>(mut v: Vec<T>, name: &str) -> R<T> {
    match (v.pop(), v.is_empty()) {
        (Some(x), true) => Ok(x),
        _ => Err(refused(format!("that token doesn't name exactly one {name}"))),
    }
}

impl Token {
    pub fn bearer(&self) -> &Bearer {
        &self.bearer
    }

    pub fn network(&self) -> Option<&NetworkId> {
        self.network.as_ref()
    }

    pub fn rights(&self) -> &[Action] {
        &self.rights
    }

    /// When its authority ends; a narrowing may end it sooner.
    pub fn until(&self) -> i64 {
        self.until
    }

    pub fn root(&self) -> &RootCertificate {
        &self.root
    }

    /// Its Biscuit revocation ids, one per block: revoking the first takes
    /// back the token and everything narrowed from it.
    pub fn revocation_ids(&self) -> Vec<Vec<u8>> {
        self.biscuit.revocation_identifiers()
    }

    /// The text to hand over: `prefix` and the base64url of the token.
    pub fn encode(&self, prefix: &str) -> R<String> {
        check_prefix(prefix)?;
        let bytes = self.biscuit.to_vec().map_err(biscuit_error)?;
        Ok(format!("{prefix}{}", b64(&bytes)))
    }

    /// A narrower token: this one with `narrowing` appended.
    pub fn attenuate(&self, narrowing: &Narrowing) -> R<Token> {
        let until_check =
            |b: BlockBuilder, until: i64| b.code_with_params("check if time($time), $time <= {until}", HashMap::from([("until".to_string(), to_date(until))]), HashMap::new());
        let block = || -> Result<BlockBuilder, BiscuitError> {
            let b = BlockBuilder::new();
            match narrowing {
                Narrowing::Door(d) => b.check(format!("check if door(\"{}\")", d.as_str()).as_str()),
                Narrowing::Until(t) => until_check(b, *t),
                Narrowing::Actions(list) if list.is_empty() => b.check("check if false"),
                Narrowing::Actions(list) => {
                    let any: Vec<String> = list.iter().map(|a| format!("action(\"{}\")", a.as_str())).collect();
                    b.check(format!("check if {}", any.join(" or ")).as_str())
                }
                Narrowing::Cursor { scope, filters, until } => {
                    until_check(b.check(format!("check if op(\"{}\"), filters(\"{}\")", scope.as_str(), hex(filters)).as_str())?, *until)
                }
            }
        };
        let biscuit = self.biscuit.append(block().map_err(biscuit_error)?).map_err(biscuit_error)?;
        Ok(Token { biscuit, root: self.root.clone(), bearer: self.bearer.clone(), network: self.network.clone(), rights: self.rights.clone(), until: self.until })
    }

    /// A page cursor for an assistant: this token, narrowed to one read
    /// with these filters, for five minutes from `now`.
    pub fn cursor(&self, scope: ReadScope, filters: &[u8; 32], now: i64) -> R<Token> {
        if !matches!(self.bearer, Bearer::Assistant { .. }) {
            return Err(AuthzError::new("token", "only an assistant's token makes page cursors"));
        }
        self.attenuate(&Narrowing::Cursor { scope, filters: *filters, until: now.saturating_add(CURSOR_MS) })
    }

    /// A token from its text, verified against the roots the ground holds
    /// at `now`: refused when the prefix isn't `prefix`, the root is
    /// unknown, uncertified or out of its time, any of its ids is revoked,
    /// it expired, or it isn't a token minted as here.
    pub fn parse(text: &str, prefix: &str, ground: &dyn Ground, now: i64) -> R<Token> {
        check_prefix(prefix)?;
        let body = text.strip_prefix(prefix).ok_or_else(|| refused(format!("that isn't a token that starts {prefix}")))?;
        if body.len() > TEXT_MAX {
            return Err(refused("that token is too long"));
        }
        let bytes = unb64(body).ok_or_else(|| refused("that token isn't base64url"))?;
        let unverified = UnverifiedBiscuit::from(&bytes).map_err(|_| refused("that isn't a token"))?;
        let key_id = unverified.root_key_id().ok_or_else(|| refused("that token names no root"))?;
        let mut why = AuthzError::new("root", "that token's root isn't one this machine knows");
        for cert in ground.token_roots(key_id) {
            if cert.key_id() != key_id {
                continue;
            }
            let Some(machine_key) = ground.signing_key(&cert.machine) else {
                why = AuthzError::new("root", format!("{} isn't a machine this one knows the key of", cert.machine));
                continue;
            };
            if let Err(e) = cert.verify(&machine_key, now) {
                why = e;
                continue;
            }
            match Biscuit::from(&bytes, cert.biscuit_key()?) {
                Ok(biscuit) => return Token::read(biscuit, cert, ground, now),
                Err(_) => why = AuthzError::new("root", "that token isn't signed by its root"),
            }
        }
        Err(why)
    }

    // A verified Biscuit, read: revocation, the authority block's shape,
    // whom it's for and until when.
    fn read(biscuit: Biscuit, root: RootCertificate, ground: &dyn Ground, now: i64) -> R<Token> {
        if biscuit.revocation_identifiers().iter().any(|id| ground.revoked(id)) {
            return Err(AuthzError::new("revoked", "that token was revoked"));
        }
        let source = biscuit.print_block_source(0).map_err(|_| refused("that token's authority can't be read"))?;
        let mut expiry = 0;
        for line in source.split(";\n").map(str::trim).filter(|l| !l.is_empty()) {
            let line = line.trim_end_matches(';');
            if line == EXPIRY {
                expiry += 1;
                continue;
            }
            let fact = FACTS.iter().any(|f| line.strip_prefix(f).is_some_and(|rest| rest.starts_with('(') && rest.ends_with(')')));
            if !fact || line.contains("<-") || line.contains('$') || line.contains(',') {
                return Err(refused("that token's authority holds more than XenoNet mints"));
            }
        }
        if expiry != 1 {
            return Err(refused("that token's authority holds more than XenoNet mints"));
        }
        let mut a = AuthorizerBuilder::new().set_limits(limits()).build(&biscuit).map_err(|e| refused(format!("that token couldn't be read: {e}")))?;
        let machine = MachineId::new(&one(values::<String>(&mut a, "machine")?, "machine")?)?;
        if machine != root.machine {
            return Err(refused(format!("that token speaks for {machine}, and its root for {}", root.machine)));
        }
        let actors = values::<String>(&mut a, "actor")?;
        let grants = values::<String>(&mut a, "grant")?;
        let bearer = match (actors.is_empty(), grants.is_empty()) {
            (false, true) => Bearer::Actor { slug: ActorSlug::new(&one(actors, "actor")?)?, machine },
            (true, false) => {
                let revision = u64::try_from(one(values::<i64>(&mut a, "revision")?, "revision")?).map_err(|_| refused("that token's revision isn't one"))?;
                Bearer::Assistant { grant: GrantId::new(&one(grants, "grant")?)?, machine, revision }
            }
            _ => return Err(refused("that token is for neither an actor nor a grant")),
        };
        let network = match values::<String>(&mut a, "network")? {
            v if v.is_empty() => None,
            v => Some(NetworkId::new(&one(v, "network")?)?),
        };
        let mut rights = Vec::new();
        for r in values::<String>(&mut a, "right")? {
            rights.push(Action::parse(&r).ok_or_else(|| refused(format!("{r} isn't a right")))?);
        }
        rights.sort();
        if matches!(bearer, Bearer::Assistant { .. }) && rights != [Action::Read] {
            return Err(refused("an assistant's token reads, and nothing else"));
        }
        let until = from_date(one(values::<SystemTime>(&mut a, "expires")?, "end")?);
        if until / 1000 < now / 1000 {
            return Err(AuthzError::new("expired", "that token expired"));
        }
        Ok(Token { biscuit, root, bearer, network, rights, until })
    }

    /// Runs the token's Datalog for `request`: its own checks (expiry,
    /// narrowings) must pass, and it must hold the action's right, with the
    /// right to use the gateway when it came through the gateway or A2A.
    /// Facts from the request are the ambient ones; an appended block's
    /// facts never reach these policies, which trust the authority alone.
    pub(crate) fn authorize(&self, request: &Request) -> Result<(), String> {
        let build = || -> Result<biscuit_auth::Authorizer, BiscuitError> {
            let mut b = AuthorizerBuilder::new()
                .fact(fact("time", &[to_date(request.at())]))?
                .fact(fact("action", &[string(request.action().as_str())]))?
                .fact(fact("door", &[string(request.door().as_str())]))?;
            if let Act::Read { scope, filters } = request.act() {
                b = b.fact(fact("op", &[string(scope.as_str())]))?;
                if let Some(f) = filters {
                    b = b.fact(fact("filters", &[string(&hex(f))]))?;
                }
            }
            b.policy("allow if action($a), right($a), door(\"local\")")?
                .policy("allow if action($a), right($a), door(\"network\")")?
                .policy("allow if action($a), right($a), right(\"use-gateway\")")?
                .set_limits(limits())
                .build(&self.biscuit)
        };
        let mut a = build().map_err(|e| format!("the token couldn't be checked: {e}"))?;
        match a.authorize() {
            Ok(_) => Ok(()),
            Err(BiscuitError::FailedLogic(Logic::Unauthorized { checks, .. } | Logic::NoMatchingPolicy { checks })) if !checks.is_empty() => Err(failed(&checks)),
            Err(BiscuitError::FailedLogic(_)) if matches!(request.door(), Door::Gateway | Door::A2a) => {
                Err(format!("the token has no right to {} through the gateway", request.action().as_str()))
            }
            Err(BiscuitError::FailedLogic(_)) => Err(format!("the token has no right to {}", request.action().as_str())),
            Err(e) => Err(format!("the token couldn't be checked: {e}")),
        }
    }
}

// Words for the first of the token's checks that failed.
fn failed(checks: &[FailedCheck]) -> String {
    match checks.first() {
        Some(FailedCheck::Block(c)) if c.block_id == 0 && c.rule == EXPIRY => "the token expired".into(),
        Some(FailedCheck::Block(c)) => format!("the token is narrowed: {}", c.rule),
        Some(FailedCheck::Authorizer(c)) => format!("the token fails {}", c.rule),
        None => "the token's checks failed".into(),
    }
}

fn check_prefix(prefix: &str) -> R<()> {
    if prefix.is_empty() || prefix.len() > 32 || !prefix.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
        return Err(AuthzError::new("token", format!("{prefix:?} isn't a token prefix")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Attestation, Consent, Device, PersonId, Settings, StandingGrant};
    use xenonet_id::sign::{KeyHome, SeedSigner};

    const NOW: i64 = 1_800_000_000_000;

    // A ground that knows one machine and its one root.
    struct One {
        machine: MachineId,
        key: VerifyingKey,
        cert: RootCertificate,
    }

    impl Ground for One {
        fn own_machine(&self) -> &MachineId {
            &self.machine
        }
        fn role(&self, _: &NetworkId, _: &MachineId) -> Option<crate::Role> {
            None
        }
        fn settings(&self, _: Option<&NetworkId>) -> Settings {
            Settings::default()
        }
        fn own_person(&self) -> Option<PersonId> {
            None
        }
        fn device(&self, _: &PersonId, _: &crate::DeviceId) -> Option<Device> {
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
        fn consent(&self, _: &crate::SessionId) -> Consent {
            Consent::default()
        }
        fn standing_grant(&self, _: &MachineId) -> Option<StandingGrant> {
            None
        }
        fn asker(&self, _: &crate::AskId) -> Option<Attestation> {
            None
        }
        fn assistant_grant(&self, _: &GrantId) -> Option<u64> {
            None
        }
        fn governance_synced_at(&self) -> Option<i64> {
            None
        }
    }

    fn world() -> (TokenRoot, One) {
        let signer = SeedSigner::from_seed(&[1; 32], KeyHome::File).unwrap();
        let machine = MachineId::new("m1").unwrap();
        let root = TokenRoot::new(machine.clone(), &signer, NOW - 1000, NOW + 3_600_000).unwrap();
        let one = One { machine, key: signer.public(), cert: root.certificate().clone() };
        (root, one)
    }

    // A token whose authority is `code`, signed by the certified root, as a
    // rogue or buggy minter might make it.
    fn crafted(root: &TokenRoot, code: &str) -> String {
        let b = Biscuit::builder().root_key_id(root.cert.key_id()).code(code).unwrap().build(&root.pair).unwrap();
        format!("{ACTOR_PREFIX}{}", b64(&b.to_vec().unwrap()))
    }

    const GOOD: &str = r#"actor("muse"); machine("m1"); right("message"); expires(2027-01-15T08:00:00Z); check if time($time), expires($until), $time <= $until;"#;

    #[test]
    fn the_authority_block_is_exactly_what_is_minted() {
        let (root, one) = world();
        // What mint writes reads back, and so does the same by hand.
        let minted = root.mint_actor(&ActorGrant { actor: ActorSlug::new("muse").unwrap(), network: None, rights: vec![Action::Message], until: NOW + 1000 }).unwrap();
        assert_eq!(minted.biscuit.print_block_source(0).unwrap().lines().last(), Some(format!("{EXPIRY};").as_str()));
        Token::parse(&minted.encode(ACTOR_PREFIX).unwrap(), ACTOR_PREFIX, &one, NOW).unwrap();
        Token::parse(&crafted(&root, GOOD), ACTOR_PREFIX, &one, NOW).unwrap();
        // A rule that would make any right, a fact of the request's, a
        // second machine, a fact with two terms, a missing or second end,
        // an actor and a grant at once: all refused.
        for code in [
            GOOD.replace(r#"right("message");"#, r"right($a) <- action($a);"),
            GOOD.replace(r#"right("message");"#, r#"door("local");"#),
            GOOD.replace(r#"machine("m1");"#, r#"machine("m1"); machine("m2");"#),
            GOOD.replace(r#"right("message");"#, r#"right("message", "m2");"#),
            GOOD.replace("check if time($time), expires($until), $time <= $until;", ""),
            GOOD.replace("check if time($time), expires($until), $time <= $until;", "check if true;"),
            GOOD.replace(r#"actor("muse");"#, r#"actor("muse"); grant("g1");"#),
            GOOD.replace(r#"machine("m1");"#, r#"machine("m2");"#),
            GOOD.replace(r#"right("message");"#, r#"right("fly");"#),
            GOOD.replace(r#"actor("muse");"#, r#"actor("two words");"#),
        ] {
            assert!(Token::parse(&crafted(&root, &code), ACTOR_PREFIX, &one, NOW).is_err(), "{code}");
        }
    }

    #[test]
    fn a_holders_block_cant_add_rights() {
        let (root, one) = world();
        let token = root.mint_actor(&ActorGrant { actor: ActorSlug::new("muse").unwrap(), network: None, rights: vec![Action::Message], until: NOW + 1000 }).unwrap();
        // The holder appends facts of its own: they never reach the
        // authorizer's policies, which trust the authority block alone.
        let widened = token.biscuit.append(BlockBuilder::new().code(r#"right("start"); right("use-gateway"); door("local");"#).unwrap()).unwrap();
        let text = format!("{ACTOR_PREFIX}{}", b64(&widened.to_vec().unwrap()));
        let parsed = Token::parse(&text, ACTOR_PREFIX, &one, NOW).unwrap();
        assert_eq!(parsed.rights(), &[Action::Message]);
        let actor = crate::Principal::Actor { slug: ActorSlug::new("muse").unwrap(), machine: MachineId::new("m1").unwrap() };
        let run = crate::Resource::Run { machine: MachineId::new("m1").unwrap(), app: crate::AppId::new("claude").unwrap(), folder: crate::Folder::new("/w").unwrap() };
        let start = Act::Start { app: crate::AppId::new("claude").unwrap(), folder: crate::Folder::new("/w").unwrap(), mode: crate::StartMode::Asks };
        let r = Request::new(actor.clone(), run, start, Door::Local, NOW, None);
        assert!(parsed.authorize(&r).is_err());
        let r = Request::new(actor, crate::Resource::Machine(MachineId::new("m1").unwrap()), Act::Message, Door::Gateway, NOW, None);
        assert!(parsed.authorize(&r).is_err(), "no gateway right from a holder's block");
    }
}
