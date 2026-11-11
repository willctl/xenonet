// link/2 on the simulated network (`sim`): seeded runs, the same every
// time, where machines write their feeds, seal entries for each other and
// take them into real feeds (xenonet-log's), while the network delays,
// reorders, partitions and drops connections.
//
// One test per property of formal/delivery.qnt, as the transport carries
// it:
//
// 1. atMostOnce: `every_entry_is_applied_at_most_once_however_often_it_comes`
// 2. perAuthorOrder: `kept_entries_apply_in_their_authors_order_over_a_reordering_network`
// 3. gapDetected: `a_gap_is_held_asked_for_and_filled_never_skipped`
// 4. forkDetected: `two_entries_at_one_place_freeze_the_authors_feed`
// 5. compactEnds: `a_compact_entry_ends_what_it_leaves_out`
// 6. revokedRefused: `a_removed_members_entries_are_refused_even_signed_before`
// 7. storedAfterReconnect: `after_a_partition_the_receiver_stands_where_the_author_does`
//
// And the transport's own gates: a reconnect storm, a restored author, a
// machine removed from one network while in another, and clocks.

use super::*;
use crate::admission::Admission;
use crate::envelope::Me as Recipient;
use crate::keys::HostKey;
use crate::roster::{Member, Settings, View};
use crate::sim::SimDial;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use xenonet_id::sign::{KeyHome, SeedSigner, Signer};
use xenonet_log::feed::{Feed, FeedEntry, Kind, MemoryStore, Offer, Writer as FeedWriter};

const N1: &str = "net_00000000000000000000000000000001";
const N2: &str = "net_00000000000000000000000000000002";
// 2026-10-20T16:00:00Z: every machine's clock starts here.
const T: u64 = 1_792_512_000_000;

// Machines by number: their keys come from it.
const A: u8 = 10;
const B: u8 = 20;
const C: u8 = 30;
const D: u8 = 40;

fn host_of(n: u8) -> &'static str {
    match n {
        A => "a",
        B => "b",
        C => "c",
        _ => "d",
    }
}

fn transport(n: u8) -> HostKey {
    HostKey::from_secrets(&[n; 32], [n + 1; 32]).unwrap()
}

fn signer(n: u8) -> SeedSigner {
    SeedSigner::from_seed(&[n + 2; 32], KeyHome::File).unwrap()
}

fn peer(n: u8) -> PeerId {
    PeerId(transport(n).public().sign)
}

fn author(n: u8) -> String {
    signer(n).public().id()
}

fn hosts() -> BTreeMap<PeerId, String> {
    [A, B, C, D].into_iter().map(|n| (peer(n), host_of(n).to_string())).collect()
}

fn member(n: u8) -> Member {
    Member {
        id: host_of(n).into(),
        keys: *transport(n).public(),
        window: false,
        device: None,
        machine: signer(n).public(),
        settings: Settings::default(),
        settings_by: None,
        since: 1,
    }
}

fn view(network: &str, members: &[u8]) -> View {
    View { network: network.into(), founder: host_of(members[0]).into(), members: members.iter().map(|n| (host_of(*n).to_string(), member(*n))).collect(), ..View::default() }
}

/// How a machine answers a Want for another member's feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Holds {
    Honest,
    /// Changes each entry's body before it seals it.
    Tampers,
    /// Answers nothing.
    Withholds,
}

use crate::holder::Fetching;

/// A machine: its keys, its own copy of each network's chain, its feed in
/// each network as an author, and the feeds it takes as a receiver.
struct Party {
    n: u8,
    transport: HostKey,
    signer: SeedSigner,
    /// Added to the simulation's clock: a machine whose clock is off.
    offset: i64,
    views: Mutex<BTreeMap<String, View>>,
    writers: Mutex<BTreeMap<String, FeedWriter>>,
    /// What it wrote, by network: its kept entries, which it resends.
    kept: Mutex<BTreeMap<String, Vec<FeedEntry>>>,
    feed: Mutex<Feed<MemoryStore>>,
    /// What its feeds applied, in order, and what compact entries ended.
    applied: Mutex<Vec<FeedEntry>>,
    ended: Mutex<Vec<Pos>>,
    live: Mutex<Vec<(String, Value)>>,
    /// The networks each hello it took spoke of.
    hellos: Mutex<Vec<String>>,
    duplicates: AtomicU64,
    wants: AtomicU64,
    refused: AtomicU64,
    holds: Mutex<Holds>,
    fetching: Mutex<Fetching>,
    /// When each member was admitted, by network and host: 0 unless set.
    admitted: Mutex<BTreeMap<(String, String), u64>>,
    /// Entries served as a holder, and the Wants it sent: to whom, from
    /// where.
    served: AtomicU64,
    asked: Mutex<Vec<(u8, Pos)>>,
    links: OnceLock<Arc<Links>>,
    /// What another host's task asks of this machine, done on its own
    /// simulated host: a socket is written only from the host it's on.
    own: OnceLock<mpsc::UnboundedSender<Job>>,
}

type Job = Box<dyn FnOnce(&Arc<Links>) + Send>;

impl Party {
    fn new(n: u8, networks: &[(&str, &[u8])]) -> Arc<Party> {
        Party::with_clock(n, networks, 0)
    }

    fn with_clock(n: u8, networks: &[(&str, &[u8])], offset: i64) -> Arc<Party> {
        Arc::new(Party {
            n,
            transport: transport(n),
            signer: signer(n),
            offset,
            views: Mutex::new(networks.iter().map(|(id, m)| (id.to_string(), view(id, m))).collect()),
            writers: Mutex::new(BTreeMap::new()),
            kept: Mutex::new(BTreeMap::new()),
            feed: Mutex::new(Feed::at(MemoryStore::new(), T)),
            applied: Mutex::new(Vec::new()),
            ended: Mutex::new(Vec::new()),
            live: Mutex::new(Vec::new()),
            hellos: Mutex::new(Vec::new()),
            duplicates: AtomicU64::new(0),
            wants: AtomicU64::new(0),
            refused: AtomicU64::new(0),
            holds: Mutex::new(Holds::Honest),
            fetching: Mutex::new(Fetching::default()),
            admitted: Mutex::new(BTreeMap::new()),
            served: AtomicU64::new(0),
            asked: Mutex::new(Vec::new()),
            links: OnceLock::new(),
            own: OnceLock::new(),
        })
    }

    fn me(&self) -> String {
        self.signer.public().id()
    }

    fn links(&self) -> &Arc<Links> {
        self.links.get().unwrap()
    }

    /// Starts its endpoint on the simulated network: every member of its
    /// networks admitted, and kept connected.
    async fn start(self: &Arc<Self>) -> R<Arc<Links>> {
        let dial = SimDial::bind(peer(self.n), hosts()).await?;
        let mesh = Arc::new(Mesh::new(Arc::new(dial), Arc::new(Admission::new())));
        let mut everyone = BTreeSet::new();
        for (id, v) in self.views.locked().iter() {
            let members: Vec<PeerId> = v.members.values().map(|m| PeerId(m.keys.sign)).collect();
            everyone.extend(members.iter().copied());
            mesh.set_members(id, members);
        }
        let links = Links::new(mesh, self.clone());
        let _ = self.links.set(links.clone());
        tokio::spawn(links.clone().accept());
        tokio::spawn(links.clone().keep_up());
        let (to, mut jobs) = mpsc::unbounded_channel::<Job>();
        let _ = self.own.set(to);
        let on = links.clone();
        tokio::spawn(async move {
            while let Some(job) = jobs.recv().await {
                job(&on);
            }
        });
        links.keep(everyone);
        // The fetches' clock: a holder that ran out of time, a fetch whose
        // wait is over.
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(250)).await;
                let now = me.now_ms();
                let asks = {
                    let mut f = me.fetching.locked();
                    let Fetching { fetches, ahead, .. } = &mut *f;
                    fetches.due(now, |n, a| me.holders(ahead, n, a))
                };
                for a in asks {
                    me.ask(a);
                }
            }
        });
        Ok(links)
    }

    /// The holders to ask for `author`'s feed in `network`, best first.
    fn holders(&self, ahead: &BTreeMap<crate::holder::FeedKey, Vec<PeerId>>, network: &str, author: &str) -> Vec<PeerId> {
        let Some(links) = self.links.get() else { return Vec::new() };
        let connected: Vec<PeerId> = links.peers().into_iter().filter(|(_, s)| s.networks.contains(network)).map(|(p, _)| p).collect();
        let by_author = self.views.locked().get(network).and_then(|v| v.by_machine(author).map(|m| PeerId(m.keys.sign)));
        let ahead = ahead.get(&(network.to_string(), author.to_string())).cloned().unwrap_or_default();
        crate::holder::order(by_author, &ahead, &connected)
    }

    /// A gap in `author`'s feed at `from`: a Want to one holder.
    fn fetch(&self, network: &str, author: &str, from: Pos) {
        let ask = {
            let mut f = self.fetching.locked();
            let hs = self.holders(&f.ahead, network, author);
            f.fetches.want(network, author, from, &hs, self.now_ms())
        };
        if let Some(a) = ask {
            self.ask(a);
        }
    }

    fn lacks(&self, network: &str, author: &str, from: PeerId) {
        let ask = {
            let mut f = self.fetching.locked();
            let hs = self.holders(&f.ahead, network, author);
            f.fetches.lacks(network, author, from, &hs, self.now_ms())
        };
        if let Some(a) = ask {
            self.ask(a);
        }
    }

    fn ask(&self, a: crate::holder::Ask) {
        let n = [A, B, C, D].into_iter().find(|n| peer(*n) == a.to).unwrap_or(0);
        self.asked.locked().push((n, a.from));
        let _ = self.links().send(a.to, Out::Control(Control::Want { network: a.network, author: a.author, inc: a.from.inc, from: a.from.seq }));
    }

    /// The head of `author`'s feed in `network` here.
    fn head(&self, author: &str, network: &str) -> Pos {
        xenonet_log::feed::FeedStore::head(self.feed.locked().store(), author, network).unwrap().latest
    }

    /// This machine's chain now has `n` out of `network`.
    fn remove(&self, network: &'static str, n: u8) {
        let members: Vec<PeerId> = {
            let mut views = self.views.locked();
            let v = views.get_mut(network).unwrap();
            v.members.remove(host_of(n));
            v.members.values().map(|m| PeerId(m.keys.sign)).collect()
        };
        self.on_own(move |l| l.mesh().set_members(network, members));
    }

    /// Runs `f` on this machine's own simulated host.
    fn on_own(&self, f: impl FnOnce(&Arc<Links>) + Send + 'static) {
        let _ = self.own.get().unwrap().send(Box::new(f));
    }

    /// Every connection it has ends, as QUIC's idle timeout ends one that
    /// hears nothing, or the machine restarted.
    fn drop_links(&self) {
        self.on_own(|l| {
            for link in l.mesh().links() {
                link.close(Close::Done);
            }
        });
    }

    /// The next entry of its feed in `network`, kept if its kind is.
    fn write(&self, network: &str, kind: Kind, body: Value) -> FeedEntry {
        let now = self.now_ms();
        let mut ws = self.writers.locked();
        let w = ws.entry(network.to_string()).or_insert_with(|| FeedWriter::start(self.me(), network, T));
        let e = w.write(&self.signer, kind, now, body).unwrap();
        if e.class() == xenonet_log::feed::Class::Kept {
            self.kept.locked().entry(network.to_string()).or_default().push(e.clone());
        }
        e
    }

    fn claim(&self, network: &str, i: usize) -> FeedEntry {
        self.write(network, Kind::Claim, json!({ "project": "prj_atlas", "path": format!("src/{i}.rs") }))
    }

    fn seal(e: &FeedEntry, to: u8) -> Envelope {
        Envelope::seal_entry(e, &author(to), transport(to).public(), None).unwrap()
    }

    /// Sends an entry to `to` now, if there is a connection: what doesn't
    /// go is the hello's to send again.
    fn send(&self, e: &FeedEntry, to: u8) -> bool {
        self.links().send(peer(to), Out::Envelope(Party::seal(e, to))).is_ok()
    }

    fn member_by_peer(&self, network: &str, p: PeerId) -> Option<Member> {
        self.views.locked().get(network)?.members.values().find(|m| m.keys.sign == p.0).cloned()
    }

    fn resend(&self, network: &str, to: PeerId, after: Pos) -> Vec<Out> {
        let Some(m) = self.member_by_peer(network, to) else { return Vec::new() };
        let kept = self.kept.locked().get(network).cloned().unwrap_or_default();
        kept.iter().filter(|e| e.pos() > after).map(|e| Out::Envelope(Envelope::seal_entry(e, &m.machine.id(), &m.keys, None).unwrap())).collect()
    }

    /// The kept entries applied of `from`'s feed in `network`, by place.
    fn kept_from(&self, from: u8, network: &str) -> Vec<Pos> {
        let a = author(from);
        self.applied.locked().iter().filter(|e| e.author == a && e.group == network && e.class() == xenonet_log::feed::Class::Kept).map(FeedEntry::pos).collect()
    }

    fn ack(&self, author: &str, network: &str) -> Option<Out> {
        let a = self.feed.locked().ack(author, network).unwrap()?;
        Some(Out::Control(Control::Ack { network: network.into(), author: author.into(), inc: a.inc, high: a.high }))
    }
}

impl Machine for Party {
    fn now_ms(&self) -> u64 {
        let ms = T + u64::try_from(turmoil::elapsed().as_millis()).unwrap();
        ms.saturating_add_signed(self.offset)
    }

    fn hello(&self, _peer: PeerId, network: &str) -> Option<NetHello> {
        let v = self.views.locked().get(network)?.clone();
        let feed = self.feed.locked();
        let feeds = v
            .members
            .values()
            .map(|m| m.machine.id())
            .filter(|a| *a != self.me())
            .filter_map(|a| {
                let h = xenonet_log::feed::FeedStore::head(feed.store(), &a, network).unwrap();
                (h.latest != Pos::ORIGIN).then_some((a, h.latest))
            })
            .collect();
        Some(NetHello { network: network.into(), chain: None, feeds })
    }

    fn hello_in(&self, peer: PeerId, hello: &NetHello) -> Vec<Out> {
        self.hellos.locked().push(hello.network.clone());
        // A peer that holds more of a third member's feed than this
        // machine does: it is asked for the rest.
        for (author, at) in &hello.feeds {
            if *author == self.me() {
                continue;
            }
            let mine = self.head(author, &hello.network);
            let from = self.fetching.locked().ahead(&hello.network, author, peer, *at, mine);
            if let Some(from) = from {
                self.fetch(&hello.network, author, from);
            }
        }
        self.resend(&hello.network, peer, hello.head_of(&self.me()))
    }

    fn envelope_in(&self, peer: PeerId, env: Envelope) -> Vec<Out> {
        let Some(view) = self.views.locked().get(&env.network).cloned() else {
            self.refused.fetch_add(1, Ordering::SeqCst);
            return Vec::new();
        };
        if env.class == Class::Live {
            match env.open_live(&peer.0, &view) {
                Ok(body) => self.live.locked().push((env.network.clone(), body)),
                Err(_) => {
                    self.refused.fetch_add(1, Ordering::SeqCst);
                }
            }
            return Vec::new();
        }
        let me = self.me();
        let opened = env.open_entry(&Recipient { machine: &me, key: &self.transport }, &view);
        let Ok(v) = opened else {
            self.refused.fetch_add(1, Ordering::SeqCst);
            // From a holder, not the author: the next holder is asked.
            if view.by_machine(&env.author).is_some_and(|m| m.keys.sign != peer.0) {
                self.lacks(&env.network, &env.author, peer);
            }
            return Vec::new();
        };
        let (author, network, kept) = (v.author.clone(), v.group.clone(), v.class() == xenonet_log::feed::Class::Kept);
        let offered = self.feed.locked().offer(v).unwrap();
        match offered {
            Offer::New(applied) => {
                self.ended.locked().extend(applied.ended.iter().copied());
                self.applied.locked().extend(applied.entries);
                let head = self.head(&author, &network);
                self.fetching.locked().fetches.filled(&network, &author, head);
            }
            Offer::Duplicate => {
                self.duplicates.fetch_add(1, Ordering::SeqCst);
            }
            Offer::Gap(w) => {
                self.wants.fetch_add(1, Ordering::SeqCst);
                self.fetch(&w.group, &w.author, Pos::new(w.inc, w.from));
                return Vec::new();
            }
            _ => return Vec::new(),
        }
        if kept {
            self.ack(&author, &network).into_iter().collect()
        } else {
            Vec::new()
        }
    }

    fn want_in(&self, peer: PeerId, network: &str, author: &str, from: Pos) -> Vec<Out> {
        if author == self.me() {
            return self.resend(network, peer, Pos::new(from.inc, from.seq.saturating_sub(1)));
        }
        let holds = *self.holds.locked();
        if holds == Holds::Withholds {
            return Vec::new();
        }
        let Some(view) = self.views.locked().get(network).cloned() else { return Vec::new() };
        let ended = self.ended.locked().clone();
        let held = |from: Pos| {
            let applied = self.applied.locked();
            applied
                .iter()
                .filter(|e| e.author == author && e.group == network && e.class() == xenonet_log::feed::Class::Kept && e.pos() >= from)
                .map(|e| {
                    let mut entry = e.clone();
                    if holds == Holds::Tampers {
                        entry.body["path"] = json!("src/elsewhere.rs");
                    }
                    crate::holder::Held { entry, open: !ended.contains(&e.pos()) }
                })
                .collect()
        };
        let admitted = self.admitted.locked().clone();
        let at = |m: &Member| admitted.get(&(network.to_string(), m.id.clone())).copied().unwrap_or(0);
        let now = self.now_ms();
        let out = crate::holder::serve(&view, at, peer, (network, author, from), held, &mut self.fetching.locked().limits, now);
        self.served.fetch_add(out.iter().filter(|o| matches!(o, Out::Envelope(_))).count() as u64, Ordering::SeqCst);
        out
    }

    fn ack_in(&self, _peer: PeerId, _network: &str, _author: &str, _at: Pos) {}

    fn other_in(&self, peer: PeerId, record: Control) {
        match record {
            Control::DontHave { network, author, .. } => self.lacks(&network, &author, peer),
            Control::Compacted { network, author, body } => {
                let Some(m) = self.views.locked().get(&network).and_then(|v| v.by_machine(&author).cloned()) else { return };
                let Some(c) = crate::holder::compacted_of(&body).and_then(|c| c.verify(&m.machine).ok()) else {
                    self.refused.fetch_add(1, Ordering::SeqCst);
                    self.lacks(&network, &author, peer);
                    return;
                };
                let offered = self.feed.locked().offer_compacted(c);
                if let Ok(Offer::New(applied)) = offered {
                    self.ended.locked().extend(applied.ended.iter().copied());
                    self.applied.locked().extend(applied.entries);
                }
                let head = self.head(&author, &network);
                self.fetching.locked().fetches.filled(&network, &author, head);
            }
            _ => {}
        }
    }

    fn log(&self, words: &str) {
        if std::env::var("LINK2_TEST_LOG").is_ok() {
            eprintln!("[{}] {words}", host_of(self.n));
        }
    }
}

fn sim(seed: u64) -> turmoil::Sim<'static> {
    turmoil::Builder::new()
        .rng_seed(seed)
        .simulation_duration(Duration::from_secs(300))
        .min_message_latency(Duration::from_millis(1))
        .max_message_latency(Duration::from_millis(40))
        .enable_random_order()
        .build()
}

/// A machine that only serves: started, and left running.
fn serve(sim: &mut turmoil::Sim<'static>, m: &Arc<Party>) {
    let m = m.clone();
    sim.host(host_of(m.n), move || {
        let m = m.clone();
        async move {
            m.start().await?;
            std::future::pending::<()>().await;
            Ok(())
        }
    });
}

/// Waits, on the simulation's clock, until `ok`.
async fn until(what: &str, ok: impl Fn() -> bool) -> R<()> {
    for _ in 0..20_000 {
        if ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(NetError::internal(format!("never: {what}")))
}

/// Waits until `x` and `y` keep one connection, the same on both ends,
/// and each has the other's hello: when both dial at once, what goes on
/// the connection that loses goes again only if a hello resends it.
async fn settled(x: &Arc<Party>, y: &Arc<Party>) -> R<()> {
    until("one connection, hellos exchanged", || {
        let (Some(xl), Some(yl)) = (x.links.get(), y.links.get()) else { return false };
        let (lx, ly) = (xl.mesh().link(peer(y.n), Alpn::Link), yl.mesh().link(peer(x.n), Alpn::Link));
        let same = matches!((&lx, &ly), (Some(a), Some(b)) if a.dialled_by() == b.dialled_by());
        let hello = |m: &Arc<Party>, o: u8| m.links().peers().get(&peer(o)).is_some_and(|p| p.skew_ms.is_some());
        same && hello(x, y.n) && hello(y, x.n)
    })
    .await
}

fn in_order(ps: &[Pos]) -> bool {
    ps.windows(2).all(|w| w[0] < w[1])
}

#[test]
fn every_entry_is_applied_at_most_once_however_often_it_comes() {
    for seed in [1, 2, 3] {
        let mut s = sim(seed);
        let a = Party::new(A, &[(N1, &[A, B])]);
        let b = Party::new(B, &[(N1, &[A, B])]);
        serve(&mut s, &b);
        let (a2, b2) = (a.clone(), b.clone());
        s.client("a", async move {
            a2.start().await?;
            settled(&a2, &b2).await?;
            let mut sent = Vec::new();
            for i in 0..20 {
                let e = if i % 3 == 0 { a2.write(N1, Kind::Message, json!({ "to": "b", "text": format!("m{i}") })) } else { a2.claim(N1, i) };
                // Each twice, as the gateway's path and link/2 both carry it.
                assert!(a2.send(&e, B) && a2.send(&e, B));
                sent.push(e);
            }
            until("all applied", || b2.applied.locked().len() == sent.len()).await?;
            // And again on a new connection, whose hello resends the rest.
            a2.links().mesh().link(peer(B), Alpn::Link).unwrap().close(Close::Done);
            until("closed", || !a2.links().is_connected(peer(B))).await?;
            settled(&a2, &b2).await?;
            for e in &sent {
                a2.send(e, B);
            }
            until("the copies came", || b2.duplicates.load(Ordering::SeqCst) >= 2 * sent.len() as u64).await?;
            Ok(())
        });
        s.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let ids: Vec<String> = b.applied.locked().iter().map(FeedEntry::id).collect();
        let unique: BTreeSet<&String> = ids.iter().collect();
        assert_eq!((ids.len(), unique.len()), (20, 20), "seed {seed}: each once");
    }
}

#[test]
fn kept_entries_apply_in_their_authors_order_over_a_reordering_network() {
    for seed in [4, 5, 6] {
        let mut s = sim(seed);
        let a = Party::new(A, &[(N1, &[A, B, C])]);
        let b = Party::new(B, &[(N1, &[A, B, C])]);
        let c = Party::new(C, &[(N1, &[A, B, C])]);
        serve(&mut s, &b);
        serve(&mut s, &c);
        let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
        s.client("a", async move {
            a2.start().await?;
            for i in 0..60 {
                let e = a2.claim(N1, i);
                a2.send(&e, B);
                a2.send(&e, C);
                // Connections drop now and then, so entries come both on
                // the stream and in a later hello's resend.
                if i % 17 == 16 {
                    if let Some(l) = a2.links().mesh().link(peer(B), Alpn::Link) {
                        l.close(Close::Done);
                    }
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            until("b and c hold all 60", || b2.kept_from(A, N1).len() == 60 && c2.kept_from(A, N1).len() == 60).await?;
            Ok(())
        });
        s.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        for m in [&b, &c] {
            let got = m.kept_from(A, N1);
            assert!(in_order(&got), "seed {seed}: {got:?}");
            assert_eq!(got.first().map(|p| p.seq), Some(1));
        }
    }
}

#[test]
fn a_gap_is_held_asked_for_and_filled_never_skipped() {
    let mut s = sim(7);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        let es: Vec<FeedEntry> = (0..6).map(|i| a2.claim(N1, i)).collect();
        // 3 and 4 are lost on the way.
        for e in [&es[0], &es[1], &es[4], &es[5]] {
            a2.send(e, B);
        }
        until("b fills the gap from a's answer to its want", || b2.kept_from(A, N1).len() == 6).await?;
        Ok(())
    });
    s.run().unwrap();
    assert!(b.wants.load(Ordering::SeqCst) >= 1, "the gap was asked for");
    assert_eq!(b.kept_from(A, N1).iter().map(|p| p.seq).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6], "applied in order, 5 and 6 held until 3 and 4 came");
    assert_eq!(b.feed.locked().store().held_count(&author(A), N1), 0);
}

#[test]
fn two_entries_at_one_place_freeze_the_authors_feed() {
    let mut s = sim(8);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        let first = a2.claim(N1, 1);
        // A clone of a's writer, as a badly restored machine would be.
        let mut twin = a2.writers.locked().get(N1).unwrap().clone();
        let second = a2.claim(N1, 2);
        let other = twin.write(&a2.signer, Kind::Claim, a2.now_ms(), json!({ "project": "prj_atlas", "path": "src/twin.rs" })).unwrap();
        assert_eq!((second.pos(), second.id() == other.id()), (other.pos(), false));
        for e in [&first, &second, &other] {
            a2.send(e, B);
        }
        until("frozen", || b2.feed.locked().frozen(&author(A), N1).unwrap().is_some()).await?;
        let third = a2.claim(N1, 3);
        a2.send(&third, B);
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(b.kept_from(A, N1).len(), 2, "nothing past the fork is taken");
}

#[test]
fn a_compact_entry_ends_what_it_leaves_out() {
    let mut s = sim(9);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        let one = a2.claim(N1, 1);
        let two = a2.claim(N1, 2);
        a2.send(&one, B);
        a2.send(&two, B);
        until("both open at b", || b2.kept_from(A, N1).len() == 2).await?;
        // The first claim was released: the compact entry keeps the second.
        let compact = {
            let mut ws = a2.writers.locked();
            ws.get_mut(N1).unwrap().compact(&a2.signer, a2.now_ms(), &[two.seq]).unwrap()
        };
        a2.kept.locked().get_mut(N1).unwrap().push(compact.clone());
        a2.send(&compact, B);
        until("the compact entry applied", || b2.kept_from(A, N1).len() == 3).await?;
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(*b.ended.locked(), [Pos::new(b.kept_from(A, N1)[0].inc, 1)], "what the compact entry left out is ended");
}

#[test]
fn a_removed_members_entries_are_refused_even_signed_before() {
    let mut s = sim(10);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &a);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("b", async move {
        b2.start().await?;
        settled(&b2, &a2).await?;
        let before = b2.claim(N1, 1);
        let held = b2.claim(N1, 2);
        b2.send(&before, A);
        until("taken", || a2.kept_from(B, N1).len() == 1).await?;
        // a's chain takes b out: the connection closes, as no network is
        // left between them.
        let link = b2.links().mesh().link(peer(A), Alpn::Link).unwrap();
        a2.remove(N1, B);
        assert_eq!(link.closed().await, Some(Close::Removed));
        // b still thinks it is in: its dials are refused, and what it
        // signed before the removal never gets in.
        tokio::time::sleep(Duration::from_secs(2)).await;
        b2.send(&held, A);
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(a.kept_from(B, N1).len(), 1);
    assert!(!a.links().is_connected(peer(B)));
}

#[test]
fn after_a_partition_the_receiver_stands_where_the_author_does() {
    let mut s = sim(11);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        for i in 0..5 {
            let e = a2.claim(N1, i);
            a2.send(&e, B);
        }
        until("five at b", || b2.kept_from(A, N1).len() == 5).await?;
        turmoil::partition("a", "b");
        // QUIC's idle timeout ends both ends of a connection that hears
        // nothing; the simulated network leaves that to the test.
        for m in [&a2, &b2] {
            m.drop_links();
        }
        until("both ends closed", || !a2.links().is_connected(peer(B)) && !b2.links().is_connected(peer(A))).await?;
        for i in 5..12 {
            let e = a2.claim(N1, i);
            assert!(!a2.send(&e, B), "nothing goes while apart");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        turmoil::repair("a", "b");
        until("b stands where a does", || b2.kept_from(A, N1).len() == 12).await?;
        Ok(())
    });
    s.run().unwrap();
    let got = b.kept_from(A, N1);
    assert!(in_order(&got) && got.len() == 12, "{got:?}");
}

#[test]
fn a_reconnect_storm_ends_with_one_connection_a_pair_and_every_entry_once() {
    for seed in [12, 13] {
        let mut s = sim(seed);
        let all: &[u8] = &[A, B, C];
        let (a, b, c) = (Party::new(A, &[(N1, all)]), Party::new(B, &[(N1, all)]), Party::new(C, &[(N1, all)]));
        serve(&mut s, &b);
        serve(&mut s, &c);
        let (a2, b2, c2) = (a.clone(), b.clone(), c.clone());
        s.client("a", async move {
            a2.start().await?;
            for i in 0..40 {
                let e = a2.claim(N1, i);
                a2.send(&e, B);
                a2.send(&e, C);
                // Every few entries, every machine drops every connection
                // it has, and all of them dial again at once.
                if i % 4 == 3 {
                    for m in [&a2, &b2, &c2] {
                        m.drop_links();
                    }
                }
                tokio::time::sleep(Duration::from_millis(3)).await;
            }
            until("b and c hold all 40", || b2.kept_from(A, N1).len() == 40 && c2.kept_from(A, N1).len() == 40).await?;
            // Settled: one connection between each pair, the same on both
            // ends.
            until("settled", || [&a2, &b2, &c2].iter().all(|m| m.links().peers().len() == 2)).await?;
            Ok(())
        });
        s.run().unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        for m in [&b, &c] {
            let got = m.kept_from(A, N1);
            assert!(in_order(&got) && got.len() == 40, "seed {seed}: {got:?}");
        }
        for m in [&a, &b, &c] {
            let links = m.links().mesh().links();
            let peers: BTreeSet<PeerId> = links.iter().map(|l| l.peer()).collect();
            assert_eq!(links.len(), peers.len(), "seed {seed}: one connection a peer");
            for l in &links {
                let other = [&a, &b, &c].into_iter().find(|o| peer(o.n) == l.peer()).unwrap();
                let theirs = other.links().mesh().link(peer(m.n), Alpn::Link).unwrap();
                assert_eq!(theirs.dialled_by(), l.dialled_by(), "seed {seed}: both ends keep the same connection");
            }
        }
    }
}

#[test]
fn a_restored_author_writes_from_a_new_incarnation_and_nothing_freezes() {
    let mut s = sim(14);
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        let mut at_six = None;
        for i in 0..10 {
            let e = a2.claim(N1, i);
            a2.send(&e, B);
            if i == 5 {
                at_six = a2.writers.locked().get(N1).cloned();
            }
        }
        until("ten at b", || b2.kept_from(A, N1).len() == 10).await?;
        // a's store comes back from a backup taken at six: it lost 7 to 10,
        // which b holds, and writes on from a new incarnation.
        let six = at_six.unwrap();
        let restored = FeedWriter::restore(a2.me(), N1, six.head(), six.inc(), a2.now_ms());
        a2.writers.locked().insert(N1.into(), restored);
        a2.kept.locked().get_mut(N1).unwrap().truncate(6);
        for l in a2.links().mesh().links() {
            l.close(Close::Done);
        }
        for i in 10..13 {
            a2.claim(N1, i);
        }
        until("the new incarnation at b", || b2.kept_from(A, N1).len() == 13).await?;
        Ok(())
    });
    s.run().unwrap();
    let got = b.kept_from(A, N1);
    assert!(in_order(&got), "{got:?}");
    assert!(got[10].inc > got[9].inc, "a new incarnation");
    assert!(b.feed.locked().frozen(&author(A), N1).unwrap().is_none(), "a restore is no fork");
}

#[test]
fn a_member_removed_from_n1_while_in_n2_gets_nothing_of_n1_and_n2_flows() {
    let mut s = sim(15);
    let a = Party::new(A, &[(N1, &[A, B]), (N2, &[A, B])]);
    let b = Party::new(B, &[(N1, &[A, B]), (N2, &[A, B])]);
    serve(&mut s, &b);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("a", async move {
        a2.start().await?;
        settled(&a2, &b2).await?;
        let link = a2.links().mesh().link(peer(B), Alpn::Link).unwrap();
        assert_eq!(link.networks(), [N1.to_string(), N2.to_string()].into(), "one connection for both");
        let e1 = a2.claim(N1, 1);
        assert!(a2.send(&e1, B));
        until("n1 at b", || b2.kept_from(A, N1).len() == 1).await?;
        // a's chain of N1 takes b out; b's copy of it hasn't heard.
        a2.remove(N1, B);
        until("N1 off the connection", || link.networks() == [N2.to_string()].into()).await?;
        assert!(!link.is_closed(), "N2 still flows on it");
        let e2 = a2.claim(N1, 2);
        assert!(!a2.send(&e2, B), "a sends nothing of N1 to b");
        let f = a2.claim(N2, 1);
        assert!(a2.send(&f, B));
        until("n2 at b", || b2.kept_from(A, N2).len() == 1).await?;
        // What b sends of N1 is dropped at a; N2 is taken.
        let from_b1 = b2.claim(N1, 1);
        let from_b2 = b2.claim(N2, 1);
        assert!(b2.send(&from_b1, A), "b's chain still has a in N1");
        assert!(b2.send(&from_b2, A));
        until("b's n2 at a", || a2.kept_from(B, N2).len() == 1).await?;
        // A new connection's hello: a speaks of N2 alone, and reads only
        // N2 of b's, though b lists both.
        a2.hellos.locked().clear();
        b2.hellos.locked().clear();
        link.close(Close::Done);
        until("reconnected", || a2.links().is_connected(peer(B)) && a2.hellos.locked().len() == 1 && b2.hellos.locked().len() == 1).await?;
        assert_eq!((a2.hellos.locked().clone(), b2.hellos.locked().clone()), (vec![N2.to_string()], vec![N2.to_string()]));
        // Out of N2 too: no network is left, and the connection closes.
        let link = a2.links().mesh().link(peer(B), Alpn::Link).unwrap();
        a2.remove(N2, B);
        assert_eq!(link.closed().await, Some(Close::Removed));
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(b.kept_from(A, N1).len(), 1, "nothing of N1 after the removal");
    assert!(a.kept_from(B, N1).is_empty(), "nothing of N1 from b taken at a");
}

#[test]
fn a_clock_ahead_shows_as_skew_its_entries_are_taken_and_only_its_presence_dropped() {
    let mut s = sim(16);
    let ten_min = 10 * 60 * 1000;
    let a = Party::new(A, &[(N1, &[A, B])]);
    let b = Party::with_clock(B, &[(N1, &[A, B])], ten_min);
    serve(&mut s, &a);
    let (a2, b2) = (a.clone(), b.clone());
    s.client("b", async move {
        b2.start().await?;
        settled(&b2, &a2).await?;
        let skew = a2.links().peers()[&peer(B)].skew_ms.unwrap();
        assert!((ten_min - 1000..=ten_min + 1000).contains(&skew), "{skew}");
        // A kept entry and a message, ten minutes ahead: taken.
        let kept = b2.claim(N1, 1);
        let loose = b2.write(N1, Kind::Message, json!({ "to": "a", "text": "hi" }));
        b2.send(&kept, A);
        b2.send(&loose, A);
        // Presence ten minutes ahead: dropped; and from a clock that's
        // right again, taken.
        let live = Envelope::live(N1, &b2.me(), "presence", b2.now_ms(), &json!({ "counter": 1 })).unwrap();
        b2.links().send(peer(A), Out::Envelope(live)).unwrap();
        let fine = Envelope::live(N1, &b2.me(), "presence", a2.now_ms(), &json!({ "counter": 2 })).unwrap();
        b2.links().send(peer(A), Out::Envelope(fine)).unwrap();
        until("taken", || a2.applied.locked().len() == 2 && a2.live.locked().len() == 1).await?;
        Ok(())
    });
    s.run().unwrap();
    assert_eq!(a.live.locked()[0].1["counter"], 2, "only the presence ahead was dropped");
}

#[test]
fn records_read_back_and_a_record_past_the_limit_is_refused() {
    let hello = Control::Hello { at: T, networks: vec![NetHello { network: N1.into(), chain: Some("h".into()), feeds: vec![(author(A), Pos::new(T, 4))] }] };
    let want = Control::Want { network: N1.into(), author: author(A), inc: T, from: 3 };
    let ack = Control::Ack { network: N2.into(), author: author(B), inc: T, high: 9 };
    let held = Control::Held { network: N1.into(), body: json!({ "ids": [] }) };
    for c in [hello, want, ack, held] {
        assert_eq!(Control::decode(&c.encode()).unwrap(), c);
    }
    assert!(Control::decode(br#"{"t":"want","network":"net_x","author":"a","inc":1,"from":1}"#).is_err());
    assert!(Control::decode(br#"{"t":"hello","v":1,"at":1,"networks":[]}"#).is_err(), "only version 2");
    assert_eq!(read_header(&stream_header(Class::Kept)).unwrap(), Class::Kept);
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        let (w, r) = tokio::io::duplex(64);
        let mut w: Writer = Box::new(w);
        let mut r: Reader = Box::new(r);
        let too_big = u32::try_from(MAX_RECORD + 1).unwrap().to_be_bytes();
        tokio::spawn(async move {
            let _ = w.write_all(&too_big).await;
        });
        assert_eq!(read_record(&mut r).await.unwrap_err().code, "bad-request");
    });
}

#[path = "holder_tests.rs"]
mod holder;
