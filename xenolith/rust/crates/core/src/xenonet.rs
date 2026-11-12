//! Shared mode: what this Xenolith holds of the XenoNet on its machine, and how it binds a peer's
//! context key to that peer's machine (phase 6, commit 16; review revisions 16, 22, 23, 35).
//!
//! - **The grant.** XenoNet gives this Xenolith an actor grant (a Biscuit under the machine's
//!   token root) that names `forward(xenolith/sync/2)`, so XenoNet carries sync/2 over its own
//!   endpoint for it, and `key(<context key id>)`, so a peer can tell that this context key speaks
//!   for this machine. It's kept in `<state>/keys/xenonet.grant`, owner-only (`write_grant`).
//! - **The chains.** Over the forward socket XenoNet hands out copies of what it holds: its
//!   networks' rosters, the person chains of their owners and its own, and its token roots'
//!   certificates. They're verified here from genesis (`Known::verify`), with each person chain
//!   checking the devices that signed the rosters, and kept in `<state>/xenonet/chains.json`, so a
//!   space chain a person owns checks across restarts. XenoNet's word is taken for one thing only:
//!   which endpoint id a forwarded connection comes from.
//! - **A peer's hello** over the forward carries a copy of its grant narrowed so no XenoNet door
//!   takes it (`check if false`, for an hour) and its root's certificate (`hello_grant`). The
//!   receiver binds the hello's context key to a machine only when that machine, by the rosters,
//!   owns the endpoint the connection comes from, its machine key certified the root, and the
//!   grant names that key (`bind_hello`). A grant without `key(id)`, a copy used from another
//!   machine's connection, or a root another machine certified binds nothing.
//! - **Who owns what** (revision 8): a person owns themselves; a machine (by its signing key) and
//!   its actors are owned by the person whose network it's in (`Owners`). Projects' owners come
//!   from XenoNet later (commit 18), so a space binds to a project only then.

use crate::paths::AppPaths;
use crate::wiki;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use xenonet_authz::{
    ACTOR_PREFIX, AllowScope, AskId, Asked, Consent, Device, DeviceId, Ground, MachineId, Narrowing, NetworkId, PersonId, Principal, Role, RootCertificate, Schema, SessionId, Settings, SpaceLevel,
    StandingGrant, Token,
};
use xenonet_id::sign::VerifyingKey;
use xenonet_log::network::{Member, NetworkPolicy, Roster};
use xenonet_log::person::{Devices, PersonChain};
use xenonet_log::space::{Owners, SpacePolicy};
use xenonet_model::ids::EntityRef;

/// The ALPN XenoNet forwards for this Xenolith.
pub const ALPN: &str = "xenolith/sync/2";
/// How long the copy of a grant a hello carries lasts.
pub const HELLO_GRANT_MS: i64 = 3600 * 1000;

fn dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("xenonet")
}

fn chains_file(paths: &AppPaths) -> PathBuf {
    dir(paths).join("chains.json")
}

/// Where the grant XenoNet gave this Xenolith is kept.
pub fn grant_file(paths: &AppPaths) -> PathBuf {
    crate::keys::dir(paths).join("xenonet.grant")
}

/// The grant, when there is one.
pub fn read_grant(paths: &AppPaths) -> Option<String> {
    std::fs::read_to_string(grant_file(paths)).ok().map(|t| t.trim().to_string()).filter(|t| t.starts_with(ACTOR_PREFIX))
}

/// Keeps `text` as the grant, owner-only.
pub fn write_grant(paths: &AppPaths, text: &str) -> Result<(), String> {
    let text = text.trim();
    if !text.starts_with(ACTOR_PREFIX) || text.len() > 16 * 1024 || !text[ACTOR_PREFIX.len()..].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        return Err(format!("that isn't a XenoNet actor grant (it starts {ACTOR_PREFIX})"));
    }
    let file = grant_file(paths);
    if let Some(p) = file.parent() {
        std::fs::create_dir_all(p).map_err(|e| format!("cannot make {}: {e}", wiki::disp(p)))?;
    }
    wiki::write_private(&file, &format!("{text}\n"), &[]).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&file)))
}

/// The person chains held, as the rosters' and the space chains' rules ask them for devices.
#[derive(Clone, Default)]
pub struct Persons(pub BTreeMap<String, PersonChain>);

impl Devices for Persons {
    fn person_device(&self, person: &str, head: &str, device: &str) -> Option<xenonet_log::person::Device> {
        self.0.get(person)?.device_at(device, head).cloned()
    }
}

/// The chains XenoNet handed out, verified.
#[derive(Clone, Default)]
pub struct Known {
    pub networks: BTreeMap<String, Roster>,
    pub persons: Persons,
    /// Token root certificates, as they came: each is checked against the machine key that
    /// certified it when it's used.
    pub roots: Vec<RootCertificate>,
}

impl Known {
    /// The chains in `v` (`{networks: [{id, chain}], persons: [{person, chain}], roots: [...]}`),
    /// each verified from genesis. One that doesn't verify is left out, with why.
    pub fn verify(v: &Value) -> (Known, Vec<String>) {
        let mut k = Known::default();
        let mut why = vec![];
        for p in v["persons"].as_array().into_iter().flatten() {
            let id = p["person"].as_str().unwrap_or("");
            match PersonChain::from_json(&p["chain"]) {
                Ok(c) if c.id() == Some(id) => {
                    k.persons.0.insert(id.to_string(), c);
                }
                Ok(_) => why.push(format!("the person chain sent for {id} is another person's")),
                Err(e) => why.push(format!("the person chain of {id} doesn't check: {}", e.message)),
            }
        }
        let devices: Arc<dyn Devices> = Arc::new(k.persons.clone());
        for n in v["networks"].as_array().into_iter().flatten() {
            let id = n["id"].as_str().unwrap_or("");
            match Roster::from_json_with(NetworkPolicy::with(devices.clone()), &n["chain"]) {
                Ok(r) if r.view_ref().is_some_and(|v| v.network == id) => {
                    k.networks.insert(id.to_string(), r);
                }
                Ok(_) => why.push(format!("the chain sent for {id} is another network's")),
                Err(e) => why.push(format!("the chain of {id} doesn't check: {}", e.message)),
            }
        }
        k.roots = v["roots"].as_array().into_iter().flatten().filter_map(RootCertificate::parse).collect();
        (k, why)
    }

    /// What's kept, verified again; empty when there's nothing (standalone).
    pub fn load(paths: &AppPaths) -> Known {
        let file = chains_file(paths);
        let stamp = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        static CACHE: Mutex<Option<(PathBuf, std::time::SystemTime, Known)>> = Mutex::new(None);
        let mut c = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let (Some(t), Some((f, at, k))) = (stamp, c.as_ref())
            && *f == file
            && *at == t
        {
            return k.clone();
        }
        let k = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).map(|v| Known::verify(&v).0).unwrap_or_default();
        if let Some(t) = stamp {
            *c = Some((file, t, k.clone()));
        }
        k
    }

    /// Keeps `v` (as `verify` took it) for the next start: only the parts that verified.
    pub fn save(&self, paths: &AppPaths) -> Result<(), String> {
        let v = json!({
            "networks": self.networks.iter().map(|(id, r)| json!({ "id": id, "chain": r.to_json() })).collect::<Vec<_>>(),
            "persons": self.persons.0.iter().map(|(id, c)| json!({ "person": id, "chain": c.to_json() })).collect::<Vec<_>>(),
            "roots": self.roots.iter().map(RootCertificate::to_json).collect::<Vec<_>>(),
        });
        let file = chains_file(paths);
        std::fs::create_dir_all(dir(paths)).map_err(|e| format!("cannot make {}: {e}", wiki::disp(&dir(paths))))?;
        wiki::write_private(&file, &format!("{v}\n"), &[]).map_err(|e| format!("cannot write {}: {e}", wiki::disp(&file)))
    }

    /// The machine at `endpoint` (64 hex digits) in each network held, by the rosters.
    pub fn members_at(&self, endpoint: &str) -> Vec<(&str, &Member)> {
        self.networks.iter().filter_map(|(id, r)| r.view_ref().and_then(|v| v.members.values().find(|m| hex(&m.keys.sign) == endpoint)).map(|m| (id.as_str(), m))).collect()
    }

    /// The networks `endpoint` is a member of.
    pub fn networks_of(&self, endpoint: &str) -> BTreeSet<String> {
        self.members_at(endpoint).into_iter().map(|(n, _)| n.to_string()).collect()
    }

    /// Every member's endpoint in `network`.
    pub fn endpoints_in(&self, network: &str) -> Vec<String> {
        self.networks.get(network).and_then(|r| r.view_ref()).map(|v| v.members.values().map(|m| hex(&m.keys.sign)).collect()).unwrap_or_default()
    }

    /// The person `network` is theirs, by its chain.
    pub fn owner_of_network(&self, network: &str) -> Option<&str> {
        self.networks.get(network)?.view_ref()?.owner.as_deref()
    }

    /// The space chains' rules, with these person chains for devices and these rosters for who
    /// owns what.
    pub fn space_policy(&self) -> SpacePolicy {
        SpacePolicy::new(Some(Arc::new(self.persons.clone())), Some(Arc::new(self.clone())))
    }
}

impl Owners for Known {
    fn owner_of(&self, entity: &EntityRef) -> Option<String> {
        let machine = match entity {
            EntityRef::Machine(m) => m,
            EntityRef::Actor { machine, .. } => machine,
            _ => return None,
        };
        self.networks.values().filter_map(|r| r.view_ref()).find(|v| v.members.values().any(|m| m.machine.id() == *machine)).and_then(|v| v.owner.clone())
    }
}

/// The space chains' rules as this Xenolith holds XenoNet's chains now: what every chain it loads
/// is checked by.
pub fn space_policy(paths: &AppPaths) -> SpacePolicy {
    Known::load(paths).space_policy()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The facts a grant from another machine is checked on: that machine's root certificate and the
/// machine key the roster binds to the connection's endpoint, and nothing else.
struct One<'a> {
    cert: &'a RootCertificate,
    key: &'a VerifyingKey,
}

impl Ground for One<'_> {
    fn own_machine(&self) -> &MachineId {
        self.cert.machine()
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
    // Revocation is the issuing machine's: its XenoNet stops forwarding for a revoked grant, which
    // ends the connections, and the copy a hello carries lasts an hour.
    fn revoked(&self, _: &[u8]) -> bool {
        false
    }
    fn token_roots(&self, key_id: u32) -> Vec<RootCertificate> {
        if self.cert.key_id() == key_id { vec![self.cert.clone()] } else { vec![] }
    }
    fn signing_key(&self, machine: &MachineId) -> Option<VerifyingKey> {
        (machine == self.cert.machine()).then(|| self.key.clone())
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
        false
    }
    fn space_level(&self, _: &xenonet_authz::SpaceId, _: &Principal) -> Option<SpaceLevel> {
        None
    }
}

/// `text`, a grant of the machine at `endpoint`, checked against `cert` and the machine key the
/// rosters bind to that endpoint.
fn check_grant(known: &Known, endpoint: &str, text: &str, cert: &RootCertificate, now: i64) -> Result<(Token, Vec<String>), String> {
    let at = known.members_at(endpoint);
    if at.is_empty() {
        return Err(format!("{endpoint} is in none of the networks this machine's XenoNet holds"));
    }
    let mut why = String::from("its grant isn't rooted at that machine's key");
    for (_, m) in &at {
        match Token::parse_with(text, ACTOR_PREFIX, &Schema::XENONET, &One { cert, key: &m.machine }, now) {
            Ok(t) => return Ok((t, at.iter().map(|(n, _)| n.to_string()).collect())),
            Err(e) => why = format!("its grant doesn't check: {}", e.message),
        }
    }
    Err(why)
}

/// A peer bound to its machine: the networks it's in, its id in each, and the person's device on
/// it, by the rosters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bound {
    pub key: String,
    pub networks: BTreeSet<String>,
    /// Its id in the first network both hold (`laptop`), for logs and note stamps.
    pub machine: String,
    pub device: Option<String>,
}

/// Binds the context key `key` a hello signed with to the machine at `endpoint` (the forwarded
/// connection's remote, XenoNet's word): only when the hello's grant `grant` is rooted, through
/// `root`, at the machine key the rosters bind to that endpoint, and names `key` (`key(id)`).
pub fn bind_hello(known: &Known, endpoint: &str, key: &str, grant: Option<&str>, root: Option<&Value>, now: i64) -> Result<Bound, String> {
    let (Some(grant), Some(root)) = (grant, root) else { return Err("it carries no grant of its machine's XenoNet".into()) };
    let cert = RootCertificate::parse(root).ok_or("its grant's root certificate isn't one")?;
    let (token, networks) = check_grant(known, endpoint, grant, &cert, now)?;
    match token.key_over(&token.bearer().machine) {
        Some(k) if k == key => {}
        Some(k) => return Err(format!("its grant names another context key ({k})")),
        None => return Err("its grant names no context key (key(id)), so nothing binds the key to its machine".into()),
    }
    if token.forward() != Some(ALPN) {
        return Err(format!("its grant doesn't forward {ALPN}"));
    }
    let at = known.members_at(endpoint);
    let first = at.first().map(|(_, m)| (*m).clone());
    Ok(Bound { key: key.to_string(), networks: networks.into_iter().collect(), machine: first.as_ref().map(|m| m.id.clone()).unwrap_or_default(), device: first.and_then(|m| m.device) })
}

/// The copy of `token` a hello carries: every action refused (`check if false`), so no door of
/// XenoNet's or Xenolith's takes it, and over in an hour (review revision 16b).
pub fn narrow_for_hello(token: &Token, now: i64) -> Result<Token, String> {
    token.attenuate(&Narrowing::Actions(vec![])).and_then(|t| t.attenuate(&Narrowing::Until(now + HELLO_GRANT_MS))).map_err(|e| e.message)
}

/// What this Xenolith's hellos over the forward carry of its grant: a copy no XenoNet door takes
/// (`check if false`), for an hour, and the certificate of the root it's under. `me` is this
/// machine's endpoint (XenoNet's). Also what the grant names, so a wrong one shows in the status.
pub fn hello_grant(known: &Known, me: &str, text: &str, now: i64) -> Result<(String, Value, Option<String>), String> {
    let mut why = "none of this machine's token roots certifies it".to_string();
    for cert in &known.roots {
        match check_grant(known, me, text, cert, now) {
            Ok((t, _)) => {
                let out = narrow_for_hello(&t, now)?.encode(ACTOR_PREFIX).map_err(|e| e.message)?;
                return Ok((out, cert.to_json(), t.key().map(String::from)));
            }
            Err(e) => why = e,
        }
    }
    Err(why)
}

/// Whether a note from `bound` may keep the person's `user` source (revision 35): its machine's
/// device is an active device of `owner`, the space's person owner, by the person chain held.
pub fn speaks_for(known: &Known, bound: &Bound, owner: &str) -> bool {
    let Some(device) = &bound.device else { return false };
    known.persons.0.get(owner).and_then(|c| c.person()).is_some_and(|p| p.active_device(device).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_grant_file_is_an_actor_grant_and_owner_only() {
        let dir = std::env::temp_dir().join(format!("xl-xn-grant-{}", crate::text::random_hex(4)));
        let env: crate::paths::Env = [("XENOLITH_HOME".to_string(), dir.to_string_lossy().into_owned())].into();
        let paths = AppPaths::from_env(&env);
        assert!(write_grant(&paths, "xenolith_grant_abc").is_err(), "Xenolith's own grants aren't XenoNet's");
        assert!(write_grant(&paths, "xenonet_actor_a b").is_err());
        write_grant(&paths, "xenonet_actor_abc-_9\n").unwrap();
        assert_eq!(read_grant(&paths).as_deref(), Some("xenonet_actor_abc-_9"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(grant_file(&paths)).unwrap().permissions().mode() & 0o077, 0);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_verified_binds_nothing() {
        let (k, why) = Known::verify(&json!({ "networks": [{ "id": "net_x", "chain": [{ "n": 0 }] }], "persons": [], "roots": [] }));
        assert!(k.networks.is_empty() && why.len() == 1, "{why:?}");
        let e = "ab".repeat(32);
        assert!(bind_hello(&k, &e, "xns1_x", None, None, 0).unwrap_err().contains("no grant"));
        assert!(bind_hello(&k, &e, "xns1_x", Some("xenonet_actor_x"), Some(&json!({})), 0).unwrap_err().contains("certificate"));
    }
}
