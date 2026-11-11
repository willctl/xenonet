//! `xenonet/link/2` (phase 5's transport, docs/design/network.md): members'
//! feeds, messages and presence, machine to machine, on one connection per
//! peer that every network both machines are in shares (`dial::Mesh` keeps
//! it one, and keeps the one the lower endpoint id dialled when both dial).
//!
//! **Streams.** Each connection has four kinds:
//!
//! - *control*, the one bi stream, opened by the machine that dialled: a
//!   hello from each side first, then Want, Ack and Compacted for feeds,
//!   Held receipts and address records, each naming its network;
//! - *kept*, a uni stream each way: kept feed entries, in their gapless
//!   order;
//! - *loose*, a uni stream each way: messages and receipts;
//! - *live*, a uni stream each way: presence deltas.
//!
//! Every record is a u32 length, big-endian, and that many bytes of JSON,
//! at most 1 MiB. A uni stream's first record names its kind; the rest are
//! envelopes (`envelope`, `xenonet-net/2`) of that kind.
//!
//! **The hello** lists, for each network this machine is in with the peer by
//! its own chains, the chain's head and the feed heads it holds there: how
//! far it applied each author's kept entries. The receiver reads only the
//! networks the peer is in by *its* chains, so a hello never tells a machine
//! of a network the receiver doesn't share with it. An author that reads
//! its own feed's head in a hello sends what comes after: the kept entries
//! the peer hasn't got go again on every connect.
//!
//! **Membership.** What goes and what comes is checked against the networks
//! the connection carries now (`Link::check`), record by record: a peer
//! removed from N gets nothing of N, and nothing of N it sends is taken,
//! while N2 goes on over the same connection. The mesh closes it only when
//! no network is left.
//!
//! **Clocks.** The hello carries the sender's clock, which gives the skew
//! per peer for the doctor. Kept and loose envelopes are never dropped for
//! skew, since a dropped kept entry would leave a gap every later one waits
//! behind, and a machine whose own clock is slow would drop everyone's.
//! Only live envelopes are: presence made more than five minutes ahead of
//! this machine's clock is dropped (`SKEW_MS`).
//!
//! What a machine does with what arrives, and what it sends back, is its
//! `Machine`'s: this module is the wire, and the reconnects.

use crate::codec::is_network_id;
use crate::dial::{Alpn, Close, Link, Mesh, PeerId, Reader, Writer};
use crate::envelope::{Class, Envelope, MAX_ENVELOPE};
use crate::holder::Lack;
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, Notify};
use tokio::task::JoinSet;
use xenonet_log::feed::Pos;
use xenonet_log::person::is_key_id;
use xenonet_os::sync::Locked;

/// The most bytes a record takes: an envelope's limit.
pub const MAX_RECORD: usize = MAX_ENVELOPE;
/// How far ahead of this machine's clock a live envelope may be made.
pub const SKEW_MS: u64 = xenonet_id::hlc::MAX_AHEAD_MS;
/// How long a dial may take before it is given up and tried again later.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(15);
/// The waits between dials to a peer that doesn't answer: from the first,
/// doubled each time, up to the last.
pub const BACKOFF: (Duration, Duration) = (Duration::from_millis(250), Duration::from_secs(30));
/// How long a connection whose stream ended waits for the peer's close
/// before closing it from here.
const CLOSE_WAIT: Duration = Duration::from_secs(1);
/// The most networks a hello lists, and feed heads per network.
const MAX_NETWORKS: usize = 64;
const MAX_FEEDS: usize = 4096;

fn bad(what: &str) -> NetError {
    NetError::bad(format!("a link/2 record's {what} isn't right"))
}

fn not_connected(peer: PeerId) -> NetError {
    NetError::new(503, "not-connected", format!("{peer} has no link/2 connection now"))
}

// ---------- records ----------

/// Writes one record: its length, then its bytes.
pub async fn write_record(w: &mut Writer, bytes: &[u8]) -> R<()> {
    if bytes.len() > MAX_RECORD {
        return Err(bad("size"));
    }
    let len = u32::try_from(bytes.len()).map_err(|_| bad("size"))?;
    let io = |e: std::io::Error| NetError::new(502, "link-closed", format!("a link/2 stream ended: {e}"));
    w.write_all(&len.to_be_bytes()).await.map_err(io)?;
    w.write_all(bytes).await.map_err(io)?;
    w.flush().await.map_err(io)
}

/// Reads one record, or None where the stream ends cleanly between two.
/// A length past the limit ends the stream as an error, before any of its
/// bytes are read.
pub async fn read_record(r: &mut Reader) -> R<Option<Vec<u8>>> {
    let io = |e: std::io::Error| NetError::new(502, "link-closed", format!("a link/2 stream ended: {e}"));
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(io(e)),
    }
    let len = u32::from_be_bytes(len) as usize;
    if len > MAX_RECORD {
        return Err(bad("size"));
    }
    let mut record = vec![0u8; len];
    r.read_exact(&mut record).await.map_err(io)?;
    Ok(Some(record))
}

/// One network's part of a hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetHello {
    pub network: String,
    /// The hash of the sender's head of the network's chain.
    pub chain: Option<String>,
    /// How far the sender applied each author's kept entries, by the
    /// author's machine signing key id.
    pub feeds: Vec<(String, Pos)>,
}

impl NetHello {
    /// Where the sender stands with `author`'s feed: the origin when it
    /// lists nothing for it.
    pub fn head_of(&self, author: &str) -> Pos {
        self.feeds.iter().find(|(a, _)| a == author).map_or(Pos::ORIGIN, |(_, p)| *p)
    }

    fn to_json(&self) -> Value {
        let feeds: Vec<Value> = self.feeds.iter().map(|(a, p)| json!({ "author": a, "inc": p.inc, "seq": p.seq })).collect();
        json!({ "network": self.network, "chain": self.chain, "feeds": feeds })
    }

    fn parse(v: &Value) -> R<NetHello> {
        let network = network_of(v)?;
        let chain = match v.get("chain") {
            None | Some(Value::Null) => None,
            Some(c) => Some(c.as_str().ok_or_else(|| bad("chain"))?.to_string()),
        };
        let feeds = v.get("feeds").and_then(Value::as_array).ok_or_else(|| bad("feeds"))?;
        if feeds.len() > MAX_FEEDS {
            return Err(bad("feeds"));
        }
        let feeds = feeds.iter().map(|f| Ok((author_of(f)?, pos_of(f, "seq")?))).collect::<R<Vec<_>>>()?;
        Ok(NetHello { network, chain, feeds })
    }
}

/// A record on the control stream. Each but the hello names its network.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// The sender's clock (ms) and each network it shares with the peer.
    Hello { at: u64, networks: Vec<NetHello> },
    /// Every kept entry of `author`'s feed in `network` from (`inc`,
    /// `from`) on.
    Want { network: String, author: String, inc: u64, from: u64 },
    /// The sender holds `author`'s feed in `network` up to `high` of `inc`.
    Ack { network: String, author: String, inc: u64, high: u64 },
    /// What answers a Want below the author's floor (xenonet-log's
    /// `Compacted`).
    Compacted { network: String, author: String, body: Value },
    /// A holder's answer to a Want it sends nothing for, and why: the
    /// receiver asks the next holder or the author.
    DontHave { network: String, author: String, inc: u64, from: u64, why: Lack },
    /// A mailbox's receipt for what it holds.
    Held { network: String, body: Value },
    /// The sender's address record in `network` (xenonet-log's
    /// `AddressRecord`).
    Address { network: String, record: Value },
}

fn network_of(v: &Value) -> R<String> {
    v.get("network").and_then(Value::as_str).filter(|n| is_network_id(n)).map(str::to_string).ok_or_else(|| bad("network"))
}

fn author_of(v: &Value) -> R<String> {
    v.get("author").and_then(Value::as_str).filter(|a| is_key_id(a)).map(str::to_string).ok_or_else(|| bad("author"))
}

fn n(v: &Value, k: &str) -> R<u64> {
    v.get(k).and_then(Value::as_u64).ok_or_else(|| bad(k))
}

fn pos_of(v: &Value, seq: &str) -> R<Pos> {
    Ok(Pos::new(n(v, "inc")?, n(v, seq)?))
}

impl Control {
    /// The network a record is about; none for the hello, which lists them.
    pub fn network(&self) -> Option<&str> {
        match self {
            Control::Hello { .. } => None,
            Control::Want { network, .. }
            | Control::Ack { network, .. }
            | Control::DontHave { network, .. }
            | Control::Compacted { network, .. }
            | Control::Held { network, .. }
            | Control::Address { network, .. } => Some(network),
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Control::Hello { at, networks } => json!({ "t": "hello", "v": 2, "at": at, "networks": networks.iter().map(NetHello::to_json).collect::<Vec<_>>() }),
            Control::Want { network, author, inc, from } => json!({ "t": "want", "network": network, "author": author, "inc": inc, "from": from }),
            Control::Ack { network, author, inc, high } => json!({ "t": "ack", "network": network, "author": author, "inc": inc, "high": high }),
            Control::DontHave { network, author, inc, from, why } => {
                json!({ "t": "dont-have", "network": network, "author": author, "inc": inc, "from": from, "why": why.as_str() })
            }
            Control::Compacted { network, author, body } => json!({ "t": "compacted", "network": network, "author": author, "body": body }),
            Control::Held { network, body } => json!({ "t": "held", "network": network, "body": body }),
            Control::Address { network, record } => json!({ "t": "address", "network": network, "record": record }),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        self.to_json().to_string().into_bytes()
    }

    pub fn parse(v: &Value) -> R<Control> {
        let body = |k: &str| v.get(k).filter(|b| b.is_object()).cloned().ok_or_else(|| bad(k));
        match v.get("t").and_then(Value::as_str) {
            Some("hello") => {
                if v.get("v") != Some(&json!(2)) {
                    return Err(bad("version"));
                }
                let nets = v.get("networks").and_then(Value::as_array).ok_or_else(|| bad("networks"))?;
                if nets.len() > MAX_NETWORKS {
                    return Err(bad("networks"));
                }
                Ok(Control::Hello { at: n(v, "at")?, networks: nets.iter().map(NetHello::parse).collect::<R<_>>()? })
            }
            Some("want") => Ok(Control::Want { network: network_of(v)?, author: author_of(v)?, inc: n(v, "inc")?, from: n(v, "from")? }),
            Some("ack") => Ok(Control::Ack { network: network_of(v)?, author: author_of(v)?, inc: n(v, "inc")?, high: n(v, "high")? }),
            Some("dont-have") => Ok(Control::DontHave {
                network: network_of(v)?,
                author: author_of(v)?,
                inc: n(v, "inc")?,
                from: n(v, "from")?,
                why: v.get("why").and_then(Value::as_str).and_then(Lack::parse).ok_or_else(|| bad("why"))?,
            }),
            Some("compacted") => Ok(Control::Compacted { network: network_of(v)?, author: author_of(v)?, body: body("body")? }),
            Some("held") => Ok(Control::Held { network: network_of(v)?, body: body("body")? }),
            Some("address") => Ok(Control::Address { network: network_of(v)?, record: body("record")? }),
            _ => Err(bad("kind")),
        }
    }

    pub fn decode(bytes: &[u8]) -> R<Control> {
        let v: Value = serde_json::from_slice(bytes).map_err(|_| bad("JSON"))?;
        Control::parse(&v)
    }
}

fn stream_header(class: Class) -> Vec<u8> {
    json!({ "t": "stream", "class": class.as_str() }).to_string().into_bytes()
}

fn read_header(bytes: &[u8]) -> R<Class> {
    let v: Value = serde_json::from_slice(bytes).map_err(|_| bad("JSON"))?;
    if v.get("t").and_then(Value::as_str) != Some("stream") {
        return Err(bad("stream"));
    }
    v.get("class").and_then(Value::as_str).and_then(Class::parse).ok_or_else(|| bad("class"))
}

/// What goes to a peer: a control record, or an envelope on the stream of
/// its class.
#[derive(Debug, Clone)]
pub enum Out {
    Control(Control),
    Envelope(Envelope),
}

impl Out {
    pub fn network(&self) -> Option<&str> {
        match self {
            Out::Control(c) => c.network(),
            Out::Envelope(e) => Some(&e.network),
        }
    }
}

/// What a machine does with link/2: what it says in a hello, and what it
/// does with what arrives. Each call that answers returns what goes back to
/// the peer, on the same connection. Every network named here is one the
/// connection carries; the rest were dropped before.
pub trait Machine: Send + Sync {
    /// This machine's clock, ms.
    fn now_ms(&self) -> u64;
    /// What this machine's hello says of `network` to `peer`.
    fn hello(&self, peer: PeerId, network: &str) -> Option<NetHello>;
    /// The peer's hello's part for `network`: typically, this machine's
    /// kept entries after the head it names, and the address record.
    fn hello_in(&self, peer: PeerId, hello: &NetHello) -> Vec<Out>;
    /// A kept, loose or live envelope from the peer.
    fn envelope_in(&self, peer: PeerId, env: Envelope) -> Vec<Out>;
    /// The peer wants `author`'s kept entries in `network` from `from` on.
    fn want_in(&self, peer: PeerId, network: &str, author: &str, from: Pos) -> Vec<Out>;
    /// The peer holds `author`'s feed in `network` up to `at`.
    fn ack_in(&self, peer: PeerId, network: &str, author: &str, at: Pos);
    /// A DontHave, Compacted, Held or Address record.
    fn other_in(&self, _peer: PeerId, _record: Control) {}
    /// A connection to the peer came up, or went.
    fn up(&self, _peer: PeerId, _up: bool) {}
    /// Something the log should say.
    fn log(&self, _words: &str) {}
}

/// How a peer's connection stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerState {
    /// The peer's clock less this one's, from its hello.
    pub skew_ms: Option<i64>,
    /// "direct" or "relay", when the transport says.
    pub path: Option<&'static str>,
    pub networks: BTreeSet<String>,
}

struct Conn {
    link: Arc<Link>,
    tx: mpsc::UnboundedSender<Out>,
    skew: Mutex<Option<i64>>,
}

impl Conn {
    fn push(&self, out: Out) {
        let _ = self.tx.send(out);
    }
}

#[derive(Default)]
struct Keep {
    /// The peers to keep a connection to.
    wanted: BTreeSet<PeerId>,
    dialling: BTreeSet<PeerId>,
    /// When each peer that didn't answer is dialled next, and the wait
    /// after that.
    backoff: BTreeMap<PeerId, (tokio::time::Instant, Duration)>,
    /// The peers whose last dial failed, so the log says so once, not at
    /// every try.
    failing: BTreeSet<PeerId>,
}

/// This machine's link/2 connections, over its mesh.
pub struct Links {
    this: Weak<Links>,
    mesh: Arc<Mesh>,
    machine: Arc<dyn Machine>,
    conns: Mutex<BTreeMap<PeerId, Arc<Conn>>>,
    keep: Mutex<Keep>,
    changed: Notify,
}

impl Links {
    pub fn new(mesh: Arc<Mesh>, machine: Arc<dyn Machine>) -> Arc<Links> {
        Arc::new_cyclic(|this| Links { this: this.clone(), mesh, machine, conns: Mutex::new(BTreeMap::new()), keep: Mutex::new(Keep::default()), changed: Notify::new() })
    }

    pub fn mesh(&self) -> &Arc<Mesh> {
        &self.mesh
    }

    fn me(&self) -> Option<Arc<Links>> {
        self.this.upgrade()
    }

    fn conn(&self, peer: PeerId) -> Option<Arc<Conn>> {
        self.conns.locked().get(&peer).filter(|c| !c.link.is_closed()).cloned()
    }

    pub fn is_connected(&self, peer: PeerId) -> bool {
        self.conn(peer).is_some()
    }

    /// Every peer with a connection now, and how it stands.
    pub fn peers(&self) -> BTreeMap<PeerId, PeerState> {
        self.conns
            .locked()
            .iter()
            .filter(|(_, c)| !c.link.is_closed())
            .map(|(p, c)| (*p, PeerState { skew_ms: *c.skew.locked(), path: c.link.path(), networks: c.link.networks() }))
            .collect()
    }

    /// Sends `out` to `peer` on its connection, after the networks the
    /// connection carries. Nothing waits for a peer that isn't connected:
    /// its kept entries go again on the next connect, from the hello.
    pub fn send(&self, peer: PeerId, out: Out) -> R<()> {
        let conn = self.conn(peer).ok_or_else(|| not_connected(peer))?;
        if let Some(n) = out.network() {
            conn.link.check(n)?;
        }
        conn.tx.send(out).map_err(|_| not_connected(peer))
    }

    /// Takes the connections peers dial in, until the mesh closes.
    pub async fn accept(self: Arc<Self>) {
        while let Some(link) = self.mesh.accept().await {
            if link.alpn() == Alpn::Link {
                self.adopt(link);
            } else {
                // join/2 and svc/2 are other doors' (phase 5's commits 14
                // and 15), not this mesh's yet.
                link.close(Close::Done);
            }
        }
    }

    /// A connection to `peer`: the one there is, or one dialled now.
    pub async fn connect(&self, peer: PeerId) -> R<()> {
        if self.is_connected(peer) {
            return Ok(());
        }
        let link = tokio::time::timeout(DIAL_TIMEOUT, self.mesh.dial(peer, Alpn::Link)).await.map_err(|_| NetError::new(504, "unreachable", format!("{peer} didn't answer")))??;
        self.adopt(link);
        Ok(())
    }

    /// The peers this machine keeps connections to, in place of the ones
    /// before: those with a connection keep it, the others are dialled, and
    /// again after a wait when they don't answer or a connection ends.
    pub fn keep(&self, peers: impl IntoIterator<Item = PeerId>) {
        let me = self.mesh.me();
        let mut k = self.keep.locked();
        k.wanted = peers.into_iter().filter(|p| *p != me).collect();
        let wanted = k.wanted.clone();
        k.backoff.retain(|p, _| wanted.contains(p));
        k.failing.retain(|p| wanted.contains(p));
        drop(k);
        self.changed.notify_one();
    }

    /// Dial a peer now, without its wait: an address for it just came.
    pub fn nudge(&self, peer: PeerId) {
        self.keep.locked().backoff.remove(&peer);
        self.changed.notify_one();
    }

    /// Keeps the connections `keep` names up, until the task is stopped.
    pub async fn keep_up(self: Arc<Self>) {
        let me = self;
        loop {
            let now = tokio::time::Instant::now();
            let mut next: Option<tokio::time::Instant> = None;
            let due: Vec<PeerId> = {
                let mut k = me.keep.locked();
                let wanted: Vec<PeerId> = k.wanted.iter().copied().filter(|p| !k.dialling.contains(p)).collect();
                let mut due = Vec::new();
                for p in wanted {
                    if me.is_connected(p) {
                        continue;
                    }
                    match k.backoff.get(&p) {
                        Some((at, _)) if *at > now => next = Some(next.map_or(*at, |n| n.min(*at))),
                        _ => due.push(p),
                    }
                }
                for p in &due {
                    k.dialling.insert(*p);
                }
                due
            };
            for p in due {
                let links = me.clone();
                tokio::spawn(async move {
                    let r = links.connect(p).await;
                    let mut k = links.keep.locked();
                    k.dialling.remove(&p);
                    match r {
                        Ok(()) => {
                            k.backoff.remove(&p);
                            k.failing.remove(&p);
                        }
                        Err(e) => {
                            let wait = k.backoff.get(&p).map_or(BACKOFF.0, |(_, w)| *w);
                            k.backoff.insert(p, (tokio::time::Instant::now() + wait, (wait * 2).min(BACKOFF.1)));
                            let first = k.failing.insert(p);
                            drop(k);
                            if first {
                                links.machine.log(&format!("link/2 to {p} didn't connect, and is tried again with a growing wait: {}", e.message));
                            }
                        }
                    }
                    links.changed.notify_one();
                });
            }
            let changed = me.changed.notified();
            match next {
                Some(at) => {
                    tokio::select! {
                        () = changed => {}
                        () = tokio::time::sleep_until(at) => {}
                    }
                }
                None => changed.await,
            }
        }
    }

    // The connection for its peer from now on, served on tasks of its own.
    // A connection already served (a dial that found the one there) is
    // left as it is.
    fn adopt(&self, link: Arc<Link>) {
        let peer = link.peer();
        let (tx, rx) = mpsc::unbounded_channel();
        let conn = Arc::new(Conn { link: link.clone(), tx, skew: Mutex::new(None) });
        {
            let mut conns = self.conns.locked();
            if conns.get(&peer).is_some_and(|c| Arc::ptr_eq(&c.link, &link)) {
                return;
            }
            // The hello goes first, before anything sent from here on.
            conn.push(Out::Control(self.hello_for(&link)));
            conns.insert(peer, conn.clone());
        }
        let Some(me) = self.me() else { return };
        tokio::spawn(me.serve(conn, rx));
    }

    fn hello_for(&self, link: &Link) -> Control {
        let networks = link.networks().iter().filter_map(|n| self.machine.hello(link.peer(), n)).collect();
        Control::Hello { at: self.machine.now_ms(), networks }
    }

    async fn serve(self: Arc<Self>, conn: Arc<Conn>, rx: mpsc::UnboundedReceiver<Out>) {
        let link = conn.link.clone();
        let peer = link.peer();
        self.machine.up(peer, true);
        let control = if link.dialled_by() == self.mesh.me() { link.open_bi().await } else { link.accept_bi().await };
        match control {
            Ok((w, r)) => {
                let mut tasks = JoinSet::new();
                tasks.spawn(write_loop(link.clone(), w, rx));
                tasks.spawn(self.clone().read_control(conn.clone(), r));
                tasks.spawn(self.clone().accept_unis(conn.clone()));
                // Any of them ending ends the connection: the next dial
                // starts it over, with a hello.
                tokio::select! {
                    _ = link.closed() => {}
                    done = tasks.join_next() => {
                        if let Some(Ok(Err(e))) = done {
                            self.machine.log(&format!("link/2 with {peer} ended: {}", e.message));
                        }
                        // A stream that ended is mostly the peer closing:
                        // its close says why, so it gets a moment to.
                        let _ = tokio::time::timeout(CLOSE_WAIT, link.closed()).await;
                        link.close(Close::Done);
                    }
                }
            }
            Err(e) => {
                self.machine.log(&format!("link/2 with {peer} had no control stream: {}", e.message));
                link.close(Close::Done);
            }
        }
        {
            let mut conns = self.conns.locked();
            if conns.get(&peer).is_some_and(|c| Arc::ptr_eq(&c.link, &link)) {
                conns.remove(&peer);
            }
        }
        self.machine.up(peer, false);
        self.changed.notify_one();
    }

    async fn read_control(self: Arc<Self>, conn: Arc<Conn>, mut r: Reader) -> R<()> {
        let peer = conn.link.peer();
        let first = read_record(&mut r).await?.ok_or_else(|| bad("hello"))?;
        let Control::Hello { at, networks } = Control::decode(&first)? else {
            return Err(bad("hello"));
        };
        // Signed: the peer's clock less this one's.
        let skew = i64::try_from(at).unwrap_or(i64::MAX).saturating_sub(i64::try_from(self.machine.now_ms()).unwrap_or(i64::MAX));
        *conn.skew.locked() = Some(skew);
        for h in networks {
            // A network the peer says it shares, and this machine's chains
            // don't, is no business of this connection.
            if conn.link.carries(&h.network) {
                for out in self.machine.hello_in(peer, &h) {
                    conn.push(out);
                }
            }
        }
        while let Some(rec) = read_record(&mut r).await? {
            let c = match Control::decode(&rec) {
                Ok(c) => c,
                Err(e) => {
                    self.machine.log(&format!("a control record from {peer} was dropped: {}", e.message));
                    continue;
                }
            };
            if let Some(n) = c.network() {
                if conn.link.check(n).is_err() {
                    self.machine.log(&format!("a record for {n} from {peer}, which isn't in it for this machine, was dropped"));
                    continue;
                }
            }
            match c {
                Control::Hello { .. } => return Err(bad("second hello")),
                Control::Want { network, author, inc, from } => {
                    for out in self.machine.want_in(peer, &network, &author, Pos::new(inc, from)) {
                        conn.push(out);
                    }
                }
                Control::Ack { network, author, inc, high } => self.machine.ack_in(peer, &network, &author, Pos::new(inc, high)),
                other => self.machine.other_in(peer, other),
            }
        }
        Ok(())
    }

    async fn accept_unis(self: Arc<Self>, conn: Arc<Conn>) -> R<()> {
        // The readers go with this task: a JoinSet aborts what it holds.
        let mut readers = JoinSet::new();
        loop {
            let r = conn.link.accept_uni().await?;
            readers.spawn(self.clone().read_uni(conn.clone(), r));
            while readers.try_join_next().is_some() {}
        }
    }

    async fn read_uni(self: Arc<Self>, conn: Arc<Conn>, mut r: Reader) {
        let peer = conn.link.peer();
        let class = match read_record(&mut r).await {
            Ok(Some(h)) => match read_header(&h) {
                Ok(c) => c,
                Err(_) => return,
            },
            _ => return,
        };
        loop {
            let rec = match read_record(&mut r).await {
                Ok(Some(rec)) => rec,
                Ok(None) => return,
                Err(e) => {
                    self.machine.log(&format!("a {} stream from {peer} ended: {}", class.as_str(), e.message));
                    return;
                }
            };
            let env = match Envelope::decode(&rec) {
                Ok(e) => e,
                Err(e) => {
                    self.machine.log(&format!("an envelope from {peer} was dropped: {}", e.message));
                    continue;
                }
            };
            if env.class != class {
                self.machine.log(&format!("a {} envelope on {peer}'s {} stream was dropped", env.class.as_str(), class.as_str()));
                continue;
            }
            if conn.link.check(&env.network).is_err() {
                self.machine.log(&format!("an envelope for {} from {peer}, which isn't in it for this machine, was dropped", env.network));
                continue;
            }
            // Only presence is dropped for its clock.
            if class == Class::Live && env.at > self.machine.now_ms().saturating_add(SKEW_MS) {
                self.machine.log(&format!("presence from {peer} made more than five minutes ahead of this machine's clock was dropped (clock skew)"));
                continue;
            }
            for out in self.machine.envelope_in(peer, env) {
                conn.push(out);
            }
        }
    }
}

fn slot(c: Class) -> usize {
    match c {
        Class::Kept => 0,
        Class::Loose => 1,
        Class::Live => 2,
    }
}

// Writes what goes to the peer, in order: control records on the control
// stream, envelopes each on its class's uni stream, opened with its first.
async fn write_loop(link: Arc<Link>, mut control: Writer, mut rx: mpsc::UnboundedReceiver<Out>) -> R<()> {
    let mut unis: [Option<Writer>; 3] = [None, None, None];
    while let Some(out) = rx.recv().await {
        // The last check before the wire: the peer may have left a network
        // since this was queued.
        if out.network().is_some_and(|n| !link.carries(n)) {
            continue;
        }
        match out {
            Out::Control(c) => write_record(&mut control, &c.encode()).await?,
            Out::Envelope(e) => {
                let i = slot(e.class);
                let w = match &mut unis[i] {
                    Some(w) => w,
                    empty => {
                        let mut w = link.open_uni().await?;
                        write_record(&mut w, &stream_header(e.class)).await?;
                        empty.insert(w)
                    }
                };
                write_record(w, &e.encode()).await?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "link2_tests.rs"]
mod tests;
