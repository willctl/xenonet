//! This machine in its network (docs/design/network.md, sections 3, 4 and
//! 9): its key, its copy of the roster, its link to the gateway, and, on
//! the gateway's own host, the gateway. It's what the engine sees through
//! `xenonet_model::ops::Net`, and it calls the engine's `Sink` with
//! what arrives, after checking each frame's signature against its own
//! roster and opening it.
//!
//! **Its feed.** What the engine sends as messages, receipts and handoffs
//! are entries of this machine's feed in the network (xenonet-log's
//! `feed`), signed by its machine signing key and carried in `message`
//! frames (`network/message/v3`): a message and a receipt loose, to the
//! host they're for, and a handoff kept, to every member, in one gapless
//! sequence. Where it writes, and the kept entries it wrote, are kept
//! through `Persist`; each member acks the kept entries it applied, and
//! what a member hasn't acked goes to it again when the link comes back,
//! or when it asks with a `want` on a gap. An entry from another member is
//! taken only when its author's key is that member's in the roster, and
//! goes to the engine's `Feeds`, which applies it once.
//!
//! Nothing here waits on the gateway: `send` puts frames in the outbox and
//! returns, and the link sends them when it can. The outbox is in memory:
//! what has to outlive a restart is sent again from the store.
//!
//! **link/2** (`node_link.rs`) runs beside the gateway's link when the
//! options say so: the same entries and presence go both ways, and the copy
//! that comes second, the other way, is dropped by its entry's id before
//! anything in it is read.

use crate::codec::{frame_id, is_b64};
use crate::files::{address_kind, Dir, NetFile, Pin, GATEWAY_DB, KEY};
use crate::frame::{host_frame, open_frame, Draft, Frame};
use crate::gateway::{join_body, Gateway, NetRequest};
use crate::ground::Facts;
use crate::gwstore::{Caps, GwStore, TicketRow};
use crate::identity::{self, Identity};
use crate::keys::{HostKey, PublicKeys};
use crate::link::{self, Transport};
use crate::link2;
use crate::persist::{Persist, WriterAt};
use crate::roster::{Entry, HostRef, NetworkPolicy, Offered, Op, Removal, Roster};
use crate::sink::{entry_kind, LogFn, SinkSlot};
use crate::svc::{self, Answer, Services};
use crate::{request, ticket, NetError, R};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, Weak};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use xenonet_id::vault::Vault;
use xenonet_log::feed::{Class, FeedEntry, Pos, Writer};
use xenonet_model::ids::is_host_id;
use xenonet_model::ops::{Host, Incoming, Kind, Me, Net, Outgoing, Sink, To};
use xenonet_os::sync::{Locked, ReadWrite};

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The link's and the gateway's timings: the design's, or shorter ones for
/// tests.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub ping_ms: u64,
    pub stream_ms: u64,
    /// The most a stream may go without a line before the link drops it
    /// and is offline: three pings missed.
    pub idle_ms: u64,
    pub poll_ms: u64,
    /// How long a link request waits for the gateway's answer, its headers
    /// and its hello: a long-poll's wait (`poll_ms`) and time to spare.
    /// Past it the link drops the connection and is offline.
    pub answer_ms: u64,
    pub backoff_min_ms: f64,
    pub backoff_max_ms: f64,
    /// How long a gateway that was moved keeps answering, so the hosts
    /// linked to it hear of the move first.
    pub handover_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { ping_ms: 15_000, stream_ms: 600_000, idle_ms: 45_000, poll_ms: 25_000, answer_ms: 35_000, backoff_min_ms: 2000.0, backoff_max_ms: 60_000.0, handover_ms: 30_000 }
    }
}

pub struct Options {
    /// `<state>/network`.
    pub dir: PathBuf,
    pub version: String,
    /// darwin, linux or win32.
    pub platform: String,
    pub desktop: bool,
    pub clock: Clock,
    /// The gateway's queues.
    pub caps: Caps,
    pub outbox: Caps,
    pub timing: Timing,
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
    /// Whether link/2 runs beside the gateway's link: the service's own
    /// node does; a test's, only when it says.
    pub link: bool,
    /// Finding members on the LAN by mDNS, as the machine's setting says.
    pub lan: crate::endpoint::LanLookup,
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
            caps: Caps::default(),
            outbox: Caps::default(),
            timing: Timing::default(),
            log,
            svc_ms: svc::TIMEOUT_MS,
            vault: None,
            bind: None,
            link: false,
            lan: crate::endpoint::LanLookup::Unasked,
        }
    }
}

pub use xenonet_text::time::now_ms;

/// How the link stands.
#[derive(Debug, Clone)]
pub struct Status {
    /// off, connecting, live, offline, refused or removed. Live means the
    /// link hears the gateway: a stream that brings nothing for the idle
    /// time, or a request whose answer doesn't come in time, puts it
    /// offline (link.rs).
    pub state: &'static str,
    pub since: Option<f64>,
    pub reason: Option<String>,
    /// The gateway's clock minus this one's, learned from a refusal.
    pub offset_ms: f64,
    /// How far this clock was off the gateway's at the last hello.
    pub skew_ms: Option<i64>,
    /// The key id the gateway proved in its last hello.
    pub gateway_key: Option<String>,
    pub removed: Option<Value>,
    pub long_poll: bool,
    pub last_ok: Option<f64>,
}

impl Default for Status {
    fn default() -> Self {
        Status { state: "off", since: None, reason: None, offset_ms: 0.0, skew_ms: None, gateway_key: None, removed: None, long_poll: false, last_ok: None }
    }
}

/// A change to this machine's settings, made at its own keyboard: each
/// setting that's Some is set.
#[derive(Debug, Clone, Default)]
pub struct Change {
    pub titles_only: Option<bool>,
    pub share_load: Option<bool>,
    /// The gateway's LAN address, or None to take it away.
    pub lan: Option<Option<String>>,
    /// Who may start sessions here: off, you, network or delegate. A
    /// settings entry in the chain.
    pub start: Option<String>,
    /// Who may wake them: consent or network.
    pub wake: Option<String>,
}

/// Another host, as presence and the gateway's pings tell of it.
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

/// A ticket this host made while it isn't the gateway's, which it signs
/// its new host in for when the gateway asks. The ticket itself stays in
/// memory, never on disk, and goes with its ten minutes.
pub struct PendingTicket {
    code: String,
    pub host: String,
    pub window: bool,
    pub expires: i64,
}

/// A batch on its way up. A relay batch carries only the map's frames, or
/// one service's request or answer, and goes as `POST frames?relay=1`.
pub struct Batch {
    pub ids: Vec<String>,
    pub body: String,
    pub relay: bool,
    /// The svc frame it carries, alone.
    pub svc: Option<Frame>,
}

pub struct Shared {
    /// This node itself, for the tasks a frame starts.
    this: Weak<Shared>,
    opts: Options,
    pub dir: Dir,
    key: RwLock<Option<Arc<HostKey>>>,
    net: RwLock<Option<NetFile>>,
    roster: RwLock<Roster>,
    /// The frames that wait for the gateway to take them.
    outbox: Mutex<Outbox>,
    /// The gateway's epoch this host last saw, and the last seq in it, as
    /// `Persist` keeps them.
    cursor: Mutex<(Option<String>, u64)>,
    /// This machine's feed in its network, as its author.
    writer: Mutex<Option<Writer>>,
    sink: Arc<SinkSlot>,
    status: Mutex<Status>,
    peers: Mutex<BTreeMap<String, Peer>>,
    wake: Notify,
    volatile: Mutex<Vec<Frame>>,
    /// The map's frames waiting to go, in relay batches of their own.
    relay_out: Mutex<Vec<Frame>>,
    /// The relay and svc frames that came lately, and the oldest time a
    /// new one may have.
    relay_ids: Mutex<Recent>,
    /// Services' requests and answers waiting to go, each in a relay batch
    /// of its own.
    svc_out: Mutex<VecDeque<Frame>>,
    /// The services this host published, and the requests it waits on.
    svc: Services,
    gateway: RwLock<Option<Arc<Gateway>>>,
    /// The person on this machine, and the people's chains it holds.
    identity: Arc<Identity>,
    link_written: Mutex<f64>,
    /// Entries signed by too few, waiting for this host or another to approve.
    proposals: Mutex<Vec<Entry>>,
    /// The gateway moved: the service starts the link again, toward it.
    relink: Notify,
    /// Tickets made here for the gateway to keep, until they're used or
    /// lapse.
    pending: Mutex<Vec<PendingTicket>>,
    /// `add` entries this host signed for its tickets, for the link to take
    /// to the gateway.
    signed: Mutex<Vec<Entry>>,
    /// What this host's own presence last said of the person: whether
    /// they're here, and when they last worked at its console.
    me_you: Mutex<(bool, Option<f64>)>,
    /// This host's own agents as its presence lists them, and whether each
    /// waits on the person, for its row: `peers` has only the others.
    me_agents: Mutex<BTreeMap<String, bool>>,
    /// link/2, while it runs.
    link2: RwLock<Option<Arc<link2n::Transport2>>>,
    /// The entries taken lately and the way each came, so the second of
    /// the two copies dual sending makes is dropped.
    taken: Mutex<link2n::Taken>,
    clocks: Mutex<link2n::Clocks>,
}

/// How full the outbox is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Size {
    pub frames: u64,
    pub bytes: u64,
    pub oldest: Option<i64>,
}

/// The frames that wait for the gateway, oldest first: entries of this
/// machine's feed, acks and wants, and the person's answers. A frame stays
/// until the gateway takes it or refuses it for good; past the caps the
/// oldest go, and are counted. Kept entries go again until each member
/// acks them, and the engine sends again the messages that have no stored
/// receipt, so what a full outbox or a restart loses comes back.
#[derive(Default)]
struct Outbox {
    frames: VecDeque<(String, String, String, i64)>,
    bytes: u64,
    dropped: u64,
}

impl Outbox {
    /// A frame for `host`, as its line, made at `now`.
    fn push(&mut self, f: &Frame, host: &str, now: i64, caps: &Caps) {
        let line = f.line().trim_end().to_string();
        self.bytes += line.len() as u64;
        self.frames.push_back((f.id.clone(), host.to_string(), line, now));
        let floor = now - caps.keep_ms as i64;
        while let Some((_, _, l, at)) = self.frames.front() {
            if self.frames.len() as u64 <= caps.frames && self.bytes <= caps.bytes && *at >= floor {
                break;
            }
            self.bytes -= l.len() as u64;
            self.frames.pop_front();
            self.dropped += 1;
        }
    }

    /// The oldest frames, up to `max_bytes` (one at least) and
    /// `max_frames`: (id, line).
    fn batch(&self, max_bytes: u64, max_frames: u64) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let mut bytes = 0;
        for (id, _, line, _) in self.frames.iter().take(usize::try_from(max_frames).unwrap_or(usize::MAX)) {
            let size = line.len() as u64;
            if !out.is_empty() && bytes + size > max_bytes {
                break;
            }
            bytes += size;
            out.push((id.clone(), line.clone()));
        }
        out
    }

    /// The gateway took these, or refused them for good.
    fn done(&mut self, ids: &[String]) {
        let mut bytes = self.bytes;
        self.frames.retain(|(id, _, l, _)| {
            let keep = !ids.contains(id);
            if !keep {
                bytes -= l.len() as u64;
            }
            keep
        });
        self.bytes = bytes;
    }

    /// Frames for `host` go: it was removed.
    fn forget_host(&mut self, host: &str) {
        self.frames.retain(|(_, h, _, _)| h != host);
        self.bytes = self.frames.iter().map(|(_, _, l, _)| l.len() as u64).sum();
    }

    fn size(&self) -> Size {
        Size { frames: self.frames.len() as u64, bytes: self.bytes, oldest: self.frames.front().map(|f| f.3) }
    }
}

/// How many kept entries are read at a time to go to a member again.
const RESEND: usize = 500;

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

/// The most this host's clock may be set off its own by the gateway's.
const MAX_OFFSET_MS: f64 = 24.0 * 3600e3;
/// A host frame dated this far past now, by the gateway's clock, is
/// dropped; one older than the frame ids kept (a week) is too, since its
/// id could be forgotten and the frame taken again.
const AHEAD_MS: f64 = 5.0 * 60e3;
const BATCH_BYTES: u64 = 900 << 10;
const BATCH_FRAMES: u64 = 500;
/// A relay body's JSON, at most: sealed and in base64url, a frame of it
/// fits the gateway's relay batch (`gateway::RELAY_LIMIT`).
pub const RELAY_BODY: usize = 8 << 20;
/// A relay batch's frames, at most this many bytes of lines, past the first.
const RELAY_BATCH_BYTES: usize = 12 << 20;
/// How long a relay or svc frame is news: one older, by the gateway's
/// clock, is dropped.
const RELAY_MS: f64 = 120e3;
/// Relay and svc frame ids remembered at most.
const RELAY_IDS: usize = 16384;

/// The relay and svc frames that came lately, each id with its time, and
/// the floor: the oldest time a new one may have. Each id is kept until
/// the floor passes its time, so a frame is taken once however far ahead
/// it was dated. The floor only rises, to now less RELAY_MS by the
/// gateway's clock: the gateway sets this host's offset, and setting it
/// back mustn't make a frame it already passed on fresh again.
#[derive(Default)]
struct Recent {
    ids: HashMap<String, f64>,
    floor: f64,
}

impl Recent {
    /// Whether a frame with `id`, dated `at`, is new and fresh by the
    /// gateway's clock `gw_now`; it's noted when it is.
    fn take(&mut self, id: &str, at: f64, gw_now: f64) -> bool {
        self.floor = self.floor.max(gw_now - RELAY_MS);
        if at < self.floor || self.ids.contains_key(id) {
            return false;
        }
        if self.ids.len() >= RELAY_IDS {
            let floor = self.floor;
            self.ids.retain(|_, t| *t >= floor);
        }
        if self.ids.len() >= RELAY_IDS {
            // Still full of fresh ones: the older half go, and the floor
            // rises past them, so one of them is refused rather than taken
            // twice.
            let mut times: Vec<f64> = self.ids.values().copied().collect();
            let mid = times.len() / 2;
            let cut = *times.select_nth_unstable_by(mid, f64::total_cmp).1;
            self.ids.retain(|_, t| *t > cut);
            self.floor = self.floor.max(cut + 1.0);
            if at < self.floor {
                return false;
            }
        }
        self.ids.insert(id.to_string(), at);
        true
    }
}

/// The reserved target a person's chain entries travel to, as message
/// entries between machines: taken by the node, never by the engine.
pub const PERSON_TARGET: &str = "xenonet:person";

/// A message entry's body to `xenonet:person` carrying `text`.
fn person_body(text: &str) -> Value {
    json!({ "target": PERSON_TARGET, "body": text })
}

/// What a message frame carries (`network/message/v3`).
pub const MESSAGE_SCHEMA: &str = "network/message/v3";

/// A message frame's body carrying `e`, with `authority` when its act
/// rests on more than this machine's word.
pub fn entry_body(e: &FeedEntry, authority: Option<&Value>) -> Value {
    let mut b = json!({ "schema": MESSAGE_SCHEMA, "entry": e.to_json() });
    if let Some(a) = authority.filter(|a| a.as_object().is_some_and(|o| !o.is_empty())) {
        b["authority"] = a.clone();
    }
    b
}

fn not_in() -> NetError {
    NetError::new(409, "no-network", "this machine isn't in a XenoNet network")
}

impl Shared {
    pub fn now(&self) -> f64 {
        (self.opts.clock)()
    }

    /// Now by the gateway's clock, as frames and requests are timed.
    pub fn gateway_now(&self) -> f64 {
        self.now() + self.status.locked().offset_ms
    }

    pub fn timing(&self) -> Timing {
        self.opts.timing
    }

    pub fn caps(&self) -> Caps {
        self.opts.caps
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

    /// What the person shares into `network`: what they said, or the
    /// default, everything for the first network and nothing for the rest.
    pub fn sharing(&self, network: &str) -> crate::share::Sharing {
        let first = self.network().is_some_and(|n| n.id == network);
        self.opts.persist.sharing(network).ok().flatten().unwrap_or_else(|| crate::share::Sharing::default_for(first))
    }

    /// What each network `host` is in with this machine shares: what it
    /// may be shown, the union of them.
    pub fn sharings_with(&self, host: &str) -> Vec<crate::share::Sharing> {
        let Some(net) = self.network() else { return Vec::new() };
        let member = self.roster().view_ref().is_some_and(|v| v.member(host).is_some());
        if member {
            vec![self.sharing(&net.id)]
        } else {
            Vec::new()
        }
    }

    pub fn key(&self) -> Option<Arc<HostKey>> {
        self.key.reading().clone()
    }

    pub fn roster(&self) -> RwLockReadGuard<'_, Roster> {
        self.roster.reading()
    }

    /// The person on this machine.
    pub fn identity(&self) -> &Arc<Identity> {
        &self.identity
    }

    /// When governance (the chains) was last synced from another member:
    /// now on the gateway's host, which every entry passes through; else
    /// the gateway's last word on the link.
    pub fn synced_at(&self) -> Option<i64> {
        self.network()?;
        if self.is_gateway() {
            return Some(self.gateway_now() as i64);
        }
        let s = self.status.locked();
        s.last_ok.map(|t| (t + s.offset_ms) as i64)
    }

    /// The facts a decision here rests on, as of now.
    pub fn facts(&self) -> Facts {
        let net = self.network();
        let machine = net.as_ref().map_or(crate::ground::LOCAL.to_string(), |n| n.host.clone());
        let machine = xenonet_authz::MachineId::new(&machine).unwrap_or_else(|_| xenonet_authz::MachineId::new(crate::ground::LOCAL).expect("a word"));
        Facts { machine, view: net.and_then(|_| self.roster().view_ref().cloned()), identity: self.identity.clone(), synced_at: self.synced_at(), now: self.gateway_now() as i64 }
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
    pub fn draft(&self, op: Op, host: HostRef, gateway: Option<crate::roster::GatewayAt>) -> Entry {
        let r = self.roster();
        let mut e = r.draft(op, host, gateway, self.gateway_now() as i64);
        e.person_head = r.view_ref().and_then(|v| v.owner.as_deref()).and_then(|o| self.identity.chain_of(o)).and_then(|c| c.head());
        e
    }

    /// Pushes entries of this machine's person's chain to the network's
    /// other machines, as `message` frames to `xenonet:person`.
    fn push_person(&self, from: u64) {
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

    pub fn gateway(&self) -> Option<Arc<Gateway>> {
        self.gateway.reading().clone()
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

    /// Whether this host is the network's gateway.
    pub fn is_gateway(&self) -> bool {
        let me = self.net.reading().as_ref().map(|n| n.host.clone());
        me.is_some_and(|h| self.roster().view_ref().is_some_and(|v| v.gateway == h))
    }

    /// A removed host's last keys and its removal.
    pub fn removed_keys(&self, host: &str) -> Option<(PublicKeys, Removal)> {
        let r = self.roster();
        let rm = r.view_ref()?.removal(host)?.clone();
        Some((rm.keys, rm))
    }

    pub fn cursor(&self) -> (Option<String>, u64) {
        self.cursor.locked().clone()
    }

    fn set_cursor(&self, epoch: &str, seq: u64) {
        *self.cursor.locked() = (Some(epoch.to_string()), seq);
        if let Some(net) = self.network() {
            if let Err(e) = self.opts.persist.put_cursor(&net.id, epoch, seq) {
                self.log("net.cursor", &format!("where this host stands in the gateway's stream wasn't kept: {}", e.message));
            }
        }
    }

    pub fn outbox(&self) -> Size {
        self.outbox.locked().size()
    }

    /// A request to the gateway, signed: the request and its nonce.
    pub fn signed_request(&self, method: &str, target: &str, body: Vec<u8>) -> R<(NetRequest, String)> {
        let net = self.network().ok_or_else(not_in)?;
        let key = self.key().ok_or_else(not_in)?;
        let nonce = crate::codec::nonce()?;
        let time = self.gateway_now() as i64;
        let headers = request::sign(&key, &net.id, &net.host, method, target, time, &nonce, &body);
        Ok((NetRequest { method: method.into(), target: target.into(), headers, body, source: None }, nonce))
    }

    // ---------- how the link stands ----------

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
            // The map's frames that waited are old news by the time the
            // link is back: a console asks for a fresh snapshot. No answer
            // comes to a service's request either.
            self.relay_out.locked().clear();
            self.svc_out.locked().clear();
            self.svc.fail_all(&Answer::refused(503, "offline", "this machine's link to the network's gateway dropped before the answer came"));
            self.sink.linked(false);
        }
        if changed {
            self.write_link(true);
        }
    }

    /// The link's first try since it started. A try after that leaves the
    /// state as it stands: live stays live across a stream's routine end,
    /// and offline or refused keeps its reason and since while the link
    /// tries again, until the gateway's hello or another failure.
    pub fn connecting(&self) {
        if self.status.locked().state == "off" {
            self.set("connecting", None);
        }
    }

    pub fn offline(&self, why: &str) {
        if !self.is_removed() {
            self.set("offline", Some(why));
        }
    }

    pub fn refused(&self, why: &str) {
        if !self.is_removed() {
            self.set("refused", Some(why));
        }
    }

    pub fn set_long_poll(&self) {
        self.status.locked().long_poll = true;
        self.log("net.link", "the gateway's stream went quiet twice: asking it a request at a time");
    }

    /// The pinned gateway's keys, while the roster still holds that host
    /// with them. A gateway's host removed from the roster, or one whose
    /// keys changed, isn't this network's gateway any more, whatever it
    /// signs.
    pub fn pinned_gateway(&self, net: &NetFile) -> Result<PublicKeys, String> {
        let keys = PublicKeys::from_b64(&net.gateway.sign, &net.gateway.seal).ok_or("the pinned gateway key isn't readable")?;
        let held = self.roster().view_ref().and_then(|v| v.member(&net.gateway.host).map(|m| m.keys == keys));
        match held {
            Some(true) => Ok(keys),
            Some(false) => Err(format!("the gateway, {}, has other keys in the roster than the ones this host pinned", net.gateway.host)),
            None => Err(format!("the gateway, {}, isn't in the network's roster any more", net.gateway.host)),
        }
    }

    /// The gateway's time from its refusal of the request made with
    /// `nonce`: only from a frame the pinned gateway's key signed, for this
    /// host, over that nonce. Anything else is refused, so a gateway's
    /// address taken over, or a LAN in the middle, can't move this host's
    /// clock.
    pub fn clock_answer(&self, v: &Value, nonce: &str) -> Result<f64, String> {
        let net = self.network().ok_or("this machine left its network")?;
        let keys = self.pinned_gateway(&net)?;
        let f = v.get("frame").and_then(Value::as_str).and_then(|l| Frame::parse_line(l).ok()).ok_or("the gateway refused this host's time without a signed answer")?;
        if f.kind != "ping" || f.from != net.gateway.host || f.to != net.host || !f.verify(&net.id, &keys) {
            return Err("the gateway's answer about the time didn't check against its key".into());
        }
        let body = f.body_json().map_err(|e| e.message)?;
        if body.get("nonce").and_then(Value::as_str) != Some(nonce) {
            return Err("the gateway's answer about the time was for another request".into());
        }
        body.get("now").and_then(Value::as_f64).ok_or_else(|| "the gateway's answer about the time has no time".into())
    }

    /// The gateway's time, from a refusal: this host signs with it from now,
    /// unless it's more than a day off this clock, which is refused.
    pub fn learn_offset(&self, gateway_now: f64) {
        let offset = gateway_now - self.now();
        if offset.abs() > MAX_OFFSET_MS {
            self.refused("the gateway's clock is more than a day off this one's: set this computer's clock, or check the gateway's");
            return;
        }
        {
            let mut s = self.status.locked();
            s.offset_ms = offset;
            s.skew_ms = Some(offset as i64);
        }
        self.log("net.clock", &format!("this computer's clock is {} s off the gateway's: signing with its time", (offset / 1000.0).round()));
        self.write_link(true);
    }

    /// `link.json`, which the doctor reads: at once when the state changed,
    /// else at most every ten seconds.
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
        let (o, dropped) = {
            let out = self.outbox.locked();
            (out.size(), out.dropped)
        };
        let gateway = self.gateway();
        let queues: Vec<Value> = gateway.as_ref().map(|g| g.queues().into_iter().map(|(h, f, b)| json!({ "host": h, "frames": f, "bytes": b })).collect()).unwrap_or_default();
        let join_closed = gateway.as_ref().and_then(|g| g.join_closed_until(t)).map(|x| x as i64);
        let v = json!({
            "state": s.state,
            "since": s.since.map(|x| x as i64),
            "reason": s.reason,
            "skewMs": s.skew_ms,
            "gatewayKeyId": s.gateway_key,
            "pinnedKeyId": net.gateway.key_id,
            "address": net.gateway.address,
            "kind": net.gateway.address.as_deref().map(address_kind),
            "gateway": self.is_gateway(),
            "removed": s.removed,
            "longPoll": s.long_poll,
            "lastOk": s.last_ok.map(|x| x as i64),
            "outbox": { "frames": o.frames, "bytes": o.bytes, "oldest": o.oldest, "dropped": dropped },
            "queues": queues,
            "joinClosedUntil": join_closed,
            "rosterMismatch": self.roster_mismatch(),
            "at": t as i64,
        });
        let _ = self.dir.write_link(&net.id, &v);
    }

    pub fn woken(&self) -> tokio::sync::futures::Notified<'_> {
        self.wake.notified()
    }

    // ---------- the roster ----------

    /// Adds an entry to this host's roster and acts on it: a removed host's
    /// frames go, and the engine hears; this host removed stops.
    pub fn append(&self, e: Entry) -> R<Offered> {
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
        match e.op {
            Op::Remove => {
                self.peers.locked().remove(&e.host.id);
                self.outbox.locked().forget_host(&e.host.id);
                if me.as_deref() == Some(e.host.id.as_str()) {
                    let by = e.signers.iter().map(|s| s.by.clone()).collect::<Vec<_>>();
                    self.set_removed(json!({ "by": by, "at": e.at, "n": e.n }));
                } else {
                    self.sink.removed(&e.host.id);
                }
            }
            Op::Gateway => self.follow_gateway(&e),
            // A ticket made here did its work: it no longer waits.
            Op::Add => self.pending.locked().retain(|p| p.host != e.host.id),
            _ => {}
        }
        self.members_changed();
        Ok(offered)
    }

    /// A `gateway` entry moved the gateway: the pin follows it, and the
    /// service starts the link again (this host's gateway with it, when
    /// it's this host that took over, or stopped, when it's this host that
    /// gave it up).
    fn follow_gateway(&self, e: &Entry) {
        {
            let mut net = self.net.writing();
            if let Some(n) = net.as_mut() {
                n.gateway = Pin {
                    host: e.host.id.clone(),
                    key_id: e.host.keys.key_id(),
                    sign: e.host.keys.sign_b64(),
                    seal: e.host.keys.seal_b64(),
                    address: e.gateway.as_ref().map(|g| g.address.clone()),
                };
                let _ = self.dir.write_network(n);
            }
        }
        self.log("net.gateway", &format!("the network's gateway is now {}", e.host.id));
        self.relink.notify_one();
    }

    /// Wakes when the gateway moved, or this host followed it by hand.
    pub fn relinked(&self) -> tokio::sync::futures::Notified<'_> {
        self.relink.notified()
    }

    /// Entries waiting for approval, as the gateway passed them on.
    pub fn note_proposals(&self, entries: Vec<Entry>) {
        let len = self.roster().len();
        let mut waiting = self.proposals.locked();
        waiting.retain(|p| p.n == len);
        for e in entries.into_iter().filter(|e| e.n == len) {
            match waiting.iter_mut().find(|p| p.signing_bytes() == e.signing_bytes()) {
                Some(p) => p.merge_signers(&e),
                None => waiting.push(e),
            }
        }
        let extra = waiting.len().saturating_sub(crate::gateway::MAX_PROPOSALS);
        waiting.drain(..extra);
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

    /// The gateway's `removed` answer, checked against the pinned key.
    pub fn removed_frame(&self, f: &Frame) -> bool {
        let Some(net) = self.network() else { return false };
        let Ok(keys) = self.pinned_gateway(&net) else { return false };
        if f.kind != "removed" || f.from != net.gateway.host || f.to != net.host || !f.verify(&net.id, &keys) {
            return false;
        }
        let body = f.body_json().unwrap_or(Value::Null);
        if body.get("host").and_then(Value::as_str) != Some(net.host.as_str()) {
            return false;
        }
        self.set_removed(body);
        true
    }

    fn roster_mismatch(&self) -> Vec<String> {
        let head = self.roster().head();
        self.peers.locked().iter().filter(|(_, p)| p.roster.is_some() && p.roster != head).map(|(h, _)| h.clone()).collect()
    }

    // ---------- the down stream ----------

    /// The stream's first frame: the gateway proves its key over our
    /// nonce. Ok(true) when its roster is ahead of ours.
    pub fn hello(&self, f: &Frame, nonce: &str) -> Result<bool, String> {
        let net = self.network().ok_or("this machine left its network")?;
        let keys = self.pinned_gateway(&net)?;
        if f.kind != "hello" || f.from != net.gateway.host || !f.verify(&net.id, &keys) {
            return Err("the gateway didn't prove the key this host pinned".into());
        }
        let body = f.body_json().map_err(|e| e.message)?;
        if body.get("nonce").and_then(Value::as_str) != Some(nonce) || body.get("network").and_then(Value::as_str) != Some(net.id.as_str()) {
            return Err("the gateway's hello wasn't for this request".into());
        }
        let epoch = body.get("epoch").and_then(Value::as_str).filter(|e| is_b64(e)).ok_or("the gateway's hello has no epoch")?;
        let (had, _) = self.cursor();
        let mut lost = false;
        if had.as_deref() != Some(epoch) {
            self.set_cursor(epoch, 0);
            if had.is_some() {
                // A new store at the gateway: what the old one held may be
                // gone, so the engine sends again what has no receipt, and
                // the kept entries go again to whoever hasn't acked them.
                self.log("net.epoch", "the gateway's queues are new: resuming from the start");
                self.sink.gap();
                lost = true;
            }
        }
        let now = self.now();
        let was_live = {
            let mut s = self.status.locked();
            s.gateway_key = Some(keys.key_id());
            s.last_ok = Some(now);
            if let Some(gw_now) = body.get("now").and_then(Value::as_f64) {
                s.skew_ms = Some((gw_now - now) as i64);
            }
            s.state == "live"
        };
        if let Some(hosts) = body.get("hosts").and_then(Value::as_object) {
            let mut peers = self.peers.locked();
            for (id, h) in hosts {
                if id != &net.host {
                    peers.entry(id.clone()).or_default().platform = h.get("platform").and_then(Value::as_str).map(str::to_string);
                }
            }
        }
        if !was_live {
            self.set("live", None);
            self.sink.linked(true);
        }
        if !was_live || lost {
            self.feeds_linked();
        }
        self.wake.notify_one();
        let behind = body.get("rosterN").and_then(Value::as_u64).is_some_and(|n| n > self.roster().len());
        Ok(behind)
    }

    /// A frame from the stream after the hello. true when the roster needs
    /// fetching.
    pub fn frame(&self, f: &Frame) -> bool {
        let Some(net) = self.network() else { return false };
        let Some(key) = self.key() else { return false };
        if f.to != net.host {
            return false;
        }
        let fetch = if f.is_gateway_kind() { self.gateway_frame(&net, f) } else { self.host_frame(&net, &key, f) };
        if let Some(seq) = f.seq {
            let (epoch, last) = self.cursor();
            if let Some(e) = epoch.filter(|_| seq > last) {
                self.set_cursor(&e, seq);
            }
        }
        fetch
    }

    fn gateway_frame(&self, net: &NetFile, f: &Frame) -> bool {
        let keys = match self.pinned_gateway(net) {
            Ok(k) => k,
            Err(why) => {
                self.refused(&why);
                return false;
            }
        };
        if f.from != net.gateway.host || !f.verify(&net.id, &keys) {
            self.log("net.frame", &format!("a {} frame that the gateway's key didn't sign was dropped", f.kind));
            return false;
        }
        let body = f.body_json().unwrap_or(Value::Null);
        match f.kind.as_str() {
            "ping" => {
                let now = self.now();
                let linked: Vec<&str> = body.get("linked").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                let members: Vec<String> = self.roster().view().members.keys().filter(|h| **h != net.host).cloned().collect();
                let mut peers = self.peers.locked();
                let mut away = Vec::new();
                for h in members {
                    let p = peers.entry(h.clone()).or_default();
                    p.online = linked.contains(&h.as_str());
                    if p.online {
                        p.seen = Some(now);
                    } else {
                        away.push(h);
                    }
                }
                self.status.locked().last_ok = Some(now);
                drop(peers);
                // A host that isn't linked won't answer what it was asked.
                for h in away {
                    self.svc.fail_host(&h, &Answer::refused(503, "offline", format!("{h} went offline before it answered")));
                }
                self.write_link(false);
                false
            }
            "gap" => {
                self.log("net.gap", &format!("the gateway let frames go: {body}"));
                self.sink.gap();
                self.feeds_linked();
                false
            }
            "roster" => {
                let mut fetch = false;
                if let Some(p) = body.get("person") {
                    self.offer_person(p);
                }
                for e in body.get("entries").and_then(Value::as_array).into_iter().flatten() {
                    match Entry::parse(e).map_err(NetError::from).and_then(|e| self.append(e)) {
                        Ok(_) => {}
                        Err(e) if e.code == "roster-gap" => fetch = true,
                        Err(e) => self.log("net.roster", &format!("a roster entry was refused: {}", e.message)),
                    }
                }
                let proposed: Vec<Entry> = body.get("proposals").and_then(Value::as_array).into_iter().flatten().filter_map(|e| Entry::parse(e).ok()).collect();
                if !proposed.is_empty() {
                    self.note_proposals(proposed);
                }
                for ask in body.get("asks").and_then(Value::as_array).into_iter().flatten() {
                    match self.sign_ask(net, ask) {
                        Ok(e) => self.signed.locked().push(e),
                        Err(why) => self.log("net.ticket", &format!("the gateway asked this machine to sign a host in, and it didn't: {why}")),
                    }
                }
                fetch
            }
            "removed" => {
                self.removed_frame(f);
                false
            }
            _ => false,
        }
    }

    /// The gateway's ask to sign a new host in, for a ticket made here: an
    /// `add` entry this host signs only when the join request matches a
    /// ticket it keeps (the host id, the window choice, and keys the
    /// request proves it holds), with a proof keyed by the ticket itself,
    /// which the gateway can't make.
    fn sign_ask(&self, net: &NetFile, ask: &Value) -> Result<Entry, String> {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let host = s(ask, "host");
        let window = ask.get("window").and_then(Value::as_bool).ok_or("the ask says no window choice")?;
        let join = ask.get("join").cloned().unwrap_or(Value::Null);
        let now = self.now() as i64;
        let code = {
            let mut pending = self.pending.locked();
            pending.retain(|p| p.expires > now);
            let p = pending.iter().find(|p| p.host == host).ok_or_else(|| format!("this machine keeps no ticket for {host}"))?;
            if p.window != window {
                return Err(format!("the ticket for {host} was made with another window choice"));
            }
            p.code.clone()
        };
        let keys = PublicKeys::from_b64(&s(&join, "sign"), &s(&join, "seal")).ok_or("the join request has no keys")?;
        let key_id = keys.key_id();
        let (nonce, proof) = (s(&join, "nonce"), s(&join, "proof"));
        let sig = crate::codec::unb64(&s(&join, "sig")).unwrap_or_default();
        let ok = nonce.len() >= 16
            && ticket::check_join(&ticket::hash(&code), &nonce, &key_id, &proof)
            && ticket::check_maker(&code, &nonce, &key_id, &s(&join, "makerProof"))
            && keys.verify(crate::gateway::join_signing(&nonce, &key_id, &proof).as_bytes(), &sig);
        if !ok {
            return Err(format!("the join request for {host} didn't prove its ticket"));
        }
        {
            let r = self.roster();
            let v = r.view_ref().ok_or("this machine's roster is empty")?;
            if !v.is_window(&net.host) {
                return Err("this machine isn't a window host now".into());
            }
            match v.member(&host) {
                Some(m) if m.keys == keys => return Err(format!("{host} is in already")),
                Some(_) => return Err(format!("{host} is in the network already, with other keys")),
                None => {}
            }
        }
        let e = self.sign_in(&host, keys, window, &join).map_err(|e| e.message)?;
        self.log("net.ticket", &format!("this machine signed {host} into the network, with the ticket it made"));
        Ok(e)
    }

    /// Signs `host` in as this admin: pairs the device its join request
    /// brings into the person, when this machine acts as the network's
    /// owner, then signs the `add` entry with this device's presence. The
    /// person made the ticket at XenoNet's window, and this is its join,
    /// within the ticket's ten minutes, so it may ask them: once to admit,
    /// and to pair once more (twice where the root key is sealed to the
    /// device key).
    pub fn sign_in(&self, host: &str, keys: PublicKeys, window: bool, join: &Value) -> R<Entry> {
        let net = self.network().ok_or_else(not_in)?;
        let now = self.gateway_now() as i64;
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

    /// The `add` entries signed for this host's tickets, for the gateway.
    pub fn take_signed(&self) -> Vec<Entry> {
        std::mem::take(&mut *self.signed.locked())
    }

    /// The tickets made here that the gateway keeps, still in their ten
    /// minutes.
    pub fn pending_tickets(&self) -> Vec<(String, bool, i64)> {
        let now = self.now() as i64;
        let mut p = self.pending.locked();
        p.retain(|t| t.expires > now);
        p.iter().map(|t| (t.host.clone(), t.window, t.expires)).collect()
    }

    fn host_frame(&self, net: &NetFile, key: &HostKey, f: &Frame) -> bool {
        let (member, window) = {
            let r = self.roster();
            let Some(v) = r.view_ref() else { return false };
            match v.member(&f.from) {
                Some(m) => (m.clone(), v.is_window(&f.from)),
                None => {
                    self.log("net.frame", &format!("a frame from {}, which isn't in the roster, was dropped", f.from));
                    return false;
                }
            }
        };
        if !f.verify(&net.id, &member.keys) {
            self.log("net.frame", &format!("a frame from {} whose signature didn't check was dropped", f.from));
            return false;
        }
        let gw_now = self.gateway_now();
        let at = f.at as f64;
        if at > gw_now + AHEAD_MS || at < gw_now - crate::gwstore::WEEK_MS {
            self.log("net.frame", &format!("a {} frame from {} dated outside the week the network keeps was dropped", f.kind, f.from));
            return false;
        }
        let now = self.now();
        // The map's frames and services' are live: one older than RELAY_MS
        // is dropped, and their ids are kept in memory until then. A feed
        // entry is taken once by its place or id in the author's feed, and
        // the rest are taken again harmlessly: presence by its counter,
        // claims as a set, an answer once by its ask.
        if f.in_relay_batch() && !self.relay_ids.locked().take(&f.id, at, gw_now) {
            return false;
        }
        let mut body = match open_frame(&net.id, key, f) {
            Ok(b) => b,
            Err(e) => {
                self.log("net.frame", &format!("a frame from {} didn't open: {}", f.from, e.message));
                return false;
            }
        };
        {
            let mut peers = self.peers.locked();
            let p = peers.entry(f.from.clone()).or_default();
            p.seen = Some(now);
            p.online = true;
        }
        let Some(kind) = Kind::parse(&f.kind) else { return false };
        let at = f.at as f64 - self.status.locked().offset_ms;
        // The map's frames go to the relay and the hub, never the engine.
        // What a relay reads (the map, transcripts) is an admin's to read:
        // decided here, and the relay serves only an allowed read.
        if kind == Kind::Relay {
            let allowed = self.decide_read(&f.from);
            self.sink.relay(Incoming { network: net.id.clone(), from: f.from.clone(), kind, body, at: at.min(now), from_window_host: allowed });
            return false;
        }
        // Services' requests and answers are this node's own.
        if kind == Kind::Svc {
            self.svc_in(&f.from, body);
            return false;
        }
        // A message frame is a feed's: an entry, an ack or a want.
        if kind == Kind::Message {
            return self.message_in(&f.from, window, &body);
        }
        if kind == Kind::Presence && !self.note_presence(&f.from, &mut body) {
            return false;
        }
        self.sink.receive(Incoming { network: net.id.clone(), from: f.from.clone(), kind, body, at: at.min(now), from_window_host: window });
        false
    }

    /// A presence from `from` that came on link/2: kept for the hosts'
    /// list and passed to the engine as one through the gateway is, once
    /// whichever way it came first.
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

    /// Whether `from` may read this machine's map and what it relays
    /// (transcripts, search): an admin member's read.
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

    /// A message frame's body from `from`: an entry of its feed, which goes
    /// to the engine's feeds (or, to `xenonet:person`, the person's chain
    /// here), or its ack of or want for this machine's.
    fn message_in(&self, from: &str, window: bool, body: &Value) -> bool {
        let Some(net) = self.network() else { return false };
        if body.get("schema").and_then(Value::as_str) != Some(MESSAGE_SCHEMA) {
            self.log("net.frame", &format!("a message frame from {from} that isn't {MESSAGE_SCHEMA} was dropped"));
            return false;
        }
        let place = |v: &Value, n: &str| -> Option<Pos> { Some(Pos::new(v.get("inc")?.as_u64()?, v.get(n)?.as_u64()?)) };
        if let Some(at) = body.get("ack").and_then(|a| place(a, "high")) {
            self.ack_in(&net, from, at);
        }
        if let Some(at) = body.get("want").and_then(|w| place(w, "from")) {
            self.resend_to(&net, from, Pos::new(at.inc, at.seq.saturating_sub(1)));
        }
        let Some(entry) = body.get("entry") else { return false };
        let (host, v) = match self.admit(&net, from, entry) {
            Ok(a) => a,
            Err(why) => {
                self.log("net.feed", &why);
                return false;
            }
        };
        let authority = body.get("authority").cloned().unwrap_or_else(|| json!({}));
        self.take_entry(host, v, authority, window, link2n::Via::Gateway)
    }

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
        let e = w.write(key.as_ref(), kind, self.gateway_now().max(0.0) as u64, body)?;
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
        let now = self.gateway_now().max(0.0) as u64;
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

    /// A frame of `body` for `to`, into the outbox.
    fn queue(&self, net: &NetFile, key: &HostKey, to: &str, body: &Value) -> Result<(), String> {
        let keys = {
            let r = self.roster();
            r.view_ref().and_then(|v| v.member(to).map(|m| m.keys)).ok_or_else(|| format!("{to} isn't in the network"))?
        };
        let d = Draft { network: &net.id, id: frame_id().map_err(|e| e.message)?, kind: Kind::Message.as_str(), from: &net.host, to, at: self.gateway_now() as i64 };
        let f = host_frame(&d, key, &keys, body, None).map_err(|e| e.message)?;
        self.outbox.locked().push(&f, to, self.now() as i64, &self.opts.outbox);
        self.wake.notify_one();
        Ok(())
    }

    /// A body the engine sends as an entry of this machine's feed: a loose
    /// one to the host it's for (or every other host), a kept one to every
    /// member, whoever the engine named.
    fn send_entry(&self, net: &NetFile, key: &HostKey, out: Outgoing) -> Result<(), String> {
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
        let e = self.write_entry(kind, out.body).map_err(|e| e.message)?;
        let body = entry_body(&e, out.authority.as_ref());
        for to in &others {
            self.queue(net, key, to, &body)?;
        }
        self.link2_entry(net, &e, &others, out.authority.as_ref());
        Ok(())
    }

    /// The kept entries `host` hasn't got, from after `after`: all of them,
    /// a batch at a time.
    fn resend_to(&self, net: &NetFile, host: &str, mut after: Pos) {
        let Some(key) = self.key() else { return };
        loop {
            let entries = match self.opts.persist.feed_out_after(&net.id, after, RESEND) {
                Ok(e) => e,
                Err(e) => {
                    self.log("net.feed", &format!("this machine's feed couldn't be read to send again: {}", e.message));
                    return;
                }
            };
            let Some(last) = entries.last().map(FeedEntry::pos) else { return };
            for e in &entries {
                if let Err(why) = self.queue(net, &key, host, &entry_body(e, None)) {
                    self.log("net.feed", &format!("an entry for {host} wasn't queued again: {why}"));
                    return;
                }
            }
            if entries.len() < RESEND {
                return;
            }
            after = last;
        }
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

    /// The link came back, or the gateway lost frames: each member gets the
    /// kept entries it hasn't acked, and hears how far this machine holds
    /// its feed, so one restored from a backup learns it before it writes.
    fn feeds_linked(&self) {
        let Some(net) = self.network() else { return };
        let members: Vec<(String, String)> = {
            let r = self.roster();
            r.view_ref().map(|v| v.members.values().filter(|m| m.id != net.host).map(|m| (m.id.clone(), m.machine.id())).collect()).unwrap_or_default()
        };
        for (host, _) in &members {
            let after = self.opts.persist.feed_ack(&net.id, host).ok().flatten().unwrap_or(Pos::ORIGIN);
            self.resend_to(&net, host, after);
        }
        let heads = self.opts.persist.feed_heads(&net.id).unwrap_or_default();
        for (author, at) in heads {
            if let Some((host, _)) = members.iter().find(|(_, k)| *k == author) {
                Net::ack(self, &net.id, host, at.inc, at.seq);
            }
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
    /// worked here goes by the gateway's clock, as the frame's time does.
    fn own_presence(&self, mut body: Value) -> Value {
        // Where its endpoint is, for link/2: the gateway carries it sealed
        // to each member, which finds this machine with it.
        if let Some(r) = self.address_record() {
            body["address"] = r;
        }
        let you = body.get("you").and_then(Value::as_bool).unwrap_or(false);
        let at = body.get("youAt").and_then(Value::as_f64).filter(|t| t.is_finite() && *t > 0.0);
        *self.me_you.locked() = (you, at);
        fold_agents(&mut self.me_agents.locked(), &body);
        if let Some(t) = at {
            body["youAt"] = json!((t + self.status.locked().offset_ms).round() as i64);
        }
        body
    }

    /// A presence, kept for the hosts' list: false when it's older than one
    /// already had, or the same one again (the copy the other way brought).
    /// The address record in it goes to link/2, not the engine.
    fn note_presence(&self, from: &str, body: &mut Value) -> bool {
        if let Some(r) = body.as_object_mut().and_then(|o| o.remove("address")) {
            self.address_in(from, &r, "presence");
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
        // Sent by the gateway's clock, kept by this one's, so consoles on
        // clocks that disagree still agree where the person last worked.
        let offset = self.status.locked().offset_ms;
        p.you_at = body.get("youAt").and_then(Value::as_f64).filter(|t| t.is_finite() && *t > 0.0).map(|t| t - offset);
        // An empty head says nothing about the sender's roster: no mismatch.
        p.roster = body.get("roster").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string);
        p.can = body.get("can").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).take(16).map(str::to_string).collect()).unwrap_or_default();
        true
    }

    // ---------- the up batch ----------

    /// The next batch: what waits in memory, then the outbox's oldest; once
    /// they've gone, a service's request or answer, alone, and then the
    /// map's frames, each in a relay batch of their own, which the gateway
    /// takes only as one.
    pub fn next_batch(&self) -> Option<Batch> {
        let volatile: Vec<Frame> = std::mem::take(&mut *self.volatile.locked());
        let queued = self.outbox.locked().batch(BATCH_BYTES, BATCH_FRAMES);
        let (epoch, seq) = self.cursor();
        let ack = epoch.map_or(Value::Null, |e| json!({ "epoch": e, "seq": seq }));
        if volatile.is_empty() && queued.is_empty() {
            // A service's request or answer goes before the map's frames:
            // someone waits on it.
            if let Some(f) = self.svc_out.locked().pop_front() {
                let body = format!("{{\"frames\":[{}],\"ack\":{ack}}}", f.line().trim_end());
                return Some(Batch { ids: Vec::new(), body, relay: true, svc: Some(f) });
            }
            return self.relay_batch(&ack);
        }
        let mut lines: Vec<String> = volatile.iter().map(|f| f.line().trim_end().to_string()).collect();
        let ids: Vec<String> = queued.iter().map(|(id, _)| id.clone()).collect();
        lines.extend(queued.into_iter().map(|(_, l)| l));
        Some(Batch { ids, body: format!("{{\"frames\":[{}],\"ack\":{ack}}}", lines.join(",")), relay: false, svc: None })
    }

    /// The map's frames that wait, oldest first, as many as fit
    /// RELAY_BATCH_BYTES (one at least); the rest wait for the next.
    fn relay_batch(&self, ack: &Value) -> Option<Batch> {
        let mut waiting = self.relay_out.locked();
        if waiting.is_empty() {
            return None;
        }
        let mut lines = Vec::new();
        let mut bytes = 0;
        let mut taken = 0;
        for f in waiting.iter() {
            let line = f.line().trim_end().to_string();
            if taken > 0 && bytes + line.len() > RELAY_BATCH_BYTES {
                break;
            }
            bytes += line.len();
            lines.push(line);
            taken += 1;
        }
        waiting.drain(..taken);
        Some(Batch { ids: Vec::new(), body: format!("{{\"frames\":[{}],\"ack\":{ack}}}", lines.join(",")), relay: true, svc: None })
    }

    /// The gateway's answer to a batch: what it took, and what it refused
    /// for good, leave the outbox; what it held back for its rate waits.
    pub fn batch_done(&self, b: &Batch, answer: &Value) {
        if let Some(f) = &b.svc {
            let refused = answer.get("refused").and_then(Value::as_array).into_iter().flatten().find(|r| r.get("id").and_then(Value::as_str) == Some(f.id.as_str()));
            if let Some(r) = refused {
                let why = r.get("reason").and_then(Value::as_str).unwrap_or("").to_string();
                let a = match r.get("code").and_then(Value::as_str) {
                    Some("rate") => Answer::refused(429, "rate", format!("the network's gateway refused the request: {why}")),
                    Some("too-big") => Answer::refused(413, "too-big", format!("the network's gateway refused the request: {why}")),
                    _ => Answer::refused(502, "refused", format!("the network's gateway refused the request: {why}")),
                };
                self.log("net.svc", &format!("the gateway refused a service's frame: {why}"));
                self.svc.fail_frame(&f.id, &a);
            }
            return;
        }
        let mut done: Vec<String> = answer.get("accepted").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        for r in answer.get("refused").and_then(Value::as_array).into_iter().flatten() {
            if r.get("code").and_then(Value::as_str) == Some("rate") {
                continue;
            }
            if let Some(id) = r.get("id").and_then(Value::as_str) {
                self.log("net.up", &format!("the gateway refused a frame: {}", r.get("reason").and_then(Value::as_str).unwrap_or("")));
                done.push(id.to_string());
            }
        }
        done.retain(|id| b.ids.contains(id));
        self.outbox.locked().done(&done);
        self.write_link(false);
    }

    /// A batch the gateway didn't take. With `again`, after the gateway
    /// refused this host's time, a service's frame in it goes again in a
    /// new request signed with the gateway's time, but only while its own
    /// date, which can't change without sealing it again, is within a
    /// minute of that time, so the host it's for takes it. Otherwise its
    /// caller hears why now rather than at its timeout.
    pub fn batch_failed(&self, b: &Batch, why: &str, again: bool) {
        let Some(f) = &b.svc else { return };
        if again && (f.at as f64 - self.gateway_now()).abs() <= 60e3 {
            self.svc_out.locked().push_front(f.clone());
        } else if again {
            self.svc.fail_frame(&f.id, &Answer::refused(503, "clock", "this machine's clock was off the gateway's, and the request was dated by it: try again"));
        } else {
            self.svc.fail_frame(&f.id, &Answer::refused(502, "unreachable", format!("the request didn't reach the network's gateway: {why}")));
        }
    }

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
        let made = self.identity.revoke_device(at_window, device, self.gateway_now() as i64)?;
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

    /// A svc frame for `to`, sealed and signed, to go up alone. Live only:
    /// refused while the link is down, and never kept in the outbox, since
    /// it may carry the service's own credential.
    fn send_svc(&self, to: &str, body: &Value, id: String) -> Result<(), String> {
        let net = self.network().ok_or("this machine isn't in a XenoNet network")?;
        if self.is_removed() {
            return Err("this machine was removed from its network".into());
        }
        if !self.is_live() {
            return Err("this machine's link to the network's gateway is down".into());
        }
        if body.to_string().len() > RELAY_BODY {
            return Err(format!("a service's request or answer is at most {} MiB in a frame", RELAY_BODY >> 20));
        }
        let key = self.key().ok_or("this machine has no network key")?;
        let keys = self.roster().view_ref().and_then(|v| v.member(to).map(|m| m.keys)).ok_or_else(|| format!("{to} isn't in the network"))?;
        let d = Draft { network: &net.id, id, kind: "svc", from: &net.host, to, at: self.gateway_now() as i64 };
        let f = host_frame(&d, &key, &keys, body, None).map_err(|e| e.message)?;
        self.svc_out.locked().push_back(f);
        self.wake.notify_one();
        Ok(())
    }

    /// A program here asks for `name` on `host`: the answer, or why there's
    /// none. 503 when this host or `host` isn't linked, by what the
    /// gateway's pings last said, since the gateway takes a live frame for
    /// a host that's away and drops it; 504 when no answer comes in time.
    pub async fn svc_call(&self, host: &str, name: &str, call: svc::Call) -> Answer {
        let Some(net) = self.network() else { return Answer::refused(503, "no-network", "this machine isn't in a XenoNet network") };
        if self.is_removed() {
            return Answer::refused(503, "removed", "this machine was removed from its network");
        }
        if !is_host_id(host) || !svc::is_name(name) {
            return Answer::refused(400, "bad-request", "reach a service at /api/svc/<host>/<name>/<path>");
        }
        if host == net.host {
            return Answer::refused(400, "this-host", "that's this machine: reach the service at its own address");
        }
        if self.roster().view_ref().is_none_or(|v| v.member(host).is_none()) {
            return Answer::refused(404, "no-host", format!("{host} isn't in the network"));
        }
        if !svc::METHODS.contains(&call.method.as_str()) {
            return Answer::refused(405, "method", format!("{} isn't passed on: a service is reached with {}", call.method, svc::METHODS.join(", ")));
        }
        if call.body.len() > svc::REQUEST_BODY {
            return Answer::refused(413, "too-big", "a request's body is at most 1 MiB");
        }
        if !self.is_live() {
            return Answer::refused(503, "offline", "this machine's link to the network's gateway is down");
        }
        if !self.peers.locked().get(host).is_some_and(|p| p.online) {
            return Answer::refused(503, "offline", format!("{host} is offline: it isn't linked to the network's gateway"));
        }
        let (Ok(id), Ok(frame)) = (svc::request_id(), frame_id()) else { return Answer::refused(500, "internal", "no randomness for a request's id") };
        // This host stops waiting when `wait` goes: with the answer, at
        // the timeout, or when the program that asked goes away, which
        // drops this call.
        let mut wait = match self.svc.wait(&id, host, &frame) {
            Ok(w) => w,
            Err(a) => return a,
        };
        let req = svc::Request { id, name: name.to_string(), method: call.method, path: call.path, headers: svc::passed(&call.headers), body: call.body };
        if let Err(why) = self.send_svc(host, &req.to_json(), frame) {
            return Answer::refused(503, "offline", why);
        }
        match tokio::time::timeout(Duration::from_millis(self.opts.svc_ms), &mut wait.answer).await {
            Ok(Ok(a)) => a,
            Ok(Err(_)) => Answer::refused(502, "dropped", "the request was dropped before its answer came"),
            Err(_) => Answer::refused(504, "timeout", format!("{host} didn't answer within {} s", self.opts.svc_ms / 1000)),
        }
    }

    /// A svc frame from `from`, whose signature checked: an answer to a
    /// request made here, or a request for a service published here.
    fn svc_in(&self, from: &str, body: Value) {
        match body.get("op").and_then(Value::as_str) {
            Some("answer") => match Answer::parse(&body) {
                Ok((id, a)) => {
                    if !self.svc.answered(from, &id, a) {
                        self.log("net.svc", &format!("an answer from {from} that nothing here waits on was dropped"));
                    }
                }
                Err(why) => self.log("net.svc", &format!("an answer from {from} was dropped: {why}")),
            },
            Some("request") => self.svc_serve(from, body),
            _ => self.log("net.svc", &format!("a svc frame from {from} that's neither a request nor an answer was dropped")),
        }
    }

    /// A request from `from` for a service here: checked, passed to the
    /// service, and answered, in a task of its own so the link reads on.
    fn svc_serve(&self, from: &str, body: Value) {
        let Some(id) = svc::id_of(&body) else {
            self.log("net.svc", &format!("a request from {from} with no id to answer was dropped"));
            return;
        };
        let (Some(this), Ok(rt)) = (self.this.upgrade(), tokio::runtime::Handle::try_current()) else { return };
        let from = from.to_string();
        rt.spawn(async move {
            let now = this.now();
            let (name, answer) = match svc::Request::parse(&body) {
                Ok(req) => (req.name.clone(), this.svc.serve(&from, req, now).await),
                Err(why) => (String::new(), Answer::refused(400, "bad-request", why)),
            };
            if let Some(code) = answer.code.as_deref() {
                if this.svc.say(&format!("{from}|{name}|{code}"), now) {
                    this.log(
                        "net.svc",
                        &format!("a request from {from} to {} was answered {}: {}", if name.is_empty() { "a service" } else { &name }, answer.status, answer.message()),
                    );
                }
            }
            let frame = match frame_id() {
                Ok(f) => f,
                Err(e) => return this.log("net.svc", &e.message),
            };
            if let Err(why) = this.send_svc(&from, &answer.to_json(&id), frame) {
                this.log("net.svc", &format!("the answer to {from} didn't go: {why}"));
            }
        });
    }
}

impl Net for Shared {
    // One network on the gateway's path: several come with link/2's
    // networks (phase 5's commit 13), which this list already names.
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
        let (Some(net), Some(key)) = (self.in_network(network), self.key()) else { return };
        if let Err(why) = self.queue(&net, &key, host, &json!({ "schema": MESSAGE_SCHEMA, "ack": { "inc": inc, "high": high } })) {
            self.log("net.feed", &format!("an ack for {host} wasn't queued: {why}"));
        }
        let author = self.roster().view_ref().and_then(|v| v.member(host).map(|m| m.machine.id()));
        if let Some(author) = author {
            self.link2_filled(&net.id, &author, Pos::new(inc, high));
            self.link2_control(host, link2::Control::Ack { network: net.id.clone(), author, inc, high });
        }
    }

    fn want(&self, network: &str, host: &str, inc: u64, from: u64) {
        let (Some(net), Some(key)) = (self.in_network(network), self.key()) else { return };
        if let Err(why) = self.queue(&net, &key, host, &json!({ "schema": MESSAGE_SCHEMA, "want": { "inc": inc, "from": from } })) {
            self.log("net.feed", &format!("a want for {host} wasn't queued: {why}"));
        }
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
        let key = self.key().ok_or("this machine has no network key")?;
        let mut body = out.body;
        if out.kind == Kind::Presence {
            body = self.own_presence(body);
        }
        if out.kind == Kind::Svc {
            return Err("a service's requests and answers go through the network's services".into());
        }
        if out.kind == Kind::Relay && body.to_string().len() > RELAY_BODY {
            return Err(format!("a relay body is at most {} MiB", RELAY_BODY >> 20));
        }
        if out.kind.fed() {
            let r = self.send_entry(&net, &key, Outgoing { body, ..out });
            self.wake.notify_one();
            return r;
        }
        let targets: Vec<(String, PublicKeys)> = {
            let r = self.roster();
            let v = r.view_ref().ok_or("this machine's roster is empty")?;
            match &out.to {
                To::Host(h) if *h == net.host => return Err("that's this machine".into()),
                To::Host(h) => vec![(h.clone(), v.member(h).ok_or_else(|| format!("{h} isn't in the network"))?.keys)],
                To::All => v.members.values().filter(|m| m.id != net.host).map(|m| (m.id.clone(), m.keys)).collect(),
            }
        };
        let at = self.gateway_now() as i64;
        let now = self.now() as i64;
        for (to, keys) in targets {
            // Each member sees what this machine shares into the networks
            // it is in with it.
            let shown;
            let body = if out.kind == Kind::Presence {
                shown = crate::share::presence_for(&body, &self.sharings_with(&to));
                &shown
            } else {
                &body
            };
            let id = frame_id().map_err(|e| e.message)?;
            let d = Draft { network: &net.id, id, kind: out.kind.as_str(), from: &net.host, to: &to, at };
            let f = host_frame(&d, &key, &keys, body, None).map_err(|e| e.message)?;
            if out.kind == Kind::Presence {
                self.link2_presence(&net, &to, body);
            }
            if out.kind.queued() {
                self.outbox.locked().push(&f, &to, now, &self.opts.outbox);
            } else if self.is_live() {
                // Not kept while the link is down: a full set goes, or a
                // console asks again, once it's back.
                if out.kind == Kind::Relay {
                    self.relay_out.locked().push(f);
                } else {
                    self.volatile.locked().push(f);
                }
            }
        }
        self.wake.notify_one();
        Ok(())
    }
}

/// This machine in its network, and its tasks.
pub struct Node {
    shared: Arc<Shared>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    /// The invites open here, and their door on join/2.
    invites: join2::Invites,
}

impl Node {
    /// The node, as its files leave it; nothing runs until `start`.
    pub fn open(opts: Options, sink: Arc<SinkSlot>) -> R<Arc<Node>> {
        let dir = Dir::new(opts.dir.clone(), opts.persist.clone());
        let identity = Identity::open(dir.identity_dir(), opts.persist.clone(), opts.vault.clone())?;
        let policy = NetworkPolicy::with(identity.clone());
        let net = dir.read_network()?;
        let (key, roster, cursor) = match &net {
            Some(n) => {
                let key = HostKey::load(&dir.file(KEY))?.ok_or_else(|| NetError::internal("this host's key is missing"))?;
                let cursor = opts.persist.cursor(&n.id)?.map_or((None, 0), |(e, s)| (Some(e), s));
                (Some(Arc::new(key)), dir.read_roster(&n.id, policy)?, cursor)
            }
            None => (None, Roster::with_policy(policy), (None, 0)),
        };
        let services = Services::open(opts.persist.clone());
        let shared = Arc::new_cyclic(|this| Shared {
            this: this.clone(),
            opts,
            dir,
            key: RwLock::new(key),
            net: RwLock::new(net),
            roster: RwLock::new(roster),
            outbox: Mutex::new(Outbox::default()),
            cursor: Mutex::new(cursor),
            writer: Mutex::new(None),
            sink,
            status: Mutex::new(Status::default()),
            peers: Mutex::new(BTreeMap::new()),
            wake: Notify::new(),
            volatile: Mutex::new(Vec::new()),
            relay_out: Mutex::new(Vec::new()),
            relay_ids: Mutex::new(Recent::default()),
            svc_out: Mutex::new(VecDeque::new()),
            svc: services,
            gateway: RwLock::new(None),
            identity: identity.clone(),
            link_written: Mutex::new(0.0),
            proposals: Mutex::new(Vec::new()),
            relink: Notify::new(),
            pending: Mutex::new(Vec::new()),
            signed: Mutex::new(Vec::new()),
            me_you: Mutex::new((false, None)),
            me_agents: Mutex::new(BTreeMap::new()),
            link2: RwLock::new(None),
            taken: Mutex::new(link2n::Taken::default()),
            clocks: Mutex::new(link2n::Clocks::default()),
        });
        // What the person appends here, the network's other machines hear.
        let weak = Arc::downgrade(&shared);
        identity.on_append(Box::new(move |e| {
            if let Some(n) = weak.upgrade() {
                n.push_person(e.n);
            }
        }));
        Ok(Arc::new(Node { shared, tasks: Mutex::new(Vec::new()), invites: join2::Invites::default() }))
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

    pub fn gateway(&self) -> Option<Arc<Gateway>> {
        self.shared.gateway()
    }

    /// Starts the gateway here when this host is the gateway, and the link.
    pub fn start(&self) -> R<()> {
        self.stop();
        let n = &self.shared;
        let Some(net) = n.network() else { return Ok(()) };
        if n.is_removed() {
            return Ok(());
        }
        let mut tasks = Vec::new();
        let transport = if n.is_gateway() {
            let g = Gateway::new(Arc::downgrade(n), GwStore::open(&n.dir.file(GATEWAY_DB))?)?;
            *n.gateway.writing() = Some(g.clone());
            let weak = Arc::downgrade(&g);
            tasks.push(tokio::spawn(async move {
                let mut every = tokio::time::interval(Duration::from_secs(3600));
                loop {
                    every.tick().await;
                    let Some(g) = weak.upgrade() else { return };
                    let _ = g.prune();
                }
            }));
            Transport::Local(Arc::downgrade(&g))
        } else {
            let Some(address) = net.gateway.address.as_deref() else {
                n.offline("the gateway's address isn't known: xenonet net gateway set <address>");
                return Ok(());
            };
            link::http(address)?
        };
        tasks.extend(link::spawn(n.clone(), transport));
        if n.opts.link {
            tasks.push(tokio::spawn(link2n::run(Arc::downgrade(n))));
        }
        *self.tasks.locked() = tasks;
        Ok(())
    }

    /// Stops the link and the gateway: its streams end.
    pub fn stop(&self) {
        for t in self.tasks.locked().drain(..) {
            t.abort();
        }
        link2n::stop(&self.shared);
        if let Some(g) = self.shared.gateway.writing().take() {
            g.stop();
        }
        let s = &self.shared;
        if !s.is_removed() && s.status.locked().state != "off" {
            s.set("off", None);
        }
    }

    fn transport(&self) -> R<Transport> {
        if let Some(g) = self.shared.gateway() {
            return Ok(Transport::Local(Arc::downgrade(&g)));
        }
        let net = self.shared.network().ok_or_else(not_in)?;
        link::http(net.gateway.address.as_deref().ok_or_else(|| NetError::new(409, "no-address", "the gateway's address isn't known"))?)
    }

    fn me(&self) -> R<(NetFile, Arc<HostKey>)> {
        let n = &self.shared;
        Ok((n.network().ok_or_else(not_in)?, n.key().ok_or_else(not_in)?))
    }

    fn adopt(&self, net: NetFile, key: HostKey, roster: Roster) -> R<()> {
        let n = &self.shared;
        // The network's row before its roster, which is kept under it.
        n.dir.write_network(&net)?;
        n.dir.write_roster(&net.id, &roster)?;
        *n.outbox.locked() = Outbox::default();
        *n.cursor.locked() = (None, 0);
        *n.writer.locked() = None;
        *n.key.writing() = Some(Arc::new(key));
        *n.roster.writing() = roster;
        *n.net.writing() = Some(net);
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        n.svc.load();
        Ok(())
    }

    /// `xenonet net create`: a new network with this machine as its founder
    /// and gateway, owned by the person here. `address` is where hosts
    /// reach it: the LAN address its gateway listens on, or the tunnel's
    /// host name. With no person here yet, it's founded first, which asks
    /// them once: from the window (`at_window`) only. A machine that can't
    /// ask the person makes a network with no owner, where nobody can admit
    /// another machine.
    pub fn create(&self, name: &str, host: &str, address: Option<&str>, lan: Option<&str>, at_window: bool) -> R<Value> {
        let n = &self.shared;
        if let Some(net) = n.network() {
            return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already: xenonet net leave first", net.name)));
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
        let p = key.public();
        let net = NetFile {
            id,
            name: name.into(),
            host: host.into(),
            founder: host.into(),
            gateway: Pin { host: host.into(), key_id: p.key_id(), sign: p.sign_b64(), seal: p.seal_b64(), address: lan.or(address).map(str::to_string) },
            lan: lan.map(str::to_string),
            titles_only: false,
            share_load: true,
            joined: at,
        };
        self.adopt(net, key, roster)?;
        self.start()?;
        n.log("net.create", &format!("this machine made the network {name}, as {host}"));
        Ok(self.state())
    }

    /// `xenonet net join <address> <ticket>`.
    pub async fn join(&self, address: &str, typed: &str) -> R<Value> {
        let n = &self.shared;
        if let Some(net) = n.network() {
            return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already: xenonet net leave first", net.name)));
        }
        let address = address.trim().trim_end_matches('/');
        if address.is_empty() || address.len() > 300 || address.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(NetError::bad("say the gateway's address, as the join line shows it"));
        }
        let code = ticket::normalize(typed).ok_or_else(|| NetError::bad("a ticket is 10 letters and digits, as KX7Q-49MZ-T2"))?;
        let key = HostKey::load_or_create(&n.dir.file(KEY))?;
        let nonce = crate::codec::nonce()?;
        let mut body = join_body(&key, &code, &nonce, &n.opts.platform, &n.opts.version, n.opts.desktop);
        // This machine's signing key, and its device for the inviter to
        // pair into the person: unless it holds a person of its own that
        // other devices share, which stays its own.
        let identity = n.identity.clone();
        body["machine"] = identity.machine_key()?.public().to_json();
        let local = identity.chain().person().is_none_or(|p| p.active_devices().all(|d| Some(&d.id) == identity.device_id().as_ref()));
        if let (Some(d), Some(kind), true) = (identity.device_key()?, identity.device_kind(), local) {
            body["device"] = identity::device_json(&d.signer.public(), kind, &n.opts.platform);
        }
        let t = link::http(address)?;
        let req = NetRequest { method: "POST".into(), target: "/net/v1/join".into(), headers: Vec::new(), body: body.to_string().into_bytes(), source: None };
        let reply =
            t.request(req, Some(Duration::from_secs(30))).await.map_err(|why| NetError::new(502, "unreachable", format!("the gateway at {address} didn't answer: {why}")))?;
        let v = reply.body_json();
        match reply.status {
            200 => {}
            404 => {
                return Err(NetError::new(
                    404,
                    "no-ticket",
                    "That ticket didn't work: it was used, it's past its ten minutes, or it was mistyped. Make a new one with Add a machine.",
                ))
            }
            // The ticket's maker hasn't signed this machine in yet, or a
            // second window host hasn't approved it: joining again waits on.
            409 if v.get("code").and_then(Value::as_str) == Some("waiting") => {
                return Err(NetError::new(409, "waiting", v.get("error").and_then(Value::as_str).unwrap_or("the join waits for the ticket's maker").to_string()))
            }
            s => return Err(NetError::new(s, "refused", v.get("error").and_then(Value::as_str).unwrap_or("the gateway refused the join").to_string())),
        }
        let g = v.get("gateway").cloned().unwrap_or(Value::Null);
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let gw_keys = PublicKeys::from_b64(&s(&g, "sign"), &s(&g, "seal")).ok_or_else(|| NetError::new(502, "bad-answer", "the gateway's answer has no key"))?;
        if s(&g, "keyId") != gw_keys.key_id() || !ticket::check_gateway(&ticket::hash(&code), &gw_keys.key_id(), &nonce, &s(&v, "proof")) {
            return Err(NetError::new(502, "bad-answer", "the gateway couldn't prove it knew the ticket: check the address"));
        }
        // The owner's chain first, which the roster's admits are checked
        // against: this machine's own person from now, when it pairs this
        // device.
        if let Some(p) = v.get("person").filter(|p| p.is_object()) {
            let person = p.get("person").and_then(Value::as_str).unwrap_or("");
            let entries = p.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
            let chain = xenonet_log::person::PersonChain::from_json(&Value::Array(entries))
                .map_err(|e| NetError::new(502, "bad-answer", format!("the gateway's person chain didn't check: {}", e.message)))?;
            if chain.id() != Some(person) {
                return Err(NetError::new(502, "bad-answer", "the gateway's person chain isn't the person it names"));
            }
            if !identity.adopt(&chain) {
                identity.offer(person, &chain.to_json().as_array().cloned().unwrap_or_default())?;
            }
        }
        let roster = Roster::from_json_with(NetworkPolicy::with(identity.clone()), v.get("roster").unwrap_or(&Value::Null))?;
        let view = roster.view();
        let host = s(&v, "host");
        let mine = view.member(&host).is_some_and(|m| m.keys == *key.public());
        let theirs = view.gateway_member().is_some_and(|m| m.keys == gw_keys && m.id == s(&g, "host"));
        if !mine || !theirs || view.network != s(&v.get("network").cloned().unwrap_or(Value::Null), "id") {
            return Err(NetError::new(502, "bad-answer", "the gateway's roster doesn't hold this machine or the gateway"));
        }
        let net = NetFile {
            id: view.network.clone(),
            name: s(&v.get("network").cloned().unwrap_or(Value::Null), "name"),
            host: host.clone(),
            founder: view.founder.clone(),
            gateway: Pin { host: view.gateway, key_id: gw_keys.key_id(), sign: gw_keys.sign_b64(), seal: gw_keys.seal_b64(), address: Some(address.to_string()) },
            lan: None,
            titles_only: false,
            share_load: true,
            joined: n.now() as i64,
        };
        let name = net.name.clone();
        self.adopt(net, key, roster)?;
        self.start()?;
        n.log("net.join", &format!("this machine joined the network {name}, as {host}"));
        Ok(self.state())
    }

    /// `xenonet net leave`: asks to be removed, then forgets the network.
    pub async fn leave(&self, force: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        // The founder's key is the network's root, which nothing hands on,
        // so the founder leaves only with --force, alone or not; the
        // gateway's machine can move the gateway first.
        let founder = net.founder == net.host;
        if !force && (founder || n.is_gateway()) {
            let others = n.roster().view().members.len() > 1;
            let why = match (founder, others) {
                (true, false) => "this machine founded the network, and its key is the network's root: the network ends when it leaves. Leave with --force to end it.",
                (true, true) => "this machine founded the network, and its key is the network's root: the other hosts lose it if it leaves. Leave with --force all the same.",
                (false, _) => "this machine is the network's gateway: the other hosts lose it if it leaves. Move the gateway first, or leave with --force.",
            };
            return Err(NetError::new(409, "needed", why));
        }
        let mut asked = false;
        if net.founder != net.host && !n.is_removed() {
            let me = n.roster().host_ref(&net.host);
            if let Some(m) = me {
                let mut e = n.draft(Op::Remove, m, None);
                e.sign_by(&net.host, n.identity.machine_key()?.as_ref())?;
                asked = self.submit(e).await.is_ok();
            }
        }
        // The stores close before their files go: Windows won't delete a
        // file that's open, and the gateway's streams may not have ended yet.
        let gateway = n.gateway();
        self.stop();
        if let Some(g) = gateway {
            g.close();
        }
        *n.outbox.locked() = Outbox::default();
        *n.cursor.locked() = (None, 0);
        *n.writer.locked() = None;
        n.dir.forget()?;
        *n.key.writing() = None;
        *n.roster.writing() = Roster::with_policy(NetworkPolicy::with(n.identity.clone()));
        *n.net.writing() = None;
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        n.svc.forget();
        n.identity.forget_others();
        self.invites.close();
        n.log("net.leave", &format!("this machine left the network {}", net.name));
        Ok(json!({ "left": net.name, "removed": asked, "founder": founder }))
    }

    /// An entry this host signed, to the gateway: here when it's the
    /// gateway, else through the link's route; then into this roster.
    async fn submit(&self, e: Entry) -> R<Offered> {
        let n = &self.shared;
        n.roster().would_take(&e)?;
        if let Some(g) = n.gateway() {
            return g.append(n, e);
        }
        let t = self.transport()?;
        link::call(n, &t, "POST", "/net/v1/roster", json!({ "entry": e.to_json(), "person": n.person_json() })).await?;
        match n.append(e) {
            Ok(o) => Ok(o),
            Err(err) if err.code == "roster-fork" => Ok(Offered::Duplicate),
            Err(err) => Err(err),
        }
    }

    /// **Add a machine**: a ticket for `host`, and the line to run there.
    /// The person's alone: the routes check that. On the gateway's host the
    /// gateway keeps it at once; on another window host it goes to the
    /// gateway over the link (its hash and terms, in a signed request), and
    /// this host signs the new host in when it joins. A window host added by
    /// anyone but the founder waits, at its join, for a second window host
    /// to approve it. The person's act at the window (`at_window`), since
    /// signing the machine in asks them, at its join.
    pub async fn ticket(&self, host: &str, window: bool, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        if !at_window {
            return Err(identity::not_at_window("Add a machine"));
        }
        if !is_host_id(host) {
            return Err(NetError::bad("a host id is 2 to 32 lowercase letters, digits and dashes, starting with a letter"));
        }
        let view = n.roster().view();
        if view.owner.is_none() || view.owner != n.identity.person_id() || !n.identity.can_act() {
            return Err(NetError::new(
                409,
                "no-person",
                "this machine can't sign a machine in: adding one needs your presence at a device of the network's owner, and this machine has none",
            ));
        }
        if view.member(host).is_some() {
            return Err(NetError::new(409, "taken", format!("{host} is in the network already: Replace it to add a reinstalled machine")));
        }
        if !view.is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "a ticket is made on a window host"));
        }
        let Some(address) = net.gateway.address.clone() else {
            return Err(NetError::new(409, "no-address", "the gateway has no address yet: give it its tailnet or LAN address (xenonet net set --lan <ip:port>)"));
        };
        let code = ticket::make()?;
        let now = n.now() as i64;
        let expires = match n.gateway() {
            Some(g) => {
                let expires = now + ticket::TICKET_MS as i64;
                g.add_ticket(&TicketRow { hash: ticket::hash_b64(&code), host: host.into(), window, maker: net.host.clone(), made: now, expires }, now)?;
                expires
            }
            None => {
                let t = self.transport()?;
                let v = link::call(n, &t, "POST", "/net/v1/tickets", json!({ "hash": ticket::hash_b64(&code), "host": host, "window": window })).await?;
                let expires = v.get("expires").and_then(Value::as_i64).unwrap_or(now + ticket::TICKET_MS as i64).min(now + ticket::TICKET_MS as i64);
                let mut pending = n.pending.locked();
                pending.retain(|p| p.expires > now && p.host != host);
                pending.push(PendingTicket { code: code.clone(), host: host.into(), window, expires });
                expires
            }
        };
        let shown = ticket::display(&code);
        n.log("net.ticket", &format!("a ticket for {host} was made; it works once, for ten minutes"));
        let mut out = json!({ "host": host, "window": window, "ticket": shown, "line": format!("xenonet net join {address} {shown}"), "expires": expires });
        if window && view.founder != net.host {
            out["approval"] = json!(true);
        }
        Ok(out)
    }

    /// Removes another host: an entry this host signs, which the roster's
    /// rules decide.
    pub async fn remove(&self, host: &str) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        if host == net.host {
            return Err(NetError::bad("to take this machine out, run xenonet net leave"));
        }
        let m = n.roster().host_ref(host).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        let mut e = n.draft(Op::Remove, m, None);
        e.sign_by(&net.host, n.identity.machine_key()?.as_ref())?;
        self.submit(e).await?;
        n.log("net.remove", &format!("{host} was removed from the network"));
        Ok(json!({ "removed": host, "state": self.state() }))
    }

    /// **Replace**: the old entry goes, and a ticket comes for the same id
    /// and window choice, for the reinstalled machine to join with.
    pub async fn replace(&self, host: &str, at_window: bool) -> R<Value> {
        let window = self.shared.roster().view().member(host).map(|m| m.window).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        if !at_window {
            return Err(identity::not_at_window("Replace"));
        }
        self.remove(host).await?;
        let mut t = self.ticket(host, window, at_window).await?;
        t["replaced"] = json!(host);
        Ok(t)
    }

    /// Settings: titles only, whether this host shares its load, and the
    /// LAN address the gateway listens on, kept here; who may start and
    /// wake sessions here, a settings entry in the chain (`set_reach`).
    pub async fn settings(&self, change: Change, at_window: bool) -> R<Value> {
        let Change { titles_only, share_load, lan, start, wake } = change;
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
            if let Some(lan) = lan {
                if net.gateway.host != net.host {
                    return Err(NetError::new(409, "not-gateway", "the LAN address is the gateway's: set it on its host"));
                }
                net.gateway.address.clone_from(&lan);
                net.lan = lan;
            }
            n.dir.write_network(net)?;
        }
        if start.is_some() || wake.is_some() {
            let me = n.network().ok_or_else(not_in)?.host;
            return self.set_reach(&me, start.as_deref(), wake.as_deref(), at_window).await;
        }
        Ok(self.state())
    }

    /// The gateway's address as its tunnel gives it, on the gateway's host
    /// in tunnel mode: kept, so tickets name it.
    pub fn tunnel_address(&self, address: Option<&str>) {
        let n = &self.shared;
        let mut guard = n.net.writing();
        let Some(net) = guard.as_mut() else { return };
        if net.gateway.host != net.host || net.lan.is_some() || address.is_none() || net.gateway.address.as_deref() == address {
            return;
        }
        net.gateway.address = address.map(str::to_string);
        let _ = n.dir.write_network(net);
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
        let now = n.gateway_now() as i64;
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
        let mut e = n.draft(Op::Settings, target, None);
        e.settings = Some(to);
        let presence = match e.presence_for(&n.roster().view()) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), &format!("let your network reach {host} further"), now)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.decide_entry(e, "this machine asked to widen what the network may do there; the founder or another window host approves it").await
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

    /// An entry this machine signed: to the gateway, or, signed by too few,
    /// kept there as a proposal for another admin to approve.
    async fn decide_entry(&self, e: Entry, waits: &str) -> R<Value> {
        let n = &self.shared;
        let verdict = n.roster().would_take(&e);
        match verdict {
            Ok(()) => {}
            Err(err) if err.code == "needs-approval" => {
                if let Some(g) = n.gateway() {
                    g.propose(n, e, n.now())?;
                } else {
                    let t = self.transport()?;
                    link::call(n, &t, "POST", "/net/v1/roster", json!({ "entry": e.to_json(), "propose": true, "person": n.person_json() })).await?;
                    n.note_proposals(vec![e]);
                }
                n.log("net.settings", waits);
                let mut out = self.state();
                out["proposed"] = json!(true);
                return Ok(out);
            }
            Err(err) => return Err(err.into()),
        }
        self.submit(e).await?;
        Ok(self.state())
    }

    /// `network/state/v1`: the network, this host, its gateway and link,
    /// the hosts, the tickets waiting, the outbox and the queues.
    pub fn state(&self) -> Value {
        let n = &self.shared;
        let Some(net) = n.network() else {
            return json!({
                "schema": "network/state/v1",
                "network": null,
                "self": null,
                "gateway": null,
                "hosts": [],
                "tickets": [],
                "outbox": { "frames": 0, "bytes": 0, "oldest": null },
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
        let is_gateway = view.gateway == net.host;
        let gw = n.gateway();
        let info = gw.as_ref().map(|g| g.host_info()).unwrap_or_default();
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
            let platform = if me { Some(n.opts.platform.clone()) } else { p.and_then(|p| p.platform.clone()).or_else(|| info.get(id).and_then(|i| i.0.clone())) };
            hosts.push(json!({
                "id": id,
                "keyId": m.keys.key_id(),
                "window": view.is_window(id),
                "founder": *id == view.founder,
                "gateway": *id == view.gateway,
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
                "found": if me { None } else { found.get(id).map(|f| f.iter().copied().collect::<Vec<_>>()) },
                "skewMs": if me { None } else { on_link.get(&crate::endpoint::peer_id(&m.keys)).and_then(|l| l.skew_ms) },
                // This machine's entries a mailbox holds for it, not
                // delivered yet.
                "held": if me { None } else { Some(n.mail_standing(&net.id, id).held) },
            }));
        }
        // The gateway's host lists every ticket that waits; another window
        // host, the ones it made.
        let tickets: Vec<Value> = match gw.as_ref() {
            Some(g) => g.waiting_tickets(now as i64).into_iter().map(|t| json!({ "host": t.host, "window": t.window, "expires": t.expires })).collect(),
            None => n.pending_tickets().into_iter().map(|(host, window, expires)| json!({ "host": host, "window": window, "expires": expires })).collect(),
        };
        let o = n.outbox();
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
        let address = net.gateway.address.clone().unwrap_or_default();
        let kind = if address.is_empty() { "tunnel" } else { address_kind(&address) };
        let mut out = json!({
            "schema": "network/state/v1",
            "network": { "id": net.id, "name": net.name },
            "self": { "id": net.host, "keyId": key_id, "window": view.is_window(&net.host), "founder": view.founder == net.host, "gateway": is_gateway, "hlcAheadMs": n.hlc_ahead_ms(), "link2": t2.is_some() },
            "gateway": {
                "address": address,
                "keyId": net.gateway.key_id,
                "kind": kind,
                "link": s.state,
                "since": s.since.map(|x| x as i64),
                "reason": s.reason,
                "skewMs": s.skew_ms,
            },
            "hosts": hosts,
            "tickets": tickets,
            "outbox": { "frames": o.frames, "bytes": o.bytes, "oldest": o.oldest },
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
        // Every network this machine is in, each with its own members: one
        // on the gateway's path (several come with commit 13).
        out["networks"] = json!([{
            "id": net.id,
            "name": net.name,
            "host": net.host,
            "hosts": out["hosts"].clone(),
            "relays": out["relays"].clone(),
            "mailboxes": out["mailboxes"].clone(),
            "frozen": out["frozen"].clone(),
            "sharing": n.sharing(&net.id).to_json(),
        }]);
        out["invites"] = self.invites.json(now as i64);
        if let Some(g) = gw {
            out["queues"] = Value::Array(g.queues().into_iter().map(|(h, f, b)| json!({ "host": h, "frames": f, "bytes": b })).collect());
            out["epoch"] = json!(g.epoch());
            out["joinClosedUntil"] = g.join_closed_until(now).map_or(Value::Null, |t| json!(t as i64));
        }
        if let Some(r) = s.removed {
            out["removed"] = r;
        }
        if let Some(lan) = net.lan {
            out["lan"] = json!(lan);
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
                    "gateway": p.gateway.as_ref().map(|g| g.address.clone()),
                    "signers": p.signers.iter().map(|s| s.by.clone()).collect::<Vec<_>>(),
                    "at": p.at,
                })
            })
            .collect();
        out["proposals"] = Value::Array(proposals);
        out
    }

    // ---------- moving the gateway, by hand ----------

    /// `xenonet net gateway take`: makes this host the network's gateway, at
    /// `address` (its LAN address, `lan`, or its tunnel's host name). The
    /// founder's own signature is enough; anyone else's waits for the
    /// founder or a second window host to approve it. A gateway that's gone
    /// for good isn't asked: the move is this host's, and the others follow
    /// with `gateway set`.
    pub async fn take_gateway(&self, address: Option<&str>, lan: Option<&str>, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        let view = n.roster().view();
        if view.gateway == net.host {
            return Err(NetError::new(409, "already", "this machine is the network's gateway already"));
        }
        let Some(address) = lan.or(address).map(str::to_string) else {
            return Err(NetError::new(409, "no-address", "give this machine an address first: use a tailnet or LAN address (--lan <ip:port>)"));
        };
        let m = n.roster().host_ref(&net.host).ok_or_else(|| NetError::new(410, "removed", "this machine isn't in the network"))?;
        let mut e = n.draft(Op::Gateway, m, Some(crate::roster::GatewayAt { address, version: view.gateway_version + 1 }));
        let block = n.identity.prove(at_window, crate::roster::WIDEN, &e.digest(), "move your XenoNet network's gateway here", n.gateway_now() as i64)?;
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), Some(block))?;
        {
            let mut guard = n.net.writing();
            if let Some(f) = guard.as_mut() {
                f.lan = lan.map(str::to_string);
                n.dir.write_network(f)?;
            }
        }
        let verdict = n.roster().would_take(&e);
        match verdict {
            Ok(()) => {}
            Err(err) if err.code == "needs-approval" => {
                let t = self.transport()?;
                link::call(n, &t, "POST", "/net/v1/roster", json!({ "entry": e.to_json(), "propose": true, "person": n.person_json() })).await?;
                n.note_proposals(vec![e]);
                n.log("net.gateway", "this machine asked to become the network's gateway; the founder or another window host approves it");
                let mut out = self.state();
                out["proposed"] = json!(true);
                return Ok(out);
            }
            Err(err) => return Err(err.into()),
        }
        match self.submit(e.clone()).await {
            Ok(_) => {}
            // The old gateway is gone: the move is this host's alone.
            Err(err) if matches!(err.code, "unreachable" | "refused") && err.status != 403 => {
                n.log("net.gateway", &format!("the old gateway didn't take the move ({}): the other hosts follow with xenonet net gateway set", err.message));
                n.append(e)?;
            }
            Err(err) => return Err(err),
        }
        Ok(self.state())
    }

    /// Approves an entry waiting for approval, by its id (`Entry::id`, as
    /// the state lists it): this host signs exactly that entry too, with
    /// the person's presence where it admits or widens (from the window).
    pub async fn approve(&self, id: &str, at_window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        if !n.roster().view().is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "approving is a window host's"));
        }
        let Some(mut e) = n.proposals().into_iter().find(|p| p.id() == id) else {
            return Err(NetError::new(404, "nothing", "nothing waits for approval by that id: xenonet net status lists what does"));
        };
        let (op, host) = (e.op.as_str(), e.host.id.clone());
        let presence = match e.presence_for(&n.roster().view()) {
            Some(what) => Some(n.identity.prove(at_window, what, &e.digest(), &format!("approve {op} for {host} in your XenoNet network"), n.gateway_now() as i64)?),
            None => None,
        };
        e.sign_with(&net.host, n.identity.machine_key()?.as_ref(), presence)?;
        self.submit(e).await?;
        n.log("net.approve", &format!("this machine approved {op} for {host}"));
        Ok(self.state())
    }

    /// `xenonet net gateway set <address>`: follows the gateway to a new
    /// address, for a host that missed the move. Taken only from a gateway
    /// whose roster extends this one's with a newer `gateway` entry (or the
    /// same gateway at a new address), and that proves that gateway's key
    /// over this host's nonce.
    pub async fn set_gateway(&self, address: &str) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        let address = address.trim().trim_end_matches('/');
        if address.is_empty() || address.len() > 300 {
            return Err(NetError::bad("say the gateway's address"));
        }
        let t = link::http(address)?;
        let had = n.roster().view();
        let v = link::call(n, &t, "GET", &format!("/net/v1/roster?from={}", n.roster().len()), Value::Null).await?;
        if let Some(p) = v.get("person") {
            n.offer_person(p);
        }
        let mut r = n.roster().clone();
        let mut new = Vec::new();
        for e in v.get("entries").and_then(Value::as_array).into_iter().flatten() {
            let e = Entry::parse(e)?;
            if r.offer(e.clone())? == Offered::Appended {
                new.push(e);
            }
        }
        let view = r.view();
        let moved = view.gateway_version > had.gateway_version;
        if !moved && view.gateway != net.gateway.host {
            return Err(NetError::new(409, "no-move", "that gateway shows no newer gateway entry"));
        }
        let gw = view.gateway_member().cloned().ok_or_else(|| NetError::new(409, "no-move", "the roster's gateway isn't in the network"))?;
        // The gateway proves its key over our nonce, in a long-poll's hello.
        let (req, nonce) = n.signed_request("GET", "/net/v1/link?after=0&wait=1", Vec::new())?;
        let reply =
            t.request(req, Some(Duration::from_secs(30))).await.map_err(|why| NetError::new(502, "unreachable", format!("the gateway at {address} didn't answer: {why}")))?;
        let status = reply.status;
        let first = link::first_line(reply.body, Duration::from_secs(30)).await.unwrap_or_default();
        let hello = Frame::parse_line(&first).ok().filter(|f| f.kind == "hello" && f.from == gw.id && f.verify(&net.id, &gw.keys));
        let proved = hello.and_then(|f| f.body_json().ok()).is_some_and(|b| b.get("nonce").and_then(Value::as_str) == Some(nonce.as_str()));
        if status != 200 || !proved {
            return Err(NetError::new(502, "bad-answer", format!("the gateway at {address} didn't prove the key the roster has for {}", gw.id)));
        }
        for e in new {
            n.append(e)?;
        }
        {
            let mut guard = n.net.writing();
            if let Some(f) = guard.as_mut() {
                f.gateway = Pin { host: gw.id.clone(), key_id: gw.keys.key_id(), sign: gw.keys.sign_b64(), seal: gw.keys.seal_b64(), address: Some(address.to_string()) };
                n.dir.write_network(f)?;
            }
        }
        n.relink.notify_one();
        n.log("net.gateway", &format!("this machine follows the gateway {} at {address}", gw.id));
        Ok(self.state())
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        for t in self.tasks.locked().drain(..) {
            t.abort();
        }
        self.invites.close();
    }
}

// join/2: `invite` and `join_invite`.
#[path = "node_join.rs"]
mod join2;

// link/2 beside the gateway's link.
#[path = "node_link.rs"]
pub mod link2n;

#[cfg(test)]
mod tests {
    use super::*;

    const T: f64 = 1_792_512_345_678.0;
    const MIN: f64 = 60e3;

    #[test]
    fn a_relay_or_svc_frame_dated_ahead_is_taken_once() {
        let mut r = Recent::default();
        // Dated four minutes ahead: taken, and refused for as long as its
        // time is fresh, well past two minutes after it came.
        assert!(r.take("a", T + 4.0 * MIN, T));
        for later in [0.0, 2.5, 3.0, 5.9] {
            assert!(!r.take("a", T + 4.0 * MIN, T + later * MIN), "{later} min later");
        }
        // Once its time is past, it's stale.
        assert!(!r.take("a", T + 4.0 * MIN, T + 6.5 * MIN));
        assert!(r.take("b", T + 6.0 * MIN, T + 6.5 * MIN), "a fresh one is taken");
    }

    #[test]
    fn setting_the_clock_back_makes_nothing_fresh_again() {
        let mut r = Recent::default();
        assert!(r.take("a", T, T));
        // Ten minutes on, "a" is stale, and its id may be let go.
        assert!(r.take("b", T + 10.0 * MIN, T + 10.0 * MIN));
        r.ids.remove("a");
        // The gateway sets this host's clock back to when "a" came: still
        // refused, as is anything else older than the floor.
        assert!(!r.take("a", T, T));
        assert!(!r.take("c", T + MIN, T));
    }

    #[test]
    fn a_full_memory_raises_the_floor_rather_than_forget() {
        let mut r = Recent::default();
        for i in 0..RELAY_IDS {
            assert!(r.take(&format!("f{i}"), T + i as f64, T));
        }
        // All fresh, and full: the older half go, and so does their time.
        assert!(r.take("next", T + RELAY_IDS as f64, T));
        assert!(r.ids.len() <= RELAY_IDS / 2 + 1, "{}", r.ids.len());
        assert!(!r.take("f0", T, T), "an id it let go is refused, not taken again");
        assert!(!r.take(&format!("f{}", RELAY_IDS - 1), T + (RELAY_IDS - 1) as f64, T), "and one it kept is seen");
    }
}
