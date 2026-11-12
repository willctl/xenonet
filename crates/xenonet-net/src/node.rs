//! This machine in one network (docs/design/network.md, sections 3, 4 and
//! 9): its key, its copy of the network's chain, and what it sends and
//! takes on link/2 (`node_link.rs`), machine to machine. It's what the
//! engine sees through `xenonet_model::ops::Net`, and it calls the
//! engine's `Sink` with what arrives, after checking each entry's
//! signature against its own chain.
//!
//! **Its feed.** What the engine sends as messages, receipts, handoffs and
//! the person's acts are entries of this machine's feed in the network
//! (xenonet-log's `feed`), signed by its machine signing key and carried
//! in envelopes sealed for each recipient: a message and a receipt loose,
//! to the host they're for, and a handoff kept, to every member, in one
//! gapless sequence. Where it writes, and the kept entries it wrote, are
//! kept through `Persist`; each member acks the kept entries it applied,
//! and what a member hasn't acked goes to it again on its next connect, or
//! when it asks with a Want on a gap. An entry from another member is
//! taken only when its author's key is that member's in the chain, and
//! goes to the engine's `Feeds`, which applies it once.
//!
//! Nothing here waits on another machine: `send` hands envelopes to the
//! connections that are up, the network's mailboxes, or the queue of what
//! waits for a member that's away, and returns.
//!
//! **The chain** goes machine to machine too (`node_chain.rs`): what this
//! machine appends, and what a member sends it, goes on to the others.

use crate::files::{Dir, NetFile, KEY};
use crate::ground::Facts;
use crate::identity::{self, Identity};
use crate::keys::{HostKey, PublicKeys};
use crate::link2;
use crate::persist::{Persist, WriterAt};
use crate::roster::{Entry, HostRef, NetworkPolicy, Offered, Op, Removal, Roster};
use crate::sink::{entry_kind, LogFn, SinkSlot};
use crate::svc::{self, Services};
use crate::svc2;
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, Weak};
use tokio::sync::Notify;
use xenonet_id::vault::Vault;
use xenonet_log::feed::{Class, FeedEntry, Pos, Writer};
use xenonet_model::ids::is_host_id;
use xenonet_model::ops::{Host, Incoming, Kind, Me, Net, Outgoing, Sink, To};
use xenonet_os::sync::{Locked, ReadWrite};

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Clone)]
pub struct Options {
    /// `<state>/network`.
    pub dir: PathBuf,
    pub version: String,
    /// darwin, linux or win32.
    pub platform: String,
    pub desktop: bool,
    pub clock: Clock,
    pub log: LogFn,
    /// How long a request to another host's service waits for its answer.
    pub svc_ms: u64,
    /// This machine's keys: the machine signing key, the person's device
    /// key, sealing the person's root. None: no device, so nothing is done
    /// as the person (a test's node, a machine with no way to ask).
    pub vault: Option<Arc<dyn Vault>>,
    /// Where the network and the person are kept (the store, through the
    /// runtime).
    pub persist: Arc<dyn Persist>,
    /// Where the network's endpoints bind, join/2's and link/2's: None for
    /// every interface (the tests give loopback).
    pub bind: Option<std::net::SocketAddr>,
    /// Finding members on the LAN by mDNS, as the machine's setting says.
    pub lan: crate::endpoint::LanLookup,
    /// The actors' forwards (`forward`): link/2's endpoint admits the
    /// members of this machine's networks on their ALPNs too, and hands
    /// those connections to them. None: nothing is forwarded.
    pub forwards: Option<Arc<crate::forward::Forwards>>,
}

impl Options {
    pub fn new(dir: impl Into<PathBuf>, persist: Arc<dyn Persist>, version: &str, platform: &str, log: LogFn) -> Options {
        Options {
            persist,
            dir: dir.into(),
            version: version.into(),
            platform: platform.into(),
            desktop: false,
            clock: Arc::new(now_ms),
            log,
            svc_ms: svc::TIMEOUT_MS,
            vault: None,
            bind: None,
            lan: crate::endpoint::LanLookup::Unasked,
            forwards: None,
        }
    }
}

pub use xenonet_text::time::now_ms;

/// How this machine stands in its network.
#[derive(Debug, Clone)]
pub struct Status {
    /// off, live or removed: live while it's in the network and its node
    /// runs, whichever members it reaches (each member's row says whether
    /// it's online, while its connection is up).
    pub state: &'static str,
    pub since: Option<f64>,
    pub reason: Option<String>,
    pub removed: Option<Value>,
}

impl Default for Status {
    fn default() -> Self {
        Status { state: "off", since: None, reason: None, removed: None }
    }
}

/// A change to this machine's settings, made at its own keyboard: each
/// setting that's Some is set.
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub titles_only: Option<bool>,
    pub share_load: Option<bool>,
    /// Who may start sessions here: off, you, network or delegate. A
    /// settings entry in the chain.
    pub start: Option<String>,
    /// Who may wake them: consent or network.
    pub wake: Option<String>,
}

/// Another host, as presence and its connection tell of it.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub online: bool,
    pub seen: Option<f64>,
    pub counter: u64,
    /// Its agents, and whether each waits on the person.
    pub agents: BTreeMap<String, bool>,
    pub you: bool,
    /// When the person last worked at its console, by this host's clock.
    pub you_at: Option<f64>,
    pub roster: Option<String>,
    pub platform: Option<String>,
    /// What it takes beyond messages (`xenonet_model::ops::CAN`).
    pub can: Vec<String>,
}

pub struct Shared {
    /// This node itself, for the tasks a frame starts.
    this: Weak<Shared>,
    opts: Options,
    pub dir: Dir,
    key: RwLock<Option<Arc<HostKey>>>,
    net: RwLock<Option<NetFile>>,
    roster: RwLock<Roster>,
    /// This machine's feed in its network, as its author.
    writer: Mutex<Option<Writer>>,
    sink: Arc<SinkSlot>,
    status: Mutex<Status>,
    peers: Mutex<BTreeMap<String, Peer>>,
    /// The services this host published, and the requests it waits on.
    svc: Services,
    /// The person on this machine, and the people's chains it holds.
    identity: Arc<Identity>,
    link_written: Mutex<f64>,
    /// Entries signed by too few, waiting for this host or another to approve.
    proposals: Mutex<Vec<Entry>>,
    /// What this host's own presence last said of the person: whether
    /// they're here, and when they last worked at its console.
    me_you: Mutex<(bool, Option<f64>)>,
    /// This host's own agents as its presence lists them, and whether each
    /// waits on the person, for its row: `peers` has only the others.
    me_agents: Mutex<BTreeMap<String, bool>>,
    /// What this machine's networks share: its person, link/2, its HLC,
    /// and the list of them.
    pub(crate) common: Arc<Common>,
    /// Chain records heard from members on link/2, counted, and the wake
    /// for a machine bringing its chain up to date before it drafts.
    chain_heard: Mutex<u64>,
    chain_wake: Notify,
}

/// What every network this machine is in shares: the person here, one
/// link/2 endpoint (one connection per peer, whatever networks it
/// carries), the entries taken lately, the HLC, and the networks
/// themselves, in the order this machine founded or joined them.
pub struct Common {
    pub identity: Arc<Identity>,
    /// link/2, while it runs.
    pub(crate) link2: RwLock<Option<Arc<link2n::Transport2>>>,
    pub(crate) clocks: Mutex<link2n::Clocks>,
    /// The networks, first founded or joined first.
    nets: RwLock<Vec<Weak<Shared>>>,
    /// When this machine last exchanged chain heads with a member of each
    /// network (a hello on link/2), by its clock.
    pub(crate) heads_at: Mutex<HashMap<String, f64>>,
    /// A network's relays or members changed: `Nodes` looks at the
    /// endpoint's relays again.
    pub(crate) relays_due: Notify,
    /// The endpoint is started again at the next `relays_due`: its key or
    /// its LAN setting changed, which iroh takes only at bind.
    pub(crate) restart: std::sync::atomic::AtomicBool,
    /// The sessions each peer was shown on link/2.
    pub(crate) shown: Mutex<HashMap<crate::dial::PeerId, std::collections::BTreeSet<String>>>,
    /// The window's key for reading other machines' transcripts, made once
    /// a launch and certified by the person's device (`svc2`).
    pub(crate) window: Mutex<Option<Arc<svc2::WindowKey>>>,
    /// The window reads this machine answered lately, by nonce.
    pub(crate) nonces: Mutex<svc2::Nonces>,
    /// This machine's own API, as window reads read it: the composition
    /// root's.
    pub(crate) reads: RwLock<Option<Arc<dyn svc2::LocalReads>>>,
    /// Address records a join brought, by network, until link/2 takes them.
    pub(crate) joined: Mutex<Vec<(String, Vec<Value>)>>,
    /// The latest load each member sent (`load`), by network and host id.
    pub(crate) loads: Mutex<HashMap<(String, String), Value>>,
}

impl Common {
    pub fn new(identity: Arc<Identity>) -> Arc<Common> {
        Arc::new(Common {
            identity,
            link2: RwLock::new(None),
            clocks: Mutex::new(link2n::Clocks::default()),
            nets: RwLock::new(Vec::new()),
            heads_at: Mutex::new(HashMap::new()),
            relays_due: Notify::new(),
            restart: std::sync::atomic::AtomicBool::new(false),
            shown: Mutex::new(HashMap::new()),
            window: Mutex::new(None),
            nonces: Mutex::new(svc2::Nonces::default()),
            reads: RwLock::new(None),
            joined: Mutex::new(Vec::new()),
            loads: Mutex::new(HashMap::new()),
        })
    }

    /// Where window reads from other machines read this machine's API.
    pub fn set_reads(&self, reads: Arc<dyn svc2::LocalReads>) {
        *self.reads.writing() = Some(reads);
    }

    /// The networks this machine is in, in order: each one's node.
    pub fn shareds(&self) -> Vec<Arc<Shared>> {
        self.nets.reading().iter().filter_map(Weak::upgrade).filter(|s| s.network().is_some()).collect()
    }

    /// The node of `network`, when this machine is in it.
    pub fn shared_of(&self, network: &str) -> Option<Arc<Shared>> {
        self.shareds().into_iter().find(|s| s.network().is_some_and(|n| n.id == network))
    }

    pub(crate) fn set_nets(&self, nets: Vec<Weak<Shared>>) {
        *self.nets.writing() = nets;
    }
}

/// How many kept entries are read at a time to go to a member again.
const RESEND: usize = 500;
/// Entries waiting for approval, at most.
pub const MAX_PROPOSALS: usize = 16;

/// How long after its chain was last synced with a member a network still
/// counts as in contact (question 3, revision 21).
pub const IN_CONTACT_MS: i64 = 10 * 60 * 1000;

/// Where a writer stands, as `Persist` keeps it.
fn writer_at(w: &Writer) -> WriterAt {
    WriterAt { inc: w.inc(), kept: w.kept(), loose: w.loose(), link: w.link(), head: w.head() }
}

/// A presence's agents folded into what a host had: all of them on a full
/// one, else those that changed and those it removed.
fn fold_agents(agents: &mut BTreeMap<String, bool>, body: &Value) {
    if body.get("full").and_then(Value::as_bool) == Some(true) {
        agents.clear();
    }
    for a in body.get("agents").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = a.get("id").and_then(Value::as_str) {
            agents.insert(id.to_string(), a.get("waitsOnYou").and_then(Value::as_bool).unwrap_or(false));
        }
    }
    for r in body.get("removed").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = r.as_str() {
            agents.remove(id);
        }
    }
}

/// The reserved target a person's chain entries travel to, as message
/// entries between machines: taken by the node, never by the engine.
pub const PERSON_TARGET: &str = "xenonet:person";

/// The reserved target the person's acts at another machine travel to
/// (`network/operator/v2`, an answer to its ask), as message entries:
/// taken by the node, which hands the act to the engine as one.
pub const OPERATOR_TARGET: &str = "xenonet:operator";

/// A message entry's body to `xenonet:person` carrying `text`.
fn person_body(text: &str) -> Value {
    json!({ "target": PERSON_TARGET, "body": text })
}

fn not_in() -> NetError {
    NetError::new(409, "no-network", "this machine isn't in a XenoNet network")
}

impl Shared {
    pub fn now(&self) -> f64 {
        (self.opts.clock)()
    }

    pub fn log(&self, code: &str, words: &str) {
        (self.opts.log)(code, words);
    }

    pub fn network(&self) -> Option<NetFile> {
        self.net.reading().clone()
    }

    /// The network with id `network`, when this machine is in it: what
    /// names a network is checked against the ones it is in, and never
    /// taken for another.
    pub fn in_network(&self, network: &str) -> Option<NetFile> {
        self.network().filter(|n| n.id == network)
    }

    /// What the person shares into `network`: what they said, or
    /// everything. A network joined or founded after the first is given
    /// "nothing" when it's adopted, so no row means the first.
    pub fn sharing(&self, network: &str) -> crate::share::Sharing {
        self.opts.persist.sharing(network).ok().flatten().unwrap_or(crate::share::Sharing::All)
    }

    /// What each network `host` (a member of this one) is in with this
    /// machine shares: what it may be shown, the union of them. A machine
    /// is the same machine across networks by its signing key, whatever
    /// its host id in each.
    pub fn sharings_with(&self, host: &str) -> Vec<crate::share::Sharing> {
        let Some(machine) = self.roster().view_ref().and_then(|v| v.member(host).map(|m| m.machine.id())) else { return Vec::new() };
        self.common
            .shareds()
            .iter()
            .filter(|s| !s.is_removed())
            .filter_map(|s| {
                let net = s.network()?;
                s.roster().view_ref()?.by_machine(&machine)?;
                Some(s.sharing(&net.id))
            })
            .collect()
    }

    pub fn key(&self) -> Option<Arc<HostKey>> {
        self.key.reading().clone()
    }

    /// The machine's transport key after a rekey: every network's node
    /// holds the one key, as the endpoint does.
    pub(crate) fn set_key(&self, key: Arc<HostKey>) {
        *self.key.writing() = Some(key);
    }

    pub fn roster(&self) -> RwLockReadGuard<'_, Roster> {
        self.roster.reading()
    }

    /// The person on this machine.
    pub fn identity(&self) -> &Arc<Identity> {
        &self.identity
    }

    /// When governance (this network's chain) was last synced from another
    /// member, which "in contact" is decided by, for this network alone:
    /// now while link/2 holds a connection to a member of it, else the
    /// last hello (chain heads exchanged).
    pub fn synced_at(&self) -> Option<i64> {
        let net = self.network()?;
        if self.link2().is_some_and(|t| t.links.peers().values().any(|p| p.networks.contains(&net.id))) {
            return Some(self.now() as i64);
        }
        self.common.heads_at.locked().get(&net.id).map(|t| *t as i64)
    }

    /// The facts a decision here rests on, as of now.
    pub fn facts(&self) -> Facts {
        let net = self.network();
        let machine = net.as_ref().map_or(crate::ground::LOCAL.to_string(), |n| n.host.clone());
        let machine = xenonet_authz::MachineId::new(&machine).unwrap_or_else(|_| xenonet_authz::MachineId::new(crate::ground::LOCAL).expect("a word"));
        Facts { machine, view: net.and_then(|_| self.roster().view_ref().cloned()), identity: self.identity.clone(), synced_at: self.synced_at(), now: self.now() as i64 }
    }

    /// The network's owner's chain, as roster frames, answers and joins
    /// carry it beside the entries it checks: `{ person, entries }`, or
    /// null when the network names no owner or this machine holds none.
    pub fn person_json(&self) -> Value {
        let owner = self.roster().view_ref().and_then(|v| v.owner.clone()).or_else(|| self.identity.person_id());
        let Some(owner) = owner else { return Value::Null };
        let entries = self.identity.since(&owner, 0);
        if entries.is_empty() {
            return Value::Null;
        }
        json!({ "person": owner, "entries": entries })
    }

    /// A person's chain from another machine (`person_json`'s shape),
    /// offered before the entries it checks. True when it brought
    /// something new.
    pub fn offer_person(&self, v: &Value) -> bool {
        let (Some(person), Some(entries)) = (v.get("person").and_then(Value::as_str), v.get("entries").and_then(Value::as_array)) else { return false };
        match self.identity.offer(person, entries) {
            Ok(new) => new,
            Err(e) => {
                self.log("net.person", &format!("a person's chain from the network wasn't taken: {}", e.message));
                false
            }
        }
    }

    /// Refuses an entry made under a person head older than the one this
    /// machine holds: a live receiver's check. A chain catching up checks
    /// entries by what they sign, never this.
    pub fn fresh_person_head(&self, e: &Entry) -> R<()> {
        let owner = self.roster().view_ref().and_then(|v| v.owner.clone()).or_else(|| e.owner.clone());
        let (Some(owner), Some(head)) = (owner, e.person_head.as_deref()) else { return Ok(()) };
        match self.identity.where_is(&owner, head) {
            (Some(now), Some(at)) if at < now => {
                Err(NetError::new(409, "stale-person-head", "that entry was made under an older person chain than this machine holds: make it again"))
            }
            _ => Ok(()),
        }
    }

    /// The next roster entry, unsigned, made under the owner's person head
    /// as this machine holds it.
    pub fn draft(&self, op: Op, host: HostRef) -> Entry {
        let r = self.roster();
        let mut e = r.draft(op, host, None, self.now() as i64);
        e.person_head = r.view_ref().and_then(|v| v.owner.as_deref()).and_then(|o| self.identity.chain_of(o)).and_then(|c| c.head());
        e
    }

    /// Pushes entries of this machine's person's chain to the network's
    /// other machines, as `message` frames to `xenonet:person`.
    pub(crate) fn push_person(&self, from: u64) {
        let Some(person) = self.identity.person_id() else { return };
        let entries = self.identity.since(&person, from);
        if entries.is_empty() || self.network().is_none() {
            return;
        }
        let text = json!({ "op": "entries", "person": person, "entries": entries }).to_string();
        if let Err(why) = self.send(Outgoing::new(To::All, Kind::Message, person_body(&text))) {
            self.log("net.person", &format!("this person's new entries didn't go to the network: {why}"));
        }
    }

    /// Asks `host` for `person`'s chain past where this machine's copy
    /// stands: a newer head came from it.
    pub fn want_person(&self, host: &str, person: &str) {
        let from = self.identity.chain_of(person).map_or(0, |c| c.len());
        let text = json!({ "op": "want", "person": person, "from": from }).to_string();
        let _ = self.send(Outgoing::new(To::Host(host.into()), Kind::Message, person_body(&text)));
    }

    /// Who may start and wake sessions on `host`, as the chain's settings
    /// for it say, with the settings entries for it that wait for a second
    /// admin (as presence and the state list them).
    pub fn reach_of(&self, host: &str) -> xenonet_model::ops::Reach {
        let view = self.roster().view();
        let Some(m) = view.member(host) else { return xenonet_model::ops::Reach::default() };
        let pending = self
            .proposals()
            .iter()
            .filter(|p| p.op == Op::Settings && p.host.id == host)
            .flat_map(|p| {
                let to = p.settings.clone().unwrap_or_default();
                let by = p.signers.first().map(|s| s.by.clone()).unwrap_or_default();
                let mut out = Vec::new();
                if to.start != m.settings.start {
                    out.push(json!({ "id": p.id(), "setting": "start", "value": to.start.as_str(), "by": by, "at": p.at }));
                }
                if to.wake != m.settings.wake {
                    out.push(json!({ "id": p.id(), "setting": "wake", "value": to.wake.as_str(), "by": by, "at": p.at }));
                }
                out
            })
            .collect();
        let delegated = (m.settings.start == crate::roster::Start::Delegate).then(|| m.settings_by.clone()).flatten().map(|(by, at)| (by.join(" and "), at));
        xenonet_model::ops::Reach { start: m.settings.start.as_str().into(), wake: m.settings.wake.as_str().into(), pending, delegated }
    }

    /// A `message` to `xenonet:person` from `from`: a person's entries, or
    /// a want for them. True when the roster may now take entries it
    /// couldn't before, so it's fetched again.
    fn person_in(&self, from: &str, body: &Value) -> bool {
        let Some(v) = body.get("body").and_then(Value::as_str).and_then(|t| serde_json::from_str::<Value>(t).ok()) else { return false };
        match v.get("op").and_then(Value::as_str) {
            Some("entries") => self.offer_person(&v),
            Some("want") => {
                let (Some(person), Some(from_n)) = (v.get("person").and_then(Value::as_str), v.get("from").and_then(Value::as_u64)) else { return false };
                let entries = self.identity.since(person, from_n);
                if !entries.is_empty() {
                    let text = json!({ "op": "entries", "person": person, "entries": entries }).to_string();
                    let _ = self.send(Outgoing::new(To::Host(from.into()), Kind::Message, person_body(&text)));
                }
                false
            }
            _ => false,
        }
    }

    pub fn status(&self) -> Status {
        self.status.locked().clone()
    }

    pub fn peers(&self) -> BTreeMap<String, Peer> {
        self.peers.locked().clone()
    }

    pub fn sink(&self) -> &Arc<SinkSlot> {
        &self.sink
    }

    pub fn is_removed(&self) -> bool {
        self.status.locked().state == "removed"
    }

    pub fn is_live(&self) -> bool {
        self.status.locked().state == "live"
    }

    /// A removed host's last keys and its removal.
    pub fn removed_keys(&self, host: &str) -> Option<(PublicKeys, Removal)> {
        let r = self.roster();
        let rm = r.view_ref()?.removal(host)?.clone();
        Some((rm.keys, rm))
    }

    // ---------- how this machine stands ----------

    fn set(&self, state: &'static str, reason: Option<&str>) {
        let (was, changed) = {
            let mut s = self.status.locked();
            let was = s.state;
            let changed = was != state || s.reason.as_deref() != reason;
            if was != state {
                s.since = Some(self.now());
            }
            s.state = state;
            s.reason = reason.map(str::to_string);
            (was, changed)
        };
        if was == "live" && state != "live" {
            for p in self.peers.locked().values_mut() {
                p.online = false;
            }
            self.sink.linked(false);
        }
        if changed {
            self.write_link(true);
        }
    }

    /// `link.json`, which the doctor reads: how this machine stands in the
    /// network, and how it reaches each member (online, the path, how its
    /// address was found, its clock, what a mailbox holds of this
    /// machine's for it). At once when the state changed, else at most
    /// every ten seconds.
    pub fn write_link(&self, now: bool) {
        let t = self.now();
        {
            let mut last = self.link_written.locked();
            if !now && t - *last < 10e3 {
                return;
            }
            *last = t;
        }
        let Some(net) = self.network() else { return };
        let s = self.status();
        let view = self.roster().view();
        let peers = self.peers();
        let t2 = self.link2();
        let on_link = t2.as_ref().map(|t| t.links.peers()).unwrap_or_default();
        let found = t2.as_ref().map(|t| t.found.locked().clone()).unwrap_or_default();
        let hosts: Vec<Value> = view
            .members
            .values()
            .filter(|m| m.id != net.host)
            .map(|m| {
                let link = on_link.get(&crate::endpoint::peer_id(&m.keys));
                json!({
                    "id": m.id,
                    "online": peers.get(&m.id).is_some_and(|p| p.online),
                    "seen": peers.get(&m.id).and_then(|p| p.seen).map(|x| x as i64),
                    "path": link.and_then(|l| l.path),
                    "found": found.get(&(net.id.clone(), m.id.clone())).map(|f| f.iter().copied().collect::<Vec<_>>()),
                    "skewMs": link.and_then(|l| l.skew_ms),
                    "held": self.mail_standing(&net.id, &m.id).held,
                })
            })
            .collect();
        let v = json!({
            "state": s.state,
            "since": s.since.map(|x| x as i64),
            "reason": s.reason,
            "removed": s.removed,
            "link2": t2.is_some(),
            "hosts": hosts,
            "relays": view.relays,
            "mailboxes": view.mailboxes,
            "rosterMismatch": self.roster_mismatch(),
            "at": t as i64,
        });
        let _ = self.dir.write_link(&net.id, &v);
    }

    // ---------- the roster ----------

    /// Adds an entry to this host's roster and acts on it: a removed host's
    /// frames go, and the engine hears; this host removed stops.
    pub fn append(&self, e: Entry) -> R<Offered> {
        self.append_from(e, None)
    }

    /// `append`, for an entry that came from `from` on link/2 (None: made
    /// here). An entry this machine hadn't held
    /// goes on to every other member connected on link/2, so the chain
    /// reaches each member from any other; a removal reaches the machine
    /// it removes first, while its connection still carries the network.
    pub fn append_from(&self, e: Entry, from: Option<crate::dial::PeerId>) -> R<Offered> {
        if e.op == Op::Remove {
            self.chain_to_removed(&e);
        }
        let offered = {
            let mut r = self.roster.writing();
            let o = r.offer(e.clone())?;
            if o == Offered::Appended {
                if let Some(net) = self.network() {
                    self.dir.write_roster(&net.id, &r)?;
                }
            }
            o
        };
        if offered == Offered::Duplicate {
            return Ok(offered);
        }
        let me = self.network().map(|n| n.host);
        if e.op == Op::Remove {
            self.peers.locked().remove(&e.host.id);
            if me.as_deref() == Some(e.host.id.as_str()) {
                let by = e.signers.iter().map(|s| s.by.clone()).collect::<Vec<_>>();
                self.set_removed(json!({ "by": by, "at": e.at, "n": e.n }));
            } else {
                self.sink.removed(&e.host.id);
            }
        }
        self.members_changed();
        self.chain_out(std::slice::from_ref(&e), &[], from);
        Ok(offered)
    }

    /// Entries waiting for approval, made here or as another member passed
    /// them on.
    pub fn note_proposals(&self, entries: Vec<Entry>) {
        self.note_proposals_from(entries, None);
    }

    /// `note_proposals`, for entries from `from` on link/2. Each is kept
    /// only when this machine's own chain would take it but for a second
    /// signer (`needs-approval`): one signed wrongly, or by a machine that
    /// may not, is dropped, so no member can put a proposal in the window
    /// that its signers didn't make. One signed by enough already is
    /// appended. What's new here goes on to the other members connected.
    pub fn note_proposals_from(&self, entries: Vec<Entry>, from: Option<crate::dial::PeerId>) {
        let len = self.roster().len();
        let mut new = Vec::new();
        for e in entries.into_iter().filter(|e| e.n == len) {
            let verdict = self.roster().would_take(&e);
            match verdict {
                Ok(()) => {
                    if let Err(err) = self.append_from(e, from) {
                        self.log("net.roster", &format!("an entry passed on for approval, signed by enough, was refused: {}", err.message));
                    }
                    continue;
                }
                Err(err) if err.code == "needs-approval" => {}
                Err(err) => {
                    self.log("net.roster", &format!("an entry passed on for approval was dropped: {}", err.message));
                    continue;
                }
            }
            let mut waiting = self.proposals.locked();
            waiting.retain(|p| p.n == len);
            match waiting.iter_mut().find(|p| p.signing_bytes() == e.signing_bytes()) {
                Some(p) => {
                    let had = p.signers.len();
                    p.merge_signers(&e);
                    if p.signers.len() > had {
                        new.push(p.clone());
                    }
                }
                None => {
                    waiting.push(e.clone());
                    new.push(e);
                }
            }
            let extra = waiting.len().saturating_sub(MAX_PROPOSALS);
            waiting.drain(..extra);
        }
        self.chain_out(&[], &new, from);
    }

    /// What waits for approval and could still be taken.
    pub fn proposals(&self) -> Vec<Entry> {
        let len = self.roster().len();
        self.proposals.locked().iter().filter(|p| p.n == len).cloned().collect()
    }

    fn set_removed(&self, info: Value) {
        let by = info.get("by").and_then(Value::as_array).map(|b| b.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" and ")).unwrap_or_default();
        let at = info.get("at").and_then(Value::as_f64).map(xenonet_text::time::iso).unwrap_or_default();
        let words = format!("Removed on {} by {by}. Run xenonet net leave.", at.get(..10).unwrap_or(&at));
        self.status.locked().removed = Some(info);
        self.set("removed", Some(&words));
        self.log("net.removed", &words);
    }

    fn roster_mismatch(&self) -> Vec<String> {
        let head = self.roster().head();
        self.peers.locked().iter().filter(|(_, p)| p.roster.is_some() && p.roster != head).map(|(h, _)| h.clone()).collect()
    }

    // ---------- the down stream ----------

    /// Signs `host` in as this admin: pairs the device its join request
    /// brings into the person, when this machine acts as the network's
    /// owner, then signs the `add` entry with this device's presence. The
    /// person made the ticket at XenoNet's window, and this is its join,
    /// within the ticket's ten minutes, so it may ask them: once to admit,
    /// and to pair once more (twice where the root key is sealed to the
    /// device key).
    pub fn sign_in(&self, host: &str, keys: PublicKeys, window: bool, join: &Value) -> R<Entry> {
        let net = self.network().ok_or_else(not_in)?;
        let now = self.now() as i64;
        let owner = self.roster().view_ref().and_then(|v| v.owner.clone());
        let machine = join.get("machine").and_then(xenonet_id::sign::VerifyingKey::parse).ok_or_else(|| NetError::bad("the join request names no machine key"))?;
        let mut device = None;
        if let Some(d) = join.get("device").and_then(identity::parse_device) {
            if owner.is_some() && owner == self.identity.person_id() {
                let id = d.key.id();
                match self.identity.pair(true, d, now) {
                    Ok(_) => device = Some(id),
                    // Only the machine holding the person's root key pairs a
                    // device (copying the root to others is to come): from
                    // another admin the machine joins as a plain member,
                    // which does nothing as the person.
                    Err(e) if e.code == "no-root" => {
                        self.log("net.ticket", &format!("{host} joins without your identity: this machine doesn't hold its root key, which pairs a device"));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        let mut e = self.roster().draft(Op::Add, HostRef { id: host.into(), keys, window, device, machine }, None, now);
        e.person_head = owner.as_deref().and_then(|o| self.identity.chain_of(o)).and_then(|c| c.head());
        let block = self.identity.prove(true, crate::roster::ADMIT, &e.digest(), &format!("add {host} to your XenoNet network"), now)?;
        e.sign_with(&net.host, self.identity.machine_key()?.as_ref(), Some(block))?;
        Ok(e)
    }

    /// A presence from `from` that came on link/2: kept for the hosts'
    /// list and passed to the engine.
    pub(crate) fn presence_in(&self, net: &NetFile, from: &str, mut body: Value, window: bool) {
        {
            let mut peers = self.peers.locked();
            let p = peers.entry(from.to_string()).or_default();
            p.seen = Some(self.now());
            p.online = true;
        }
        if !self.note_presence(from, &mut body) {
            return;
        }
        self.sink.receive(Incoming { network: net.id.clone(), from: from.to_string(), kind: Kind::Presence, body, at: self.now(), from_window_host: window });
    }

    /// Whether `host`, a member, has a link/2 connection up: it's online
    /// while one is, and was seen when it came or went.
    pub(crate) fn peer_up(&self, host: &str, up: bool) {
        let now = self.now();
        let mut peers = self.peers.locked();
        let p = peers.entry(host.to_string()).or_default();
        p.online = up;
        p.seen = Some(now);
    }

    /// `from`'s act at this machine, decided on its own facts: a member of
    /// the network, which a frame it signed came from. Why it was refused.
    fn decide(&self, from: &str, resource: xenonet_authz::Resource, act: xenonet_authz::Act) -> Result<(), String> {
        let facts = self.facts();
        let id = xenonet_authz::MachineId::new(from).map_err(|e| e.message)?;
        let request = xenonet_authz::Request::new(xenonet_authz::Principal::Machine { id }, resource, act, xenonet_authz::Door::Network, facts.now, facts.network());
        match xenonet_authz::decide(None, &request, &facts, None) {
            xenonet_authz::Decision::Allow(_) => Ok(()),
            d => Err(d.reason().map(|r| r.text().to_string()).unwrap_or_default()),
        }
    }

    /// Whether `from` may read this machine's transcripts and stats: an
    /// admin member's read, as the window's reads are decided.
    fn decide_read(&self, from: &str) -> bool {
        let own = self.facts().machine;
        let d = self.decide(from, xenonet_authz::Resource::Machine(own), xenonet_authz::Act::Read { scope: xenonet_authz::ReadScope::Transcripts, filters: None });
        d.is_ok()
    }

    /// Whether `from` is a member whose machine may message this one: what
    /// a person's entries and a want for them need besides their own
    /// signatures.
    fn decide_member(&self, from: &str) -> bool {
        let own = self.facts().machine;
        match self.decide(from, xenonet_authz::Resource::Machine(own), xenonet_authz::Act::Message) {
            Ok(()) => true,
            Err(why) => {
                self.log("net.person", &format!("a person's entries from {from} were refused: {why}"));
                false
            }
        }
    }

    // ---------- the feed ----------

    /// An entry from `from`, taken only when its author is a member of the
    /// network, by the machine signing key the chain lists for it, and it
    /// checks against that key: the member's host id, and the entry. Its
    /// time is shown, never a reason to drop it.
    pub fn admit(&self, net: &NetFile, from: &str, entry: &Value) -> Result<(String, xenonet_log::feed::Verified), String> {
        let e = FeedEntry::parse(entry).map_err(|e| format!("a feed entry from {from} was dropped: {}", e.message))?;
        if e.group != net.id {
            return Err(format!("a feed entry from {from} for another network was dropped"));
        }
        let member = {
            let r = self.roster();
            r.view_ref().and_then(|v| v.members.values().find(|m| m.machine.id() == e.author).map(|m| (m.id.clone(), m.machine.clone())))
        };
        let Some((host, key)) = member else {
            return Err(format!("a feed entry from {from} whose author ({}) isn't a member of the network was refused", e.author));
        };
        // An entry comes from its author's own machine until the transport
        // fetches feeds from any member that holds them (phase 5).
        if host != from {
            return Err(format!("{from} sent an entry of {host}'s feed, which was refused: entries come from their authors"));
        }
        let v = e.verify(&key).map_err(|e| format!("a feed entry from {from} was refused: {}", e.message))?;
        Ok((host, v))
    }

    /// The writer of this machine's feed in `net`: as it was kept, or a new
    /// one, kept before its first entry.
    fn writer_in<'a>(&self, slot: &'a mut Option<Writer>, net: &NetFile) -> R<&'a mut Writer> {
        if slot.is_none() {
            let author = self.identity.machine_key()?.public().id();
            let w = match self.opts.persist.feed_writer(&net.id)? {
                Some(at) => Writer::resume(author, net.id.as_str(), at.inc, at.kept, at.loose, at.link, at.head),
                None => {
                    let w = Writer::start(author.clone(), net.id.as_str(), self.now() as u64);
                    self.opts.persist.put_feed_writer(&net.id, &author, writer_at(&w), None)?;
                    w
                }
            };
            *slot = Some(w);
        }
        slot.as_mut().ok_or_else(not_in)
    }

    /// The next entry of this machine's feed, signed by its machine signing
    /// key, and kept (the writer's place, and a kept entry itself) before
    /// it goes.
    pub fn write_entry(&self, kind: xenonet_log::feed::Kind, body: Value) -> R<FeedEntry> {
        let net = self.network().ok_or_else(not_in)?;
        let key = self.identity.machine_key()?;
        let mut slot = self.writer.locked();
        let w = self.writer_in(&mut slot, &net)?;
        let before = w.clone();
        let e = w.write(key.as_ref(), kind, self.now().max(0.0) as u64, body)?;
        let kept = (e.class() == Class::Kept).then_some(&e);
        let done = e.well_formed().map_err(|err| NetError::bad(err.message)).and_then(|()| self.opts.persist.put_feed_writer(&net.id, &e.author, writer_at(w), kept));
        if let Err(err) = done {
            *w = before;
            return Err(err);
        }
        Ok(e)
    }

    /// A compact entry of this machine's feed, now: over what its
    /// incarnation wrote, keeping live what the kinds' retention keeps
    /// (`open` says which of the kinds kept while open still are), and the
    /// rest of the range dropped from what it sends again. Kept as
    /// `write_entry` keeps an entry; when the link writes them is link/2's.
    pub fn compact_entry(&self, open: impl Fn(&FeedEntry) -> bool) -> R<FeedEntry> {
        let net = self.network().ok_or_else(not_in)?;
        let key = self.identity.machine_key()?;
        let mut slot = self.writer.locked();
        let w = self.writer_in(&mut slot, &net)?;
        let now = self.now().max(0.0) as u64;
        let held = self.opts.persist.feed_out_after(&net.id, Pos::new(w.inc(), 0), usize::MAX)?;
        let live = xenonet_log::feed::live_set(&held, w.inc(), now, open).map_err(|e| NetError::bad(e.message))?;
        let before = w.clone();
        let e = w.compact(key.as_ref(), now, &live)?;
        if let Err(err) = self.opts.persist.put_feed_writer(&net.id, &e.author, writer_at(w), Some(&e)) {
            *w = before;
            return Err(err);
        }
        Ok(e)
    }

    /// A body the engine sends as an entry of this machine's feed: a loose
    /// one to the host it's for (or every other host), a kept one to every
    /// member, whoever the engine named.
    fn send_entry(&self, net: &NetFile, out: Outgoing) -> Result<(), String> {
        let kind = entry_kind(out.kind).ok_or("that isn't a feed's")?;
        if !out.body.is_object() {
            return Err(format!("a {} is a JSON object", out.kind.as_str()));
        }
        let others: Vec<String> = {
            let r = self.roster();
            let v = r.view_ref().ok_or("this machine's roster is empty")?;
            let all = || v.members.keys().filter(|h| **h != net.host).cloned().collect::<Vec<_>>();
            match (&out.to, kind.class()) {
                (To::Host(h), Class::Loose) if *h == net.host => return Err("that's this machine".into()),
                (To::Host(h), Class::Loose) if v.member(h).is_none() => return Err(format!("{h} isn't in the network")),
                (To::Host(h), Class::Loose) => vec![h.clone()],
                _ => all(),
            }
        };
        // An act carrying the person's presence goes only to a recipient
        // that's connected: refused before it's written, rather than left
        // to rest where the presence would be stale on delivery.
        if crate::mailbox::carries_presence(out.authority.as_ref()) {
            let t = self.link2();
            let view = self.roster().view();
            if let Some(away) = others.iter().find(|h| !view.member(h).is_some_and(|m| t.as_ref().is_some_and(|t| t.links.is_connected(crate::endpoint::peer_id(&m.keys))))) {
                return Err(format!("{away} is offline: an act with your presence goes only to a machine that's connected"));
            }
        }
        let e = self.write_entry(kind, out.body).map_err(|e| e.message)?;
        self.link2_entry(net, &e, &others, out.authority.as_ref());
        Ok(())
    }

    /// `from` holds this machine's feed up to `at`. When that's past what
    /// this machine wrote, its store was restored from a backup: it writes
    /// from a new incarnation from now on, so it never signs a second entry
    /// at a place the others hold.
    fn ack_in(&self, net: &NetFile, from: &str, at: Pos) {
        if let Err(e) = self.opts.persist.put_feed_ack(&net.id, from, at) {
            self.log("net.feed", &format!("{from}'s ack wasn't kept: {}", e.message));
        }
        self.link2_acked(net, from, at);
        let mut slot = self.writer.locked();
        let Ok(w) = self.writer_in(&mut slot, net) else { return };
        if at.inc < w.inc() || (at.inc == w.inc() && at.seq <= w.kept()) {
            return;
        }
        let Ok(key) = self.identity.machine_key() else { return };
        let author = key.public().id();
        let fresh = Writer::restore(author.clone(), net.id.as_str(), w.head(), at.inc.max(w.inc()), self.now() as u64);
        match self.opts.persist.put_feed_writer(&net.id, &author, writer_at(&fresh), None) {
            Ok(()) => {
                self.log("net.feed", &format!("{from} holds more of this machine's feed than it has: its store was restored, so it writes from a new incarnation"));
                *w = fresh;
            }
            Err(e) => self.log("net.feed", &format!("a new incarnation wasn't kept: {}", e.message)),
        }
    }

    /// Takes a member's feed again after a fork froze it, keeping what this
    /// machine had: the person's, from the window alone, since a fork is a
    /// machine signing two things at one place and only they can say which
    /// machine to trust again.
    pub fn unfreeze(&self, author: &str, at_window: bool) -> R<Value> {
        if !at_window {
            return Err(NetError::new(403, "window", "unfreezing a machine's feed is yours, so it's done from XenoNet's window"));
        }
        let net = self.network().ok_or_else(not_in)?;
        if !self.opts.persist.frozen_feeds(&net.id)?.iter().any(|(a, _)| a == author) {
            return Err(NetError::new(404, "not-frozen", format!("no feed from {author} is frozen here")));
        }
        let applied = self.sink.unfreeze(author, &net.id).map_err(NetError::internal)?;
        self.log("net.feed", &format!("you unfroze {author}'s feed: {applied} held entries applied"));
        Ok(json!({ "unfrozen": author, "applied": applied }))
    }

    /// The members' feeds a fork froze here, as the state lists them.
    fn frozen(&self, net: &NetFile) -> Vec<Value> {
        let frozen = self.opts.persist.frozen_feeds(&net.id).unwrap_or_default();
        let r = self.roster();
        frozen
            .into_iter()
            .map(|(author, fork)| {
                let host = r.view_ref().and_then(|v| v.members.values().find(|m| m.machine.id() == author).map(|m| m.id.clone()));
                json!({ "author": author, "host": host, "at": { "inc": fork.at.inc, "seq": fork.at.seq } })
            })
            .collect()
    }

    /// This host's own presence on its way out: what it says of its agents
    /// and the person is kept for this host's row, and when they last
    /// worked here goes by this machine's clock.
    fn own_presence(&self, body: Value) -> Value {
        let you = body.get("you").and_then(Value::as_bool).unwrap_or(false);
        let at = body.get("youAt").and_then(Value::as_f64).filter(|t| t.is_finite() && *t > 0.0);
        *self.me_you.locked() = (you, at);
        fold_agents(&mut self.me_agents.locked(), &body);
        body
    }

    /// A presence, kept for the hosts' list: false when it's older than one
    /// already had. Address records travel on link/2's control stream, never
    /// in presence.
    /// The member another machine's agent is on, by the agt_ its
    /// presence lists it by.
    pub fn host_of_agent(&self, id: &str) -> Option<String> {
        self.peers.locked().iter().find(|(_, p)| p.agents.contains_key(id)).map(|(h, _)| h.clone())
    }

    fn note_presence(&self, from: &str, body: &mut Value) -> bool {
        if let Some(o) = body.as_object_mut() {
            o.remove("address");
        }
        let counter = body.get("counter").and_then(Value::as_u64).unwrap_or(0);
        let mut peers = self.peers.locked();
        let p = peers.entry(from.to_string()).or_default();
        if counter != 0 && counter <= p.counter {
            return false;
        }
        p.counter = counter;
        fold_agents(&mut p.agents, body);
        p.you = body.get("you").and_then(Value::as_bool).unwrap_or(false);
        p.you_at = body.get("youAt").and_then(Value::as_f64).filter(|t| t.is_finite() && *t > 0.0);
        // An empty head says nothing about the sender's roster: no mismatch.
        p.roster = body.get("roster").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string);
        p.can = body.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(16).map(str::to_string).collect()).unwrap_or_default();
        true
    }

    // ---------- the up batch ----------

    // ---------- services ----------

    /// The services this host published, and the requests it waits on.
    pub fn services(&self) -> &Services {
        &self.svc
    }

    /// The person publishes a service on this machine to the network, in
    /// place of one by its name. Publishing widens what the network can
    /// reach, so it's decided as the person's act (`Act::Publish`), with
    /// their presence for exactly this publication: only from XenoNet's
    /// window (`at_window`), where they can be asked.
    pub fn svc_publish(&self, name: &str, target: &str, paths: &[String], hosts: &[String], at_window: bool) -> R<Value> {
        let net = self.network().ok_or_else(not_in)?;
        let p = svc::Published::new(name, target, paths, hosts, self.now() as i64).map_err(NetError::bad)?;
        {
            let r = self.roster();
            let v = r.view_ref().ok_or_else(not_in)?;
            for h in &p.hosts {
                if *h == net.host {
                    return Err(NetError::bad(format!("{h} is this machine: --hosts names the other hosts that may call")));
                }
                if v.member(h).is_none() {
                    return Err(NetError::bad(format!("{h} isn't in the network")));
                }
            }
        }
        if !at_window {
            return Err(identity::not_at_window("publishing a service"));
        }
        self.decide_publish(&net.id, &p)?;
        let words =
            format!("this machine published {} at {} to {}", p.name, p.target.url(), if p.hosts.is_empty() { "every host in the network".to_string() } else { p.hosts.join(", ") });
        self.svc.publish(p)?;
        self.log("net.svc", &words);
        Ok(self.svc_list())
    }

    /// The person's publication of `p`, decided here: their presence for
    /// it from this device (it asks them once), checked and taken once,
    /// then `decide` at this machine's own door.
    fn decide_publish(&self, network: &str, p: &svc::Published) -> R<()> {
        use xenonet_authz::{Act, Decision, Door, NetworkId, PersonId, PersonSigned, Principal, Request, Resource, ServiceId, VerifiedPresence};
        let refused = |why: String| NetError::new(403, "not-allowed", why);
        let id = &self.identity;
        let person =
            id.person_id().filter(|_| id.can_act()).ok_or_else(|| NetError::new(409, "no-person", "this machine has no device of yours that can sign, so it publishes nothing"))?;
        let spec = json!({ "target": p.target.url(), "paths": p.paths, "hosts": p.hosts }).to_string();
        let act = Act::Publish { service: ServiceId::new(&p.name).map_err(|e| NetError::bad(e.message))?, spec: xenonet_id::codec::sha256(spec.as_bytes()) };
        let resource = Resource::Network(NetworkId::new(network).map_err(|e| NetError::bad(e.message))?);
        let facts = self.facts();
        let block = id.prove(true, "publish", &xenonet_authz::act_hash(&resource, &act), &format!("publish {} to your network", p.name), facts.now)?;
        let person = PersonId::new(&person).map_err(|e| NetError::bad(e.message))?;
        let signed = PersonSigned::from_presence(&facts, &person, &block, &resource, &act).map_err(|e| refused(e.message))?;
        let request = Request::new(Principal::Person(signed), resource, act, Door::Local, facts.now, None);
        let presence = id.with_seen(|seen| VerifiedPresence::check(seen, &block, &facts, &request).map_err(|e| NetError::new(403, "presence", e.message)))?;
        match xenonet_authz::decide(None, &request, &facts, Some(&presence)) {
            Decision::Allow(_) => Ok(()),
            d => Err(refused(d.reason().map_or_else(|| "not allowed".to_string(), |r| r.text().to_string()))),
        }
    }

    /// Revokes one of the person's other devices, as for a lost machine:
    /// an entry in their chain signed by this device with its presence,
    /// pushed to the network's machines, each of which then refuses
    /// anything that device signs. Only from XenoNet's window.
    pub fn revoke_device(&self, device: &str, at_window: bool) -> R<Value> {
        let made = self.identity.revoke_device(at_window, device, self.now() as i64)?;
        if !made.is_empty() {
            self.log("net.person", &format!("you revoked the device {device}: nothing it signs counts any more"));
        }
        Ok(json!({ "revoked": device }))
    }

    /// `xenonet svc unpublish`.
    pub fn svc_unpublish(&self, name: &str) -> R<Value> {
        self.network().ok_or_else(not_in)?;
        if !self.svc.unpublish(name)? {
            return Err(NetError::new(404, "not-published", format!("this machine publishes no service named {name}")));
        }
        self.log("net.svc", &format!("this machine took {name} off the network"));
        Ok(self.svc_list())
    }

    /// `xenonet svc list`: this host and what it published.
    pub fn svc_list(&self) -> Value {
        let host = self.network().map(|n| n.host);
        json!({ "host": host, "services": self.svc.list().iter().map(svc::Published::to_json).collect::<Vec<_>>() })
    }
}

impl Net for Shared {
    // This node's one network: `Nodes` lists them all.
    fn networks(&self) -> Vec<Me> {
        self.network().map(|n| Me { network: n.id, name: n.name, host: n.host }).into_iter().collect()
    }

    fn roster_head(&self) -> Option<String> {
        self.network()?;
        self.roster().head()
    }

    fn titles_only(&self) -> bool {
        self.network().is_some_and(|n| n.titles_only)
    }

    fn share_load(&self) -> bool {
        self.network().is_some_and(|n| n.share_load)
    }

    fn is_window_host(&self) -> bool {
        self.network().is_some_and(|n| self.roster().view_ref().is_some_and(|v| v.is_window(&n.host)))
    }

    fn reach(&self) -> xenonet_model::ops::Reach {
        let Some(n) = self.network() else { return xenonet_model::ops::Reach::default() };
        self.reach_of(&n.host)
    }

    fn host_can(&self, host: &str, what: &str) -> bool {
        self.peers.locked().get(host).is_some_and(|p| p.can.iter().any(|c| c == what))
    }

    fn ack(&self, network: &str, host: &str, inc: u64, high: u64) {
        let Some(net) = self.in_network(network) else { return };
        let author = self.roster().view_ref().and_then(|v| v.member(host).map(|m| m.machine.id()));
        if let Some(author) = author {
            self.link2_filled(&net.id, &author, Pos::new(inc, high));
            self.link2_control(host, link2::Control::Ack { network: net.id.clone(), author, inc, high });
        }
    }

    fn want(&self, network: &str, host: &str, inc: u64, from: u64) {
        let Some(net) = self.in_network(network) else { return };
        // On link/2, from one holder at a time: the author when it is
        // connected, else any member that holds the entries.
        let author = self.roster().view_ref().and_then(|v| v.member(host).map(|m| m.machine.id()));
        if let Some(author) = author {
            self.link2_fetch(&net.id, &author, Pos::new(inc, from));
        }
    }

    fn hosts(&self) -> Vec<Host> {
        let Some(net) = self.network() else { return Vec::new() };
        let view = self.roster().view();
        let peers = self.peers.locked();
        view.members
            .values()
            .filter(|m| m.id != net.host)
            .map(|m| {
                let p = peers.get(&m.id);
                Host { id: m.id.clone(), window: view.is_window(&m.id), online: p.is_some_and(|p| p.online), seen: p.and_then(|p| p.seen) }
            })
            .collect()
    }

    fn send(&self, out: Outgoing) -> Result<(), String> {
        let net = self.network().ok_or("this machine isn't in a XenoNet network")?;
        if let Some(n) = out.network.as_deref().filter(|n| *n != net.id) {
            return Err(format!("this machine isn't in {n}"));
        }
        if self.is_removed() {
            return Err("this machine was removed from its network".into());
        }
        let mut body = out.body;
        // Presence goes on link/2 (`Nodes`' send), once to each peer,
        // whatever networks it shares with this machine: here it's only
        // kept for this machine's own row.
        if out.kind == Kind::Presence {
            self.own_presence(body);
            return Ok(());
        }
        // The person's act at another machine (an answer to its ask): a
        // message entry of this machine's feed to `xenonet:operator`, for
        // that one machine, with what it rests on beside it, so it goes on
        // link/2 as every message does and is decided there.
        if out.kind == Kind::Operator {
            let To::Host(_) = &out.to else { return Err("the person's act goes to one machine".into()) };
            let authority = body.as_object_mut().and_then(|o| o.remove("authority")).or(out.authority);
            let entry = json!({ "target": OPERATOR_TARGET, "body": body });
            return self.send_entry(&net, Outgoing { network: out.network, to: out.to, kind: Kind::Message, body: entry, authority });
        }
        if out.kind.fed() {
            return self.send_entry(&net, Outgoing { body, ..out });
        }
        Err(format!("a {} isn't carried between machines", out.kind.as_str()))
    }
}

/// This machine in its network, and its tasks.
pub struct Node {
    shared: Arc<Shared>,
    /// The invites open here, and their door on join/2.
    invites: join2::Invites,
}

impl Node {
    /// The node of `net` (None: a node in no network yet, which `create`
    /// or a join puts in one), as its files leave it, sharing `common`
    /// with this machine's other networks; nothing runs until `start`.
    pub fn open_in(opts: Options, sink: Arc<SinkSlot>, common: Arc<Common>, net: Option<NetFile>) -> R<Arc<Node>> {
        let dir = Dir::new(opts.dir.clone(), opts.persist.clone());
        let identity = common.identity.clone();
        let policy = NetworkPolicy::with(identity.clone());
        let (key, roster) = match &net {
            Some(n) => {
                let key = HostKey::load(&dir.file(KEY))?.ok_or_else(|| NetError::internal("this host's key is missing"))?;
                (Some(Arc::new(key)), dir.read_roster(&n.id, policy)?)
            }
            None => (None, Roster::with_policy(policy)),
        };
        let services = Services::open(opts.persist.clone(), net.as_ref().map(|n| n.id.clone()));
        let shared = Arc::new_cyclic(|this| Shared {
            this: this.clone(),
            opts,
            dir,
            key: RwLock::new(key),
            net: RwLock::new(net),
            roster: RwLock::new(roster),
            writer: Mutex::new(None),
            sink,
            status: Mutex::new(Status::default()),
            peers: Mutex::new(BTreeMap::new()),
            svc: services,
            identity: identity.clone(),
            link_written: Mutex::new(0.0),
            proposals: Mutex::new(Vec::new()),
            me_you: Mutex::new((false, None)),
            me_agents: Mutex::new(BTreeMap::new()),
            common,
            chain_heard: Mutex::new(0),
            chain_wake: Notify::new(),
        });
        Ok(Arc::new(Node { shared, invites: join2::Invites::default() }))
    }

    pub fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// What the engine sends through.
    pub fn net(&self) -> Arc<dyn Net> {
        self.shared.clone()
    }

    pub fn sink(&self) -> &Arc<SinkSlot> {
        self.shared.sink()
    }

    /// Starts this machine in its network: it's in it as soon as its node
    /// runs, and each member is online while its link/2 connection is up.
    /// link/2 is the machine's, for every network at once (`Nodes`).
    pub fn start(&self) -> R<()> {
        self.stop();
        let n = &self.shared;
        if n.network().is_none() || n.is_removed() {
            return Ok(());
        }
        n.set("live", None);
        n.sink.linked(true);
        n.members_changed();
        Ok(())
    }

    /// Stops this machine in its network. link/2 is the machine's, and
    /// stops with `Nodes`.
    pub fn stop(&self) {
        let s = &self.shared;
        if !s.is_removed() && s.status.locked().state != "off" {
            s.set("off", None);
        }
    }

    fn me(&self) -> R<(NetFile, Arc<HostKey>)> {
        let n = &self.shared;
        Ok((n.network().ok_or_else(not_in)?, n.key().ok_or_else(not_in)?))
    }

    fn adopt(&self, net: NetFile, key: HostKey, roster: Roster) -> R<()> {
        let n = &self.shared;
        let net_id = net.id.clone();
        if let Some(had) = n.common.shared_of(&net.id).and_then(|s| s.network()) {
            return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already, as {}", had.name, had.host)));
        }
        // The network's row before its roster, which is kept under it.
        n.dir.write_network(&net)?;
        n.dir.write_roster(&net.id, &roster)?;
        // The first network this machine is in sees all of it; every one
        // after starts with nothing, until the person shares projects
        // into it at the window (`share`).
        if n.common.shareds().iter().any(|s| s.network().is_some_and(|o| o.id != net.id)) && n.opts.persist.sharing(&net.id)?.is_none() {
            n.opts.persist.put_sharing(&net.id, &crate::share::Sharing::default_for(false))?;
        }
        *n.writer.locked() = None;
        *n.key.writing() = Some(Arc::new(key));
        *n.roster.writing() = roster;
        *n.net.writing() = Some(net);
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        n.svc.load(Some(net_id));
        Ok(())
    }

    /// `xenonet net create`: a new network with this machine as its
    /// founder, owned by the person here. With no person here yet, it's
    /// founded first, which asks them once: from the window (`at_window`)
    /// only. A machine that can't ask the person makes a network with no
    /// owner, where nobody can admit another machine.
    pub fn create(&self, name: &str, host: &str, at_window: bool) -> R<Value> {
        let n = &self.shared;
        if n.network().is_some() {
            return Err(NetError::internal("a network is made on a node in none (Nodes::create)"));
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
            return Err(NetError::bad("a network's name is 1 to 64 characters"));
        }
        if !is_host_id(host) {
            return Err(NetError::bad("a host id is 2 to 32 lowercase letters, digits and dashes, starting with a letter"));
        }
        let at = n.now() as i64;
        let identity = n.identity.clone();
        if identity.device_key()?.is_some() {
            identity.found(at_window, host, at)?;
        }
        let machine = identity.machine_key()?.public();
        let device = identity.can_act().then(|| identity.device_id()).flatten();
        let key = HostKey::load_or_create(&n.dir.file(KEY))?;
        let id = crate::codec::network_id()?;
        let me = HostRef { id: host.into(), keys: *key.public(), window: true, device: device.clone(), machine };
        let owner = device.and(identity.person_id());
        let head = owner.as_ref().and(identity.head());
        let roster =
            Roster::from_entries_with(NetworkPolicy::with(identity.clone()), vec![crate::roster::found(&id, me, identity.machine_key()?.as_ref(), owner, head, None, at)?])?;
        let net = NetFile { id, name: name.into(), host: host.into(), founder: host.into(), titles_only: false, share_load: true, joined: at };
        self.adopt(net, key, roster)?;
        self.start()?;
        n.log("net.create", &format!("this machine made the network {name}, as {host}"));
        Ok(self.state())
    }

    /// `xenonet net leave`: asks to be removed, then forgets the network.
    pub fn leave(&self, force: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        // The founder's key is the network's root, which nothing hands on,
        // so the founder leaves only with --force, alone or not.
        let founder = net.founder == net.host;
        if !force && founder {
            let why = if n.roster().view().members.len() > 1 {
                "this machine founded the network, and its key is the network's root: the other hosts lose it if it leaves. Leave with --force all the same."
            } else {
                "this machine founded the network, and its key is the network's root: the network ends when it leaves. Leave with --force to end it."
            };
            return Err(NetError::new(409, "needed", why));
        }
        let mut asked = false;
        if net.founder != net.host && !n.is_removed() {
            let me = n.roster().host_ref(&net.host);
            if let Some(m) = me {
                let mut e = n.draft(Op::Remove, m);
                e.sign_by(&net.host, n.identity.machine_key()?.as_ref())?;
                asked = self.submit(e).is_ok();
            }
        }
        self.stop();
        // Nothing of this network on link/2 from now; the connections the
        // other networks share stay.
        if let Some(t) = n.link2() {
            t.mesh.leave(&net.id);
        }
        // The key and the other people's chains stay while another network
        // needs them.
        let last = !n.common.shareds().iter().any(|s| s.network().is_some_and(|o| o.id != net.id));
        *n.writer.locked() = None;
        n.dir.forget(&net.id, last)?;
        *n.key.writing() = None;
        *n.roster.writing() = Roster::with_policy(NetworkPolicy::with(n.identity.clone()));
        *n.net.writing() = None;
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        n.svc.forget();
        if last {
            n.identity.forget_others();
        }
        self.invites.close();
        n.log("net.leave", &format!("this machine left the network {}", net.name));
        Ok(json!({ "left": net.name, "removed": asked, "founder": founder }))
    }

    /// An entry this host signed, into this roster, and from it to the
    /// members on link/2.
    fn submit(&self, e: Entry) -> R<Offered> {
        let n = &self.shared;
        n.roster().would_take(&e)?;
        match n.append(e) {
            Ok(o) => Ok(o),
            Err(err) if err.code == "roster-fork" => Ok(Offered::Duplicate),
            Err(err) => Err(err),
        }
    }

    /// Removes another host: an entry this host signs, which the roster's
    /// rules decide.
    pub async fn remove(&self, host: &str) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        if host == net.host {
            return Err(NetError::bad("to take this machine out, run xenonet net leave"));
        }
        let m = n.roster().host_ref(host).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        let mut e = n.draft(Op::Remove, m);
        e.sign_by(&net.host, n.identity.machine_key()?.as_ref())?;
        self.submit(e)?;
        n.log("net.remove", &format!("{host} was removed from the network"));
        Ok(json!({ "removed": host, "state": self.state() }))
    }

    /// **Replace**: the old entry goes, and an invite comes for the same id
    /// and window choice, for the reinstalled machine to join with.
    pub async fn replace(self: &Arc<Self>, host: &str, at_window: bool) -> R<Value> {
        let window = self.shared.roster().view().member(host).map(|m| m.window).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        if !at_window {
            return Err(identity::not_at_window("Replace"));
        }
        self.remove(host).await?;
        let mut t = self.invite(host, window, false, at_window).await?;
        t["replaced"] = json!(host);
        Ok(t)
    }

    /// Settings: titles only and whether this host shares its load, kept
    /// here; who may start and wake sessions here, a settings entry in the
    /// chain (`set_reach`).
    pub async fn settings(&self, change: Change, at_window: bool) -> R<Value> {
        let Change { titles_only, share_load, start, wake } = change;
        let n = &self.shared;
        {
            let mut guard = n.net.writing();
            let net = guard.as_mut().ok_or_else(not_in)?;
            if let Some(t) = titles_only {
                net.titles_only = t;
            }
            if let Some(s) = share_load {
                net.share_load = s;
            }
            n.dir.write_network(net)?;
        }
        if start.is_some() || wake.is_some() {
            let me = n.network().ok_or_else(not_in)?.host;
            return self.set_reach(&me, start.as_deref(), wake.as_deref(), at_window).await;
        }
        Ok(self.state())
    }

    /// Who may start and wake sessions on `host` (this machine or another),
    /// as a settings entry this machine signs. A narrowing is done at once
    /// by an admin, or by the machine itself; a widening (start to network
    /// or delegate, wake to network) takes the founder, or waits for a
    /// second admin, each with the person's presence, so it's done from
    /// the window. Delegate lets the machine take delegated work; what that
    /// work may do is a delegation of the person's, in their own chain, that
    /// names the machine's device. Set to delegate from the window with no
    /// delegation naming that device, the person's standard one is issued
    /// too (every device, starts and answers within the delegation judge's
    /// limits, for a year), signed by this machine's device with presence.
    pub async fn set_reach(&self, host: &str, start: Option<&str>, wake: Option<&str>, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        let start = start.map(|s| crate::roster::Start::parse(s).ok_or_else(|| NetError::bad("start is off, you, network or delegate"))).transpose()?;
        let wake = wake.map(|w| crate::roster::Wake::parse(w).ok_or_else(|| NetError::bad("wake is consent or network"))).transpose()?;
        let (target, had, device) = {
            let r = n.roster();
            let v = r.view_ref().ok_or_else(not_in)?;
            let m = v.member(host).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
            if host != net.host && !v.is_window(&net.host) {
                return Err(NetError::new(403, "not-window", "another machine's settings are changed from a window host"));
            }
            (r.host_ref(host).ok_or_else(not_in)?, m.settings.clone(), m.device.clone())
        };
        let now = n.now() as i64;
        let mut to = had.clone();
        if let Some(s) = start {
            to.start = s;
        }
        if let Some(w) = wake {
            to.wake = w;
        }
        if to == had {
            return Ok(self.state());
        }
        if to.start == crate::roster::Start::Delegate && had.start != crate::roster::Start::Delegate {
            self.delegation_for(host, device.as_deref(), at_window, now)?;
        }
        let mut e = n.draft(Op::Settings, target);
        e.settings = Some(to);
        let presence = match e.presence_for(&n.roster().view()) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), &format!("let your network reach {host} further"), now)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.decide_entry(e, "this machine asked to widen what the network may do there; the founder or another window host approves it")
    }

    /// A delegation of the person's that names `host`'s device for starts
    /// and answers: the one there is, or the standard one, issued now.
    fn delegation_for(&self, host: &str, device: Option<&str>, at_window: bool, now: i64) -> R<()> {
        let Some(device) = device else {
            return Err(NetError::new(409, "no-device", format!("{host} has no device of yours, so no delegation can name it, and it takes no delegated work")));
        };
        let id = &self.shared.identity;
        let named = id.chain().person().is_some_and(|p| p.delegations_of(device).any(|d| d.until > now && d.covers("start") && d.covers("answer")));
        if named {
            return Ok(());
        }
        let dlg = format!("dlg_{}", xenonet_id::codec::hex(&xenonet_id::codec::random::<8>()?));
        id.delegate(at_window, xenonet_log::person::Delegation::standard(dlg, now + 365 * 24 * 3600 * 1000), now)?;
        Ok(())
    }

    /// An entry this machine signed: into the chain, or, signed by too
    /// few, kept as a proposal for another admin to approve.
    fn decide_entry(&self, e: Entry, waits: &str) -> R<Value> {
        let n = &self.shared;
        let verdict = n.roster().would_take(&e);
        match verdict {
            Ok(()) => {}
            Err(err) if err.code == "needs-approval" => {
                self.propose(e);
                n.log("net.settings", waits);
                let mut out = self.state();
                out["proposed"] = json!(true);
                return Ok(out);
            }
            Err(err) => return Err(err.into()),
        }
        self.submit(e)?;
        Ok(self.state())
    }

    /// An entry signed by too few, for another admin to approve: kept here
    /// and passed on to the members.
    pub(super) fn propose(&self, e: Entry) {
        self.shared.note_proposals(vec![e]);
    }

    /// This network as `network/state/v1`'s `networks` lists it: its id
    /// and name, this machine's host id in it, its members, how it stands
    /// in it and its settings, and what the person shares into it.
    pub fn network_json(&self) -> Value {
        Node::network_of(&self.state())
    }

    /// `network_json` from this network's `state`.
    pub fn network_of(s: &Value) -> Value {
        let net = s.get("network").cloned().unwrap_or(Value::Null);
        let mut out = json!({ "id": net["id"], "name": net["name"], "host": s["self"]["id"] });
        for k in [
            "self",
            "link",
            "hosts",
            "invites",
            "rosterHead",
            "rosterMismatch",
            "frozen",
            "titlesOnly",
            "shareLoad",
            "start",
            "wake",
            "delegated",
            "relays",
            "mailboxes",
            "sharing",
            "inContact",
            "proposals",
            "removed",
        ] {
            if let Some(v) = s.get(k) {
                out[k] = v.clone();
            }
        }
        out
    }

    /// The relays the network lists, in place of what it had: `urls` put
    /// in (`add`) or taken out. Adding widens what reaches the network, so
    /// it takes the founder with the person's presence, or waits for a
    /// second admin (the decided table), from the window; taking one out
    /// is done at once.
    pub async fn set_relays(&self, urls: &[String], add: bool, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        let had = n.roster().view().relays;
        let mut to = had.clone();
        if add {
            for u in urls {
                if !to.contains(u) {
                    to.push(u.clone());
                }
            }
        } else {
            to.retain(|u| !urls.contains(u));
        }
        if to == had {
            return Ok(self.state());
        }
        if to.len() > xenonet_log::network::MAX_RELAYS {
            return Err(NetError::bad(format!("a network lists at most {} relays", xenonet_log::network::MAX_RELAYS)));
        }
        let me = n.roster().host_ref(&net.host).ok_or_else(|| NetError::new(410, "removed", "this machine isn't in the network"))?;
        let mut e = n.draft(Op::Relays, me);
        e.relays = Some(to);
        let now = n.now() as i64;
        let presence = match e.presence_for(&n.roster().view()) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), "let your network's machines reach each other through another relay", now)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.decide_entry(e, "this machine asked to add a relay to the network; the founder or another window host approves it")
    }

    /// This machine's `rekey` entry, signed and checked but not yet in the
    /// chain: its new transport keys `to`, under its machine signing key,
    /// with the person's presence (`admit`), so only from the window.
    pub(crate) async fn rekey_entry(&self, to: PublicKeys, at_window: bool) -> R<Entry> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        let mut me = n.roster().host_ref(&net.host).ok_or_else(|| NetError::new(410, "removed", "this machine isn't in the network"))?;
        me.keys = to;
        let mut e = n.draft(Op::Rekey, me);
        let what = e.presence_for(&n.roster().view()).unwrap_or(crate::roster::ADMIT);
        let presence = n.identity.prove(at_window, what, &e.digest(), "give this machine a new network key", n.now() as i64)?;
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), Some(presence))?;
        n.roster().would_take(&e)?;
        Ok(e)
    }

    /// A `rekey_entry` into the chain, and on to the members connected.
    pub(crate) fn submit_rekey(&self, e: Entry) -> R<()> {
        self.submit(e).map(|_| ())
    }

    /// The network's mailboxes, in place of what it had: `host`, a member,
    /// named one (`add`) or taken off. Naming one widens what a member
    /// sees of the others' traffic, so it takes a window host with the
    /// person's presence, from the window (the decided table); taking one
    /// off narrows, and is a window host's or the mailbox's own, from
    /// anywhere.
    pub async fn set_mailboxes(&self, host: &str, add: bool, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        let view = n.roster().view();
        if view.member(host).is_none() {
            return Err(NetError::new(404, "no-host", format!("{host} isn't in the network")));
        }
        let had = view.mailboxes.clone();
        let mut to = had.clone();
        if add {
            if !to.iter().any(|m| m == host) {
                to.push(host.to_string());
            }
        } else {
            to.retain(|m| m != host);
        }
        if to == had {
            return Ok(self.state());
        }
        if to.len() > xenonet_log::network::MAX_MAILBOXES {
            return Err(NetError::bad(format!("a network names at most {} mailboxes", xenonet_log::network::MAX_MAILBOXES)));
        }
        if !view.is_window(&net.host) && (host != net.host || add) {
            return Err(NetError::new(403, "not-window", "a network's mailboxes are named from a window host; a mailbox may step down itself"));
        }
        let me = n.roster().host_ref(&net.host).ok_or_else(|| NetError::new(410, "removed", "this machine isn't in the network"))?;
        let mut e = n.draft(Op::Mailboxes, me);
        e.mailboxes = Some(to);
        let presence = match e.presence_for(&view) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), &format!("let {host} keep your network's mail for machines that are away"), n.now() as i64)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.decide_entry(e, "this machine asked to name a mailbox; another window host approves it")
    }

    /// `network/state/v1`: the network, this host and how it stands in it,
    /// the hosts, and the invites open.
    pub fn state(&self) -> Value {
        let n = &self.shared;
        let Some(net) = n.network() else {
            return json!({
                "schema": "network/state/v1",
                "network": null,
                "self": null,
                "link": null,
                "hosts": [],
                "rosterHead": null,
                "rosterMismatch": [],
                "titlesOnly": false,
                "shareLoad": true,
                "start": "you",
                "wake": "consent",
                "networks": [],
                "relays": [],
                "mailboxes": [],
            });
        };
        let (view, head) = {
            let r = n.roster();
            (r.view(), r.head())
        };
        let s = n.status();
        let peers = n.peers();
        let me_you = *n.me_you.locked();
        let me_agents = n.me_agents.locked().clone();
        let key_id = n.key().map(|k| k.key_id()).unwrap_or_default();
        let now = n.now();
        // How each member stands on link/2: the path its connection takes,
        // how its address was found, and its clock against this one's.
        let t2 = n.link2();
        let on_link = t2.as_ref().map(|t| t.links.peers()).unwrap_or_default();
        let found = t2.as_ref().map(|t| t.found.locked().clone()).unwrap_or_default();
        let mut hosts = Vec::new();
        let mut ids: Vec<&String> = view.members.keys().collect();
        ids.sort_by_key(|h| (**h != net.host, (*h).clone()));
        for id in ids {
            let Some(m) = view.member(id) else { continue };
            let me = *id == net.host;
            let p = peers.get(id);
            let platform = if me { Some(n.opts.platform.clone()) } else { p.and_then(|p| p.platform.clone()) };
            hosts.push(json!({
                "id": id,
                "keyId": m.keys.key_id(),
                "window": view.is_window(id),
                "founder": *id == view.founder,
                "online": if me { true } else { p.is_some_and(|p| p.online) },
                "seen": if me { Some(now as i64) } else { p.and_then(|p| p.seen).map(|x| x as i64) },
                "agents": if me { me_agents.len() } else { p.map_or(0, |p| p.agents.len()) },
                "waitsOnYou": if me { me_agents.values().filter(|w| **w).count() } else { p.map_or(0, |p| p.agents.values().filter(|w| **w).count()) },
                "you": if me { me_you.0 } else { p.is_some_and(|p| p.you) },
                "youAt": if me { me_you.1 } else { p.and_then(|p| p.you_at) }.map(|t| t.round() as i64),
                "platform": platform,
                "self": me,
                // Who may start and wake sessions there, and what it takes:
                // null and none for a host whose XenoNet doesn't say.
                "reach": n.reach_of(id).to_json(),
                "can": if me { json!(xenonet_model::ops::CAN) } else { json!(p.map(|p| p.can.clone()).unwrap_or_default()) },
                "path": if me { None } else { on_link.get(&crate::endpoint::peer_id(&m.keys)).and_then(|l| l.path) },
                "found": if me { None } else { found.get(&(net.id.clone(), id.clone())).map(|f| f.iter().copied().collect::<Vec<_>>()) },
                "skewMs": if me { None } else { on_link.get(&crate::endpoint::peer_id(&m.keys)).and_then(|l| l.skew_ms) },
                // This machine's entries a mailbox holds for it, not
                // delivered yet.
                "held": if me { None } else { Some(n.mail_standing(&net.id, id).held) },
            }));
        }
        let reach = n.reach_of(&net.host);
        // Who this machine acts as, as the window shows it: the person, the
        // device here and how it asks, and whether it's the network's owner.
        let id = &n.identity;
        let here = id.device_id();
        let devices: Vec<Value> = id
            .chain()
            .person()
            .map(|p| {
                p.devices
                    .values()
                    .map(|d| json!({ "id": d.id, "label": d.label, "kind": d.kind.as_str(), "active": d.active(), "here": here.as_deref() == Some(d.id.as_str()) }))
                    .collect()
            })
            .unwrap_or_default();
        let person = json!({
            "id": id.person_id(),
            "owner": view.owner.is_some() && view.owner == id.person_id(),
            "device": here,
            "kind": id.device_kind().map(xenonet_log::person::DeviceKind::as_str),
            "acts": id.can_act(),
            "rootSealed": id.root_sealed(),
            "devices": devices,
        });
        let mut out = json!({
            "schema": "network/state/v1",
            "network": { "id": net.id, "name": net.name },
            "self": { "id": net.host, "keyId": key_id, "window": view.is_window(&net.host), "founder": view.founder == net.host, "hlcAheadMs": n.hlc_ahead_ms(), "link2": t2.is_some() },
            // How this machine stands in the network: live while its node
            // runs, whichever members it reaches (each host's online).
            "link": { "state": s.state, "since": s.since.map(|x| x as i64), "reason": s.reason },
            "hosts": hosts,
            "rosterHead": head,
            "rosterMismatch": n.roster_mismatch(),
            "frozen": n.frozen(&net),
            "titlesOnly": net.titles_only,
            "shareLoad": net.share_load,
            "start": reach.start,
            "wake": reach.wake,
            "delegated": reach.delegated.as_ref().map(|(by, at)| json!({ "by": by, "at": at })),
            "person": person,
            "relays": view.relays,
            "mailboxes": view.mailboxes,
        });
        out["sharing"] = n.sharing(&net.id).to_json();
        out["inContact"] = json!(n.synced_at().is_some_and(|t| n.now() as i64 - t <= IN_CONTACT_MS));
        out["invites"] = self.invites.json(now as i64);
        if let Some(r) = s.removed {
            out["removed"] = r;
        }
        // Settings proposals show in their machine's reach instead.
        let proposals: Vec<Value> = n
            .proposals()
            .iter()
            .filter(|p| p.op != Op::Settings)
            .map(|p| {
                json!({
                    "id": p.id(),
                    "op": p.op.as_str(),
                    "host": p.host.id,
                    "window": p.host.window,
                    "signers": p.signers.iter().map(|s| s.by.clone()).collect::<Vec<_>>(),
                    "at": p.at,
                })
            })
            .collect();
        out["proposals"] = Value::Array(proposals);
        out
    }

    /// Approves an entry waiting for approval, by its id (`Entry::id`, as
    /// the state lists it): this host signs exactly that entry too, with
    /// the person's presence where it admits or widens (from the window).
    pub async fn approve(&self, id: &str, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        n.sync_chain().await;
        if !n.roster().view().is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "approving is a window host's"));
        }
        let Some(mut e) = n.proposals().into_iter().find(|p| p.id() == id) else {
            return Err(NetError::new(404, "nothing", "nothing waits for approval by that id: xenonet net status lists what does"));
        };
        let (op, host) = (e.op.as_str(), e.host.id.clone());
        let presence = match e.presence_for(&n.roster().view()) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), &format!("approve {op} for {host} in your XenoNet network"), n.now() as i64)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.submit(e)?;
        n.log("net.approve", &format!("this machine approved {op} for {host}"));
        Ok(self.state())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.invites.close();
    }
}

// join/2: `invite` and `join_invite`.
#[path = "node_join.rs"]
mod join2;

// link/2: what this machine sends and takes, machine to machine.
#[path = "node_link.rs"]
pub mod link2n;

#[path = "node_svc.rs"]
mod svc2n;

// The network's chain on link/2.
#[path = "node_chain.rs"]
mod chain2;
