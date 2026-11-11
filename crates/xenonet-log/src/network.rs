//! A network's chain, the roster (docs/design/network.md, section 3;
//! docs/design/direction.md, "Domain model"): the signed chain of a
//! network's machines, their keys and their settings
//! (`network/roster-entry/v2`). Every machine keeps the chain and checks it
//! whole, and accepts only entries that extend its own head, so a gateway
//! can withhold an entry but never add or change one.
//!
//! An admin is a window host; the founder counts as one. What each entry
//! needs:
//!
//! | op | signed by |
//! |---|---|
//! | `found` | the founder, once, as entry 0, naming the owner person |
//! | `add` a machine that isn't an admin | any admin, with presence (`admit`) |
//! | `add` an admin | the founder, or two admins, each with presence (`admit`) |
//! | `window` on | the founder, or two admins, each with presence (`widen`) |
//! | `window` off, `remove` an admin | the founder, or two admins |
//! | `gateway` | the founder, or two admins, each with presence (`widen`) |
//! | `remove` a machine that isn't an admin | any admin |
//! | `remove` the founder | nobody |
//! | `settings` that narrow | any admin, or the machine itself |
//! | `settings` that widen (start to network or delegate, wake to network) | the founder, or two admins, each with presence (`widen`) |
//!
//! And one more, for leaving: a machine may sign its own `remove` (any but
//! the founder), since taking yourself out harms no one.
//!
//! **Presence.** An admin's signature counts toward an admit or a widening
//! only with a presence block from that admin's device (the device its
//! `add` entry named), for what the entry does and its digest
//! (`Entry::digest`), and only when the owner's person chain had that
//! device active at the head the entry names (`personHead`). The policy
//! asks the person chains it was given (`person::Devices`); a policy with
//! none counts no presence. Blocks are checked for what they sign, never
//! for their age: a machine catching up reads entries that were fresh when
//! made. A live receiver checks that the head an entry names isn't older
//! than the one it holds (the network crate does, as entries arrive).
//!
//! Duplicates: an entry's identity is everything it says but its time.
//! One offered where the chain already has the same at its index is
//! dropped quietly (two admins approving the same thing at once); a
//! different one there is a fork, and refused. An entry signed rightly but
//! by too few is refused as `needs-approval`: the gateway may keep it as a
//! proposal, for the founder or a second admin to sign too.

use crate::chain::{self, Chain, ChainError, Codes, Format, Policy, Signature, R};
use crate::person::{is_key_id, is_person_id, Devices};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use xenonet_id::codec::is_b64;
use xenonet_id::keys::{HostKey, PublicKeys};
use xenonet_id::sign::{Purpose, VerifyingKey};

pub const ROSTER_TAG: &str = Purpose::ROSTER.tag();
/// The longest gateway address an entry carries, in bytes.
pub const ADDRESS_MAX: usize = 300;
/// What a presence block approves to add a machine.
pub const ADMIT: &str = "admit";
/// What a presence block approves to widen what the network may do.
pub const WIDEN: &str = "widen";

/// A network's chain.
pub type Roster = Chain<NetworkPolicy>;
/// An entry of a network's chain.
pub type Entry = chain::Entry<NetworkPolicy>;

fn address_ok(address: &str) -> bool {
    !address.is_empty() && address.len() <= ADDRESS_MAX
}

/// `net_<32 hex>`, as the founder makes it.
pub fn is_network_id(id: &str) -> bool {
    id.strip_prefix("net_").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

/// A host's id: 2 to 32 lowercase letters, digits and dashes, starting
/// with a letter. The same rule as `xenonet_model::ids::is_host_id`, which
/// this crate doesn't depend on (a test in the network crate holds the two
/// together).
pub fn is_host_id(host: &str) -> bool {
    let b = host.as_bytes();
    (2..=32).contains(&b.len()) && b[0].is_ascii_lowercase() && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Found,
    Add,
    Remove,
    Window,
    Gateway,
    Settings,
}

impl Op {
    pub const ALL: [Op; 6] = [Op::Found, Op::Add, Op::Remove, Op::Window, Op::Gateway, Op::Settings];

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Found => "found",
            Op::Add => "add",
            Op::Remove => "remove",
            Op::Window => "window",
            Op::Gateway => "gateway",
            Op::Settings => "settings",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.as_str() == s)
    }
}

/// Who may start runs on a machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Start {
    /// Nobody.
    Off,
    /// The person alone.
    #[default]
    You,
    /// The person, and the network's agents and actors.
    Network,
    /// As network, and what they start runs under the person's standing
    /// grant.
    Delegate,
}

impl Start {
    pub const ALL: [Start; 4] = [Start::Off, Start::You, Start::Network, Start::Delegate];

    pub fn as_str(self) -> &'static str {
        match self {
            Start::Off => "off",
            Start::You => "you",
            Start::Network => "network",
            Start::Delegate => "delegate",
        }
    }

    pub fn parse(s: &str) -> Option<Start> {
        Start::ALL.into_iter().find(|v| v.as_str() == s)
    }

    /// How much it lets others do: from 2 up it's the network's.
    fn rank(self) -> u8 {
        match self {
            Start::Off => 0,
            Start::You => 1,
            Start::Network => 2,
            Start::Delegate => 3,
        }
    }
}

/// Who may wake a machine's sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wake {
    /// Those the session's consent allows.
    #[default]
    Consent,
    /// Any member's principal.
    Network,
}

impl Wake {
    pub const ALL: [Wake; 2] = [Wake::Consent, Wake::Network];

    pub fn as_str(self) -> &'static str {
        match self {
            Wake::Consent => "consent",
            Wake::Network => "network",
        }
    }

    pub fn parse(s: &str) -> Option<Wake> {
        Wake::ALL.into_iter().find(|v| v.as_str() == s)
    }
}

/// The weakest check of an agent a standing grant covers, by its word:
/// never an unchecked one.
pub const REQUIRES: [&str; 2] = ["process-checked", "harness-signed"];
/// What a standing grant allows, by its rule set's name: today's one is
/// the delegation judge's (routine work inside the folder the run started
/// in; `xenonet_engine::delivery::delegation`).
pub const LIMITS: [&str; 1] = ["routine-in-folder"];

/// The person's standing grant for a machine set to delegate: what it
/// covers and until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Runs may be started under it.
    pub starts: bool,
    /// It answers those runs' permission prompts, within `limits`.
    pub answers: bool,
    /// One of `REQUIRES`.
    pub requires: String,
    /// One of `LIMITS`.
    pub limits: String,
    /// Milliseconds since 1970 when it ends.
    pub until: i64,
}

impl Grant {
    /// Today's delegation: starts and answers, for process-checked agents,
    /// within the judge's limits, until `until`.
    pub fn standard(until: i64) -> Grant {
        Grant { starts: true, answers: true, requires: REQUIRES[0].into(), limits: LIMITS[0].into(), until }
    }

    fn to_json(&self) -> Value {
        json!({ "starts": self.starts, "answers": self.answers, "requires": self.requires, "limits": self.limits, "until": self.until })
    }

    fn parse(v: &Value) -> Option<Grant> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Grant {
            starts: v.get("starts")?.as_bool()?,
            answers: v.get("answers")?.as_bool()?,
            requires: text("requires").filter(|r| REQUIRES.contains(&r.as_str()))?,
            limits: text("limits").filter(|l| LIMITS.contains(&l.as_str()))?,
            until: v.get("until")?.as_i64()?,
        })
    }

    fn ok(&self) -> bool {
        REQUIRES.contains(&self.requires.as_str()) && LIMITS.contains(&self.limits.as_str())
    }
}

/// A machine's settings: who may start runs there and wake its sessions,
/// and the standing grant when it's set to delegate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Settings {
    pub start: Start,
    pub wake: Wake,
    /// Present exactly when `start` is delegate.
    pub grant: Option<Grant>,
}

impl Settings {
    pub fn to_json(&self) -> Value {
        json!({ "start": self.start.as_str(), "wake": self.wake.as_str(), "grant": self.grant.as_ref().map_or(Value::Null, Grant::to_json) })
    }

    pub fn parse(v: &Value) -> Option<Settings> {
        let start = Start::parse(v.get("start")?.as_str()?)?;
        let wake = Wake::parse(v.get("wake")?.as_str()?)?;
        let grant = match v.get("grant") {
            None | Some(Value::Null) => None,
            Some(g) => Some(Grant::parse(g)?),
        };
        let s = Settings { start, wake, grant };
        s.ok().then_some(s)
    }

    fn ok(&self) -> bool {
        (self.start == Start::Delegate) == self.grant.is_some() && self.grant.as_ref().is_none_or(Grant::ok)
    }

    /// Whether going from `self` to `to` lets the network do more: start
    /// to network or delegate, wake to network, or a new grant.
    pub fn widens_to(&self, to: &Settings) -> bool {
        let start = to.start.rank() >= 2 && to.start.rank() > self.start.rank();
        let wake = to.wake == Wake::Network && self.wake != Wake::Network;
        let grant = to.grant.is_some() && to.grant != self.grant;
        start || wake || grant
    }
}

/// The machine an entry is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRef {
    pub id: String,
    /// Its transport keys: they sign frames and entries.
    pub keys: PublicKeys,
    pub window: bool,
    /// The person's device key on it, by id; None on a machine with no way
    /// to ask the person.
    pub device: Option<String>,
    /// Its machine signing key, which roots its tokens.
    pub machine: VerifyingKey,
}

/// For a `gateway` entry: where the network's gateway is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayAt {
    pub address: String,
    pub version: u64,
}

/// What a roster entry says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub network: String,
    pub op: Op,
    pub host: HostRef,
    pub gateway: Option<GatewayAt>,
    /// For `found`: the person the network is theirs, when the founder has
    /// one.
    pub owner: Option<String>,
    /// For `settings`: the machine's settings from now.
    pub settings: Option<Settings>,
    /// The owner's person chain head the entry was made under.
    pub person_head: Option<String>,
    pub at: i64,
}

/// A machine in the network now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: String,
    pub keys: PublicKeys,
    pub window: bool,
    pub device: Option<String>,
    pub machine: VerifyingKey,
    pub settings: Settings,
    /// Who signed its settings last, and when; None while they're the
    /// defaults.
    pub settings_by: Option<(Vec<String>, i64)>,
    /// The entry that added it.
    pub since: u64,
}

/// A machine that was removed: by which entry, whom, when, and the keys it
/// had, which still sign its requests until it hears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub n: u64,
    pub by: Vec<String>,
    pub at: i64,
    pub keys: PublicKeys,
}

/// The network as the chain leaves it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct View {
    pub network: String,
    pub founder: String,
    /// The person it's theirs, as `found` named them.
    pub owner: Option<String>,
    pub members: BTreeMap<String, Member>,
    pub removed: BTreeMap<String, Removal>,
    /// The machine whose service answers the network's routes.
    pub gateway: String,
    pub gateway_address: Option<String>,
    pub gateway_version: u64,
}

impl View {
    pub fn member(&self, id: &str) -> Option<&Member> {
        self.members.get(id)
    }

    /// Whether `id` is an admin (a window host): the founder is one.
    pub fn is_window(&self, id: &str) -> bool {
        id == self.founder || self.members.get(id).is_some_and(|m| m.window)
    }

    pub fn gateway_member(&self) -> Option<&Member> {
        self.members.get(&self.gateway)
    }

    /// A removed machine's last removal, unless it was added again since.
    pub fn removal(&self, id: &str) -> Option<&Removal> {
        if self.members.contains_key(id) {
            return None;
        }
        self.removed.get(id)
    }
}

/// Signed rightly, but by too few: the entry may wait for the founder or a
/// second admin to sign it too.
pub fn needs_approval() -> ChainError {
    ChainError::needs_approval("that needs the founder, or two window hosts")
}

fn no_host(id: &str) -> ChainError {
    ChainError::new(404, "no-host", format!("{id} isn't in the network with those keys"))
}

fn ed25519(keys: &PublicKeys) -> VerifyingKey {
    VerifyingKey::Ed25519(keys.sign)
}

/// The roster's rules, with the person chains it checks admins' presence
/// against.
#[derive(Clone, Default)]
pub struct NetworkPolicy {
    devices: Option<Arc<dyn Devices>>,
}

impl fmt::Debug for NetworkPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "NetworkPolicy({})", if self.devices.is_some() { "with person chains" } else { "with no person chain" })
    }
}

impl NetworkPolicy {
    /// The rules, with `devices` to check presence against.
    pub fn with(devices: Arc<dyn Devices>) -> NetworkPolicy {
        NetworkPolicy { devices: Some(devices) }
    }

    /// Why `s`'s presence doesn't count for `what` on `e`, or None when it
    /// does: a block from the device `s`'s machine named, signed for this
    /// entry, by a device the owner held at the head the entry names.
    fn presence_fault(&self, view: &View, e: &Entry, s: &Signature, what: &str) -> Option<String> {
        let block = s.presence.as_ref()?;
        let fault = |m: &str| Some(format!("{}'s presence {m}", s.by));
        let Some(device) = view.member(&s.by).and_then(|m| m.device.as_deref()) else { return fault("can't count: its machine has no device") };
        if block.device != device {
            return fault("is from another device than its machine's");
        }
        let (Some(owner), Some(head)) = (view.owner.as_deref(), e.person_head.as_deref()) else {
            return fault("can't count: the network has no owner, or the entry names no person head");
        };
        let Some(devices) = &self.devices else { return fault("can't be checked here: no person chain") };
        let Some(d) = devices.person_device(owner, head, device) else {
            return Some(format!("{}'s device wasn't the owner's, active, at the person head the entry names", s.by));
        };
        block.signed_for(&d.key, what, &e.digest()).err().map(|err| format!("{}: {}", s.by, err.message))
    }

    /// The founder's and the admins' signatures that count for `e`: with a
    /// presence for `what` when one is needed. The first fault seen, for a
    /// refusal that says why a signature didn't count.
    fn count(&self, view: &View, e: &Entry, what: Option<&str>) -> (bool, usize, Option<String>) {
        let (mut founder, mut admins, mut fault) = (false, 0, None);
        for s in &e.signers {
            if !view.is_window(&s.by) {
                continue;
            }
            if let Some(what) = what {
                let why = if s.presence.is_none() { Some(format!("{} signed without the person's presence", s.by)) } else { self.presence_fault(view, e, s, what) };
                if let Some(why) = why {
                    fault.get_or_insert(why);
                    continue;
                }
            }
            founder |= s.by == view.founder;
            admins += 1;
        }
        (founder, admins, fault)
    }

    /// The founder or two admins, each with presence for `what` if given:
    /// else needs-approval, or the presence fault that kept enough from
    /// counting.
    fn strong(&self, view: &View, e: &Entry, what: Option<&str>) -> R<()> {
        let (founder, admins, fault) = self.count(view, e, what);
        if founder || admins >= 2 {
            return Ok(());
        }
        let (all_founder, all, _) = self.count(view, e, None);
        match fault {
            Some(why) if all_founder || all >= 2 => Err(presence_refused(why)),
            _ => Err(needs_approval()),
        }
    }

    /// Any admin, with presence for `what` if given.
    fn any(&self, view: &View, e: &Entry, what: Option<&str>, refused: &str) -> R<()> {
        let (founder, admins, fault) = self.count(view, e, what);
        if founder || admins >= 1 {
            return Ok(());
        }
        match fault {
            Some(why) => Err(presence_refused(why)),
            None => Err(NetworkPolicy::FORMAT.refused(refused)),
        }
    }
}

fn presence_refused(why: String) -> ChainError {
    ChainError::new(403, "presence", format!("that needs the person there, at an admin's device: {why}"))
}

fn host_json(h: &HostRef) -> Value {
    json!({
        "id": h.id,
        "sign": h.keys.sign_b64(),
        "seal": h.keys.seal_b64(),
        "keyId": h.keys.key_id(),
        "window": h.window,
        "device": h.device,
        "machine": h.machine.to_json(),
    })
}

impl Policy for NetworkPolicy {
    type Body = Body;
    type State = View;

    const FORMAT: Format = Format {
        tag: Purpose::ROSTER,
        noun: "roster",
        signer: "host",
        signer_key: "host",
        presence: true,
        codes: Codes { gap: "roster-gap", fork: "roster-fork", refused: "roster-refused" },
    };

    fn write_body(b: &Body, out: &mut Map<String, Value>) {
        out.insert("network".into(), Value::String(b.network.clone()));
        out.insert("op".into(), Value::String(b.op.as_str().into()));
        out.insert("host".into(), host_json(&b.host));
        out.insert("gateway".into(), b.gateway.as_ref().map_or(Value::Null, |g| json!({ "address": g.address, "version": g.version })));
        out.insert("owner".into(), b.owner.as_ref().map_or(Value::Null, |o| Value::String(o.clone())));
        out.insert("settings".into(), b.settings.as_ref().map_or(Value::Null, Settings::to_json));
        out.insert("personHead".into(), b.person_head.as_ref().map_or(Value::Null, |h| Value::String(h.clone())));
        out.insert("at".into(), Value::from(b.at));
    }

    fn parse_body(v: &Value) -> R<Body> {
        let bad = |what: &str| Self::FORMAT.bad(what);
        let network = v.get("network").and_then(Value::as_str).filter(|n| is_network_id(n)).ok_or_else(|| bad("network"))?;
        let op = v.get("op").and_then(Value::as_str).and_then(Op::parse).ok_or_else(|| bad("op"))?;
        let h = v.get("host").ok_or_else(|| bad("host"))?;
        let id = h.get("id").and_then(Value::as_str).filter(|i| is_host_id(i)).ok_or_else(|| bad("host id"))?;
        let keys = PublicKeys::from_b64(h.get("sign").and_then(Value::as_str).unwrap_or(""), h.get("seal").and_then(Value::as_str).unwrap_or("")).ok_or_else(|| bad("keys"))?;
        if h.get("keyId").and_then(Value::as_str) != Some(keys.key_id().as_str()) {
            return Err(bad("key id"));
        }
        let window = h.get("window").and_then(Value::as_bool).ok_or_else(|| bad("window"))?;
        let device = match h.get("device") {
            None | Some(Value::Null) => None,
            Some(d) => Some(d.as_str().filter(|d| is_key_id(d)).ok_or_else(|| bad("device"))?.to_string()),
        };
        let machine = h.get("machine").and_then(VerifyingKey::parse).ok_or_else(|| bad("machine key"))?;
        let gateway = match v.get("gateway") {
            None | Some(Value::Null) => None,
            Some(g) => Some(GatewayAt {
                address: g.get("address").and_then(Value::as_str).filter(|a| address_ok(a)).ok_or_else(|| bad("gateway address"))?.to_string(),
                version: g.get("version").and_then(Value::as_u64).ok_or_else(|| bad("gateway version"))?,
            }),
        };
        let owner = match v.get("owner") {
            None | Some(Value::Null) => None,
            Some(o) => Some(o.as_str().filter(|o| is_person_id(o)).ok_or_else(|| bad("owner"))?.to_string()),
        };
        let settings = match v.get("settings") {
            None | Some(Value::Null) => None,
            Some(s) => Some(Settings::parse(s).ok_or_else(|| bad("settings"))?),
        };
        let person_head = match v.get("personHead") {
            None | Some(Value::Null) => None,
            Some(p) => Some(p.as_str().filter(|p| is_b64(p) && !p.is_empty()).ok_or_else(|| bad("person head"))?.to_string()),
        };
        let at = v.get("at").and_then(Value::as_i64).ok_or_else(|| bad("time"))?;
        let body = Body { network: network.into(), op, host: HostRef { id: id.into(), keys, window, device, machine }, gateway, owner, settings, person_head, at };
        Self::body_well_formed(&body)?;
        Ok(body)
    }

    fn body_well_formed(b: &Body) -> R<()> {
        let bad = |what: &str| Self::FORMAT.bad(what);
        if !is_network_id(&b.network) {
            return Err(bad("network"));
        }
        if !is_host_id(&b.host.id) {
            return Err(bad("host id"));
        }
        if b.host.device.as_deref().is_some_and(|d| !is_key_id(d)) {
            return Err(bad("device"));
        }
        if b.gateway.as_ref().is_some_and(|g| !address_ok(&g.address)) {
            return Err(bad("gateway address"));
        }
        if b.owner.as_deref().is_some_and(|o| !is_person_id(o)) || (b.owner.is_some() && b.op != Op::Found) {
            return Err(bad("owner"));
        }
        if b.settings.is_some() != (b.op == Op::Settings) || b.settings.as_ref().is_some_and(|s| !s.ok()) {
            return Err(bad("settings"));
        }
        if b.person_head.as_deref().is_some_and(|p| p.is_empty() || !is_b64(p)) {
            return Err(bad("person head"));
        }
        Ok(())
    }

    fn is_signer(id: &str) -> bool {
        is_host_id(id)
    }

    fn sig_ok(sig: &[u8]) -> bool {
        sig.len() == 64
    }

    fn same(a: &Body, b: &Body) -> bool {
        a.op == b.op && a.host == b.host && a.gateway == b.gateway && a.network == b.network && a.owner == b.owner && a.settings == b.settings && a.person_head == b.person_head
    }

    fn scope(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(view) = view else {
            // Entry 0: the founder's, signed by itself.
            if e.op != Op::Found {
                return Err(Self::FORMAT.refused("a roster starts with its founder"));
            }
            if e.signers.len() != 1 || e.signers[0].by != e.host.id {
                return Err(founder_alone());
            }
            return Ok(());
        };
        if e.network != view.network {
            return Err(Self::FORMAT.refused("that entry is for another network"));
        }
        Ok(())
    }

    fn signer_key(&self, view: Option<&View>, e: &Entry, by: &str) -> R<VerifyingKey> {
        match view {
            None => Ok(ed25519(&e.host.keys)),
            Some(view) => view.member(by).map(|m| ed25519(&m.keys)).ok_or_else(|| Self::FORMAT.refused(format!("{by} isn't in the network"))),
        }
    }

    fn bad_signature(&self, view: Option<&View>, by: &str) -> ChainError {
        match view {
            None => founder_alone(),
            Some(_) => Self::FORMAT.refused(format!("{by}'s signature doesn't check")),
        }
    }

    fn check(&self, view: Option<&View>, e: &Entry) -> R<()> {
        let Some(view) = view else { return Ok(()) };
        let refused = |m: String| Self::FORMAT.refused(m);
        let current = view.member(&e.host.id);
        // An entry about a member names it as the chain has it.
        let same_host = |m: &Member| m.keys == e.host.keys && m.device == e.host.device && m.machine == e.host.machine;
        if e.gateway.is_some() && e.op != Op::Gateway {
            return Err(ChainError::bad("only a gateway entry names a gateway"));
        }
        match e.op {
            Op::Found => Err(refused("a network has one founder".into())),
            Op::Add => {
                if current.is_some() {
                    return Err(ChainError::taken(format!("{} is in the network already", e.host.id)));
                }
                if view.members.values().any(|m| m.keys == e.host.keys) {
                    return Err(ChainError::taken("those keys are another host's"));
                }
                if e.host.window {
                    self.strong(view, e, Some(ADMIT))
                } else {
                    self.any(view, e, Some(ADMIT), "adding a host needs a window host")
                }
            }
            Op::Remove => {
                let Some(m) = current.filter(|m| same_host(m)) else { return Err(no_host(&e.host.id)) };
                if m.id == view.founder {
                    return Err(refused("the founder can't be removed: a network whose founder is lost is made again".into()));
                }
                // Removed, the gateway's host would be cut off from a network
                // still pinned to it; and every host would lose its gateway.
                if m.id == view.gateway {
                    return Err(refused(format!("{} is the network's gateway: move the gateway first", m.id)));
                }
                if e.signers.len() == 1 && e.signers[0].by == m.id {
                    return Ok(());
                }
                if view.is_window(&m.id) {
                    self.strong(view, e, None)
                } else {
                    self.any(view, e, None, "removing a host needs a window host")
                }
            }
            Op::Window => {
                let Some(m) = current.filter(|m| same_host(m)) else { return Err(no_host(&e.host.id)) };
                if m.window == e.host.window {
                    return Err(ChainError::no_change("that's the window choice it has"));
                }
                self.strong(view, e, e.host.window.then_some(WIDEN))
            }
            Op::Gateway => {
                if current.is_none_or(|m| !same_host(m)) {
                    return Err(no_host(&e.host.id));
                }
                let Some(g) = &e.gateway else { return Err(ChainError::bad("a gateway entry names where the gateway is")) };
                if g.version <= view.gateway_version {
                    return Err(ChainError::stale("a gateway entry needs a higher version than the last"));
                }
                self.strong(view, e, Some(WIDEN))
            }
            Op::Settings => {
                let Some(m) = current.filter(|m| same_host(m)) else { return Err(no_host(&e.host.id)) };
                let Some(to) = &e.settings else { return Err(ChainError::bad("a settings entry names the settings")) };
                if *to == m.settings {
                    return Err(ChainError::no_change("those are the settings it has"));
                }
                if m.settings.widens_to(to) {
                    return self.strong(view, e, Some(WIDEN));
                }
                // Narrowing: any admin, or the machine itself.
                if e.signers.iter().any(|s| s.by == m.id) {
                    return Ok(());
                }
                self.any(view, e, None, "changing a machine's settings needs a window host, or the machine itself")
            }
        }
    }

    fn apply(&self, view: Option<View>, e: &Entry) -> View {
        let mut v = view.unwrap_or_default();
        let member = |e: &Entry, window: bool| Member {
            id: e.host.id.clone(),
            keys: e.host.keys,
            window,
            device: e.host.device.clone(),
            machine: e.host.machine.clone(),
            settings: Settings::default(),
            settings_by: None,
            since: e.n,
        };
        match e.op {
            Op::Found => {
                v.network.clone_from(&e.network);
                v.founder.clone_from(&e.host.id);
                v.owner.clone_from(&e.owner);
                v.gateway.clone_from(&e.host.id);
                v.gateway_address = e.gateway.as_ref().map(|g| g.address.clone());
                v.gateway_version = e.gateway.as_ref().map_or(0, |g| g.version);
                v.members.insert(e.host.id.clone(), member(e, true));
            }
            Op::Add => {
                v.removed.remove(&e.host.id);
                v.members.insert(e.host.id.clone(), member(e, e.host.window));
            }
            Op::Remove => {
                v.members.remove(&e.host.id);
                v.removed.insert(e.host.id.clone(), Removal { n: e.n, by: e.signers.iter().map(|s| s.by.clone()).collect(), at: e.at, keys: e.host.keys });
            }
            Op::Window => {
                if let Some(m) = v.members.get_mut(&e.host.id) {
                    m.window = e.host.window;
                }
            }
            Op::Gateway => {
                v.gateway.clone_from(&e.host.id);
                if let Some(g) = &e.gateway {
                    v.gateway_address = Some(g.address.clone());
                    v.gateway_version = g.version;
                }
            }
            Op::Settings => {
                if let (Some(m), Some(s)) = (v.members.get_mut(&e.host.id), &e.settings) {
                    m.settings = s.clone();
                    m.settings_by = Some((e.signers.iter().map(|s| s.by.clone()).collect(), e.at));
                }
            }
        }
        v
    }
}

fn founder_alone() -> ChainError {
    NetworkPolicy::FORMAT.refused("the founder signs the first entry, alone")
}

impl Entry {
    /// Adds `host`'s signature, with the presence block it brings.
    pub fn sign_by(&mut self, host: &str, key: &HostKey) {
        self.sign_with(host, key, None);
    }

    /// Adds `host`'s signature and its device's presence block.
    pub fn sign_with(&mut self, host: &str, key: &HostKey, presence: Option<xenonet_id::presence::PresenceBlock>) {
        let sig = key.sign(&self.signing_bytes());
        self.add_signature(host, sig, presence);
    }

    /// What a presence block for this entry approves, when its signers
    /// need one: `admit` for an add, `widen` for a widening (as `view`,
    /// the chain before it, has the machine's settings).
    pub fn presence_for(&self, view: &View) -> Option<&'static str> {
        match self.op {
            Op::Add => Some(ADMIT),
            Op::Window if self.host.window => Some(WIDEN),
            Op::Gateway => Some(WIDEN),
            Op::Settings => {
                let had = view.member(&self.host.id).map(|m| m.settings.clone()).unwrap_or_default();
                self.settings.as_ref().filter(|to| had.widens_to(to)).map(|_| WIDEN)
            }
            _ => None,
        }
    }
}

impl Roster {
    /// The next entry, unsigned: its index, `prev` and network filled in,
    /// with no owner, settings or person head.
    pub fn draft(&self, op: Op, host: HostRef, gateway: Option<GatewayAt>, at: i64) -> Entry {
        self.next(Body { network: self.view().network, op, host, gateway, owner: None, settings: None, person_head: None, at })
    }

    /// The `HostRef` the chain has for member `id`, for an entry about it.
    pub fn host_ref(&self, id: &str) -> Option<HostRef> {
        let m = self.view_ref()?.member(id)?.clone();
        Some(HostRef { id: m.id, keys: m.keys, window: m.window, device: m.device, machine: m.machine })
    }
}

/// Entry 0 of a new network, signed by its founder: `owner` is the person
/// it's theirs, at their chain's head `person_head`.
pub fn found(network: &str, host: HostRef, key: &HostKey, owner: Option<String>, person_head: Option<String>, gateway: Option<GatewayAt>, at: i64) -> Entry {
    let id = host.id.clone();
    let mut e = Entry::new(0, "", Body { network: network.into(), op: Op::Found, host: HostRef { window: true, ..host }, gateway, owner, settings: None, person_head, at });
    e.sign_by(&id, key);
    e
}

#[cfg(test)]
mod tests;
