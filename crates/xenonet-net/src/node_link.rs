//! link/2 on this machine (`link2` has the protocol), beside the link to
//! the gateway until the gateway goes (phase 5's commit 20).
//!
//! The node runs an iroh endpoint keyed by host.key, admits the members of
//! its network's chain on it, and keeps a connection to each one it can
//! reach. Everything the engine sends goes both ways: frames through the
//! gateway as before, and envelopes on link/2. A kept entry a peer missed
//! goes again on the next connect, from its hello; a loose one the gateway
//! carries anyway.
//!
//! **Two copies.** Each entry may arrive twice, once each way. The copy
//! that comes second, by the other way, is dropped here, by the entry's id,
//! before anything reads its authority or a presence block in it: it is a
//! silent duplicate, never a refused receipt or a second ask. A copy by the
//! same way is the author sending again, which the feeds take as before.
//!
//! **Gaps** are filled from any member that holds the entries (`holder`):
//! a Want goes to one holder at a time, the author first when it is
//! connected, then the members whose hello said they hold more, and the
//! next after a "don't have", an entry that doesn't check, or ten seconds
//! of nothing. An entry a holder serves is taken as the author's: the
//! receiver checks its signature against the author's key in its own
//! chain, and its `prev` against what it applied. This machine serves the
//! entries it applied of the others' feeds to the current members of the
//! network, as `holder::serve` rules.
//!
//! What link/2 doesn't carry yet: a message whose act rests on an
//! authority (the envelope has no place for it beside the entry yet), so
//! those go through the gateway alone; svc/2 and the map's relay; Held
//! receipts and Compacted answers, which are read and set aside (a
//! holder's Compacted answer counts as "don't have", so the next holder or
//! the author is asked).
//!
//! **Finding members.** Each machine signs an address record (its endpoint
//! id and direct addresses, under its machine signing key) and puts it in
//! its presence, which the gateway carries sealed, and on each link/2
//! connection's control stream. mDNS on the LAN joins in when the machine's
//! setting is on. A relay is used only when the network's chain lists one.

use super::{json, Arc, Duration, Locked, NetFile, ReadWrite, Shared, Value, Weak, RESEND};
use crate::dial::{Close, Dial, Mesh, PeerId};
use crate::endpoint::{peer_id, IrohDial, Relay, Relays, Settings};
use crate::envelope::{Class as EnvClass, Envelope, Me as Recipient};
use crate::holder::{self, Ask, Fetching};
use crate::link2::{Control, Links, Machine, NetHello, Out};
use crate::roster::Member;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use xenonet_id::hlc::{Clock, Hlc};
use xenonet_log::address::AddressRecord;
use xenonet_log::feed::{FeedEntry, Pos, Verified};

/// How long this machine's address record holds.
const ADDRESS_TTL_S: u64 = 600;
/// How many entry ids are remembered to tell the second copy from the
/// first.
const TAKEN: usize = 65_536;

/// Which way an entry came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Gateway,
    Link,
}

/// The entries taken lately, by id, and the way each came first.
#[derive(Default)]
pub struct Taken {
    by: HashMap<String, Via>,
    order: VecDeque<String>,
    /// Copies dropped because they came the other way.
    pub second: u64,
}

impl Taken {
    /// Whether an entry that came `via` is to be taken: the first copy, or
    /// another by the same way (its author sent it again). A copy by the
    /// other way is the second of the two dual sending makes.
    pub fn take(&mut self, id: &str, via: Via) -> bool {
        match self.by.get(id) {
            Some(v) if *v != via => {
                self.second += 1;
                false
            }
            Some(_) => true,
            None => {
                self.by.insert(id.to_string(), via);
                self.order.push_back(id.to_string());
                while self.order.len() > TAKEN {
                    if let Some(old) = self.order.pop_front() {
                        self.by.remove(&old);
                    }
                }
                true
            }
        }
    }
}

/// How this machine stands on link/2: its endpoint, mesh and connections,
/// and what it learned of where members are.
pub struct Transport2 {
    pub dial: Arc<IrohDial>,
    pub mesh: Arc<Mesh>,
    pub links: Arc<Links>,
    /// The ways each member's address was found, by host id: in its
    /// presence through the gateway, or in a record on link/2.
    pub found: std::sync::Mutex<std::collections::BTreeMap<String, std::collections::BTreeSet<&'static str>>>,
    /// This machine's own address record, and when it was signed.
    record: std::sync::Mutex<Option<(Value, Instant)>>,
    /// The feeds with a gap and the holders asked, and the requesters this
    /// machine serves as a holder.
    pub fetching: std::sync::Mutex<Fetching>,
    /// A fetch began: the clock that moves fetches on looks again.
    fetch_due: tokio::sync::Notify,
}

/// This machine's clock, and how far its HLC is ahead of it.
#[derive(Default)]
pub struct Clocks {
    pub hlc: Clock,
}

struct NodeMachine(Weak<Shared>);

impl Shared {
    pub fn link2(&self) -> Option<Arc<Transport2>> {
        self.link2.reading().clone()
    }

    /// The member whose transport key is `peer`'s endpoint id.
    fn member_at(&self, peer: PeerId) -> Option<Member> {
        self.roster().view_ref()?.members.values().find(|m| m.keys.sign == peer.0).cloned()
    }

    fn machine_id(&self) -> Option<String> {
        self.identity.machine_key().ok().map(|k| k.public().id())
    }

    /// The chain's members on link/2: admitted, and kept connected.
    pub fn members_changed(&self) {
        let (Some(t), Some(net)) = (self.link2(), self.network()) else { return };
        let members: Vec<PeerId> = self.roster().view_ref().map(|v| v.members.values().map(|m| peer_id(&m.keys)).collect()).unwrap_or_default();
        t.mesh.set_members(&net.id, members.iter().copied());
        t.links.keep(members);
    }

    /// Takes an entry another member's feed sent, which came `via`: once,
    /// whichever way came first. Its time moves this machine's HLC, never
    /// more than five minutes past its own clock.
    pub(super) fn take_entry(&self, host: String, v: Verified, authority: Value, window: bool, via: Via) -> bool {
        if !self.taken.locked().take(&v.id(), via) {
            return false;
        }
        let now = self.now().max(0.0) as u64;
        self.clocks.locked().hlc.receive(Hlc::new(v.at, 0), now);
        if v.kind == xenonet_log::feed::Kind::Message && v.body.get("target").and_then(Value::as_str) == Some(super::PERSON_TARGET) {
            return self.decide_member(&host) && self.person_in(&host, &v.body);
        }
        self.sink.entry(crate::sink::Arrived { host, entry: v, authority, window });
        false
    }

    /// How many copies were dropped for coming second, the other way.
    pub fn second_copies(&self) -> u64 {
        self.taken.locked().second
    }

    /// How far this machine's HLC is ahead of its clock, in ms.
    pub fn hlc_ahead_ms(&self) -> i64 {
        let now = self.now().max(0.0) as u64;
        let last = self.clocks.locked().hlc.last().ms();
        i64::try_from(last).unwrap_or(i64::MAX).saturating_sub(i64::try_from(now).unwrap_or(i64::MAX)).max(0)
    }

    /// An entry of this machine's feed on link/2 as well, to each of `to`
    /// that is connected. An act that rests on an authority goes through
    /// the gateway alone, since an envelope has no place for it yet.
    pub(super) fn link2_entry(&self, net: &NetFile, e: &FeedEntry, to: &[String], authority: Option<&Value>) {
        let Some(t) = self.link2() else { return };
        if authority.is_some_and(|a| a.as_object().is_none_or(|o| !o.is_empty())) {
            return;
        }
        let members: Vec<Member> = {
            let r = self.roster();
            let Some(v) = r.view_ref() else { return };
            to.iter().filter_map(|h| v.member(h).cloned()).collect()
        };
        for m in members {
            match Envelope::seal_entry(e, &m.machine.id(), &m.keys, None) {
                Ok(env) => {
                    let _ = t.links.send(peer_id(&m.keys), Out::Envelope(env));
                }
                Err(err) => self.log("net.link2", &format!("an entry for {} wasn't sealed: {}", m.id, err.message)),
            }
        }
        let _ = net;
    }

    /// A control record for `host` on link/2, when it's connected.
    pub(super) fn link2_control(&self, host: &str, c: Control) {
        let Some(t) = self.link2() else { return };
        let Some(m) = self.roster().view_ref().and_then(|v| v.member(host).cloned()) else { return };
        let _ = t.links.send(peer_id(&m.keys), Out::Control(c));
    }

    /// This machine's presence on link/2 as well, to `host`: live, never
    /// kept or forwarded.
    pub(super) fn link2_presence(&self, net: &NetFile, host: &str, body: &Value) {
        let (Some(t), Some(author)) = (self.link2(), self.machine_id()) else { return };
        let Some(m) = self.roster().view_ref().and_then(|v| v.member(host).cloned()) else { return };
        match Envelope::live(&net.id, &author, "presence", self.now().max(0.0) as u64, body) {
            Ok(env) => {
                let _ = t.links.send(peer_id(&m.keys), Out::Envelope(env));
            }
            Err(e) => self.log("net.link2", &format!("presence for {host} didn't go on link/2: {}", e.message)),
        }
    }

    /// This machine's address record in its network, signed by its machine
    /// key, made again once half its time has gone.
    pub(super) fn address_record(&self) -> Option<Value> {
        let t = self.link2()?;
        let net = self.network()?;
        {
            let had = t.record.locked();
            if let Some((v, at)) = had.as_ref() {
                if at.elapsed() < Duration::from_secs(ADDRESS_TTL_S / 2) {
                    return Some(v.clone());
                }
            }
        }
        let key = self.identity.machine_key().ok()?;
        let relays = self.roster().view_ref().map(|v| v.relays.clone()).unwrap_or_default();
        // The home relay only when the network lists it (revision 9).
        let relay = t.dial.endpoint().addr().relay_urls().map(ToString::to_string).find(|u| relays.contains(u));
        let mut addrs: Vec<String> = t.dial.direct_addrs().into_iter().filter(|a| a.port() != 0).map(|a| a.to_string()).collect();
        addrs.dedup();
        addrs.truncate(xenonet_log::address::MAX_ADDRS);
        let mut r = AddressRecord { network: net.id, host: net.host, endpoint: t.dial.me().0, relay, addrs, at: self.now().max(0.0) as u64, ttl: ADDRESS_TTL_S, sig: Vec::new() };
        if let Err(e) = r.sign(key.as_ref()) {
            self.log("net.link2", &format!("this machine's address record wasn't signed: {}", e.message));
            return None;
        }
        let v = r.to_json();
        *t.record.locked() = Some((v.clone(), Instant::now()));
        Some(v)
    }

    /// An address record from `from`, taken when this machine's copy of
    /// the chain says it is `from`'s own and holds now.
    pub(super) fn address_in(&self, from: &str, record: &Value, how: &'static str) {
        let Some(t) = self.link2() else { return };
        let r = match AddressRecord::parse(record) {
            Ok(r) => r,
            Err(e) => {
                self.log("net.link2", &format!("an address record from {from} was dropped: {}", e.message));
                return;
            }
        };
        let now = self.now().max(0.0) as u64;
        let checked = {
            let roster = self.roster();
            match roster.view_ref() {
                Some(v) => r.verify(v, now).map_err(|e| e.message),
                None => Err("this machine's chain is empty".into()),
            }
        };
        if let Err(why) = checked.and_then(|()| if r.host == from { Ok(()) } else { Err(format!("{from} sent {}'s address record", r.host)) }) {
            self.log("net.link2", &format!("an address record from {from} was refused: {why}"));
            return;
        }
        let direct: Vec<std::net::SocketAddr> = r.addrs.iter().filter_map(|a| a.parse().ok()).collect();
        let ttl = Duration::from_millis(r.until().saturating_sub(now));
        let peer = PeerId(r.endpoint);
        if t.dial.addresses().put(peer, &direct, r.relay.as_deref(), ttl).is_ok() {
            t.found.locked().entry(from.to_string()).or_default().insert(how);
            t.links.nudge(peer);
        }
    }

    /// The holders for `author`'s feed in `network`, best first, among the
    /// members connected now.
    fn holders_of(&self, t: &Transport2, f: &Fetching, network: &str, author: &str) -> Vec<PeerId> {
        let connected: Vec<PeerId> = t.links.peers().into_iter().filter(|(_, s)| s.networks.contains(network)).map(|(p, _)| p).collect();
        let by_author = self.roster().view_ref().and_then(|v| v.by_machine(author).map(|m| peer_id(&m.keys)));
        holder::order(by_author, f.ahead_of(network, author), &connected)
    }

    fn link2_ask(&self, t: &Transport2, a: Ask) {
        let want = Control::Want { network: a.network, author: a.author, inc: a.from.inc, from: a.from.seq };
        if let Err(e) = t.links.send(a.to, Out::Control(want)) {
            self.log("net.link2", &format!("a want didn't go: {}", e.message));
        }
    }

    /// A gap in `author`'s feed in `network` at `from`: a Want to one
    /// holder, unless one asked has time still.
    pub(super) fn link2_fetch(&self, network: &str, author: &str, from: Pos) {
        let Some(t) = self.link2() else { return };
        let now = self.now().max(0.0) as u64;
        let ask = {
            let mut f = t.fetching.locked();
            let hs = self.holders_of(&t, &f, network, author);
            f.fetches.want(network, author, from, &hs, now)
        };
        if let Some(a) = ask {
            self.link2_ask(&t, a);
        }
        t.fetch_due.notify_one();
    }

    /// `peer` doesn't have what it was asked for, or served what didn't
    /// check: the next holder is asked.
    fn link2_lacks(&self, network: &str, author: &str, peer: PeerId) {
        let Some(t) = self.link2() else { return };
        let now = self.now().max(0.0) as u64;
        let ask = {
            let mut f = t.fetching.locked();
            let hs = self.holders_of(&t, &f, network, author);
            f.fetches.lacks(network, author, peer, &hs, now)
        };
        if let Some(a) = ask {
            self.link2_ask(&t, a);
        }
    }

    /// This machine holds `author`'s feed in `network` up to `head`.
    pub(super) fn link2_filled(&self, network: &str, author: &str, head: Pos) {
        if let Some(t) = self.link2() {
            t.fetching.locked().fetches.filled(network, author, head);
        }
    }

    /// Moves on the fetches whose holder ran out of time, and the ones
    /// whose wait is over; says when to look again.
    fn link2_due(&self) -> Option<u64> {
        let t = self.link2()?;
        let now = self.now().max(0.0) as u64;
        let asks = {
            let mut f = t.fetching.locked();
            let holders: Vec<(holder::FeedKey, Vec<PeerId>)> = f.fetches.keys().map(|k| (k.clone(), self.holders_of(&t, &f, &k.0, &k.1))).collect();
            f.limits.prune(now);
            f.fetches.due(now, |n, a| holders.iter().find(|(k, _)| k.0 == n && k.1 == a).map(|(_, h)| h.clone()).unwrap_or_default())
        };
        for a in asks {
            self.link2_ask(&t, a);
        }
        let next = t.fetching.locked().fetches.next_due();
        next
    }

    /// The kept entries of this machine's feed in `net` after `after`,
    /// sealed for `m`.
    fn resend_sealed(&self, net: &NetFile, m: &Member, after: Pos) -> Vec<Out> {
        let mut out = Vec::new();
        let mut from = Pos::new(after.inc, after.seq.saturating_add(1));
        loop {
            let entries = match self.opts.persist.range(&net.id, from, RESEND) {
                Ok(e) => e,
                Err(e) => {
                    self.log("net.link2", &format!("this machine's feed couldn't be read to send again: {}", e.message));
                    return out;
                }
            };
            for e in &entries {
                if let Ok(env) = Envelope::seal_entry(e, &m.machine.id(), &m.keys, None) {
                    out.push(Out::Envelope(env));
                }
            }
            match entries.last() {
                Some(last) if entries.len() == RESEND => from = Pos::new(last.inc, last.seq + 1),
                _ => return out,
            }
        }
    }
}

impl NodeMachine {
    fn shared(&self) -> Option<Arc<Shared>> {
        self.0.upgrade()
    }
}

impl Machine for NodeMachine {
    fn now_ms(&self) -> u64 {
        self.shared().map_or(0, |n| n.now().max(0.0) as u64)
    }

    fn hello(&self, _peer: PeerId, network: &str) -> Option<NetHello> {
        let n = self.shared()?;
        let net = n.in_network(network)?;
        let chain = n.roster().head();
        let feeds = n.opts.persist.feed_heads(&net.id).unwrap_or_default();
        Some(NetHello { network: net.id, chain, feeds })
    }

    fn hello_in(&self, peer: PeerId, hello: &NetHello) -> Vec<Out> {
        let Some(n) = self.shared() else { return Vec::new() };
        let (Some(net), Some(m), Some(me)) = (n.in_network(&hello.network), n.member_at(peer), n.machine_id()) else { return Vec::new() };
        // Where the peer stands with this machine's feed: past what it
        // wrote means this machine's store was restored (`ack_in`).
        let at = hello.head_of(&me);
        if at != Pos::ORIGIN {
            n.ack_in(&net, &m.id, at);
        }
        // A peer that holds more of a third member's feed than this
        // machine does is asked for the rest.
        let mine = n.opts.persist.feed_heads(&net.id).unwrap_or_default();
        let peer_machine = m.machine.id();
        for (author, at) in &hello.feeds {
            if *author == me || *author == peer_machine {
                continue;
            }
            let had = mine.iter().find(|(a, _)| a == author).map_or(Pos::ORIGIN, |(_, p)| *p);
            let from = n.link2().and_then(|t| t.fetching.locked().ahead(&net.id, author, peer, *at, had));
            if let Some(from) = from {
                n.link2_fetch(&net.id, author, from);
            }
        }
        let mut out = n.resend_sealed(&net, &m, at);
        if let Some(record) = n.address_record() {
            out.push(Out::Control(Control::Address { network: net.id, record }));
        }
        out
    }

    fn envelope_in(&self, peer: PeerId, env: Envelope) -> Vec<Out> {
        let Some(n) = self.shared() else { return Vec::new() };
        let (Some(net), Some(m)) = (n.in_network(&env.network), n.member_at(peer)) else { return Vec::new() };
        let Some(key) = n.key() else { return Vec::new() };
        let view = n.roster().view();
        let window = view.is_window(&m.id);
        if env.class == EnvClass::Live {
            match env.open_live(&peer.0, &view) {
                Ok(body) => n.presence_in(&net, &m.id, body, window),
                Err(e) => n.log("net.link2", &format!("presence from {} was dropped: {}", m.id, e.message)),
            }
            return Vec::new();
        }
        let Some(me) = n.machine_id() else { return Vec::new() };
        let from_holder = env.author != m.machine.id();
        let v = match env.open_entry(&Recipient { machine: &me, key: &key }, &view) {
            Ok(v) => v,
            Err(e) => {
                n.log("net.link2", &format!("an envelope from {} was refused: {}", m.id, e.message));
                // A holder's: the next one is asked.
                if from_holder && env.class == EnvClass::Kept {
                    n.link2_lacks(&env.network, &env.author, peer);
                }
                return Vec::new();
            }
        };
        if !from_holder {
            n.take_entry(m.id, v, json!({}), window, Via::Link);
            return Vec::new();
        }
        // Another member's entry, which `m` holds: a kept one, checked
        // against its author's key above, and against what this machine
        // applied of its feed by the feed. Loose entries come from their
        // authors.
        let Some(author) = view.by_machine(&v.author).cloned().filter(|_| env.class == EnvClass::Kept) else {
            n.log("net.link2", &format!("{} sent a message of another member's feed, which was refused: messages come from their authors", m.id));
            return Vec::new();
        };
        n.take_entry(author.id.clone(), v, json!({}), view.is_window(&author.id), Via::Link);
        Vec::new()
    }

    fn want_in(&self, peer: PeerId, network: &str, author: &str, from: Pos) -> Vec<Out> {
        let Some(n) = self.shared() else { return Vec::new() };
        let (Some(net), Some(m), Some(me)) = (n.in_network(network), n.member_at(peer), n.machine_id()) else { return Vec::new() };
        if author == me {
            return n.resend_sealed(&net, &m, Pos::new(from.inc, from.seq.saturating_sub(1)));
        }
        // Another member's feed, served as a holder.
        let Some(t) = n.link2() else { return Vec::new() };
        let now = n.now().max(0.0) as u64;
        let held = |from: Pos| {
            n.opts.persist.held(&net.id, author, from, holder::SERVE).unwrap_or_else(|e| {
                n.log("net.link2", &format!("what this machine holds of a feed couldn't be read: {}", e.message));
                Vec::new()
            })
        };
        let roster = n.roster();
        let Some(view) = roster.view_ref() else { return Vec::new() };
        let mut f = t.fetching.locked();
        holder::serve(view, |mm| holder::admitted_at(&roster, mm), peer, (&net.id, author, from), held, &mut f.limits, now)
    }

    fn ack_in(&self, peer: PeerId, network: &str, author: &str, at: Pos) {
        let Some(n) = self.shared() else { return };
        let (Some(net), Some(m), Some(me)) = (n.in_network(network), n.member_at(peer), n.machine_id()) else { return };
        if author == me {
            n.ack_in(&net, &m.id, at);
        }
    }

    fn other_in(&self, peer: PeerId, record: Control) {
        let Some(n) = self.shared() else { return };
        let Some(m) = n.member_at(peer) else { return };
        match record {
            Control::Address { record, .. } => n.address_in(&m.id, &record, "record"),
            Control::DontHave { network, author, why, .. } => {
                n.log("net.link2", &format!("{} doesn't hold what was wanted of a feed ({})", m.id, why.as_str()));
                n.link2_lacks(&network, &author, peer);
            }
            // Taking a compaction is commit 17's: until then the next
            // holder, or the author, is asked.
            Control::Compacted { network, author, .. } => {
                n.log("net.link2", &format!("a compacted range from {} was set aside: not taken yet", m.id));
                n.link2_lacks(&network, &author, peer);
            }
            // Held receipts and Compacted answers are commits 11 and 17's.
            other => n.log("net.link2", &format!("a {} record from {} was set aside: not taken yet", other.to_json()["t"].as_str().unwrap_or("control"), m.id)),
        }
    }

    fn up(&self, peer: PeerId, up: bool) {
        let Some(n) = self.shared() else { return };
        let who = n.member_at(peer).map_or_else(|| peer.to_string(), |m| m.id);
        n.log("net.link2", &format!("link/2 with {who} is {}", if up { "up" } else { "down" }));
    }

    fn log(&self, words: &str) {
        if let Some(n) = self.shared() {
            n.log("net.link2", words);
        }
    }
}

/// The endpoint for link/2, with the network's relays and the machine's
/// LAN setting, its doors open for the chain's members; and the tasks that
/// take connections and keep them up, until the node stops.
pub async fn run(n: Weak<Shared>) {
    let Some(s) = n.upgrade() else { return };
    let Some(key) = s.key() else { return };
    let relays: Vec<String> = s.roster().view_ref().map(|v| v.relays.clone()).unwrap_or_default();
    let settings = Settings {
        relays: if relays.is_empty() { Relays::None } else { Relays::Listed(relays.into_iter().map(|url| Relay { url, auth_token: None }).collect()) },
        lan: s.opts.lan,
        bind: s.opts.bind,
    };
    let admission = Arc::new(crate::admission::Admission::new());
    let dial = match IrohDial::bind(&key, &settings, admission.clone()).await {
        Ok(d) => Arc::new(d),
        Err(e) => {
            s.log("net.link2", &format!("link/2 didn't start: {}", e.message));
            return;
        }
    };
    let mesh = Arc::new(Mesh::new(dial.clone(), admission));
    let links = Links::new(mesh.clone(), Arc::new(NodeMachine(n.clone())));
    let t = Arc::new(Transport2 {
        dial,
        mesh,
        links: links.clone(),
        found: std::sync::Mutex::default(),
        record: std::sync::Mutex::new(None),
        fetching: std::sync::Mutex::default(),
        fetch_due: tokio::sync::Notify::new(),
    });
    *s.link2.writing() = Some(t);
    s.members_changed();
    s.log("net.link2", "link/2 is listening");
    drop(s);
    tokio::join!(links.clone().accept(), links.keep_up(), fetch_clock(n));
}

/// Moves fetches on when a holder runs out of time or a wait is over, and
/// sleeps until the next, or until a fetch begins.
async fn fetch_clock(n: Weak<Shared>) {
    loop {
        let (wait, t) = {
            let Some(s) = n.upgrade() else { return };
            let Some(t) = s.link2() else { return };
            let next = s.link2_due();
            let now = s.now().max(0.0) as u64;
            (next.map(|at| Duration::from_millis(at.saturating_sub(now).max(10))), t)
        };
        let woke = t.fetch_due.notified();

        match wait {
            Some(wait) => {
                tokio::select! {
                    () = woke => {}
                    () = tokio::time::sleep(wait) => {}
                }
            }
            None => woke.await,
        }
    }
}

/// Stops link/2: every connection closes, and the endpoint.
pub fn stop(s: &Shared) {
    let Some(t) = s.link2.writing().take() else { return };
    for l in t.mesh.links() {
        l.close(Close::Done);
    }
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move { t.mesh.close().await });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_second_copy_by_the_other_way_is_dropped_and_a_resend_by_the_same_way_is_not() {
        let mut t = Taken::default();
        assert!(t.take("e1", Via::Gateway));
        assert!(!t.take("e1", Via::Link), "the copy link/2 brought second");
        assert!(t.take("e1", Via::Gateway), "the author sent it again the same way: the feed says it's a duplicate");
        assert!(t.take("e2", Via::Link));
        assert!(!t.take("e2", Via::Gateway));
        assert_eq!(t.second, 2);
    }
}
