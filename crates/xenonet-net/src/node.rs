//! This machine in its network (docs/design/network.md, sections 3, 4 and
//! 9): its key, its copy of the roster, its link to the gateway, and, on
//! the gateway's own host, the gateway. It's what the engine sees through
//! `xenonet_base::network::Net`, and it calls the engine's `Sink` with
//! what arrives, after checking each frame's signature against its own
//! roster, opening it and dropping repeats.
//!
//! Nothing here waits on the gateway: `send` puts frames in the outbox and
//! returns, and the link sends them when it can.

use crate::codec::{frame_id, is_b64};
use crate::files::{address_kind, Dir, NetFile, Pin, GATEWAY_DB, HOST_DB, KEY};
use crate::frame::{host_frame, open_frame, Draft, Frame};
use crate::gateway::{join_body, Gateway, NetRequest};
use crate::gwstore::{Caps, GwStore, TicketRow};
use crate::hoststore::{HostStore, Size};
use crate::keys::{HostKey, PublicKeys};
use crate::link::{self, Transport};
use crate::roster::{Entry, HostRef, Offered, Op, Removal, Roster};
use crate::sink::{LogFn, SinkSlot};
use crate::{request, ticket, NetError, R};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use xenonet_base::ids::is_host_id;
use xenonet_base::network::{Host, Incoming, Kind, Me, Net, Outgoing, Sink, To};
use xenonet_base::sync::{Locked, ReadWrite};

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

/// The link's and the gateway's timings: the design's, or shorter ones for
/// tests.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub ping_ms: u64,
    pub stream_ms: u64,
    pub idle_ms: u64,
    pub poll_ms: u64,
    pub backoff_min_ms: f64,
    pub backoff_max_ms: f64,
    /// How long a gateway that was moved keeps answering, so the hosts
    /// linked to it hear of the move first.
    pub handover_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing { ping_ms: 15_000, stream_ms: 600_000, idle_ms: 45_000, poll_ms: 25_000, backoff_min_ms: 2000.0, backoff_max_ms: 60_000.0, handover_ms: 30_000 }
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
}

impl Options {
    pub fn new(dir: impl Into<PathBuf>, version: &str, platform: &str, log: LogFn) -> Options {
        Options {
            dir: dir.into(),
            version: version.into(),
            platform: platform.into(),
            desktop: false,
            clock: Arc::new(now_ms),
            caps: Caps::default(),
            outbox: Caps::default(),
            timing: Timing::default(),
            log,
        }
    }
}

pub fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// How the link stands.
#[derive(Debug, Clone)]
pub struct Status {
    /// off, connecting, live, offline, refused or removed.
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

/// Another host, as presence and the gateway's pings tell of it.
#[derive(Debug, Clone, Default)]
pub struct Peer {
    pub online: bool,
    pub seen: Option<f64>,
    pub counter: u64,
    /// Its agents, and whether each waits on the person.
    pub agents: BTreeMap<String, bool>,
    pub you: bool,
    pub roster: Option<String>,
    pub platform: Option<String>,
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

/// A batch on its way up.
pub struct Batch {
    pub ids: Vec<String>,
    pub body: String,
}

pub struct Shared {
    opts: Options,
    pub dir: Dir,
    key: RwLock<Option<Arc<HostKey>>>,
    net: RwLock<Option<NetFile>>,
    roster: RwLock<Roster>,
    store: Mutex<Option<HostStore>>,
    sink: Arc<SinkSlot>,
    status: Mutex<Status>,
    peers: Mutex<BTreeMap<String, Peer>>,
    wake: Notify,
    volatile: Mutex<Vec<Frame>>,
    gateway: RwLock<Option<Arc<Gateway>>>,
    operator_nonces: Mutex<HashMap<String, f64>>,
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
}

const OPERATOR_ACTIONS: [&str; 4] = ["message", "ticket", "remove", "replace"];
const OPERATOR_MS: f64 = 60e3;
const BATCH_BYTES: u64 = 900 << 10;
const BATCH_FRAMES: u64 = 500;

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

    pub fn key(&self) -> Option<Arc<HostKey>> {
        self.key.reading().clone()
    }

    pub fn roster(&self) -> RwLockReadGuard<'_, Roster> {
        self.roster.reading()
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

    fn with_store<T>(&self, f: impl FnOnce(&HostStore) -> R<T>) -> R<T> {
        match self.store.locked().as_ref() {
            Some(s) => f(s),
            None => Err(not_in()),
        }
    }

    pub fn cursor(&self) -> (Option<String>, u64) {
        self.with_store(HostStore::cursor).unwrap_or((None, 0))
    }

    pub fn outbox(&self) -> Size {
        self.with_store(HostStore::size).unwrap_or_default()
    }

    /// A request to the gateway, signed: the request and its nonce.
    pub fn signed_request(&self, method: &str, target: &str, body: Vec<u8>) -> R<(NetRequest, String)> {
        let net = self.network().ok_or_else(not_in)?;
        let key = self.key().ok_or_else(not_in)?;
        let nonce = crate::codec::nonce()?;
        let time = self.gateway_now() as i64;
        let headers = request::sign(&key, &net.id, &net.host, method, target, time, &nonce, &body);
        Ok((NetRequest { method: method.into(), target: target.into(), headers, body }, nonce))
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
            self.sink.linked(false);
        }
        if changed {
            self.write_link(true);
        }
    }

    pub fn connecting(&self) {
        let state = self.status.locked().state;
        if !matches!(state, "live" | "removed") {
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

    /// The gateway's time, from a refusal: this host signs with it from now.
    pub fn learn_offset(&self, gateway_now: f64) {
        let offset = gateway_now - self.now();
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
        let o = self.outbox();
        let dropped = self.with_store(|st| st.meta("dropped")).ok().flatten().and_then(|d| d.parse::<u64>().ok()).unwrap_or(0);
        let gateway = self.gateway();
        let queues: Vec<Value> = gateway.map(|g| g.queues().into_iter().map(|(h, f, b)| json!({ "host": h, "frames": f, "bytes": b })).collect()).unwrap_or_default();
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
            "rosterMismatch": self.roster_mismatch(),
            "at": t as i64,
        });
        let _ = self.dir.write_link(&v);
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
                self.dir.write_roster(&r)?;
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
                let _ = self.with_store(|s| s.forget_host(&e.host.id));
                if me.as_deref() == Some(e.host.id.as_str()) {
                    let by = e.signers.iter().map(|s| s.host.clone()).collect::<Vec<_>>();
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
        let at = info.get("at").and_then(Value::as_f64).map(xenonet_base::time::iso).unwrap_or_default();
        let words = format!("Removed on {} by {by}. Run xenonet net leave.", at.get(..10).unwrap_or(&at));
        self.status.locked().removed = Some(info);
        self.set("removed", Some(&words));
        self.log("net.removed", &words);
    }

    /// The gateway's `removed` answer, checked against the pinned key.
    pub fn removed_frame(&self, f: &Frame) -> bool {
        let Some(net) = self.network() else { return false };
        let Some(keys) = PublicKeys::from_b64(&net.gateway.sign, &net.gateway.seal) else { return false };
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
        let keys = PublicKeys::from_b64(&net.gateway.sign, &net.gateway.seal).ok_or("the pinned gateway key isn't readable")?;
        if f.kind != "hello" || f.from != net.gateway.host || !f.verify(&net.id, &keys) {
            return Err("the gateway didn't prove the key this host pinned".into());
        }
        let body = f.body_json().map_err(|e| e.message)?;
        if body.get("nonce").and_then(Value::as_str) != Some(nonce) || body.get("network").and_then(Value::as_str) != Some(net.id.as_str()) {
            return Err("the gateway's hello wasn't for this request".into());
        }
        let epoch = body.get("epoch").and_then(Value::as_str).filter(|e| is_b64(e)).ok_or("the gateway's hello has no epoch")?;
        let (had, _) = self.cursor();
        if had.as_deref() != Some(epoch) {
            let _ = self.with_store(|s| s.set_cursor(epoch, 0));
            if had.is_some() {
                // A new store at the gateway: what the old one held may be
                // gone, so the engine sends again what has no receipt.
                self.log("net.epoch", "the gateway's queues are new: resuming from the start");
                self.sink.gap();
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
                let _ = self.with_store(|s| s.set_cursor(&e, seq));
            }
        }
        fetch
    }

    fn gateway_frame(&self, net: &NetFile, f: &Frame) -> bool {
        let Some(keys) = PublicKeys::from_b64(&net.gateway.sign, &net.gateway.seal) else { return false };
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
                for h in members {
                    let p = peers.entry(h.clone()).or_default();
                    p.online = linked.contains(&h.as_str());
                    if p.online {
                        p.seen = Some(now);
                    }
                }
                self.status.locked().last_ok = Some(now);
                drop(peers);
                self.write_link(false);
                false
            }
            "gap" => {
                self.log("net.gap", &format!("the gateway let frames go: {body}"));
                self.sink.gap();
                false
            }
            "roster" => {
                let mut fetch = false;
                for e in body.get("entries").and_then(Value::as_array).into_iter().flatten() {
                    match Entry::parse(e).and_then(|e| self.append(e)) {
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
        let key = self.key().ok_or("this machine has no key")?;
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
        let mut e = r.draft(Op::Add, HostRef { id: host.clone(), keys, window }, None, self.gateway_now() as i64);
        drop(r);
        e.sign_by(&net.host, &key);
        self.log("net.ticket", &format!("this machine signed {host} into the network, with the ticket it made"));
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
        let now = self.now();
        if self.with_store(|s| s.seen(&f.id, now as i64)).unwrap_or(true) {
            return false;
        }
        let body = match open_frame(&net.id, key, f) {
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
        let Some(kind) = Kind::parse(&f.kind) else {
            // relay frames are the map's, which comes later.
            return false;
        };
        if kind == Kind::Operator {
            if let Err(why) = self.check_operator(&f.from, window, &body) {
                self.log("net.operator", &format!("an action from {} was refused: {why}", f.from));
                return false;
            }
        }
        if kind == Kind::Presence && !self.note_presence(&f.from, &body) {
            return false;
        }
        let at = f.at as f64 - self.status.locked().offset_ms;
        self.sink.receive(Incoming { from: f.from.clone(), kind, body, at: at.min(now), from_window_host: window });
        false
    }

    /// Whether the person's action from another host may be acted on: its
    /// host is a window host, the action is one another machine may do,
    /// it's under a minute old, it's new, and it was checked strongly.
    fn check_operator(&self, from: &str, window: bool, body: &Value) -> Result<(), String> {
        if !window {
            return Err(format!("{from} isn't a window host"));
        }
        let action = body.get("action").and_then(Value::as_str).unwrap_or("");
        if !OPERATOR_ACTIONS.contains(&action) {
            return Err(format!("{action} isn't done from another machine"));
        }
        let at = body.get("at").and_then(Value::as_f64).unwrap_or(0.0);
        let now = self.gateway_now();
        if (now - at).abs() > OPERATOR_MS {
            return Err("it's more than a minute old".into());
        }
        let nonce = body.get("nonce").and_then(Value::as_str).unwrap_or("");
        {
            let mut seen = self.operator_nonces.locked();
            seen.retain(|_, t| now - *t < 2.0 * OPERATOR_MS);
            if nonce.is_empty() || seen.insert(format!("{from}|{nonce}"), now).is_some() {
                return Err("it was seen before".into());
            }
        }
        let checked = body.get("checked").and_then(Value::as_str);
        if !matches!(checked, Some("window" | "person")) || body.get("strength").and_then(Value::as_str) != Some("strong") {
            return Err(format!("it was checked {} on {from}, which isn't strong enough to act on", body.get("checked").and_then(Value::as_str).unwrap_or("not at all")));
        }
        Ok(())
    }

    /// A presence, kept for the hosts' list: false when it's older than one
    /// already had.
    fn note_presence(&self, from: &str, body: &Value) -> bool {
        let counter = body.get("counter").and_then(Value::as_u64).unwrap_or(0);
        let mut peers = self.peers.locked();
        let p = peers.entry(from.to_string()).or_default();
        if counter != 0 && counter < p.counter {
            return false;
        }
        p.counter = counter;
        if body.get("full").and_then(Value::as_bool) == Some(true) {
            p.agents.clear();
        }
        for a in body.get("agents").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = a.get("id").and_then(Value::as_str) {
                p.agents.insert(id.to_string(), a.get("waitsOnYou").and_then(Value::as_bool).unwrap_or(false));
            }
        }
        for r in body.get("removed").and_then(Value::as_array).into_iter().flatten() {
            if let Some(id) = r.as_str() {
                p.agents.remove(id);
            }
        }
        p.you = body.get("you").and_then(Value::as_bool).unwrap_or(false);
        // An empty head says nothing about the sender's roster: no mismatch.
        p.roster = body.get("roster").and_then(Value::as_str).filter(|r| !r.is_empty()).map(str::to_string);
        true
    }

    // ---------- the up batch ----------

    /// The next batch: what waits in memory, then the outbox's oldest.
    pub fn next_batch(&self) -> Option<Batch> {
        let volatile: Vec<Frame> = std::mem::take(&mut *self.volatile.locked());
        let queued = self.with_store(|s| s.batch(BATCH_BYTES, BATCH_FRAMES)).unwrap_or_default();
        if volatile.is_empty() && queued.is_empty() {
            return None;
        }
        let mut lines: Vec<String> = volatile.iter().map(|f| f.line().trim_end().to_string()).collect();
        let ids: Vec<String> = queued.iter().map(|(id, _)| id.clone()).collect();
        lines.extend(queued.into_iter().map(|(_, l)| l));
        let (epoch, seq) = self.cursor();
        let ack = epoch.map_or(Value::Null, |e| json!({ "epoch": e, "seq": seq }));
        Some(Batch { ids, body: format!("{{\"frames\":[{}],\"ack\":{ack}}}", lines.join(",")) })
    }

    /// The gateway's answer to a batch: what it took, and what it refused
    /// for good, leave the outbox; what it held back for its rate waits.
    pub fn batch_done(&self, b: &Batch, answer: &Value) {
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
        let _ = self.with_store(|s| s.done(&done));
        self.write_link(false);
    }
}

impl Net for Shared {
    fn me(&self) -> Option<Me> {
        self.network().map(|n| Me { network: n.id, name: n.name, host: n.host })
    }

    fn roster_head(&self) -> Option<String> {
        self.network()?;
        self.roster().head()
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
        if self.is_removed() {
            return Err("this machine was removed from its network".into());
        }
        let key = self.key().ok_or("this machine has no network key")?;
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
            let id = frame_id().map_err(|e| e.message)?;
            let d = Draft { network: &net.id, id, kind: out.kind.as_str(), from: &net.host, to: &to, at };
            let f = host_frame(&d, &key, &keys, &out.body, None).map_err(|e| e.message)?;
            if out.kind.queued() {
                let line = f.line();
                self.with_store(|s| s.push(&f.id, &to, &f.kind, line.trim_end(), now, &self.opts.outbox)).map_err(|e| e.message)?;
            } else if self.is_live() {
                self.volatile.locked().push(f);
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
}

impl Node {
    /// The node, as its files leave it; nothing runs until `start`.
    pub fn open(opts: Options, sink: Arc<SinkSlot>) -> R<Arc<Node>> {
        let dir = Dir::new(opts.dir.clone());
        let net = dir.read_network()?;
        let (key, roster, store) = match &net {
            Some(_) => {
                let key = HostKey::load(&dir.file(KEY))?.ok_or_else(|| NetError::internal("this host's key is missing"))?;
                (Some(Arc::new(key)), dir.read_roster()?, Some(HostStore::open(&dir.file(HOST_DB))?))
            }
            None => (None, Roster::new(), None),
        };
        let shared = Arc::new(Shared {
            opts,
            dir,
            key: RwLock::new(key),
            net: RwLock::new(net),
            roster: RwLock::new(roster),
            store: Mutex::new(store),
            sink,
            status: Mutex::new(Status::default()),
            peers: Mutex::new(BTreeMap::new()),
            wake: Notify::new(),
            volatile: Mutex::new(Vec::new()),
            gateway: RwLock::new(None),
            operator_nonces: Mutex::new(HashMap::new()),
            link_written: Mutex::new(0.0),
            proposals: Mutex::new(Vec::new()),
            relink: Notify::new(),
            pending: Mutex::new(Vec::new()),
            signed: Mutex::new(Vec::new()),
        });
        Ok(Arc::new(Node { shared, tasks: Mutex::new(Vec::new()) }))
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
        let store = n.store.locked();
        if let Some(s) = store.as_ref() {
            let _ = s.prune(n.now() as i64);
        }
        *self.tasks.locked() = tasks;
        Ok(())
    }

    /// Stops the link and the gateway: its streams end.
    pub fn stop(&self) {
        for t in self.tasks.locked().drain(..) {
            t.abort();
        }
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
        n.dir.write_roster(&roster)?;
        n.dir.write_network(&net)?;
        let store = HostStore::open(&n.dir.file(HOST_DB))?;
        *n.store.locked() = Some(store);
        *n.key.writing() = Some(Arc::new(key));
        *n.roster.writing() = roster;
        *n.net.writing() = Some(net);
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        Ok(())
    }

    /// `xenonet net create`: a new network with this machine as its founder
    /// and gateway. `address` is where hosts reach it: the LAN address its
    /// gateway listens on, or the tunnel's host name.
    pub fn create(&self, name: &str, host: &str, address: Option<&str>, lan: Option<&str>) -> R<Value> {
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
        let key = HostKey::load_or_create(&n.dir.file(KEY))?;
        let id = crate::codec::network_id()?;
        let at = n.now() as i64;
        let roster = Roster::from_entries(vec![crate::roster::found(&id, host, &key, None, at)])?;
        let p = key.public();
        let net = NetFile {
            id,
            name: name.into(),
            host: host.into(),
            founder: host.into(),
            gateway: Pin { host: host.into(), key_id: p.key_id(), sign: p.sign_b64(), seal: p.seal_b64(), address: lan.or(address).map(str::to_string) },
            lan: lan.map(str::to_string),
            titles_only: false,
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
        let body = join_body(&key, &code, &nonce, &n.opts.platform, &n.opts.version, n.opts.desktop);
        let t = link::http(address)?;
        let req = NetRequest { method: "POST".into(), target: "/net/v1/join".into(), headers: Vec::new(), body: body.to_string().into_bytes() };
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
        let roster = Roster::from_json(v.get("roster").unwrap_or(&Value::Null))?;
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
            gateway: Pin { host: view.gateway.clone(), key_id: gw_keys.key_id(), sign: gw_keys.sign_b64(), seal: gw_keys.seal_b64(), address: Some(address.to_string()) },
            lan: None,
            titles_only: false,
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
        let (net, key) = self.me()?;
        let others = n.roster().view().members.len() > 1;
        if others && !force && (n.is_gateway() || net.founder == net.host) {
            let what = if net.founder == net.host { "founded the network, and its key is the network's root" } else { "is the network's gateway" };
            return Err(NetError::new(
                409,
                "needed",
                format!("this machine {what}: the other hosts lose it if it leaves. Move the gateway or remove the other hosts first, or leave with --force."),
            ));
        }
        let mut asked = false;
        if net.founder != net.host && !n.is_removed() {
            let me = n.roster().view().member(&net.host).cloned();
            if let Some(m) = me {
                let mut e = n.roster().draft(Op::Remove, HostRef { id: m.id.clone(), keys: m.keys, window: m.window }, None, n.gateway_now() as i64);
                e.sign_by(&net.host, &key);
                asked = self.submit(e).await.is_ok();
            }
        }
        self.stop();
        n.dir.forget()?;
        *n.store.locked() = None;
        *n.key.writing() = None;
        *n.roster.writing() = Roster::new();
        *n.net.writing() = None;
        *n.status.locked() = Status::default();
        n.peers.locked().clear();
        n.log("net.leave", &format!("this machine left the network {}", net.name));
        Ok(json!({ "left": net.name, "removed": asked }))
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
        link::call(n, &t, "POST", "/net/v1/roster", json!({ "entry": e.to_json() })).await?;
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
    /// to approve it.
    pub async fn ticket(&self, host: &str, window: bool) -> R<Value> {
        let n = &self.shared;
        let (net, _) = self.me()?;
        if !is_host_id(host) {
            return Err(NetError::bad("a host id is 2 to 32 lowercase letters, digits and dashes, starting with a letter"));
        }
        let view = n.roster().view();
        if view.member(host).is_some() {
            return Err(NetError::new(409, "taken", format!("{host} is in the network already: Replace it to add a reinstalled machine")));
        }
        if !view.is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "a ticket is made on a window host"));
        }
        let Some(address) = net.gateway.address.clone() else {
            return Err(NetError::new(
                409,
                "no-address",
                "the gateway has no address yet: turn the gateway on with a tunnel (xenonet gateway on --tunnel <kind>), or give it a LAN address (xenonet net set --lan <ip:port>)",
            ));
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
        let (net, key) = self.me()?;
        if host == net.host {
            return Err(NetError::bad("to take this machine out, run xenonet net leave"));
        }
        let m = n.roster().view().member(host).cloned().ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        let mut e = n.roster().draft(Op::Remove, HostRef { id: m.id.clone(), keys: m.keys, window: m.window }, None, n.gateway_now() as i64);
        e.sign_by(&net.host, &key);
        self.submit(e).await?;
        n.log("net.remove", &format!("{host} was removed from the network"));
        Ok(json!({ "removed": host, "state": self.state() }))
    }

    /// **Replace**: the old entry goes, and a ticket comes for the same id
    /// and window choice, for the reinstalled machine to join with.
    pub async fn replace(&self, host: &str) -> R<Value> {
        let window = self.shared.roster().view().member(host).map(|m| m.window).ok_or_else(|| NetError::new(404, "no-host", format!("{host} isn't in the network")))?;
        self.remove(host).await?;
        let mut t = self.ticket(host, window).await?;
        t["replaced"] = json!(host);
        Ok(t)
    }

    /// Settings: titles only, and the LAN address the gateway listens on.
    pub fn settings(&self, titles_only: Option<bool>, lan: Option<Option<String>>) -> R<Value> {
        let n = &self.shared;
        {
            let mut guard = n.net.writing();
            let net = guard.as_mut().ok_or_else(not_in)?;
            if let Some(t) = titles_only {
                net.titles_only = t;
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

    /// An `operator` frame for another host: the person's action, with how
    /// it was checked here.
    pub fn operator(&self, to: &str, action: &str, args: Value, checked: &str, strong: bool) -> R<()> {
        let body = json!({
            "schema": "network/operator/v1",
            "action": action,
            "args": args,
            "checked": checked,
            "strength": if strong { "strong" } else { "weak" },
            "nonce": crate::codec::nonce()?,
            "at": self.shared.gateway_now() as i64,
        });
        self.shared.send(Outgoing { to: To::Host(to.into()), kind: Kind::Operator, body }).map_err(|e| NetError::new(409, "refused", e))
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
            });
        };
        let (view, head) = {
            let r = n.roster();
            (r.view(), r.head())
        };
        let s = n.status();
        let peers = n.peers();
        let key_id = n.key().map(|k| k.key_id()).unwrap_or_default();
        let is_gateway = view.gateway == net.host;
        let gw = n.gateway();
        let info = gw.as_ref().map(|g| g.host_info()).unwrap_or_default();
        let now = n.now();
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
                "agents": p.map_or(0, |p| p.agents.len()),
                "waitsOnYou": p.map_or(0, |p| p.agents.values().filter(|w| **w).count()),
                "you": p.is_some_and(|p| p.you),
                "platform": platform,
                "self": me,
            }));
        }
        // The gateway's host lists every ticket that waits; another window
        // host, the ones it made.
        let tickets: Vec<Value> = match gw.as_ref() {
            Some(g) => g.waiting_tickets(now as i64).into_iter().map(|t| json!({ "host": t.host, "window": t.window, "expires": t.expires })).collect(),
            None => n.pending_tickets().into_iter().map(|(host, window, expires)| json!({ "host": host, "window": window, "expires": expires })).collect(),
        };
        let o = n.outbox();
        let address = net.gateway.address.clone().unwrap_or_default();
        let kind = if address.is_empty() { "tunnel" } else { address_kind(&address) };
        let mut out = json!({
            "schema": "network/state/v1",
            "network": { "id": net.id, "name": net.name },
            "self": { "id": net.host, "keyId": key_id, "window": view.is_window(&net.host), "founder": view.founder == net.host, "gateway": is_gateway },
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
            "titlesOnly": net.titles_only,
        });
        if let Some(g) = gw {
            out["queues"] = Value::Array(g.queues().into_iter().map(|(h, f, b)| json!({ "host": h, "frames": f, "bytes": b })).collect());
            out["epoch"] = json!(g.epoch());
        }
        if let Some(r) = s.removed {
            out["removed"] = r;
        }
        if let Some(lan) = net.lan {
            out["lan"] = json!(lan);
        }
        let proposals: Vec<Value> = n
            .proposals()
            .iter()
            .map(|p| {
                json!({
                    "id": p.id(),
                    "op": p.op.as_str(),
                    "host": p.host.id,
                    "window": p.host.window,
                    "gateway": p.gateway.as_ref().map(|g| g.address.clone()),
                    "signers": p.signers.iter().map(|s| s.host.clone()).collect::<Vec<_>>(),
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
    pub async fn take_gateway(&self, address: Option<&str>, lan: Option<&str>) -> R<Value> {
        let n = &self.shared;
        let (net, key) = self.me()?;
        let view = n.roster().view();
        if view.gateway == net.host {
            return Err(NetError::new(409, "already", "this machine is the network's gateway already"));
        }
        let Some(address) = lan.or(address).map(str::to_string) else {
            return Err(NetError::new(409, "no-address", "give this machine an address first: a LAN address (--lan <ip:port>), or a tunnel (xenonet gateway on --tunnel <kind>)"));
        };
        let m = view.member(&net.host).cloned().ok_or_else(|| NetError::new(410, "removed", "this machine isn't in the network"))?;
        let mut e = n.roster().draft(
            Op::Gateway,
            HostRef { id: m.id.clone(), keys: m.keys, window: m.window },
            Some(crate::roster::GatewayAt { address, version: view.gateway_version + 1 }),
            n.gateway_now() as i64,
        );
        e.sign_by(&net.host, &key);
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
                link::call(n, &t, "POST", "/net/v1/roster", json!({ "entry": e.to_json(), "propose": true })).await?;
                n.note_proposals(vec![e]);
                n.log("net.gateway", "this machine asked to become the network's gateway; the founder or another window host approves it");
                let mut out = self.state();
                out["proposed"] = json!(true);
                return Ok(out);
            }
            Err(err) => return Err(err),
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
    /// the state lists it): this host signs exactly that entry too.
    pub async fn approve(&self, id: &str) -> R<Value> {
        let n = &self.shared;
        let (net, key) = self.me()?;
        if !n.roster().view().is_window(&net.host) {
            return Err(NetError::new(403, "not-window", "approving is a window host's"));
        }
        let Some(mut e) = n.proposals().into_iter().find(|p| p.id() == id) else {
            return Err(NetError::new(404, "nothing", "nothing waits for approval by that id: xenonet net status lists what does"));
        };
        let (op, host) = (e.op.as_str(), e.host.id.clone());
        e.sign_by(&net.host, &key);
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
    }
}
