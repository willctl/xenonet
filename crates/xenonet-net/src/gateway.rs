//! The gateway's routes, `/net/v1/…` (docs/design/network.md, sections 2
//! and 9, and "The wire"), over any transport: the service's front door
//! hands them requests from the network, and the gateway's own host calls
//! them in-process.
//!
//! - `POST join`: a ticket, else a 404; 20 bad tickets in an hour shut the
//!   route for an hour.
//! - `GET link?epoch=&after=`: the down stream, NDJSON: a `hello` signed
//!   over the host's nonce, the queue after `after`, then frames as they
//!   come, a `ping` every 15 s, and the end after 10 minutes. `&wait=25`
//!   answers after the first batch or 25 s, for proxies that hold streams.
//! - `POST frames`: the up batch; each frame's signature is checked
//!   against the roster before it's queued, a frame id seen in the last
//!   week is accepted again without being queued again, and each link may
//!   send 600 frames and 60 messages a minute. The map's relay frames and
//!   services' svc frames go in batches of their own (`?relay=1`), each
//!   kind in a bucket of its own (120 relay and 600 svc frames a minute).
//!   They're never written to the store: they're handed to the host's
//!   link in memory, and dropped when it isn't linked, since a svc frame
//!   may carry a service's own credential.
//! - `GET roster?from=n`, and `POST roster`, an entry from a host, which
//!   the roster's own rules decide.
//!
//! Every route but `join` wants a signed request: method, target, time,
//! nonce and the body's SHA-256. A removed host is answered, once its old
//! key signed the request, with a signed `removed` frame.

use crate::codec::{b64, frame_id, unb64};
use crate::frame::{gateway_frame, Draft, Frame, HOST_KINDS};
use crate::gwstore::{Dropped, GwStore, Item, TicketRow};
use crate::keys::PublicKeys;
use crate::node::Shared;
use crate::request::{Signed, NONCE_MS, SKEW_MS};
use crate::roster::{Entry, HostRef, Offered, Op};
use crate::ticket;
use crate::{NetError, R};
use bytes::Bytes;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, Notify};
use xenonet_base::sync::Locked;

pub const UP_LIMIT: usize = 1 << 20;
pub const RELAY_LIMIT: usize = 16 << 20;
pub const SMALL_LIMIT: usize = 64 << 10;
/// Bad tickets from one source in an hour that shut the join route to it
/// for an hour; from everywhere, that shut it for everyone.
const BAD_TICKETS: usize = 20;
const BAD_TICKETS_ALL: usize = 200;
/// Wrong proofs aimed at one ticket (by its tag) that retire it.
const WRONG_PROOFS: u32 = 5;
const HOUR_MS: f64 = 3600e3;
const FRAMES_A_MINUTE: f64 = 600.0;
const MESSAGES_A_MINUTE: f64 = 60.0;
/// The map's relay frames a link sends, in a bucket of their own.
const RELAY_A_MINUTE: f64 = 120.0;
/// Services' requests and answers a link sends, in a bucket of their own:
/// a host lets each caller make 300 requests a minute to each service.
const SVC_A_MINUTE: f64 = 600.0;
/// A relay frame on the wire, at most: a body of `node::RELAY_BODY`, sealed
/// and in base64url, with its head.
pub const RELAY_FRAME_LIMIT: usize = 12 << 20;
const BATCH_ROWS: u64 = 500;
/// A host that long-polls counts as linked this long after its last poll.
const POLLED_MS: f64 = 40e3;
/// Entries waiting for approval, at most.
pub const MAX_PROPOSALS: usize = 16;
/// A ticket's maker that hasn't signed its new host in is asked again
/// after this.
const ASK_AGAIN_MS: f64 = 30e3;

/// A request at the gateway: its method, target (path and query), headers
/// with lowercase names, and body.
#[derive(Debug, Clone, Default)]
pub struct NetRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Where it came from, as the front door saw it: the peer's address on
    /// a LAN, the tunnel's forwarded address behind one; None in-process.
    pub source: Option<String>,
}

impl NetRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn path(&self) -> &str {
        self.target.split('?').next().unwrap_or("")
    }

    /// A query parameter's value; None when it's missing, or given more
    /// than once, so the front door and the routes read it alike.
    pub fn query(&self, name: &str) -> Option<String> {
        let q = self.target.split_once('?')?.1;
        let mut found = q.split('&').filter_map(|p| p.split_once('=')).filter(|(k, _)| *k == name);
        let first = found.next()?;
        found.next().is_none().then(|| first.1.to_string())
    }
}

pub enum NetBody {
    Full(Vec<u8>),
    /// The down stream, a chunk at a time; it ends when the sender drops.
    Stream(mpsc::UnboundedReceiver<Bytes>),
}

pub struct NetReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: NetBody,
}

impl NetReply {
    pub fn json(status: u16, v: &Value) -> NetReply {
        NetReply {
            status,
            headers: vec![("content-type".into(), "application/json".into()), ("cache-control".into(), "no-store".into())],
            body: NetBody::Full(v.to_string().into_bytes()),
        }
    }

    pub fn error(e: &NetError) -> NetReply {
        NetReply::json(e.status, &json!({ "error": e.message, "code": e.code }))
    }

    fn not_found() -> NetReply {
        NetReply::json(404, &json!({ "error": "not found" }))
    }

    fn lines(lines: Vec<String>) -> NetReply {
        NetReply { status: 200, headers: ndjson(), body: NetBody::Full(lines.concat().into_bytes()) }
    }

    /// The body, when it's all there, as JSON.
    pub fn body_json(&self) -> Value {
        match &self.body {
            NetBody::Full(b) => serde_json::from_slice(b).unwrap_or(Value::Null),
            NetBody::Stream(_) => Value::Null,
        }
    }
}

fn ndjson() -> Vec<(String, String)> {
    vec![("content-type".into(), "application/x-ndjson".into()), ("cache-control".into(), "no-store".into())]
}

/// A host's link as the gateway sees it.
#[derive(Default)]
struct Live {
    notify: Arc<Notify>,
    streams: usize,
    polled: f64,
    /// Its open streams, oldest first, each with what ends it.
    open: std::collections::VecDeque<(u64, Arc<Notify>)>,
    next: u64,
    /// Relay and svc frames for it, in memory only, until its link reads
    /// them: their lines, and when each came.
    held: std::collections::VecDeque<(std::time::Instant, String)>,
    held_bytes: usize,
}

impl Live {
    fn linked(&self, now: f64) -> bool {
        self.streams > 0 || now - self.polled < POLLED_MS
    }

    fn drop_held(&mut self) {
        self.held.clear();
        self.held_bytes = 0;
    }
}

/// Relay and svc frames held in memory for one host's link, at most: room
/// for two of the largest.
const HELD_BYTES: usize = 32 << 20;
/// How long a relay or svc frame waits in memory for a host's link: a host
/// drops one older than this (node.rs, RELAY_MS) anyway.
const HELD_FOR: Duration = Duration::from_secs(120);

/// Down streams one host may hold open at once: a new one ends the oldest.
const STREAMS_PER_HOST: usize = 2;

#[derive(Clone, Copy)]
pub(crate) struct Bucket {
    tokens: f64,
    at: f64,
}

impl Bucket {
    pub(crate) fn full(burst: f64, now: f64) -> Bucket {
        Bucket { tokens: burst, at: now }
    }

    /// Takes one, or says how long until there's one.
    pub(crate) fn take(&mut self, per_minute: f64, now: f64) -> Result<(), f64> {
        let rate = per_minute / 60e3;
        self.tokens = (self.tokens + (now - self.at) * rate).min(per_minute);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err((1.0 - self.tokens) / rate)
        }
    }
}

/// A link's rates: its frames, its messages among them, and the map's relay
/// frames and services' svc frames apart.
struct Rates {
    frames: Bucket,
    messages: Bucket,
    relay: Bucket,
    svc: Bucket,
}

impl Rates {
    fn full(now: f64) -> Rates {
        Rates {
            frames: Bucket::full(FRAMES_A_MINUTE, now),
            messages: Bucket::full(MESSAGES_A_MINUTE, now),
            relay: Bucket::full(RELAY_A_MINUTE, now),
            svc: Bucket::full(SVC_A_MINUTE, now),
        }
    }
}

#[derive(Default)]
struct BadTickets {
    at: Vec<f64>,
    closed_until: f64,
}

impl BadTickets {
    /// One more in the last hour: true when that shuts the route.
    fn count(&mut self, now: f64, most: usize) -> bool {
        self.at.retain(|t| now - *t < HOUR_MS);
        self.at.push(now);
        if self.at.len() >= most {
            self.closed_until = now + HOUR_MS;
            self.at.clear();
            return true;
        }
        false
    }
}

/// Bad tickets: from everywhere, from each source, and wrong proofs aimed
/// at each waiting ticket.
#[derive(Default)]
struct Bad {
    all: BadTickets,
    by_source: HashMap<String, BadTickets>,
    wrong: HashMap<String, u32>,
}

pub struct Gateway {
    node: Weak<Shared>,
    /// Its store, until `close`.
    store: Mutex<Option<GwStore>>,
    epoch: String,
    live: Mutex<HashMap<String, Live>>,
    nonces: Mutex<HashMap<String, f64>>,
    bad: Mutex<Bad>,
    rates: Mutex<HashMap<String, Rates>>,
    stopped: Notify,
    /// Entries signed by too few, waiting for approval, and when they came.
    proposals: Mutex<Vec<(Entry, f64)>>,
    /// The roster grew, or an entry came to wait for approval: a join
    /// waiting on a ticket's maker looks again.
    roster_changed: Notify,
    /// Tickets whose makers were asked to sign their new host in: the
    /// roster's length then, and when.
    asked: Mutex<HashMap<String, (u64, f64)>>,
}

/// A gateway's store while it's open: `Gateway::store` makes one only
/// over a store that's there.
struct Open<'a>(MutexGuard<'a, Option<GwStore>>);

impl std::ops::Deref for Open<'_> {
    type Target = GwStore;
    fn deref(&self) -> &GwStore {
        self.0.as_ref().expect("an Open is made only over an open store")
    }
}

/// What the gateway knows of itself, from its host's node.
struct Me {
    network: String,
    name: String,
    host: String,
    key: Arc<crate::keys::HostKey>,
    address: Option<String>,
}

impl Gateway {
    pub fn new(node: Weak<Shared>, store: GwStore) -> R<Arc<Gateway>> {
        let epoch = store.epoch()?;
        Ok(Arc::new(Gateway {
            node,
            store: Mutex::new(Some(store)),
            epoch,
            live: Mutex::new(HashMap::new()),
            nonces: Mutex::new(HashMap::new()),
            bad: Mutex::new(Bad::default()),
            rates: Mutex::new(HashMap::new()),
            stopped: Notify::new(),
            proposals: Mutex::new(Vec::new()),
            roster_changed: Notify::new(),
            asked: Mutex::new(HashMap::new()),
        }))
    }

    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    /// Ends every stream: the gateway stops answering.
    pub fn stop(&self) {
        self.stopped.notify_waiters();
    }

    /// Stops it and closes its store for good, as its host leaves the
    /// network: the store's file goes next, and Windows won't delete a file
    /// that's open. Streams that haven't ended yet hold the gateway, not
    /// the file; anything it's asked from now on is refused.
    pub fn close(&self) {
        self.stop();
        *self.store.locked() = None;
    }

    /// Its store, while it has one.
    fn store(&self) -> R<Open<'_>> {
        let store = self.store.locked();
        if store.is_none() {
            return Err(NetError::new(503, "stopped", "the gateway stopped"));
        }
        Ok(Open(store))
    }

    fn me(n: &Shared) -> R<Me> {
        let net = n.network().ok_or_else(|| NetError::new(404, "no-network", "not found"))?;
        let key = n.key().ok_or_else(|| NetError::internal("this host has no key"))?;
        Ok(Me { network: net.id, name: net.name, host: net.host, key, address: net.gateway.address })
    }

    fn now(n: &Shared) -> f64 {
        n.now()
    }

    /// A gateway frame for `to`.
    fn frame(me: &Me, kind: &str, to: &str, at: i64, body: &Value) -> R<Frame> {
        Ok(gateway_frame(&Draft { network: &me.network, id: frame_id()?, kind, from: &me.host, to, at }, &me.key, body))
    }

    /// Hosts with a link open now, this one among them.
    pub fn linked(&self, now: f64) -> Vec<String> {
        let mut out: Vec<String> = self.live.locked().iter().filter(|(_, l)| l.linked(now)).map(|(h, _)| h.clone()).collect();
        out.sort();
        out
    }

    fn is_linked(&self, host: &str, now: f64) -> bool {
        self.live.locked().get(host).is_some_and(|l| l.linked(now))
    }

    /// Hands a relay or svc frame's line to `host`'s link, in memory only,
    /// never the store: a svc frame may carry a service's own credential.
    /// false, and the frame dropped, when the host isn't linked.
    fn hand_over(&self, host: &str, line: &str, now: f64) -> bool {
        let mut live = self.live.locked();
        let Some(l) = live.get_mut(host) else { return false };
        if !l.linked(now) {
            l.drop_held();
            return false;
        }
        let line = format!("{line}\n");
        // The oldest go first: what's past its time, then what doesn't fit.
        while l.held.front().is_some_and(|(t, _)| t.elapsed() > HELD_FOR || l.held_bytes + line.len() > HELD_BYTES) {
            if let Some((_, old)) = l.held.pop_front() {
                l.held_bytes -= old.len();
            }
        }
        l.held_bytes += line.len();
        l.held.push_back((std::time::Instant::now(), line));
        true
    }

    /// What waits in memory for `host`'s link, oldest first, less what's
    /// past its time.
    fn take_held(&self, host: &str) -> Vec<String> {
        let mut live = self.live.locked();
        let Some(l) = live.get_mut(host) else { return Vec::new() };
        l.held_bytes = 0;
        std::mem::take(&mut l.held).into_iter().filter(|(t, _)| t.elapsed() <= HELD_FOR).map(|(_, line)| line).collect()
    }

    /// How many relay and svc frames wait in memory for `host`'s link.
    pub fn held(&self, host: &str) -> usize {
        self.live.locked().get(host).map_or(0, |l| l.held.len())
    }

    fn notifier(&self, host: &str) -> Arc<Notify> {
        self.live.locked().entry(host.to_string()).or_default().notify.clone()
    }

    fn wake(&self, host: &str) {
        self.notifier(host).notify_waiters();
    }

    /// Whether the gateway remembers a frame's id, and so takes a repeat
    /// of it as accepted without queueing it again.
    pub fn remembers(&self, id: &str) -> bool {
        self.store().and_then(|s| s.was_seen(id)).unwrap_or(false)
    }

    /// Queue sizes, for the state and the doctor: (host, frames, bytes).
    pub fn queues(&self) -> Vec<(String, u64, u64)> {
        self.store().and_then(|s| s.sizes()).unwrap_or_default()
    }

    /// What each host said of itself when it joined: id → (platform, version).
    pub fn host_info(&self) -> HashMap<String, (Option<String>, Option<String>)> {
        self.store().and_then(|s| s.hosts()).unwrap_or_default().into_iter().map(|(id, p, v)| (id, (p, v))).collect()
    }

    /// One request, by its path.
    pub async fn handle(self: &Arc<Self>, req: NetRequest) -> NetReply {
        if req.header("origin").is_some() {
            return NetReply::json(403, &json!({ "error": "the network's routes take no requests from web pages" }));
        }
        let Some(n) = self.node.upgrade() else { return NetReply::not_found() };
        // A gateway that was moved keeps its streams a while, so the hosts
        // on them hear of the move, but takes nothing new: frames wait in
        // their senders' outboxes for the new gateway.
        if req.method == "POST" && !n.is_gateway() {
            let to = n.roster().view().gateway;
            return NetReply::json(409, &json!({ "error": format!("the network's gateway moved to {to}"), "code": "moved" }));
        }
        let answer = match (req.method.as_str(), req.path()) {
            ("POST", "/net/v1/join") => self.join(&n, &req).await,
            ("POST", "/net/v1/tickets") => self.ticket_post(&n, &req),
            ("GET", "/net/v1/link") => self.link(&n, &req).await,
            ("POST", "/net/v1/frames") => self.frames(&n, &req),
            ("GET", "/net/v1/roster") => Ok(self.roster_get(&n, &req)),
            ("POST", "/net/v1/roster") => self.roster_post(&n, &req),
            _ => return NetReply::not_found(),
        };
        answer.unwrap_or_else(|e| if e.status == 404 { NetReply::not_found() } else { NetReply::error(&e) })
    }

    /// The host a signed request is from, or the answer that refuses it.
    fn authenticate(&self, n: &Shared, req: &NetRequest) -> Result<Signed, NetReply> {
        let me = Self::me(n).map_err(|e| NetReply::error(&e))?;
        let Some(signed) = Signed::from_headers(|k| req.header(k).map(str::to_string)) else {
            return Err(NetReply::json(401, &json!({ "error": "the network's routes take signed requests", "code": "unsigned" })));
        };
        let view = n.roster().view();
        let now = Self::now(n);
        if let Some((keys, removal)) = n.removed_keys(&signed.host).filter(|_| view.member(&signed.host).is_none()) {
            if !signed.verify(&keys, &me.network, &req.method, &req.target, &req.body) {
                return Err(NetReply::json(401, &json!({ "error": "the request's signature doesn't check", "code": "bad-signature" })));
            }
            let body = json!({ "network": me.network, "host": signed.host, "by": removal.by, "at": removal.at, "n": removal.n });
            let f = Self::frame(&me, "removed", &signed.host, now as i64, &body).map_err(|e| NetReply::error(&e))?;
            return Err(NetReply { status: 410, headers: ndjson(), body: NetBody::Full(f.line().into_bytes()) });
        }
        let Some(member) = view.member(&signed.host) else {
            return Err(NetReply::json(401, &json!({ "error": "that host isn't in this network", "code": "unknown-host" })));
        };
        if !signed.verify(&member.keys, &me.network, &req.method, &req.target, &req.body) {
            return Err(NetReply::json(401, &json!({ "error": "the request's signature doesn't check", "code": "bad-signature" })));
        }
        if (signed.time as f64 - now).abs() > SKEW_MS {
            // The gateway's time, in a frame its key signs over this
            // request's nonce: a host takes a time only from its own
            // gateway, and only in answer to the request it made.
            let f = Self::frame(&me, "ping", &signed.host, now as i64, &json!({ "now": now as i64, "nonce": signed.nonce })).map_err(|e| NetReply::error(&e))?;
            return Err(NetReply::json(
                401,
                &json!({ "error": "the request's time is more than a minute off the gateway's", "code": "clock", "now": now as i64, "frame": f.line().trim_end() }),
            ));
        }
        let mut nonces = self.nonces.locked();
        nonces.retain(|_, at| now - *at < NONCE_MS);
        let k = format!("{}|{}", signed.host, signed.nonce);
        if nonces.contains_key(&k) {
            return Err(NetReply::json(409, &json!({ "error": "that request was made before", "code": "replay" })));
        }
        nonces.insert(k, now);
        Ok(signed)
    }

    // ---------- the down stream ----------

    fn hello(&self, n: &Shared, me: &Me, host: &str, nonce: &str) -> R<String> {
        let roster = n.roster();
        let mut hosts = Map::new();
        for (id, (platform, version)) in self.host_info() {
            hosts.insert(id, json!({ "platform": platform, "version": version }));
        }
        let now = Self::now(n);
        let body = json!({
            "epoch": self.epoch,
            "nonce": nonce,
            "now": now as i64,
            "network": me.network,
            "gateway": me.host,
            "keyId": me.key.key_id(),
            "rosterN": roster.len(),
            "rosterHead": roster.head(),
            "hosts": hosts,
        });
        Ok(Self::frame(me, "hello", host, now as i64, &body)?.line())
    }

    fn ping(&self, n: &Shared, me: &Me, host: &str) -> R<String> {
        let now = Self::now(n);
        Ok(Self::frame(me, "ping", host, now as i64, &json!({ "now": now as i64, "linked": self.linked(now) }))?.line())
    }

    /// The queue's lines after `after`, each with its seq: the lines and the
    /// last seq.
    fn backlog(&self, host: &str, after: u64) -> R<(Vec<String>, u64)> {
        let rows = self.store()?.after(host, after, BATCH_ROWS)?;
        let mut last = after;
        let mut out = Vec::new();
        for (seq, line) in rows {
            last = last.max(seq);
            if let Ok(mut f) = Frame::parse_line(&line) {
                f.seq = Some(seq);
                out.push(f.line());
            }
        }
        Ok((out, last))
    }

    async fn link(self: &Arc<Self>, n: &Arc<Shared>, req: &NetRequest) -> R<NetReply> {
        let signed = match self.authenticate(n, req) {
            Ok(s) => s,
            Err(reply) => return Ok(reply),
        };
        let me = Self::me(n)?;
        let host = signed.host.clone();
        let asked = req.query("after").and_then(|a| a.parse::<u64>().ok()).unwrap_or(0);
        let after = if req.query("epoch").as_deref() == Some(self.epoch.as_str()) { asked } else { 0 };
        // Never past what was given out: a host that says it has more would
        // wait for frames that never come.
        let after = after.min(self.store()?.last_seq(&host)?);
        if after > 0 {
            self.store()?.ack(&host, after)?;
        }
        let hello = self.hello(n, &me, &host, &signed.nonce)?;
        let timing = n.timing();
        if let Some(wait) = req.query("wait") {
            // The long-poll: the first batch, or nothing after the wait.
            let secs = wait.parse::<u64>().unwrap_or(25).min(timing.poll_ms / 1000).max(1);
            let notify = self.notifier(&host);
            self.live.locked().entry(host.clone()).or_default().polled = Self::now(n);
            let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
            let mut lines = vec![hello];
            loop {
                let notified = notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let (batch, _) = self.backlog(&host, after)?;
                let held = self.take_held(&host);
                if !batch.is_empty() || !held.is_empty() {
                    lines.extend(batch);
                    lines.extend(held);
                    break;
                }
                tokio::select! {
                    () = &mut notified => {}
                    () = tokio::time::sleep_until(deadline) => break,
                    () = self.stopped.notified() => break,
                }
            }
            self.live.locked().entry(host.clone()).or_default().polled = Self::now(n);
            return Ok(NetReply::lines(lines));
        }
        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
        let _ = tx.send(Bytes::from(hello));
        let presences = self.store()?.presences(&host)?;
        for p in presences {
            let _ = tx.send(Bytes::from(format!("{p}\n")));
        }
        let (notify, end, me_id) = {
            let mut live = self.live.locked();
            let l = live.entry(host.clone()).or_default();
            l.streams += 1;
            let end = Arc::new(Notify::new());
            let id = l.next;
            l.next += 1;
            l.open.push_back((id, end.clone()));
            // A host holds two streams at most: the oldest goes.
            while l.open.len() > STREAMS_PER_HOST {
                if let Some((_, old)) = l.open.pop_front() {
                    old.notify_one();
                }
            }
            (l.notify.clone(), end, id)
        };
        let me_gw = self.clone();
        let node = Arc::downgrade(n);
        tokio::spawn(async move {
            let mut last = after;
            let start = tokio::time::Instant::now();
            let until = start + Duration::from_millis(timing.stream_ms);
            let ping_every = Duration::from_millis(timing.ping_ms);
            let mut next_ping = start + ping_every;
            loop {
                let notified = notify.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let Ok((batch, upto)) = me_gw.backlog(&host, last) else { break };
                let full = batch.len() as u64 >= BATCH_ROWS;
                let mut gone = false;
                for line in batch.into_iter().chain(me_gw.take_held(&host)) {
                    gone |= tx.send(Bytes::from(line)).is_err();
                }
                last = upto;
                if gone {
                    break;
                }
                if full {
                    continue;
                }
                // A host removed while its stream was open has had its
                // `removed` frame: the stream ends.
                let Some(n) = node.upgrade() else { break };
                if n.roster().view_ref().is_none_or(|v| v.member(&host).is_none()) {
                    break;
                }
                drop(n);
                tokio::select! {
                    () = &mut notified => {}
                    () = tokio::time::sleep_until(next_ping) => {
                        next_ping += ping_every;
                        let Some(n) = node.upgrade() else { break };
                        let Ok(me) = Self::me(&n) else { break };
                        let Ok(line) = me_gw.ping(&n, &me, &host) else { break };
                        if tx.send(Bytes::from(line)).is_err() {
                            break;
                        }
                    }
                    () = tokio::time::sleep_until(until) => break,
                    () = tx.closed() => break,
                    () = me_gw.stopped.notified() => break,
                    () = end.notified() => break,
                }
            }
            let now = node.upgrade().map_or(0.0, |n| Self::now(&n));
            if let Some(l) = me_gw.live.locked().get_mut(&host) {
                l.streams = l.streams.saturating_sub(1);
                l.open.retain(|(id, _)| *id != me_id);
                // No link reads what's held for it any more: it goes.
                if !l.linked(now) {
                    l.drop_held();
                }
            }
        });
        let mut headers = ndjson();
        // The stream ends its connection: a host dials again for the next.
        headers.push(("connection".into(), "close".into()));
        Ok(NetReply { status: 200, headers, body: NetBody::Stream(rx) })
    }

    // ---------- the up batch ----------

    fn frames(&self, n: &Shared, req: &NetRequest) -> R<NetReply> {
        let signed = match self.authenticate(n, req) {
            Ok(s) => s,
            Err(reply) => return Ok(reply),
        };
        let me = Self::me(n)?;
        let host = signed.host;
        let body: Value = serde_json::from_slice(&req.body).map_err(|_| NetError::bad("the batch isn't JSON"))?;
        if let Some(ack) = body.get("ack") {
            if ack.get("epoch").and_then(Value::as_str) == Some(self.epoch.as_str()) {
                if let Some(seq) = ack.get("seq").and_then(Value::as_u64) {
                    self.store()?.ack(&host, seq)?;
                }
            }
        }
        let list = body.get("frames").and_then(Value::as_array).ok_or_else(|| NetError::bad("a batch has a list of frames"))?;
        let relay_ok = req.query("relay").as_deref() == Some("1");
        let view = n.roster().view();
        let Some(sender) = view.member(&host) else { return Err(NetError::new(401, "unknown-host", "that host isn't in this network")) };
        let sender_keys = sender.keys;
        let now = Self::now(n);
        let caps = n.caps();
        let mut accepted = Vec::new();
        let mut refused = Vec::new();
        let mut retry_ms: f64 = 0.0;
        let mut woken = Vec::new();
        let mut dropped_all = Vec::new();
        for v in list {
            let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let mut refuse = |code: &str, why: &str| refused.push(json!({ "id": id, "code": code, "reason": why }));
            let f = match Frame::parse(v) {
                Ok(f) => f,
                Err(e) => {
                    refuse("bad-frame", &e.message);
                    continue;
                }
            };
            if f.from != host {
                refuse("not-yours", "a host sends only its own frames");
                continue;
            }
            if !HOST_KINDS.contains(&f.kind.as_str()) || !f.sealed {
                refuse("bad-kind", "hosts send sealed frames of their own kinds");
                continue;
            }
            if f.in_relay_batch() != relay_ok {
                refuse("bad-kind", "relay and svc frames go in a relay batch of their own, and only they do");
                continue;
            }
            // A frame but a relay or svc one is small: presence, a message,
            // a claim.
            if !relay_ok && v.to_string().len() > SMALL_LIMIT {
                refuse("too-big", "a frame is at most 64 KiB, but a relay or svc frame");
                continue;
            }
            if relay_ok && v.to_string().len() > RELAY_FRAME_LIMIT {
                refuse("too-big", "a relay or svc frame is at most 12 MiB");
                continue;
            }
            if f.to == host || view.member(&f.to).is_none() {
                refuse("no-host", "that host isn't in the network");
                continue;
            }
            if !f.verify(&me.network, &sender_keys) {
                refuse("bad-signature", "the frame's signature doesn't check");
                continue;
            }
            // Only frames kept for a host that's away are remembered by
            // id; presence, claims and relay frames say the latest, and a
            // repeat of one does no harm, so they'd only fill the store.
            if f.queued() && self.store()?.was_seen(&f.id)? {
                accepted.push(f.id.clone());
                continue;
            }
            {
                // The map's frames have a bucket of their own, so a console
                // watching a busy host can't spend what the engine sends.
                let mut rates = self.rates.locked();
                let r = rates.entry(host.clone()).or_insert_with(|| Rates::full(now));
                let over = if f.kind == "svc" {
                    r.svc.take(SVC_A_MINUTE, now).err()
                } else if relay_ok {
                    r.relay.take(RELAY_A_MINUTE, now).err()
                } else {
                    let mut over = r.frames.take(FRAMES_A_MINUTE, now).err();
                    if over.is_none() && f.kind == "message" {
                        over = r.messages.take(MESSAGES_A_MINUTE, now).err();
                    }
                    over
                };
                if let Some(wait) = over {
                    retry_ms = retry_ms.max(wait);
                    let why = match f.kind.as_str() {
                        "svc" => "a link sends at most 600 svc frames a minute",
                        "relay" => "a link sends at most 120 relay frames a minute",
                        _ => "a link sends at most 600 frames and 60 messages a minute",
                    };
                    refuse("rate", why);
                    continue;
                }
            }
            let mut plain = f.clone();
            plain.seq = None;
            let line = plain.line();
            let line = line.trim_end();
            let store = self.store()?;
            if f.queued() {
                store.seen(&f.id, now as i64)?;
            }
            if f.kind == "presence" {
                store.set_presence(&host, &f.to, line, now as i64)?;
            }
            if f.in_relay_batch() {
                if self.hand_over(&f.to, line, now) {
                    woken.push(f.to.clone());
                }
            } else if f.queued() || self.is_linked(&f.to, now) {
                let (_, dropped) = store.enqueue(Item { host: &f.to, id: &f.id, kind: &f.kind, sender: &host, line }, now as i64, &caps)?;
                dropped_all.extend(dropped);
                woken.push(f.to.clone());
            }
            accepted.push(f.id);
        }
        if !dropped_all.is_empty() {
            woken.extend(self.gaps(&me, &dropped_all, now)?);
        }
        woken.sort();
        woken.dedup();
        for h in woken {
            self.wake(&h);
        }
        let mut answer = json!({ "accepted": accepted, "refused": refused });
        if retry_ms > 0.0 {
            answer["retryAfterMs"] = json!(retry_ms.ceil() as i64);
        }
        Ok(NetReply::json(200, &answer))
    }

    /// `gap`s for frames a full queue let go: one to the host that lost
    /// them, and one to each sender, saying which of its frames went. The
    /// hosts woken.
    fn gaps(&self, me: &Me, dropped: &[Dropped], now: f64) -> R<Vec<String>> {
        let mut lost: HashMap<&str, u64> = HashMap::new();
        let mut senders: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
        for d in dropped {
            *lost.entry(&d.host).or_default() += 1;
            if !d.sender.is_empty() {
                senders.entry((&d.sender, &d.host)).or_default().push(&d.id);
            }
        }
        let store = self.store()?;
        let mut woken = Vec::new();
        for (host, count) in lost {
            let f = Self::frame(me, "gap", host, now as i64, &json!({ "lost": count, "at": now as i64 }))?;
            store.enqueue_gap(host, &f.id, f.line().trim_end(), now as i64)?;
            woken.push(host.to_string());
        }
        // The gateway's own host among the senders hears of it through its
        // own link, as any host does.
        for ((sender, to), ids) in senders {
            let f = Self::frame(me, "gap", sender, now as i64, &json!({ "dropped": ids, "to": to, "at": now as i64 }))?;
            store.enqueue_gap(sender, &f.id, f.line().trim_end(), now as i64)?;
            woken.push(sender.to_string());
        }
        Ok(woken)
    }

    /// Frames older than a week go, with their `gap`s.
    pub fn prune(&self) -> R<()> {
        let Some(n) = self.node.upgrade() else { return Ok(()) };
        let me = Self::me(&n)?;
        let now = Self::now(&n);
        let old = self.store()?.prune(now as i64, n.caps().keep_ms)?;
        if !old.is_empty() {
            for h in self.gaps(&me, &old, now)? {
                self.wake(&h);
            }
        }
        Ok(())
    }

    // ---------- the roster ----------

    fn roster_get(&self, n: &Shared, req: &NetRequest) -> NetReply {
        if let Err(reply) = self.authenticate(n, req) {
            return reply;
        }
        let from = req.query("from").and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
        let roster = n.roster();
        let entries: Vec<Value> = roster.since(from).iter().map(Entry::to_json).collect();
        NetReply::json(200, &json!({ "entries": entries, "n": roster.len() }))
    }

    fn roster_post(&self, n: &Shared, req: &NetRequest) -> R<NetReply> {
        let signed = match self.authenticate(n, req) {
            Ok(s) => s,
            Err(reply) => return Ok(reply),
        };
        let body: Value = serde_json::from_slice(&req.body).map_err(|_| NetError::bad("the entry isn't JSON"))?;
        let mut entry = Entry::parse(body.get("entry").unwrap_or(&Value::Null))?;
        if !entry.signers.iter().any(|s| s.host == signed.host) {
            return Err(NetError::new(403, "roster-refused", "a host sends entries it signed"));
        }
        let now = Self::now(n);
        // The same entry waiting for approval: their signatures add up.
        {
            let mut waiting = self.proposals.locked();
            waiting.retain(|(p, at)| now - at < ticket::TICKET_MS && p.n == n.roster().len());
            if let Some((p, _)) = waiting.iter().find(|(p, _)| p.signing_bytes() == entry.signing_bytes()) {
                entry.merge_signers(p);
            }
        }
        match self.append(n, entry.clone()) {
            Ok(offered) => {
                self.proposals.locked().retain(|(p, _)| p.signing_bytes() != entry.signing_bytes());
                Ok(NetReply::json(200, &json!({ "ok": true, "duplicate": offered == Offered::Duplicate, "n": n.roster().len() })))
            }
            Err(e) if e.code == "needs-approval" && body.get("propose") == Some(&Value::Bool(true)) => {
                self.propose(n, entry, now)?;
                Ok(NetReply::json(202, &json!({ "ok": true, "proposed": true, "needs": e.message })))
            }
            Err(e) => Err(e),
        }
    }

    /// Keeps an entry signed by too few, for ten minutes, and asks the
    /// window hosts that haven't signed it to approve it.
    pub fn propose(&self, n: &Shared, entry: Entry, now: f64) -> R<()> {
        {
            let mut waiting = self.proposals.locked();
            waiting.retain(|(p, _)| p.signing_bytes() != entry.signing_bytes());
            waiting.push((entry.clone(), now));
            // A few at a time: the oldest go first.
            let extra = waiting.len().saturating_sub(MAX_PROPOSALS);
            waiting.drain(..extra);
        }
        self.roster_changed.notify_waiters();
        let me = Self::me(n)?;
        n.note_proposals(vec![entry.clone()]);
        let view = n.roster().view();
        let caps = n.caps();
        let body = json!({ "proposals": [entry.to_json()] });
        for host in view.members.keys().filter(|h| **h != me.host && view.is_window(h) && !entry.signers.iter().any(|s| s.host == **h)) {
            let f = Self::frame(&me, "roster", host, now as i64, &body)?;
            self.store()?.enqueue(Item { host, id: &f.id, kind: "roster", sender: "", line: f.line().trim_end() }, now as i64, &caps)?;
            self.wake(host);
        }
        Ok(())
    }

    /// Adds an entry to the network's roster, here first, then passes it on
    /// to every host as a `roster` frame.
    pub fn append(&self, n: &Shared, entry: Entry) -> R<Offered> {
        let offered = n.append(entry.clone())?;
        if offered == Offered::Duplicate {
            return Ok(offered);
        }
        self.roster_changed.notify_waiters();
        let me = Self::me(n)?;
        let now = Self::now(n);
        let view = n.roster().view();
        let caps = n.caps();
        let body = json!({ "entries": [entry.to_json()] });
        let mut to: Vec<String> = view.members.keys().filter(|h| **h != me.host).cloned().collect();
        if entry.op == Op::Remove {
            // What waited for the removed host goes, and it's told, signed,
            // on its stream if it has one open.
            let gone = &entry.host.id;
            let store = self.store()?;
            store.forget_host(gone)?;
            let by: Vec<&str> = entry.signers.iter().map(|s| s.host.as_str()).collect();
            let f = Self::frame(&me, "removed", gone, now as i64, &json!({ "network": me.network, "host": gone, "by": by, "at": entry.at, "n": entry.n }))?;
            store.enqueue(Item { host: gone, id: &f.id, kind: "removed", sender: "", line: f.line().trim_end() }, now as i64, &caps)?;
            drop(store);
            self.wake(gone);
        }
        to.sort();
        for host in to {
            let f = Self::frame(&me, "roster", &host, now as i64, &body)?;
            let line = f.line();
            self.store()?.enqueue(Item { host: &host, id: &f.id, kind: "roster", sender: "", line: line.trim_end() }, now as i64, &caps)?;
            self.wake(&host);
        }
        Ok(offered)
    }

    /// Keeps a ticket's hash, made on this host.
    pub fn add_ticket(&self, t: &TicketRow, now: i64) -> R<()> {
        self.store()?.add_ticket(t, now)
    }

    pub fn waiting_tickets(&self, now: i64) -> Vec<TicketRow> {
        self.store().and_then(|s| s.waiting_tickets(now)).unwrap_or_default()
    }

    /// `POST tickets`: a ticket made on another window host, whose hash and
    /// terms (the host it's for, and whether that's a window host) the
    /// maker's signed request carries. The gateway keeps the hash and times
    /// it from now; at the join, the maker signs the new host in.
    fn ticket_post(&self, n: &Shared, req: &NetRequest) -> R<NetReply> {
        let signed = match self.authenticate(n, req) {
            Ok(s) => s,
            Err(reply) => return Ok(reply),
        };
        let body: Value = serde_json::from_slice(&req.body).map_err(|_| NetError::bad("a ticket's terms are JSON"))?;
        let view = n.roster().view();
        if !view.is_window(&signed.host) {
            return Err(NetError::new(403, "not-window", "a ticket is made on a window host"));
        }
        let host = body.get("host").and_then(Value::as_str).filter(|h| xenonet_base::ids::is_host_id(h)).ok_or_else(|| NetError::bad("a ticket names a host id"))?;
        if view.member(host).is_some() {
            return Err(NetError::new(409, "taken", format!("{host} is in the network already: Replace it to add a reinstalled machine")));
        }
        let hash =
            body.get("hash").and_then(Value::as_str).filter(|h| unb64(h).is_some_and(|b| b.len() == 32)).ok_or_else(|| NetError::bad("a ticket's hash is 32 bytes, base64url"))?;
        let window = body.get("window").and_then(Value::as_bool).ok_or_else(|| NetError::bad("a ticket says whether it's for a window host"))?;
        let now = Self::now(n) as i64;
        let expires = now + ticket::TICKET_MS as i64;
        self.add_ticket(&TicketRow { hash: hash.into(), host: host.into(), window, maker: signed.host.clone(), made: now, expires }, now)?;
        Ok(NetReply::json(200, &json!({ "ok": true, "expires": expires })))
    }

    // ---------- joining ----------

    /// A bad ticket from `source`: 20 in an hour shut the route to that
    /// source for an hour, and 200 from everywhere shut it for everyone.
    fn bad_ticket(&self, source: Option<&str>, now: f64) {
        let mut b = self.bad.locked();
        b.by_source.retain(|_, t| t.closed_until > now || t.at.iter().any(|x| now - x < HOUR_MS));
        if let Some(src) = source {
            b.by_source.entry(src.to_string()).or_default().count(now, BAD_TICKETS);
        }
        if b.all.count(now, BAD_TICKETS_ALL) {
            if let Some(n) = self.node.upgrade() {
                n.log("net.join", "200 bad tickets in an hour: the join route is shut for an hour");
            }
        }
    }

    /// When the join route opens again for everyone, while it's shut.
    pub fn join_closed_until(&self, now: f64) -> Option<f64> {
        let until = self.bad.locked().all.closed_until;
        (until > now).then_some(until)
    }

    fn join_closed(&self, source: Option<&str>, now: f64) -> bool {
        let b = self.bad.locked();
        b.all.closed_until > now || source.and_then(|s| b.by_source.get(s)).is_some_and(|t| t.closed_until > now)
    }

    /// A wrong proof aimed at the waiting tickets with this tag: five
    /// retire a ticket.
    fn wrong_proof(&self, tag: &str, tickets: &[TicketRow], now: f64) -> R<()> {
        for t in tickets.iter().filter(|t| unb64(&t.hash).is_some_and(|h| ticket::tag(&h) == tag)) {
            let retire = {
                let mut b = self.bad.locked();
                let n = b.wrong.entry(t.hash.clone()).or_default();
                *n += 1;
                *n >= WRONG_PROOFS
            };
            if retire {
                self.store()?.use_ticket(&t.hash, now as i64)?;
                self.bad.locked().wrong.remove(&t.hash);
                if let Some(n) = self.node.upgrade() {
                    n.log("net.join", &format!("the ticket for {} had five wrong tries, and was retired: make a new one", t.host));
                }
            }
        }
        Ok(())
    }

    async fn join(&self, n: &Arc<Shared>, req: &NetRequest) -> R<NetReply> {
        let now = Self::now(n);
        let source = req.source.as_deref();
        if self.join_closed(source, now) {
            return Err(NetError::new(404, "closed", "not found"));
        }
        let me = Self::me(n)?;
        let body: Value = serde_json::from_slice(&req.body).map_err(|_| NetError::bad("a join request is JSON"))?;
        let s = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let keys = PublicKeys::from_b64(&s("sign"), &s("seal")).ok_or_else(|| NetError::bad("a join request carries its keys"))?;
        let (nonce, proof) = (s("nonce"), s("proof"));
        let key_id = keys.key_id();
        let platform = body.get("platform").and_then(Value::as_str).map(|p| p.chars().take(32).collect::<String>());
        let version = body.get("version").and_then(Value::as_str).map(|p| p.chars().take(32).collect::<String>());
        let desktop = body.get("desktop").and_then(Value::as_bool).unwrap_or(false);
        let sig = unb64(&s("sig")).unwrap_or_default();
        let signed_text = join_signing(&nonce, &key_id, &proof);
        let tickets = self.store()?.waiting_tickets(now as i64)?;
        let found = tickets.iter().find(|t| unb64(&t.hash).is_some_and(|h| ticket::check_join(&h, &nonce, &key_id, &proof))).cloned();
        let Some(t) = found.filter(|_| nonce.len() >= 16 && keys.verify(signed_text.as_bytes(), &sig)) else {
            self.bad_ticket(source, now);
            let tag = s("tag");
            if tag.len() == 2 {
                self.wrong_proof(&tag, &tickets, now)?;
            }
            return Err(NetError::new(404, "no-ticket", "not found"));
        };
        let in_roster = |n: &Shared| n.roster().view().member(&t.host).map(|m| m.keys == keys);
        match in_roster(n) {
            Some(false) => {
                return Err(NetError::new(409, "taken", format!("{} is in the network already: Replace it from a window host's console, then make a new ticket", t.host)))
            }
            Some(true) => {}
            None if t.maker == me.host => {
                // Made here: the gateway's host signs the new host in.
                if !self.store()?.use_ticket(&t.hash, now as i64)? {
                    self.bad_ticket(source, now);
                    return Err(NetError::new(404, "no-ticket", "not found"));
                }
                let mut entry = n.roster().draft(Op::Add, HostRef { id: t.host.clone(), keys, window: t.window }, None, now as i64);
                entry.sign_by(&me.host, &me.key);
                self.append(n, entry)?;
            }
            None => {
                // Made on another window host, which signs the new host in:
                // it's asked over its link, and the join waits a while.
                let join = json!({ "sign": s("sign"), "seal": s("seal"), "nonce": nonce, "proof": proof, "makerProof": s("makerProof"), "sig": s("sig") });
                self.ask_maker(n, &me, &t, &join, now)?;
                let wait = Duration::from_millis(n.timing().poll_ms * 4 / 5);
                let deadline = tokio::time::Instant::now() + wait;
                loop {
                    let changed = self.roster_changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    match in_roster(n) {
                        Some(true) => break,
                        Some(false) => return Err(NetError::new(409, "taken", format!("{} is in the network already", t.host))),
                        None => {}
                    }
                    let proposed = self.proposals.locked().iter().any(|(p, _)| p.op == Op::Add && p.host.id == t.host && p.host.keys == keys);
                    if proposed {
                        return Ok(self.waiting(n, &t, true));
                    }
                    tokio::select! {
                        () = &mut changed => {}
                        () = tokio::time::sleep_until(deadline) => return Ok(self.waiting(n, &t, false)),
                        () = self.stopped.notified() => return Ok(self.waiting(n, &t, false)),
                    }
                }
                self.asked.locked().remove(&t.hash);
            }
        }
        // The new host is in the roster with these keys: the ticket is used.
        let _ = self.store()?.use_ticket(&t.hash, now as i64)?;
        self.store()?.set_host(&t.host, platform.as_deref(), version.as_deref(), desktop, now as i64)?;
        let hash = unb64(&t.hash).unwrap_or_default();
        let gw = me.key.public();
        let roster = n.roster();
        let net = n.network().ok_or_else(|| NetError::internal("no network"))?;
        Ok(NetReply::json(
            200,
            &json!({
                "network": { "id": me.network, "name": me.name },
                "host": t.host,
                "window": t.window,
                "founder": net.founder,
                "roster": roster.to_json(),
                "gateway": { "host": me.host, "keyId": gw.key_id(), "sign": gw.sign_b64(), "seal": gw.seal_b64(), "address": me.address },
                "proof": ticket::gateway_proof(&hash, &gw.key_id(), &nonce),
                "now": now as i64,
            }),
        ))
    }

    /// Asks a ticket's maker to sign its new host in: a `roster` frame with
    /// the join request, which the maker checks against the ticket it
    /// keeps. Asked again only once the roster moved, or after a while.
    fn ask_maker(&self, n: &Shared, me: &Me, t: &TicketRow, join: &Value, now: f64) -> R<()> {
        let len = n.roster().len();
        {
            let mut asked = self.asked.locked();
            asked.retain(|_, (_, at)| now - *at < ticket::TICKET_MS);
            if asked.get(&t.hash).is_some_and(|(l, at)| *l == len && now - *at < ASK_AGAIN_MS) {
                return Ok(());
            }
            asked.insert(t.hash.clone(), (len, now));
        }
        let body = json!({ "asks": [{ "host": t.host, "window": t.window, "join": join, "expires": t.expires }] });
        let f = Self::frame(me, "roster", &t.maker, now as i64, &body)?;
        self.store()?.enqueue(Item { host: &t.maker, id: &f.id, kind: "roster", sender: "", line: f.line().trim_end() }, now as i64, &n.caps())?;
        self.wake(&t.maker);
        Ok(())
    }

    /// A join that waits on its ticket's maker, or on a second approval:
    /// what the new machine is told, and asks again with.
    fn waiting(&self, n: &Shared, t: &TicketRow, approval: bool) -> NetReply {
        let now = Self::now(n);
        let online = self.is_linked(&t.maker, now);
        let error = if approval {
            format!(
                "{} is to be a window host, which needs the founder or a second window host to approve: from one of their consoles, or xenonet net approve {} --op add. The join waits for it, for the rest of the ticket's ten minutes.",
                t.host, t.host
            )
        } else if online {
            format!("{} made this ticket and signs {} in, and hasn't yet. The join waits for it, for the rest of the ticket's ten minutes.", t.maker, t.host)
        } else {
            format!("{} made this ticket and signs {} in, and it's offline. The join waits for it, for the rest of the ticket's ten minutes.", t.maker, t.host)
        };
        NetReply::json(409, &json!({ "error": error, "code": "waiting", "maker": t.maker, "online": online, "approval": approval, "expires": t.expires }))
    }
}

/// What a join request's new key signs: its nonce, key id and proof.
pub fn join_signing(nonce: &str, key_id: &str, proof: &str) -> String {
    format!("xenonet-join-req/1\n{nonce}\n{key_id}\n{proof}")
}

/// A join request's body, made by the machine that joins.
pub fn join_body(key: &crate::keys::HostKey, code: &str, nonce: &str, platform: &str, version: &str, desktop: bool) -> Value {
    let key_id = key.key_id();
    let proof = ticket::join_proof(&ticket::hash(code), nonce, &key_id);
    let sig = key.sign(join_signing(nonce, &key_id, &proof).as_bytes());
    json!({
        "sign": key.public().sign_b64(),
        "seal": key.public().seal_b64(),
        "nonce": nonce,
        "proof": proof,
        "makerProof": ticket::maker_proof(code, nonce, &key_id),
        "tag": ticket::tag(&ticket::hash(code)),
        "platform": platform,
        "version": version,
        "desktop": desktop,
        "sig": b64(&sig),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bucket_lets_a_minute_through_then_waits() {
        let mut b = Bucket::full(60.0, 0.0);
        for _ in 0..60 {
            assert!(b.take(60.0, 0.0).is_ok());
        }
        let wait = b.take(60.0, 0.0).unwrap_err();
        assert!((wait - 1000.0).abs() < 1.0, "{wait}");
        assert!(b.take(60.0, 1000.0).is_ok());
    }

    #[test]
    fn requests_read_their_query() {
        let r = NetRequest { method: "GET".into(), target: "/net/v1/link?epoch=e1&after=12&wait=25".into(), ..Default::default() };
        assert_eq!(r.path(), "/net/v1/link");
        assert_eq!(r.query("after").as_deref(), Some("12"));
        assert_eq!(r.query("nope"), None);
        // Given twice, it's neither: the front door and the routes read alike.
        let twice = NetRequest { method: "POST".into(), target: "/net/v1/frames?relay=0&relay=1".into(), ..Default::default() };
        assert_eq!(twice.query("relay"), None);
    }
}
