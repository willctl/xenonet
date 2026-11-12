//! link/2 on this machine (`link2` has the protocol): how a network's
//! machines reach each other.
//!
//! The node runs an iroh endpoint keyed by host.key, admits the members of
//! its network's chain on it, and keeps a connection to each one it can
//! reach. Everything the engine sends goes as envelopes on link/2, the
//! network's chain included (`node_chain`). A kept entry a peer missed goes
//! again on the next connect, from its hello; an entry sent again is the
//! feeds' to tell from the first.
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
//! **Mailboxes** (`mailbox`): an entry for a member that isn't connected
//! goes to the network's mailboxes that are, sealed for the member; each
//! receipts it to this machine (Held), which shows it held, not
//! delivered, until the member acks it. As a mailbox, this machine keeps
//! what authors of its networks hand it for their other members, on its
//! store's tables, and hands it over when the member connects.
//!
//! **An act's authority** (the person's presence, a grant, a delegation)
//! goes sealed beside its message's entry in the envelope, for the one
//! recipient, and is decided there as every act is. One that carries the
//! person's presence goes only to a recipient that's connected, never to
//! a mailbox (`mailbox::route`).
//!
//! What link/2 doesn't carry yet: the map's relay; and Compacted answers,
//! which are set aside (a holder's counts as "don't have", so the next
//! holder or the author is asked).
//!
//! **Finding members.** Each machine signs an address record (its endpoint
//! id and direct addresses, under its machine signing key) and sends it on
//! each link/2 connection's control stream, with the records it holds of
//! the other members, which each member signed, so a machine that reached
//! one member finds the rest. The inviter sends the ones it holds to a
//! machine that joins. They're kept with the network's files, so after a
//! restart a machine dials the members where they were last. mDNS on the
//! LAN joins in when the machine's setting is on. A relay is used only
//! when the network's chain lists one.
//!
//! **Away members.** A loose entry for a member that isn't connected goes
//! to the network's mailboxes; when no mailbox takes it, it waits here, a
//! week at most, and goes when the member connects. Each member is online
//! here while its connection is up.
//!
//! **Several networks.** The endpoint is the machine's, one for every
//! network it is in, with one connection per peer: each connection carries
//! the networks both are in by this machine's chains, and whatever arrives
//! goes to the node of the network it names, which checks the peer against
//! its own chain. Its relays are the machine's (`nodes::home`): the ones
//! every network lists, and the ones it adds of its own.

use super::{json, Arc, Common, Locked, NetFile, ReadWrite, Shared, Value, Weak, RESEND};
use crate::dial::{Close, Dial, Mesh, PeerId};
use crate::endpoint::{added_roots, peer_id, IrohDial, Relay, Relays, Settings};
use crate::envelope::{Class as EnvClass, Envelope, Me as Recipient};
use crate::holder::{self, Ask, Fetching};
use crate::link2::{Control, Links, Machine, NetHello, Out};
use crate::mailbox::{self, Post};
use crate::presence::{self as live, Live};
use crate::roster::Member;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;
use std::time::Instant;
use xenonet_id::hlc::{Clock, Hlc};
use xenonet_log::address::AddressRecord;
use xenonet_log::feed::{FeedEntry, Pos, Verified};

/// How long this machine's address record holds.
const ADDRESS_TTL_S: u64 = 600;
/// Where the members' address records are kept, in the network's folder.
const ADDRESSES: &str = "addresses.json";

/// How this machine stands on link/2: its endpoint, mesh and connections,
/// and what it learned of where members are.
pub struct Transport2 {
    pub dial: Arc<IrohDial>,
    pub mesh: Arc<Mesh>,
    pub links: Arc<Links>,
    /// The ways each member's address was found, by network and host id:
    /// at join, or in a record on link/2.
    pub found: std::sync::Mutex<std::collections::BTreeMap<(String, String), std::collections::BTreeSet<&'static str>>>,
    /// This machine's own address record in each network, and when it was
    /// signed.
    record: std::sync::Mutex<HashMap<String, (Value, Instant)>>,
    /// The feeds with a gap and the holders asked, and the requesters this
    /// machine serves as a holder.
    pub fetching: std::sync::Mutex<Fetching>,
    /// A fetch began: the clock that moves fetches on looks again.
    fetch_due: tokio::sync::Notify,
    /// What this machine owes and hands over as a mailbox, and what it
    /// handed mailboxes as an author.
    pub post: std::sync::Mutex<Post>,
    /// This machine's presence, what each peer has of it, and the peers'.
    pub presence: std::sync::Mutex<Live>,
    /// The actors' forwards, which take the connections on their ALPNs.
    pub forwards: Option<Arc<crate::forward::Forwards>>,
    /// The latest address record of each member, by network and host id,
    /// as it checked: passed on to the other members on each connect, so
    /// a machine that reached one member finds the rest.
    records: std::sync::Mutex<HashMap<(String, String), (u64, Value)>>,
    /// Loose entries for members that weren't connected and that no
    /// mailbox took, oldest first: they go when the member connects.
    waiting: std::sync::Mutex<HashMap<PeerId, VecDeque<(Instant, Envelope)>>>,
}

/// Loose entries kept for one member that isn't connected, at most; the
/// oldest go first.
const WAITING: usize = 1000;
/// How long one is kept: the week a feed's messages are kept.
const WAITING_FOR: Duration = Duration::from_hours(7 * 24);

impl Transport2 {
    /// The loose entries that waited for `peer` in `network`, still in
    /// their week: they go now it's connected.
    fn waited(&self, peer: PeerId, network: &str) -> Vec<Out> {
        let mut w = self.waiting.locked();
        let Some(q) = w.get_mut(&peer) else { return Vec::new() };
        let (mine, rest): (VecDeque<_>, VecDeque<_>) = std::mem::take(q).into_iter().partition(|(_, e)| e.network == network);
        *q = rest;
        mine.into_iter().filter(|(at, _)| at.elapsed() < WAITING_FOR).map(|(_, e)| Out::Envelope(e)).collect()
    }
}

/// This machine's clock, and how far its HLC is ahead of it.
#[derive(Default)]
pub struct Clocks {
    pub hlc: Clock,
}

struct NodeMachine(Weak<Common>);

impl Shared {
    pub fn link2(&self) -> Option<Arc<Transport2>> {
        self.common.link2.reading().clone()
    }

    /// The member whose transport key is `peer`'s endpoint id.
    pub(crate) fn member_at(&self, peer: PeerId) -> Option<Member> {
        self.roster().view_ref()?.members.values().find(|m| m.keys.sign == peer.0).cloned()
    }

    fn machine_id(&self) -> Option<String> {
        self.identity.machine_key().ok().map(|k| k.public().id())
    }

    /// The chain's members on link/2: admitted, and kept connected. A
    /// machine removed from the network carries nothing of it from now on,
    /// on connections its other networks keep.
    pub fn members_changed(&self) {
        let (Some(t), Some(net)) = (self.link2(), self.network()) else { return };
        if self.is_removed() {
            t.mesh.leave(&net.id);
        } else {
            let members: Vec<PeerId> = self.roster().view_ref().map(|v| v.members.values().map(|m| peer_id(&m.keys)).collect()).unwrap_or_default();
            t.mesh.set_members(&net.id, members.clone());
            // A member just admitted may have been refused a moment ago: the
            // dials to the members not connected start again from the first
            // wait, not from where refusals left them.
            for p in members.into_iter().filter(|p| !t.links.is_connected(*p) && *p != t.dial.me()) {
                t.links.nudge(p);
            }
        }
        keep_all(&self.common, &t);
        self.common.relays_due.notify_one();
    }

    /// The members this network wants kept connected, each with whether it
    /// goes first (a window host or a mailbox).
    fn kept_members(&self) -> Vec<(PeerId, bool)> {
        if self.is_removed() {
            return Vec::new();
        }
        self.roster().view_ref().map(|v| v.members.values().map(|m| (peer_id(&m.keys), v.is_window(&m.id) || v.is_mailbox(&m.id))).collect()).unwrap_or_default()
    }

    /// Takes an entry another member's feed sent. Its time moves this
    /// machine's HLC, never more than five minutes past its own clock.
    pub(super) fn take_entry(&self, host: String, v: Verified, authority: Value, window: bool) -> bool {
        let now = self.now().max(0.0) as u64;
        self.common.clocks.locked().hlc.receive(Hlc::new(v.at, 0), now);
        if v.kind == xenonet_log::feed::Kind::Message && v.body.get("target").and_then(Value::as_str) == Some(super::PERSON_TARGET) {
            return self.decide_member(&host) && self.person_in(&host, &v.body);
        }
        // The person's act from another machine: to the engine as one, with
        // what it rests on back in its body, which the engine decides.
        if v.kind == xenonet_log::feed::Kind::Message && v.body.get("target").and_then(Value::as_str) == Some(super::OPERATOR_TARGET) {
            let mut body = v.body.get("body").cloned().unwrap_or(Value::Null);
            if let Some(o) = body.as_object_mut() {
                o.insert("authority".into(), authority);
            }
            let at = (v.at as f64).min(self.now());
            xenonet_model::ops::Sink::receive(
                self.sink.as_ref(),
                xenonet_model::ops::Incoming { network: v.group.clone(), from: host, kind: xenonet_model::ops::Kind::Operator, body, at, from_window_host: window },
            );
            return false;
        }
        self.sink.entry(crate::sink::Arrived { host, entry: v, authority, window });
        false
    }

    /// An envelope as `host`'s link/2 connection hands it on, whether or
    /// not one is up: what a test drives the receiving side with. What
    /// would go back to `host` goes, if it's connected.
    pub fn envelope_from(&self, host: &str, env: Envelope) {
        let Some(peer) = self.roster().view_ref().and_then(|v| v.member(host).map(|m| peer_id(&m.keys))) else { return };
        let out = NodeMachine(Arc::downgrade(&self.common)).envelope_in(peer, env);
        if let Some(t) = self.link2() {
            for o in out {
                let _ = t.links.send(peer, o);
            }
        }
    }

    /// How far this machine's HLC is ahead of its clock, in ms.
    pub fn hlc_ahead_ms(&self) -> i64 {
        let now = self.now().max(0.0) as u64;
        let last = self.common.clocks.locked().hlc.last().ms();
        i64::try_from(last).unwrap_or(i64::MAX).saturating_sub(i64::try_from(now).unwrap_or(i64::MAX)).max(0)
    }

    /// An entry of this machine's feed on link/2 as well, to each of `to`
    /// that is connected, with what its act rests on (`authority`) sealed
    /// beside it for its recipient.
    pub(super) fn link2_entry(&self, net: &NetFile, e: &FeedEntry, to: &[String], authority: Option<&Value>) {
        let Some(t) = self.link2() else { return };
        let members: Vec<Member> = {
            let r = self.roster();
            let Some(v) = r.view_ref() else { return };
            to.iter().filter_map(|h| v.member(h).cloned()).collect()
        };
        let view = self.roster().view();
        for m in members {
            let env = match Envelope::seal_act(e, &m.machine.id(), &m.keys, None, authority) {
                Ok(env) => env,
                Err(err) => {
                    self.log("net.link2", &format!("an entry for {} wasn't sealed: {}", m.id, err.message));
                    continue;
                }
            };
            let peer = peer_id(&m.keys);
            if t.links.is_connected(peer) {
                let _ = t.links.send(peer, Out::Envelope(env));
                continue;
            }
            // Away: to the network's mailboxes that are connected; a loose
            // entry no mailbox took waits here for the member to connect (a
            // kept one goes again from its hello anyway).
            match mailbox::route(&view, &net.host, &m.id, |p| t.links.is_connected(p), authority) {
                Ok(boxes) => {
                    let sent = boxes.iter().filter(|p| t.links.send(**p, Out::Envelope(env.clone())).is_ok()).count();
                    if sent > 0 && e.class() == xenonet_log::feed::Class::Kept {
                        t.post.locked().handed.handed(&net.id, &m.machine.id(), &e.id(), e.pos());
                    }
                    if sent == 0 && e.class() == xenonet_log::feed::Class::Loose {
                        let mut w = t.waiting.locked();
                        let q = w.entry(peer).or_default();
                        q.push_back((Instant::now(), env));
                        while q.len() > WAITING {
                            q.pop_front();
                        }
                    }
                }
                Err(why) => self.log("net.link2", &why.message),
            }
        }
    }

    /// `host` holds this machine's kept entries up to `at`: what mailboxes
    /// held of them for it is delivered.
    pub(super) fn link2_acked(&self, net: &NetFile, host: &str, at: Pos) {
        let (Some(t), Some(m)) = (self.link2(), self.roster().view_ref().and_then(|v| v.member(host).cloned())) else { return };
        t.post.locked().handed.acked(&net.id, &m.machine.id(), at);
    }

    /// How this machine's entries for `host` stand at the mailboxes.
    pub fn mail_standing(&self, network: &str, host: &str) -> mailbox::Standing {
        let (Some(t), Some(m)) = (self.link2(), self.roster().view_ref().and_then(|v| v.member(host).cloned())) else { return mailbox::Standing::default() };
        let s = t.post.locked().handed.standing(network, &m.machine.id());
        s
    }

    /// As a mailbox: keeps an envelope `from` handed this machine for
    /// another member, receipts it to the author soon, and hands it over
    /// at once if the member is connected.
    fn keep_mail(&self, from: PeerId, env: &Envelope, view: &crate::roster::View, host: &str) {
        let Some(t) = self.link2() else { return };
        let now = self.now().max(0.0) as u64;
        let item = match mailbox::take(self.opts.persist.as_ref(), view, host, from, env, now, &mailbox::Caps::default()) {
            Ok(i) => i,
            Err(e) => {
                self.log("net.link2", &format!("an envelope for another member wasn't kept: {}", e.message));
                return;
            }
        };
        let to = item.to.clone();
        if t.post.locked().batcher.add(&env.network, &env.author, item) {
            let me = self.this.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if let Some(s) = me.upgrade() {
                    s.send_receipts();
                }
            });
        }
        if let Some(r) = view.by_machine(&to).map(|m| peer_id(&m.keys)).filter(|p| t.links.is_connected(*p)) {
            let out = t.post.locked().handing.start(self.opts.persist.as_ref(), &env.network, &to, false).unwrap_or_default();
            for e in out {
                let _ = t.links.send(r, Out::Envelope(e));
            }
        }
    }

    /// The receipts this machine owes as a mailbox, signed and sent, each
    /// as its network names this machine.
    fn send_receipts(&self) {
        let (Some(t), Ok(key)) = (self.link2(), self.identity.machine_key()) else { return };
        let now = self.now().max(0.0) as u64;
        let receipts = t.post.locked().batcher.take("", now);
        for mut h in receipts {
            let Some(n) = self.common.shared_of(&h.network) else { continue };
            let Some(net) = n.network() else { continue };
            h.mailbox = net.host;
            let view = n.roster().view();
            let Some(author) = view.by_machine(&h.author).map(|m| peer_id(&m.keys)) else { continue };
            if let Err(e) = h.sign(key.as_ref()) {
                self.log("net.link2", &format!("a receipt wasn't signed: {}", e.message));
                continue;
            }
            if t.links.send(author, Out::Control(Control::Held { network: h.network.clone(), body: h.to_json() })).is_err() {
                self.log("net.link2", "a receipt didn't go: its author isn't connected now");
            }
        }
    }

    /// A control record for `host` on link/2, when it's connected.
    pub(super) fn link2_control(&self, host: &str, c: Control) {
        let Some(t) = self.link2() else { return };
        let Some(m) = self.roster().view_ref().and_then(|v| v.member(host).cloned()) else { return };
        let _ = t.links.send(peer_id(&m.keys), Out::Control(c));
    }

    /// A presence body as the live envelope for `peer` in this network,
    /// shared down to what this machine shares with it in every network
    /// they're both in.
    fn live_for(&self, net: &NetFile, peer: PeerId, body: &Value) -> Option<Envelope> {
        let author = self.machine_id()?;
        let host = self.member_at(peer)?.id;
        let sharings = self.sharings_with(&host);
        let shown = crate::share::presence_for(body, &sharings, self.common.shown.locked().entry(peer).or_default());
        match Envelope::live(&net.id, &author, "presence", self.now().max(0.0) as u64, &shown) {
            Ok(env) => Some(env),
            Err(e) => {
                self.log("net.link2", &format!("presence for {host} didn't go on link/2: {}", e.message));
                None
            }
        }
    }

    /// A peer whose presence this network's node took has had no
    /// connection for 30 s: its presence ends here, since presence holds
    /// only while a connection is up.
    fn presence_ended(&self, e: live::Ended) {
        let Some(net) = self.network() else { return };
        if let Some(p) = self.peers.locked().get_mut(&e.host) {
            p.online = false;
        }
        self.log("net.link2", &format!("{}'s presence ended: its link/2 connection has been gone 30 s", e.host));
        let window = self.roster().view_ref().is_some_and(|v| v.is_window(&e.host));
        xenonet_model::ops::Sink::receive(
            self.sink.as_ref(),
            xenonet_model::ops::Incoming {
                network: net.id.clone(),
                from: e.host,
                kind: xenonet_model::ops::Kind::Presence,
                body: e.body,
                at: self.now(),
                from_window_host: window,
            },
        );
    }

    /// This machine's address record in its network, signed by its machine
    /// key, made again once half its time has gone or the home relay moved.
    pub(super) fn address_record(&self) -> Option<Value> {
        let t = self.link2()?;
        let net = self.network()?;
        let relays = self.roster().view_ref().map(|v| v.relays.clone()).unwrap_or_default();
        // The home relay only when the network lists it (revision 9).
        let relay = t.dial.endpoint().addr().relay_urls().find_map(|u| crate::endpoint::as_listed(u, &relays));
        {
            let had = t.record.locked();
            if let Some((v, at)) = had.get(&net.id) {
                if at.elapsed() < Duration::from_secs(ADDRESS_TTL_S / 2) && v.get("relay").and_then(Value::as_str) == relay.as_deref() {
                    return Some(v.clone());
                }
            }
        }
        let key = self.identity.machine_key().ok()?;
        let mut addrs: Vec<String> = t.dial.direct_addrs().into_iter().filter(|a| a.port() != 0).map(|a| a.to_string()).collect();
        addrs.dedup();
        addrs.truncate(xenonet_log::address::MAX_ADDRS);
        let network = net.id.clone();
        let mut r = AddressRecord { network: net.id, host: net.host, endpoint: t.dial.me().0, relay, addrs, at: self.now().max(0.0) as u64, ttl: ADDRESS_TTL_S, sig: Vec::new() };
        if let Err(e) = r.sign(key.as_ref()) {
            self.log("net.link2", &format!("this machine's address record wasn't signed: {}", e.message));
            return None;
        }
        let v = r.to_json();
        t.record.locked().insert(network, (v.clone(), Instant::now()));
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
        // Another member's record, passed on: signed by that member's own
        // machine key, which `verify` checked, so whoever passes it on can
        // withhold it but not change it. Never one of this machine's.
        let me = self.network().map(|n| n.host);
        let how = if r.host == from { how } else { "forwarded" };
        if let Err(why) = checked.and_then(|()| if me.as_deref() == Some(r.host.as_str()) { Err(format!("{from} sent this machine's own address record")) } else { Ok(()) }) {
            self.log("net.link2", &format!("an address record from {from} was refused: {why} (made at {}, for {} s; now {now})", r.at, r.ttl));
            return;
        }
        {
            let mut held = t.records.locked();
            let key = (r.network.clone(), r.host.clone());
            if let Some((at, _)) = held.get(&key) {
                if *at > r.at {
                    return;
                }
                if *at == r.at {
                    // Held already: one more way it was found, and nothing
                    // to pass on.
                    t.found.locked().entry(key).or_default().insert(how);
                    return;
                }
            }
            held.insert(key, (r.at, record.clone()));
        }
        self.keep_records(&r.network);
        // A record this machine hadn't held goes on to the other members
        // it's connected to now, so a machine that just joined, or moved,
        // is found by members it hasn't reached yet.
        let skip = [Some(from), Some(r.host.as_str())];
        if let Some(view) = self.roster().view_ref() {
            for (host, m) in &view.members {
                let p = peer_id(&m.keys);
                if !skip.contains(&Some(host.as_str())) && t.links.is_connected(p) {
                    let _ = t.links.send(p, Out::Control(Control::Address { network: r.network.clone(), record: record.clone() }));
                }
            }
        }
        let direct: Vec<std::net::SocketAddr> = r.addrs.iter().filter_map(|a| a.parse().ok()).collect();
        let ttl = Duration::from_millis(r.until().saturating_sub(now));
        let peer = PeerId(r.endpoint);
        if t.dial.addresses().put(peer, &direct, r.relay.as_deref(), ttl).is_ok() {
            t.found.locked().entry((r.network.clone(), r.host.clone())).or_default().insert(how);
            t.links.nudge(peer);
        }
    }

    /// The members' address records, kept with the network's files: after
    /// a restart, with no one place to meet at, this machine dials the
    /// members where they were last, and hears where they are now.
    fn keep_records(&self, network: &str) {
        let records: Vec<Value> = self.link2().map(|t| t.records.locked().iter().filter(|((n, _), _)| n == network).map(|(_, (_, v))| v.clone()).collect()).unwrap_or_default();
        let kept = self.dir.net_file(network, ADDRESSES).and_then(|p| Ok(std::fs::write(p, Value::Array(records).to_string())?));
        if let Err(e) = kept {
            self.log("net.link2", &format!("where the members were found wasn't kept: {}", e.message));
        }
    }

    /// The address records kept for this network, as hints where to dial:
    /// each still signed by its member and that member's endpoint now,
    /// whatever its age, since a machine that hasn't moved is still there.
    pub(super) fn load_records(&self) {
        let (Some(t), Some(net)) = (self.link2(), self.network()) else { return };
        let Ok(path) = self.dir.net_file(&net.id, ADDRESSES) else { return };
        let Some(records) = std::fs::read(path).ok().and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok()) else { return };
        let view = self.roster().view();
        for v in records {
            let Ok(r) = AddressRecord::parse(&v) else { continue };
            if r.host == net.host || r.verify(&view, r.at).is_err() {
                continue;
            }
            let direct: Vec<std::net::SocketAddr> = r.addrs.iter().filter_map(|a| a.parse().ok()).collect();
            let peer = PeerId(r.endpoint);
            if t.dial.addresses().put(peer, &direct, r.relay.as_deref(), Duration::from_secs(ADDRESS_TTL_S)).is_ok() {
                t.found.locked().entry((net.id.clone(), r.host.clone())).or_default().insert("kept");
                t.records.locked().entry((net.id.clone(), r.host.clone())).or_insert((r.at, v.clone()));
                t.links.nudge(peer);
            }
        }
    }

    /// The other members' address records this machine holds in `network`,
    /// but `host`'s own: what it passes on to `host` when they connect.
    pub(super) fn records_for(&self, network: &str, host: &str) -> Vec<Value> {
        let Some(t) = self.link2() else { return Vec::new() };
        let view = self.roster().view();
        let held = t.records.locked();
        held.iter().filter(|((n, h), _)| n == network && h != host && view.member(h).is_some()).map(|(_, (_, v))| v.clone()).collect()
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
    /// The node of `network`, when this machine is in it.
    fn of(&self, network: &str) -> Option<Arc<Shared>> {
        self.0.upgrade()?.shared_of(network)
    }

    /// Any of the machine's nodes: for its clock and its log.
    fn any(&self) -> Option<Arc<Shared>> {
        self.0.upgrade()?.shareds().into_iter().next()
    }
}

impl Machine for NodeMachine {
    fn now_ms(&self) -> u64 {
        self.any().map_or_else(|| super::now_ms().max(0.0) as u64, |n| n.now().max(0.0) as u64)
    }

    fn hello(&self, _peer: PeerId, network: &str) -> Option<NetHello> {
        let n = self.of(network)?;
        let net = n.in_network(network)?;
        let chain = n.roster().head();
        let feeds = n.opts.persist.feed_heads(&net.id).unwrap_or_default();
        Some(NetHello { network: net.id, chain, feeds })
    }

    fn hello_in(&self, peer: PeerId, hello: &NetHello) -> Vec<Out> {
        let Some(n) = self.of(&hello.network) else { return Vec::new() };
        let (Some(net), Some(m), Some(me)) = (n.in_network(&hello.network), n.member_at(peer), n.machine_id()) else { return Vec::new() };
        // Chain heads exchanged with a member: in contact with this
        // network, whatever the others are.
        n.common.heads_at.locked().insert(net.id.clone(), n.now());
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
        // As a mailbox: what it keeps for the peer, from the start of its
        // queue, since what was on the way before may not have come.
        let mut out = Vec::new();
        if let Some(t) = n.link2() {
            let mail = t.post.locked().handing.start(n.opts.persist.as_ref(), &net.id, &peer_machine, true);
            out.extend(mail.unwrap_or_default().into_iter().map(Out::Envelope));
        }
        // This machine's presence, whole, on every new connection: once,
        // in the first network the connection carries.
        let presence_here = presence_node(&n.common, peer).is_some_and(|p| Arc::ptr_eq(&p, &n));
        if let Some(snap) = n.link2().filter(|_| presence_here).and_then(|t| t.presence.locked().connected(peer)) {
            out.extend(n.live_for(&net, peer, &snap).map(Out::Envelope));
        }
        out.extend(n.resend_sealed(&net, &m, at));
        if let Some(t) = n.link2() {
            out.extend(t.waited(peer, &net.id));
        }
        // The chain, where the heads differ, and what waits for approval.
        out.extend(n.chain_hello(hello.chain.as_deref()));
        if !n.proposals().is_empty() {
            out.extend(n.chain_since(u64::MAX));
        }
        // Where this machine is, and where the other members are.
        if let Some(record) = n.address_record() {
            out.push(Out::Control(Control::Address { network: net.id.clone(), record }));
        }
        for record in n.records_for(&net.id, &m.id) {
            out.push(Out::Control(Control::Address { network: net.id.clone(), record }));
        }
        out
    }

    fn envelope_in(&self, peer: PeerId, env: Envelope) -> Vec<Out> {
        let Some(n) = self.of(&env.network) else { return Vec::new() };
        let (Some(net), Some(m)) = (n.in_network(&env.network), n.member_at(peer)) else { return Vec::new() };
        let Some(key) = n.key() else { return Vec::new() };
        let view = n.roster().view();
        let window = view.is_window(&m.id);
        if env.class == EnvClass::Live {
            match env.open_live(&peer.0, &view) {
                Ok(body) => {
                    let counter = body.get("counter").and_then(Value::as_f64).map_or(0, |c| c.max(0.0) as u64);
                    if let Some(t) = n.link2() {
                        t.presence.locked().theirs.took(peer, &m.id, &net.id, counter);
                    }
                    n.presence_in(&net, &m.id, body, window);
                }
                Err(e) => n.log("net.link2", &format!("presence from {} was dropped: {}", m.id, e.message)),
            }
            return Vec::new();
        }
        let Some(me) = n.machine_id() else { return Vec::new() };
        if env.to.as_str() != me {
            n.keep_mail(peer, &env, &view, &net.host);
            return Vec::new();
        }
        let from_holder = env.author != m.machine.id();
        // From a mailbox: it hears the envelope was taken, whatever came of
        // it here, and lets it go.
        let from_mailbox = from_holder && view.is_mailbox(&m.id);
        let out: Vec<Out> = from_mailbox.then(|| Out::Control(Control::Took { network: env.network.clone(), ids: vec![env.id.clone()] })).into_iter().collect();
        let me_r = Recipient { machine: &me, key: &key };
        // The entry, and what its act rests on, which opens only beside it.
        let v = match env.open_entry(&me_r, &view).and_then(|v| Ok((v, env.open_authority(&me_r)?))) {
            Ok(v) => v,
            Err(e) => {
                n.log("net.link2", &format!("an envelope from {} was refused: {}", m.id, e.message));
                // A holder's: the next one is asked.
                if from_holder && env.class == EnvClass::Kept {
                    n.link2_lacks(&env.network, &env.author, peer);
                }
                return out;
            }
        };
        let (v, authority) = v;
        if !from_holder {
            n.take_entry(m.id, v, authority, window);
            return out;
        }
        // Another member's entry, which `m` holds or kept as a mailbox,
        // checked against its author's key above, and against what this
        // machine applied of its feed by the feed. Loose entries come from
        // their authors or a mailbox.
        let Some(author) = view.by_machine(&v.author).cloned().filter(|_| env.class == EnvClass::Kept || from_mailbox) else {
            n.log("net.link2", &format!("{} sent a message of another member's feed, which was refused: messages come from their authors or a mailbox", m.id));
            return out;
        };
        // A holder serves kept entries, which rest on no authority; a
        // mailbox hands on the one sealed beside a message.
        let authority = if from_mailbox { authority } else { json!({}) };
        n.take_entry(author.id.clone(), v, authority, view.is_window(&author.id));
        out
    }

    fn want_in(&self, peer: PeerId, network: &str, author: &str, from: Pos) -> Vec<Out> {
        let Some(n) = self.of(network) else { return Vec::new() };
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
        let Some(n) = self.of(network) else { return };
        let (Some(net), Some(m), Some(me)) = (n.in_network(network), n.member_at(peer), n.machine_id()) else { return };
        if author == me {
            n.ack_in(&net, &m.id, at);
        }
    }

    fn other_in(&self, peer: PeerId, record: Control) {
        let Some(n) = record.network().and_then(|net| self.of(net)) else { return };
        let Some(m) = n.member_at(peer) else { return };
        match record {
            Control::Address { record, .. } => n.address_in(&m.id, &record, "record"),
            Control::Chain { body, .. } => n.chain_in(peer, &m.id, &body),
            Control::ChainWant { from, .. } => n.chain_want_in(peer, from),
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
            Control::Held { network, body } => {
                let (Some(me), Some(t)) = (n.machine_id(), n.link2()) else { return };
                let view = n.roster().view();
                match mailbox::Held::parse(&body).and_then(|h| h.verify(&view, peer, &me).map(|()| h)) {
                    Ok(h) => t.post.locked().handed.held(&h),
                    Err(e) => n.log("net.link2", &format!("a receipt for {network} from {} was dropped: {}", m.id, e.message)),
                }
            }
            Control::Took { network, ids } => {
                let Some(t) = n.link2() else { return };
                let next = t.post.locked().handing.took(n.opts.persist.as_ref(), &network, &m.machine.id(), &ids);
                for e in next.unwrap_or_default() {
                    let _ = t.links.send(peer, Out::Envelope(e));
                }
            }
            other => n.log("net.link2", &format!("a {} record from {} was set aside: not taken yet", other.to_json()["t"].as_str().unwrap_or("control"), m.id)),
        }
    }

    fn up(&self, peer: PeerId, up: bool) {
        let Some(n) = self.any() else { return };
        // Who is online, in each network the peer is a member of: one with
        // a connection up.
        if let Some(c) = self.0.upgrade() {
            for s in c.shareds() {
                if let Some(m) = s.member_at(peer) {
                    s.peer_up(&m.id, up);
                }
            }
        }
        if let Some(t) = n.link2() {
            if up {
                t.presence.locked().theirs.up(peer);
            } else {
                t.presence.locked().disconnected(peer, n.now().max(0.0) as u64);
                t.fetch_due.notify_one();
            }
        }
        let who = self.0.upgrade().and_then(|c| c.shareds().into_iter().find_map(|s| s.member_at(peer))).map_or_else(|| peer.to_string(), |m| m.id);
        n.log("net.link2", &format!("link/2 with {who} is {}", if up { "up" } else { "down" }));
    }

    fn log(&self, words: &str) {
        if let Some(n) = self.any() {
            n.log("net.link2", words);
        }
    }
}

impl Common {
    /// The address records a join brought for `network`: the inviter's
    /// and the members' it knew, each signed by its member. Taken once
    /// link/2 runs and the network's node is listed.
    pub(crate) fn found_at_join(&self, network: &str, records: Vec<Value>) {
        if !records.is_empty() {
            self.joined.locked().push((network.to_string(), records));
        }
        self.take_joined();
    }

    /// What joins brought, taken now that link/2 runs, for the networks
    /// listed; the rest wait.
    pub(crate) fn take_joined(&self) {
        if self.link2.reading().is_none() {
            return;
        }
        let pending = std::mem::take(&mut *self.joined.locked());
        let mut left = Vec::new();
        for (network, records) in pending {
            let Some(s) = self.shared_of(&network) else {
                left.push((network, records));
                continue;
            };
            for r in &records {
                if let Some(host) = r.get("host").and_then(Value::as_str) {
                    s.address_in(host, r, "invite");
                }
            }
        }
        self.joined.locked().extend(left);
    }
}

/// The node whose network a connection to `peer` carries presence in: the
/// first of this machine's networks the connection carries and the peer is
/// a member of. Presence goes once a peer, shown what every network they
/// share shows (`Shared::sharings_with`), so it never says one thing in one
/// network and another in the next.
pub(crate) fn presence_node(c: &Common, peer: PeerId) -> Option<Arc<Shared>> {
    let t = c.link2.reading().clone()?;
    let carried = t.links.peers().get(&peer).map(|s| s.networks.clone())?;
    c.shareds().into_iter().find(|s| !s.is_removed() && s.network().is_some_and(|n| carried.contains(&n.id)) && s.member_at(peer).is_some())
}

/// This machine's presence on link/2, as the engine just said it: what
/// changed goes once to each peer connected now, in the network
/// `presence_node` picks, shared down to what this machine shares with it;
/// nothing goes when nothing changed. Live: never kept or forwarded.
pub(crate) fn link2_presence(c: &Common, body: &Value) {
    let Some(t) = c.link2.reading().clone() else { return };
    let peers: Vec<PeerId> = t.links.peers().into_keys().collect();
    let out = t.presence.locked().changed(body, &peers);
    for (peer, b) in out {
        let Some(n) = presence_node(c, peer) else { continue };
        let Some(net) = n.network() else { continue };
        if let Some(env) = n.live_for(&net, peer, &b) {
            let _ = t.links.send(peer, Out::Envelope(env));
        }
    }
}

/// What a peer may see of this machine changed: each connected peer is
/// sent its presence whole again, shown what it may see now.
pub(crate) fn presence_again(c: &Common) {
    let Some(t) = c.link2.reading().clone() else { return };
    let peers: Vec<PeerId> = t.links.peers().into_keys().collect();
    let again = t.presence.locked().again(&peers);
    for (peer, snap) in again {
        let Some(n) = presence_node(c, peer) else { continue };
        let Some(net) = n.network() else { continue };
        if let Some(env) = n.live_for(&net, peer, &snap) {
            let _ = t.links.send(peer, Out::Envelope(env));
        }
    }
}

/// Every network's members kept connected: all of them up to fifty
/// machines, since presence holds only while a connection is up; past
/// that, window hosts and mailboxes first.
fn keep_all(c: &Common, t: &Transport2) {
    let mut seen = std::collections::BTreeSet::new();
    let mut all: Vec<(PeerId, bool)> = Vec::new();
    let nets = c.shareds();
    for first in [true, false] {
        for s in &nets {
            for (p, f) in s.kept_members() {
                if f == first && seen.insert(p) {
                    all.push((p, f));
                }
            }
        }
    }
    t.links.keep(live::keep(&all, t.dial.me()));
}

/// Moves on the fetches whose holder ran out of time, and the ones whose
/// wait is over, in every network; says when to look again.
fn link2_due(c: &Common, t: &Transport2) -> Option<u64> {
    let any = c.shareds().into_iter().next()?;
    let now = any.now().max(0.0) as u64;
    let asks = {
        let mut f = t.fetching.locked();
        let holders: Vec<(holder::FeedKey, Vec<PeerId>)> =
            f.fetches.keys().map(|k| (k.clone(), c.shared_of(&k.0).map(|s| s.holders_of(t, &f, &k.0, &k.1)).unwrap_or_default())).collect();
        f.limits.prune(now);
        f.fetches.due(now, |n, a| holders.iter().find(|(k, _)| k.0 == n && k.1 == a).map(|(_, h)| h.clone()).unwrap_or_default())
    };
    for a in asks {
        any.link2_ask(t, a);
    }
    let next = t.fetching.locked().fetches.next_due();
    next
}

/// The peers whose connection has been gone 30 s: each network's node that
/// took their presence ends it. When to look again.
fn presence_due(c: &Common, t: &Transport2) -> Option<u64> {
    let any = c.shareds().into_iter().next()?;
    let now = any.now().max(0.0) as u64;
    let ended = t.presence.locked().theirs.expired(now);
    for e in ended {
        if let Some(n) = c.shared_of(&e.network) {
            n.presence_ended(e);
        }
    }
    let next = t.presence.locked().theirs.next_due();
    next
}

/// The relays by URL, as the endpoint takes them: none for an empty list.
pub fn relays_of(urls: &[String]) -> Relays {
    if urls.is_empty() {
        Relays::None
    } else {
        Relays::Listed(urls.iter().map(|url| Relay { url: url.clone(), auth_token: None }).collect())
    }
}

/// The endpoint for link/2, the machine's for every network it's in, with
/// `relays` (`Nodes`' choice) and the machine's LAN setting, its doors open
/// for the chains' members; and the tasks that take connections and keep
/// them up, until the machine stops.
pub async fn run(c: Weak<Common>, relays: Vec<String>) {
    let Some(common) = c.upgrade() else { return };
    let Some(s) = common.shareds().into_iter().next() else { return };
    let Some(key) = s.key() else { return };
    let settings = Settings { relays: relays_of(&relays), lan: s.opts.lan, bind: s.opts.bind, roots: added_roots(&s.opts.dir).unwrap_or_default() };
    // The members of its networks on the ALPNs it forwards for actors too.
    let forwards = s.opts.forwards.clone();
    let doors = forwards.as_ref().map_or_else(crate::admission::Doors::xenonet, |f| crate::admission::Doors::xenonet_forwarding(f.forwardable().alpns()));
    let admission = Arc::new(crate::admission::Admission::with(doors));
    let dial = match IrohDial::bind(&key, &settings, admission.clone()).await {
        Ok(d) => Arc::new(d),
        Err(e) => {
            s.log("net.link2", &format!("link/2 didn't start: {}", e.message));
            return;
        }
    };
    let mesh = Arc::new(Mesh::new(dial.clone(), admission));
    let links = Links::new(mesh.clone(), Arc::new(NodeMachine(c.clone())));
    if let Some(f) = &forwards {
        f.set_mesh(Some(mesh.clone()));
        let f = f.clone();
        links.hand_others(Arc::new(move |link| f.incoming(link)));
    }
    let served: Arc<dyn crate::svc2::Served> = Arc::new(super::svc2n::NodeServed(c.clone()));
    links.hand_services(Arc::new(move |link| {
        tokio::spawn(crate::svc2::serve(link, served.clone()));
    }));
    let t = Arc::new(Transport2 {
        records: std::sync::Mutex::default(),
        waiting: std::sync::Mutex::default(),
        dial,
        mesh,
        links: links.clone(),
        found: std::sync::Mutex::default(),
        record: std::sync::Mutex::default(),
        fetching: std::sync::Mutex::default(),
        fetch_due: tokio::sync::Notify::new(),
        post: std::sync::Mutex::default(),
        presence: std::sync::Mutex::default(),
        forwards,
    });
    *common.link2.writing() = Some(t);
    common.take_joined();
    for n in common.shareds() {
        n.load_records();
    }
    for n in common.shareds() {
        n.members_changed();
    }
    // The engine says its presence whole again now there's an endpoint to
    // carry it: what it said before, while none ran (a machine that just
    // joined its first network), went nowhere, and a connection sends a
    // peer only what was said.
    if let Some(n) = common.shareds().into_iter().find(|n| !n.is_removed()) {
        xenonet_model::ops::Sink::linked(n.sink.as_ref(), true);
    }
    s.log("net.link2", "link/2 is listening");
    drop((s, common));
    tokio::join!(links.clone().accept(), links.keep_up(), fetch_clock(c));
}

/// Moves fetches on when a holder runs out of time or a wait is over, and
/// ends the presence of peers gone 30 s; sleeps until the next of those,
/// or until a fetch begins or a connection goes. Nothing here runs on a
/// tick.
async fn fetch_clock(c: Weak<Common>) {
    loop {
        let (wait, t) = {
            let Some(c) = c.upgrade() else { return };
            let Some(t) = c.link2.reading().clone() else { return };
            let next = [link2_due(&c, &t), presence_due(&c, &t)].into_iter().flatten().min();
            let now = c.shareds().first().map_or_else(super::now_ms, |s| s.now()).max(0.0) as u64;
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
pub fn stop(c: &Common) {
    let Some(t) = c.link2.writing().take() else { return };
    if let Some(f) = &t.forwards {
        f.set_mesh(None);
    }
    for l in t.mesh.links() {
        l.close(Close::Done);
    }
    if let Ok(rt) = tokio::runtime::Handle::try_current() {
        rt.spawn(async move { t.mesh.close().await });
    }
}
