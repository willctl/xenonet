//! The worker behind sync/2: what arrives, what goes, and the folder. Every file it reads or
//! writes, it does here, on its own thread, so the endpoint's thread never waits on a disk
//! (review revision 36). `Machine` is link/2's side of it: it answers from a snapshot the worker
//! keeps (`Snap`) and hands everything else over as a `Job`.

use crate::hello::{self, Bindings, Nonces, Told};
use crate::{Config, Net, hex, unhex};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};
use xenolith_core::chains::{self, Level};
use xenolith_core::keys::{Keys, Signer};
use xenolith_core::spaces::{self, SpaceId};
use xenolith_core::sync::{self as cs, Feeds, Follow, Ledger, Outbox, PageAt, Taken};
use xenolith_core::text::{hash_text, now_ms};
use xenonet_id::sign::VerifyingKey;
use xenonet_log::feed::{Feed, FeedEntry, Kind, Offer, Pos};
use xenonet_log::space::Space;
use xenonet_net::dial::{Close, Link, PeerId};
use xenonet_net::envelope::Envelope;
use xenonet_net::link2::{Control, Machine, NetHello, Out};
use xenonet_net::{NetError, R};

/// How long the worker waits after a change in the folder before it looks, so a burst of writes
/// is one look.
const SETTLE: Duration = Duration::from_millis(150);
/// How long a follow record's addresses are kept in the endpoint's lookup.
const ADDR_TTL: Duration = Duration::from_secs(365 * 24 * 3600);

pub(crate) enum Job {
    /// The engine stops.
    Stop,
    /// An invite or a join needs the endpoint now, peers or not.
    OpenNow,
    /// A joiner's code checked: share the invite's spaces with it, and answer.
    Admit {
        pending: Box<crate::join::Pending>,
        key: String,
        endpoint: String,
        reply: tokio::sync::oneshot::Sender<Result<Value, String>>,
    },
    /// A share or a follow changed.
    Changed,
    /// The endpoint opened.
    Opened,
    /// Something changed in a space's folder.
    Fs,
    Forget {
        text: String,
        ignore_case: bool,
    },
    /// A peer's hello checked: its context key and the spaces it told of.
    Hello {
        peer: PeerId,
        key: VerifyingKey,
        told: Vec<Told>,
    },
    /// The peer's hello's part for one space.
    GroupHello {
        peer: PeerId,
        group: String,
        digest: Option<String>,
        heads: Vec<(String, Pos)>,
    },
    Envelope {
        peer: PeerId,
        env: Box<Envelope>,
    },
    Want {
        peer: PeerId,
        group: String,
        author: String,
        from: Pos,
    },
    Group {
        peer: PeerId,
        group: String,
        body: Value,
    },
    Down(PeerId),
}

/// A space's chain digest (None before its chain is here) and where this Xenolith stands with each
/// author's feed in it.
type GroupAt = (Option<String>, Vec<(String, Pos)>);

/// What link/2's side answers from without a file: per space, the digest of its chain and where
/// this Xenolith stands with each feed; who may speak as whom; what to tell each endpoint.
#[derive(Default)]
struct Snap {
    groups: BTreeMap<String, GroupAt>,
    bind: Bindings,
    tell: BTreeMap<String, Vec<Told>>,
}

#[derive(Default)]
pub(crate) struct Shared {
    snap: RwLock<Snap>,
    nonces: Nonces,
}

fn now() -> u64 {
    u64::try_from(now_ms()).unwrap_or(0)
}

/// The transport key as xenonet-net's host key: its Ed25519 half keys the endpoint. A space's
/// entries are never sealed (they go unsealed between admitted members), so the X25519 half is
/// derived and never used.
pub(crate) fn host_key(keys: &Keys) -> Result<xenonet_net::keys::HostKey, String> {
    let seed = keys.transport.secret();
    let mut m = b"xenolith transport seal\n".to_vec();
    m.extend_from_slice(&seed);
    xenonet_net::keys::HostKey::from_secrets(&seed, xenonet_id::codec::sha256(&m)).map_err(|e| e.message)
}

// ---------------------------------------------------------------- link/2's side

struct Side {
    cfg: Arc<Config>,
    jobs: Mutex<mpsc::Sender<Job>>,
    shared: Arc<Shared>,
    me: [u8; 32],
}

pub(crate) fn machine(cfg: &Arc<Config>, jobs: mpsc::Sender<Job>, me: [u8; 32], shared: Arc<Shared>) -> Arc<dyn Machine> {
    Arc::new(Side { cfg: cfg.clone(), jobs: Mutex::new(jobs), shared, me })
}

impl Side {
    fn job(&self, j: Job) {
        if let Ok(tx) = self.jobs.lock() {
            let _ = tx.send(j);
        }
    }
}

impl Machine for Side {
    fn now_ms(&self) -> u64 {
        now()
    }

    fn hello(&self, _peer: PeerId, network: &str) -> Option<NetHello> {
        let snap = self.shared.snap.read().ok()?;
        let (digest, feeds) = snap.groups.get(network).cloned().unwrap_or((None, vec![]));
        Some(NetHello { network: network.to_string(), chain: digest, feeds })
    }

    fn hello_in(&self, peer: PeerId, h: &NetHello) -> Vec<Out> {
        self.job(Job::GroupHello { peer, group: h.network.clone(), digest: h.chain.clone(), heads: h.feeds.clone() });
        vec![]
    }

    fn envelope_in(&self, peer: PeerId, env: Envelope) -> Vec<Out> {
        self.job(Job::Envelope { peer, env: Box::new(env) });
        vec![]
    }

    fn want_in(&self, peer: PeerId, network: &str, author: &str, from: Pos) -> Vec<Out> {
        self.job(Job::Want { peer, group: network.to_string(), author: author.to_string(), from });
        vec![]
    }

    fn ack_in(&self, _peer: PeerId, _network: &str, _author: &str, _at: Pos) {}

    fn other_in(&self, peer: PeerId, record: Control) {
        if let Control::Group { network, body } = record {
            self.job(Job::Group { peer, group: network, body });
        }
    }

    fn up(&self, peer: PeerId, up: bool) {
        if !up {
            self.job(Job::Down(peer));
        }
    }

    fn log(&self, words: &str) {
        (self.cfg.log)(&format!("sync: {words}"));
    }

    fn hello_ext(&self, link: &Link) -> Option<Value> {
        let peer = link.peer();
        let tell = self.shared.snap.read().ok()?.tell.get(&hex(peer.0)).cloned().unwrap_or_default();
        hello::make(&self.cfg.keys.context, &self.me, &peer.0, &tell, link.exporter(hello::EXPORTER).as_ref(), now_ms())
    }

    fn hello_ext_in(&self, link: &Link, ext: Option<&Value>) -> R<()> {
        let peer = link.peer();
        let checked = hello::check(ext, &self.me, &peer.0, link.exporter(hello::EXPORTER).as_ref(), &self.shared.nonces, now_ms()).map_err(|e| NetError::new(403, "refused", e))?;
        let bound = self.shared.snap.read().is_ok_and(|s| s.bind.get(&hex(peer.0)).is_some_and(|keys| keys.contains_key(&checked.key.id())));
        if !bound {
            let why = format!("{} spoke as {}, which no space here binds to that endpoint", hex(peer.0), checked.key.id());
            self.log(&format!("a hello was refused: {why}"));
            return Err(NetError::new(403, "not-a-member", why));
        }
        self.job(Job::Hello { peer, key: checked.key, told: checked.spaces });
        Ok(())
    }
}

// ---------------------------------------------------------------- the worker

/// One space this Xenolith holds a chain of.
struct Held {
    address: SpaceId,
    space: Space,
}

/// What a peer is in a space, to this Xenolith.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// The space's curator; this Xenolith is a member.
    Curator,
    /// A member, at its level; this Xenolith curates.
    Member(Level),
}

struct Worker {
    cfg: Arc<Config>,
    shared: Arc<Shared>,
    net: Arc<OnceLock<Net>>,
    status: Arc<Mutex<Value>>,
    open: Option<mpsc::Sender<()>>,
    me: String,
    /// This Xenolith's endpoint id.
    endpoint: String,
    held: BTreeMap<String, Held>,
    follows: Vec<Follow>,
    feeds: BTreeMap<String, Feeds>,
    outs: BTreeMap<String, Outbox>,
    ledgers: BTreeMap<String, Ledger>,
    peers: BTreeMap<PeerId, VerifyingKey>,
    groups_set: BTreeSet<String>,
    watch: Watch,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn(
    cfg: Arc<Config>,
    tx: mpsc::Sender<Job>,
    rx: mpsc::Receiver<Job>,
    net: Arc<OnceLock<Net>>,
    status: Arc<Mutex<Value>>,
    open: mpsc::Sender<()>,
    shared: Arc<Shared>,
) -> Result<std::thread::JoinHandle<()>, String> {
    let endpoint = hex(host_key(&cfg.keys)?.public().sign);
    let watch = Watch::new(&cfg, &tx);
    let mut w = Worker {
        me: cfg.keys.context.public().id(),
        endpoint,
        cfg,
        shared,
        net,
        status,
        open: Some(open),
        held: BTreeMap::new(),
        follows: vec![],
        feeds: BTreeMap::new(),
        outs: BTreeMap::new(),
        ledgers: BTreeMap::new(),
        peers: BTreeMap::new(),
        groups_set: BTreeSet::new(),
        watch,
    };
    std::thread::Builder::new()
        .name("xenolith-sync".into())
        .spawn(move || {
            w.refresh();
            w.scan();
            let mut due: Option<Instant> = None;
            loop {
                let job = match due {
                    Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                        Ok(j) => Some(j),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    },
                    None => match rx.recv() {
                        Ok(j) => Some(j),
                        Err(_) => return,
                    },
                };
                match job {
                    None => {
                        due = None;
                        w.scan();
                    }
                    Some(Job::Fs) => {
                        due.get_or_insert_with(|| Instant::now() + SETTLE);
                    }
                    // The open signal goes with the worker, so an endpoint that never opened ends.
                    Some(Job::Stop) => return,
                    Some(j) => {
                        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w.handle(j)));
                        if r.is_err() {
                            w.log("a job failed (internal error); the next one goes on");
                        }
                    }
                }
            }
        })
        .map_err(|e| format!("sync: no thread for the worker: {e}"))
}

/// Watches the spaces' folders: a page, a note or a day of the log written there wakes the worker.
/// The system's events (FSEvents, inotify, ReadDirectoryChangesW) watch the whole `spaces/` tree.
/// On macOS FSEvents can lag by seconds (more when fseventsd is busy), so the folders sync reads
/// (each synced space's `pages/`, `inbox/` and this year's `log/`) and the pages in them are
/// watched with kqueue too, as XenoNet's adapters follow a transcript: one descriptor each, so
/// at most `KQUEUE_MAX` (`Watch::track`).
pub(crate) struct Watch {
    fs: Option<notify::RecommendedWatcher>,
    /// `spaces/`, watched once it's there: sync never makes it.
    base: PathBuf,
    armed: bool,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    #[cfg(target_os = "macos")]
    kq: Option<notify::KqueueWatcher>,
    #[cfg(target_os = "macos")]
    tracked: BTreeSet<PathBuf>,
}

/// The most paths kqueue watches beside FSEvents, each a file descriptor.
#[cfg(target_os = "macos")]
const KQUEUE_MAX: usize = 128;

fn waker(tx: &mpsc::Sender<Job>, filter: Option<[PathBuf; 2]>) -> impl Fn(notify::Result<notify::Event>) + Send + 'static {
    let tx = Mutex::new(tx.clone());
    move |ev: notify::Result<notify::Event>| {
        let Ok(ev) = ev else { return };
        let ours = filter.as_ref().is_none_or(|wanted| {
            ev.paths.iter().any(|p| {
                wanted.iter().filter_map(|w| p.strip_prefix(w).ok()).next().is_some_and(|rel| {
                    let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                    parts.len() <= 2 || matches!(parts.get(2).map(String::as_str), Some("pages" | "inbox" | "log"))
                })
            })
        });
        if let (true, Ok(tx)) = (ours, tx.lock()) {
            let _ = tx.send(Job::Fs);
        }
    }
}

impl Watch {
    fn new(cfg: &Arc<Config>, tx: &mpsc::Sender<Job>) -> Watch {
        #[cfg(target_os = "macos")]
        use notify::Watcher as _;
        let base = cfg.root.join(spaces::SPACES_DIR);
        // Events name the folder as the OS resolves it (on macOS /var is /private/var).
        let real = std::fs::canonicalize(&cfg.root).map_or_else(|_| base.clone(), |r| r.join(spaces::SPACES_DIR));
        let fs = notify::recommended_watcher(waker(tx, Some([base.clone(), real]))).ok();
        let mut w = Watch {
            fs,
            base,
            armed: false,
            log: cfg.log.clone(),
            #[cfg(target_os = "macos")]
            kq: notify::KqueueWatcher::new(waker(tx, None), notify::Config::default()).ok(),
            #[cfg(target_os = "macos")]
            tracked: BTreeSet::new(),
        };
        w.arm();
        w
    }

    /// Watches `spaces/` once it's there.
    fn arm(&mut self) {
        use notify::Watcher as _;
        if self.armed || !self.base.is_dir() {
            return;
        }
        let Some(fs) = self.fs.as_mut() else { return };
        self.armed = true;
        if let Err(e) = fs.watch(&self.base, notify::RecursiveMode::Recursive) {
            (self.log)(&format!("sync: the spaces' folders can't be watched ({e}); changes there go out at the next start"));
        }
    }

    /// The folders and files kqueue watches from now on (macOS; elsewhere the system's events are
    /// enough). Each is watched again, since a page written whole (a rename) is a new file.
    fn track(&mut self, paths: &[PathBuf]) {
        self.arm();
        #[cfg(target_os = "macos")]
        if let Some(kq) = self.kq.as_mut() {
            use notify::Watcher as _;
            for p in std::mem::take(&mut self.tracked) {
                let _ = kq.unwatch(&p);
            }
            for p in paths.iter().filter(|p| p.exists()).take(KQUEUE_MAX) {
                if kq.watch(p, notify::RecursiveMode::NonRecursive).is_ok() {
                    self.tracked.insert(p.clone());
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = paths;
    }
}

fn envelope_err(e: &NetError) -> String {
    e.message.clone()
}

impl Worker {
    fn log(&self, words: &str) {
        (self.cfg.log)(&format!("sync: {words}"));
    }

    fn links(&self) -> Option<&Arc<xenonet_net::link2::Links>> {
        self.net.get().map(|n| &n.links)
    }

    fn send(&self, peer: PeerId, out: Out) {
        let Some(l) = self.links() else { return };
        // A peer that isn't connected gets it from its next hello.
        if let Err(e) = l.send(peer, out)
            && e.code != "not-connected"
        {
            self.log(&format!("couldn't send to {}: {}", peer, e.message));
        }
    }

    fn dir_of(&self, id: &str) -> Option<PathBuf> {
        self.held.get(id).map(|h| spaces::space_dir(&self.cfg.root, &h.address))
    }

    fn curates(&self, id: &str) -> bool {
        self.held.get(id).is_some_and(|h| h.space.view().curator.id() == self.me)
    }

    fn follow_for(&self, id: &str) -> Option<&Follow> {
        self.follows.iter().find(|f| f.space == id)
    }

    /// The endpoint the curator of `id` is admitted from, from this Xenolith's side.
    fn curator_endpoint(&self, id: &str) -> Option<(String, String)> {
        match self.held.get(id) {
            Some(h) => {
                let c = h.space.view().curator.id();
                chains::members(&h.space)
                    .into_iter()
                    .find(|m| m.key == c)
                    .map(|m| (c.clone(), m.endpoint))
                    .or_else(|| self.follows.iter().find(|f| f.curator == c).map(|f| (c.clone(), f.endpoint.clone())))
            }
            None => self.follow_for(id).map(|f| (f.curator.clone(), f.endpoint.clone())),
        }
    }

    /// What `key` (a peer's context key id, from `ep`) is in space `id`.
    fn role(&self, id: &str, key: &str, ep: &str) -> Option<Role> {
        if self.curates(id) {
            let h = self.held.get(id)?;
            return chains::members(&h.space).into_iter().find(|m| m.key == key && m.endpoint == ep).map(|m| Role::Member(m.level));
        }
        self.curator_endpoint(id).filter(|(k, e)| k == key && e == ep).map(|_| Role::Curator)
    }

    fn peer_key(&self, peer: PeerId) -> Option<VerifyingKey> {
        self.peers.get(&peer).cloned()
    }

    fn connected_in(&self, id: &str) -> Vec<(PeerId, VerifyingKey, Role)> {
        self.peers.iter().filter_map(|(p, k)| self.role(id, &k.id(), &hex(p.0)).map(|r| (*p, k.clone(), r))).collect()
    }

    fn feeds(&mut self, id: &str) -> &mut Feeds {
        let paths = &self.cfg.paths;
        self.feeds.entry(id.to_string()).or_insert_with(|| Feeds::load(paths, id))
    }

    fn ledger(&mut self, id: &str) -> &mut Ledger {
        let paths = &self.cfg.paths;
        self.ledgers.entry(id.to_string()).or_insert_with(|| Ledger::load(paths, id))
    }

    fn save_ledger(&self, id: &str) {
        if let Some(Err(e)) = self.ledgers.get(id).map(|l| l.save(&self.cfg.paths, id)) {
            self.log(&e);
        }
    }

    fn out(&mut self, id: &str) -> Option<&mut Outbox> {
        if !self.outs.contains_key(id) {
            match Outbox::load(&self.cfg.paths, id, &self.me, now()) {
                Ok(o) => {
                    self.outs.insert(id.to_string(), o);
                }
                Err(e) => {
                    self.log(&e);
                    return None;
                }
            }
        }
        self.outs.get_mut(id)
    }

    fn write(&mut self, id: &str, kind: Kind, body: Value) -> Option<FeedEntry> {
        let keys = self.cfg.keys.clone();
        match self.out(id)?.write(&keys.context, kind, body) {
            Ok(e) => Some(e),
            Err(e) => {
                self.log(&format!("couldn't write a {} for {id}: {e}", kind.as_str()));
                None
            }
        }
    }

    // ---------------------------------------------------------------- membership

    /// Reads the chains and follow records again, and puts the endpoint's admission, the
    /// snapshot link/2 answers from, and the status right.
    fn refresh(&mut self) {
        self.held = chains::held(&self.cfg.paths).into_iter().map(|(p, s)| (p.id.clone(), Held { address: p.address, space: s })).collect();
        self.follows = cs::follows(&self.cfg.paths);
        let mut bind: Bindings = BTreeMap::new();
        let mut tell: BTreeMap<String, Vec<Told>> = BTreeMap::new();
        let mut admit: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut keep: BTreeSet<String> = BTreeSet::new();
        let ids: Vec<String> = self.held.keys().cloned().collect();
        for id in &ids {
            let h = &self.held[id];
            let told = Told { space: id.clone(), genesis: h.space.entries()[0].hash(), digest: chains::digest(&h.space) };
            if self.curates(id) {
                for m in chains::members(&h.space) {
                    bind.entry(m.endpoint.clone()).or_default().entry(m.key.clone()).or_default().insert(id.clone());
                    tell.entry(m.endpoint.clone()).or_default().push(told.clone());
                    admit.entry(id.clone()).or_default().insert(m.endpoint);
                }
            } else if let Some((key, ep)) = self.curator_endpoint(id) {
                bind.entry(ep.clone()).or_default().entry(key).or_default().insert(id.clone());
                admit.entry(id.clone()).or_default().insert(ep.clone());
                keep.insert(ep);
            }
        }
        for f in &self.follows {
            if !self.held.contains_key(&f.space) {
                bind.entry(f.endpoint.clone()).or_default().entry(f.curator.clone()).or_default().insert(f.space.clone());
                admit.entry(f.space.clone()).or_default().insert(f.endpoint.clone());
                keep.insert(f.endpoint.clone());
            }
        }
        let mut groups = BTreeMap::new();
        for id in admit.keys() {
            let digest = self.held.get(id).map(|h| chains::digest(&h.space));
            let heads = self.feeds(id).heads();
            groups.insert(id.clone(), (digest, heads));
        }
        // A connected member this Xenolith can now tell of a space it couldn't before hears of it
        // only in a hello: its connection closes, and its next dial carries a hello that names
        // the space (revision 6).
        let mut grew = vec![];
        if let Ok(mut s) = self.shared.snap.write() {
            for (ep, told) in &tell {
                let before: BTreeSet<&str> = s.tell.get(ep).map(|t| t.iter().map(|x| x.space.as_str()).collect()).unwrap_or_default();
                if told.iter().any(|t| !before.contains(t.space.as_str())) {
                    grew.extend(unhex(ep).map(PeerId));
                }
            }
            *s = Snap { groups, bind: bind.clone(), tell };
        }
        if let Some(net) = self.net.get() {
            for p in grew.into_iter().filter(|p| self.peers.contains_key(p)) {
                if let Some(link) = net.links.mesh().link(p, crate::protocol().alpn) {
                    link.close(Close::Done);
                }
            }
        }
        if let (false, Some(open)) = (bind.is_empty(), self.open.as_ref()) {
            let _ = open.send(());
            self.open = None;
        }
        if let Some(net) = self.net.get() {
            let mesh = net.links.mesh();
            let me = hex(mesh.me().0);
            for (id, eps) in &admit {
                mesh.set_members(id, eps.iter().filter(|e| **e != me).filter_map(|e| unhex(e)).map(PeerId));
            }
            for gone in self.groups_set.iter().filter(|g| !admit.contains_key(*g)) {
                mesh.leave(gone);
            }
            for f in &self.follows {
                let addrs: Vec<std::net::SocketAddr> = f.addrs.iter().filter_map(|a| a.parse().ok()).collect();
                if let Some(p) = unhex(&f.endpoint) {
                    let _ = net.dial.addresses().put(PeerId(p), &addrs, None, ADDR_TTL);
                }
            }
            net.links.keep(keep.iter().filter(|e| **e != me).filter_map(|e| unhex(e)).map(PeerId));
        }
        self.groups_set = admit.keys().cloned().collect();
        self.track(&ids);
        self.write_status();
    }

    fn write_status(&self) {
        let spaces: Vec<Value> = self
            .held
            .iter()
            .map(|(id, h)| {
                let v = h.space.view();
                let members: Vec<Value> = chains::members(&h.space)
                    .iter()
                    .map(|m| json!({ "key": m.key, "endpoint": m.endpoint, "level": m.level.as_str(), "connected": self.peers.iter().any(|(p, k)| hex(p.0) == m.endpoint && k.id() == m.key) }))
                    .collect();
                let curator = self.curator_endpoint(id);
                json!({
                    "id": id,
                    "address": h.address.to_string(),
                    "role": if self.curates(id) { "curator" } else { "member" },
                    "curator": v.curator.id(),
                    "epoch": v.epoch,
                    "members": members,
                    "curatorConnected": curator.as_ref().is_some_and(|(k, e)| self.peers.iter().any(|(p, pk)| hex(p.0) == *e && pk.id() == *k)),
                })
            })
            .collect();
        let follows: Vec<Value> = self.follows.iter().map(|f| json!({ "space": f.space, "curator": f.curator, "endpoint": f.endpoint, "held": self.held.contains_key(&f.space) })).collect();
        if let Ok(mut s) = self.status.lock() {
            let endpoint = s.get("endpoint").cloned().unwrap_or(Value::Null);
            *s = json!({ "key": self.me, "endpointId": self.endpoint, "endpoint": endpoint, "spaces": spaces, "follows": follows });
        }
    }

    // ---------------------------------------------------------------- the folder

    /// Looks at every space's folder: a curator publishes what changed in it, a member sends its
    /// waiting notes and its copy's edits.
    fn scan(&mut self) {
        let ids: Vec<String> = self.held.keys().cloned().collect();
        for id in &ids {
            if self.curates(id) {
                self.publish(id);
            } else {
                self.send_mine(id);
            }
        }
        self.track(&ids);
    }

    /// What kqueue watches (`Watch::track`): each synced space's folders sync reads, and its pages.
    fn track(&mut self, ids: &[String]) {
        let year = xenolith_core::text::local_date(&xenolith_core::text::now()).get(..4).unwrap_or("").to_string();
        let mut paths = vec![];
        for id in ids {
            let synced = self.held.get(id).is_some_and(|h| !chains::members(&h.space).is_empty() || !self.curates(id));
            let Some(dir) = self.dir_of(id).filter(|_| synced) else { continue };
            paths.extend([dir.join("pages"), dir.join("inbox"), dir.join("log").join(&year)]);
        }
        for id in ids {
            let Some(dir) = self.dir_of(id) else { continue };
            if let Ok(rd) = std::fs::read_dir(dir.join("pages")) {
                paths.extend(rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "md")));
            }
        }
        self.watch.track(&paths);
    }

    fn publish(&mut self, id: &str) {
        let Some(dir) = self.dir_of(id) else { return };
        // A space nobody else is in yet has nothing to publish to; it publishes when one comes.
        if chains::members(&self.held[id].space).is_empty() {
            return;
        }
        let epoch = self.held[id].space.view().epoch;
        let ledger = self.ledger(id).clone();
        let p = cs::changes_to_publish(&dir, &ledger);
        let mut fresh = vec![];
        for (slug, text, base) in p.pages {
            let version = hash_text(&text, 16);
            let n = ledger.pages.get(&slug).map_or(0, |x| x.n) + 1;
            self.ledger(id).pages.insert(slug.clone(), PageAt { version: version.clone(), n });
            if text.len() > cs::MAX_TEXT {
                self.log(&format!("{slug} in {id} is too large to send ({} bytes); it stays on this machine", text.len()));
                continue;
            }
            if let Err(e) = cs::put_text(&self.cfg.paths, text.as_bytes()) {
                self.log(&e);
                continue;
            }
            let body = json!({ "slug": slug, "epoch": epoch, "n": n, "base": base, "version": version, "text": cs::text_ref(text.as_bytes()) });
            fresh.extend(self.write(id, Kind::CtxPage, body));
        }
        for (slug, base) in p.removed {
            self.ledger(id).pages.remove(&slug);
            fresh.extend(self.write(id, Kind::CtxRemove, json!({ "slug": slug, "epoch": epoch, "base": base })));
        }
        for (date, text) in p.logs {
            let version = hash_text(&text, 16);
            self.ledger(id).logs.insert(date.clone(), version.clone());
            if cs::put_text(&self.cfg.paths, text.as_bytes()).is_err() || text.len() > cs::MAX_TEXT {
                continue;
            }
            fresh.extend(self.write(id, Kind::CtxLog, json!({ "date": date, "epoch": epoch, "version": version, "text": cs::text_ref(text.as_bytes()) })));
        }
        self.save_ledger(id);
        if !fresh.is_empty() {
            for (peer, key, _) in self.connected_in(id) {
                self.send_entries(id, peer, &key.id(), &fresh);
            }
        }
    }

    /// A member's waiting notes and its copy's edits, to the curator.
    fn send_mine(&mut self, id: &str) {
        let Some(dir) = self.dir_of(id) else { return };
        let curator = self.held[id].space.view().curator.id();
        let mut fresh = vec![];
        for n in cs::waiting_notes(&dir) {
            let (body, text) = cs::note_body(&n);
            if cs::put_text(&self.cfg.paths, text.as_bytes()).is_err() {
                continue;
            }
            let Some(e) = self.write(id, Kind::CtxNote, body) else { continue };
            fresh.push(e);
            if let Err(e) = cs::mark_sent(&dir, &n.id, &format!("key:{curator}")) {
                self.log(&format!("a note sent from {id} stays in its inbox: {e}"));
            }
        }
        let ledger = self.ledger(id).clone();
        for edit in cs::changes_in_copy(&dir, &ledger) {
            fresh.extend(self.propose(id, &edit));
        }
        self.save_ledger(id);
        if !fresh.is_empty() {
            for (peer, key, role) in self.connected_in(id) {
                if role == Role::Curator {
                    self.send_entries(id, peer, &key.id(), &fresh);
                }
            }
        }
    }

    fn propose(&mut self, id: &str, edit: &cs::Edit) -> Option<FeedEntry> {
        if edit.text.as_ref().is_some_and(|t| t.len() > cs::MAX_TEXT || cs::put_text(&self.cfg.paths, t.as_bytes()).is_err()) {
            return None;
        }
        let e = self.write(id, Kind::CtxPropose, edit.body())?;
        self.ledger(id).proposed.insert(edit.slug.clone(), edit.version());
        Some(e)
    }

    /// Sends `entries` of this Xenolith's feed in `id` to `peer`, each with its text when it's
    /// still live (a page's text only for the version the curator has now).
    fn send_entries(&mut self, id: &str, peer: PeerId, to: &str, entries: &[FeedEntry]) {
        let ledger = self.ledger(id).clone();
        for e in entries {
            let live = match e.kind {
                Kind::CtxPage => ledger.pages.get(e.body["slug"].as_str().unwrap_or("")).is_some_and(|p| Some(p.version.as_str()) == e.body["version"].as_str()),
                Kind::CtxLog => ledger.logs.get(e.body["date"].as_str().unwrap_or("")).map(String::as_str) == e.body["version"].as_str(),
                _ => true,
            };
            let text = if live { cs::named_text(&e.body).and_then(|h| cs::text(&self.cfg.paths, &h)) } else { None };
            match Envelope::plain_entry(e, to, text.as_deref()) {
                Ok(env) => self.send(peer, Out::Envelope(env)),
                Err(err) => self.log(&format!("an entry for {id} can't go: {}", envelope_err(&err))),
            }
        }
    }

    // ---------------------------------------------------------------- what arrives

    fn handle(&mut self, job: Job) {
        match job {
            Job::Changed | Job::Opened => {
                self.refresh();
                self.scan();
            }
            Job::Fs | Job::Stop => {}
            Job::OpenNow => {
                if let Some(open) = self.open.take() {
                    let _ = open.send(());
                }
            }
            Job::Admit { pending, key, endpoint, reply } => {
                let r = crate::join::admit_on_worker(&self.cfg, &pending, &key, &endpoint);
                match &r {
                    Ok(_) => {
                        self.log(&format!("{key} joined {} with an invite, at {}", pending.addresses.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "), pending.level.as_str()));
                        self.refresh();
                        self.scan();
                    }
                    Err(e) => self.log(&format!("a join from {key} wasn't let in: {e}")),
                }
                let _ = reply.send(r);
            }
            Job::Forget { text, ignore_case } => self.forget(&text, ignore_case),
            Job::Hello { peer, key, told } => self.hello(peer, key, told),
            Job::GroupHello { peer, group, digest, heads } => self.group_hello(peer, &group, digest, &heads),
            Job::Envelope { peer, env } => self.envelope(peer, &env),
            Job::Want { peer, group, author, from } => {
                let Some(key) = self.peer_key(peer) else { return };
                if author == self.me && self.role(&group, &key.id(), &hex(peer.0)).is_some() {
                    let at = Pos::new(from.inc, from.seq.saturating_sub(1));
                    let entries = self.out(&group).map(|o| o.after(at)).unwrap_or_default();
                    self.send_entries(&group, peer, &key.id(), &entries);
                }
            }
            Job::Group { peer, group, body } => self.group(peer, &group, &body),
            Job::Down(peer) => {
                self.peers.remove(&peer);
                self.write_status();
            }
        }
    }

    fn hello(&mut self, peer: PeerId, key: VerifyingKey, told: Vec<Told>) {
        self.peers.insert(peer, key.clone());
        // A space its curator tells of that this Xenolith follows everything of that curator for,
        // and holds nothing of yet: followed from now on, and its chain asked for (revision 6).
        let mut more = vec![];
        for t in &told {
            if self.held.contains_key(&t.space) || self.follow_for(&t.space).is_some() {
                continue;
            }
            if let Some(f) = self.follows.iter().find(|f| f.all && f.curator == key.id() && f.endpoint == hex(peer.0)) {
                more.push(Follow { space: t.space.clone(), ..f.clone() });
            }
        }
        for f in &more {
            if let Err(e) = cs::follow(&self.cfg.paths, f.clone()) {
                self.log(&e);
            }
        }
        if !more.is_empty() {
            self.refresh();
            for f in more {
                self.send(peer, Out::Control(Control::Group { network: f.space.clone(), body: json!({ "digest": "-" }) }));
            }
        }
        self.write_status();
    }

    fn chain_body(&self, id: &str) -> Value {
        match self.held.get(id) {
            Some(h) => json!({ "digest": chains::digest(&h.space), "chain": h.space.to_json() }),
            None => json!({ "digest": "-" }),
        }
    }

    fn group_hello(&mut self, peer: PeerId, group: &str, digest: Option<String>, heads: &[(String, Pos)]) {
        let Some(key) = self.peer_key(peer) else { return };
        let Some(role) = self.role(group, &key.id(), &hex(peer.0)) else { return };
        let mine = self.held.get(group).map(|h| chains::digest(&h.space));
        if mine != digest || mine.is_none() {
            self.send(peer, Out::Control(Control::Group { network: group.to_string(), body: self.chain_body(group) }));
        }
        // This Xenolith's feed in the space, from where the peer stands: a member's notes go to
        // the curator, the curator's pages to each member.
        let at = heads.iter().find(|(a, _)| *a == self.me).map_or(Pos::ORIGIN, |(_, p)| *p);
        let serve = match role {
            Role::Curator => true,
            Role::Member(level) => level >= Level::Read,
        };
        if serve && self.held.contains_key(group) {
            let entries = self.out(group).map(|o| o.after(at)).unwrap_or_default();
            self.send_entries(group, peer, &key.id(), &entries);
        }
    }

    fn group(&mut self, peer: PeerId, group: &str, body: &Value) {
        let Some(key) = self.peer_key(peer) else { return };
        let Some(role) = self.role(group, &key.id(), &hex(peer.0)) else { return };
        let theirs = body["digest"].as_str().unwrap_or("-").to_string();
        if body["chain"].is_object() {
            if self.held.contains_key(group) {
                match chains::merge(&self.cfg.paths, group, &body["chain"]) {
                    Ok(true) => {
                        self.refresh();
                        // What changed goes on to the others in the space.
                        let others: Vec<PeerId> = self.connected_in(group).into_iter().map(|(p, _, _)| p).filter(|p| *p != peer).collect();
                        for p in others {
                            self.send(p, Out::Control(Control::Group { network: group.to_string(), body: self.chain_body(group) }));
                        }
                    }
                    Ok(false) => {}
                    Err(e) => self.log(&format!("{group}'s chain from {} wasn't taken: {e}", key.id())),
                }
            } else if role == Role::Curator {
                // The chain of a space this Xenolith follows, from its curator: only that space's
                // (its genesis makes its id), and it gets a folder here.
                let ok = Space::from_json(xenonet_log::space::SpacePolicy::default(), &body["chain"]).is_ok_and(|s| s.id() == group);
                if !ok {
                    self.log(&format!("{} sent a chain that isn't {group}'s; it wasn't taken", key.id()));
                    return;
                }
                match chains::receive(&self.cfg.paths, &self.cfg.root, &body["chain"], &self.cfg.keys.context) {
                    Ok(address) => {
                        self.log(&format!("{group} came from {} and is in {}", key.id(), address));
                        self.refresh();
                        // What came before the chain wasn't taken: ask again from the start.
                        self.send(peer, Out::Control(Control::Want { network: group.to_string(), author: key.id(), inc: 0, from: 1 }));
                    }
                    Err(e) => {
                        self.log(&format!("{group} from {} wasn't taken: {e}", key.id()));
                        return;
                    }
                }
            }
        }
        let mine = self.held.get(group).map(|h| chains::digest(&h.space));
        if mine.is_some() && mine.as_deref() != Some(theirs.as_str()) && !body["chain"].is_object() {
            self.send(peer, Out::Control(Control::Group { network: group.to_string(), body: self.chain_body(group) }));
        } else if mine.is_some() && body["chain"].is_object() && mine.as_deref() != Some(theirs.as_str()) {
            // It holds what the other side doesn't: once, back.
            let after = self.held.get(group).map(|h| chains::digest(&h.space));
            if after.as_deref() != Some(theirs.as_str()) {
                self.send(peer, Out::Control(Control::Group { network: group.to_string(), body: self.chain_body(group) }));
            }
        }
    }

    fn envelope(&mut self, peer: PeerId, env: &Envelope) {
        let Some(key) = self.peer_key(peer) else { return };
        let group = env.network.clone();
        let Some(role) = self.role(&group, &key.id(), &hex(peer.0)) else { return };
        if !self.held.contains_key(&group) {
            return;
        }
        // Only an author's own entries, from its own connection: holders come later (commit 15).
        if env.author != key.id() {
            return;
        }
        let (v, attach) = match env.open_plain_by(&self.me, &key) {
            Ok(x) => x,
            Err(e) => return self.log(&format!("an entry from {} was dropped: {}", key.id(), e.message)),
        };
        let kind = v.entry().kind;
        let allowed = match role {
            Role::Curator => matches!(kind, Kind::CtxPage | Kind::CtxRemove | Kind::CtxLog | Kind::CtxForget | Kind::CtxSnapshot),
            Role::Member(_) => matches!(kind, Kind::CtxNote | Kind::CtxPropose),
        };
        if !allowed {
            return self.log(&format!("a {} from {} in {group} was refused: not its to send", kind.as_str(), key.id()));
        }
        if let Some(a) = &attach {
            if cs::named_text(&v.entry().body) != Some(cs::text_id(a)) {
                return self.log(&format!("an entry from {} came with another text than it names; dropped", key.id()));
            }
            if let Err(e) = cs::put_text(&self.cfg.paths, a) {
                return self.log(&e);
            }
        }
        if role == Role::Curator {
            let epoch = v.entry().body["epoch"].as_u64().unwrap_or(0);
            if epoch > u64::from(self.held[&group].space.view().epoch) {
                // From an epoch this Xenolith's chain hasn't reached: not taken yet, so it comes
                // again once the chain has.
                return;
            }
        }
        let mut feeds = self.feeds.remove(&group).unwrap_or_else(|| Feeds::load(&self.cfg.paths, &group));
        let offered = Feed::at(&mut feeds, now()).offer(v);
        match offered {
            Ok(Offer::New(applied)) => {
                if let Err(e) = feeds.save() {
                    self.log(&e);
                }
                // The next hello says how far this Xenolith is now, so nothing comes twice.
                if let Ok(mut s) = self.shared.snap.write()
                    && let Some(g) = s.groups.get_mut(&group)
                {
                    g.1 = feeds.heads();
                }
                self.feeds.insert(group.clone(), feeds);
                for e in applied.entries {
                    self.apply(&group, role, &key, &e);
                }
                self.save_ledger(&group);
            }
            Ok(Offer::Gap(w)) => {
                let _ = feeds.save();
                self.feeds.insert(group.clone(), feeds);
                self.send(peer, Out::Control(Control::Want { network: group, author: w.author, inc: w.inc, from: w.from }));
            }
            Ok(Offer::Fork(f) | Offer::Frozen(f)) => {
                let _ = feeds.save();
                self.feeds.insert(group.clone(), feeds);
                self.log(&format!("{}'s feed in {group} forked at {}:{}; nothing more is taken from it", key.id(), f.at.inc, f.at.seq));
            }
            Ok(_) => {
                self.feeds.insert(group, feeds);
            }
            Err(e) => {
                self.feeds.insert(group, feeds);
                self.log(&e);
            }
        }
    }

    fn apply(&mut self, id: &str, role: Role, key: &VerifyingKey, e: &FeedEntry) {
        let Some(dir) = self.dir_of(id) else { return };
        let text = cs::named_text(&e.body).and_then(|h| cs::text(&self.cfg.paths, &h)).map(|t| String::from_utf8_lossy(&t).to_string());
        match role {
            Role::Member(level) => {
                let who = format!("key:{}", key.id());
                let r = cs::take(&dir, e.kind, &e.body, text.as_deref().unwrap_or(""), &who, level, &e.id());
                let what = match &r {
                    Taken::Note(n) => format!("a note from {who} is in {id}'s inbox ({n})"),
                    Taken::Held { batch } => format!("a change from {who} to {id} against an older version waits for your OK ({batch})"),
                    Taken::Removed => format!("{who} removed {} from {id}", e.body["slug"].as_str().unwrap_or("")),
                    Taken::Duplicate => return,
                    Taken::Refused(why) => format!("a {} from {who} wasn't taken: {why}", e.kind.as_str()),
                };
                self.log(&what);
            }
            Role::Curator => self.materialise(id, &dir, e, text.as_deref()),
        }
    }

    /// The curator's entry in a member's copy. A copy's edit not proposed yet goes first, so the
    /// curator's version never drops it (revision 34).
    fn materialise(&mut self, id: &str, dir: &std::path::Path, e: &FeedEntry, text: Option<&str>) {
        let epoch = u32::try_from(e.body["epoch"].as_u64().unwrap_or(0)).unwrap_or(0);
        if epoch < self.held[id].space.view().epoch {
            return;
        }
        let slug = e.body["slug"].as_str().unwrap_or("").to_string();
        if matches!(e.kind, Kind::CtxPage | Kind::CtxRemove) {
            let ledger = self.ledger(id).clone();
            let edit = cs::changes_in_copy(dir, &ledger).into_iter().find(|x| x.slug == slug);
            if let Some(p) = edit.and_then(|edit| self.propose(id, &edit)) {
                let curator: Vec<(PeerId, VerifyingKey, Role)> = self.connected_in(id).into_iter().filter(|(_, _, r)| *r == Role::Curator).collect();
                for (peer, k, _) in curator {
                    self.send_entries(id, peer, &k.id(), std::slice::from_ref(&p));
                }
            }
        }
        let r = match e.kind {
            Kind::CtxPage => {
                let version = e.body["version"].as_str().unwrap_or("").to_string();
                match text.filter(|t| hash_text(t, 16) == version) {
                    Some(t) => cs::put_page(dir, &slug, t).map(|()| {
                        let n = e.body["n"].as_u64().unwrap_or(0);
                        let l = self.ledger(id);
                        l.pages.insert(slug.clone(), PageAt { version, n });
                        l.proposed.remove(&slug);
                    }),
                    // A version since replaced, or its text forgotten: nothing to write.
                    None => Ok(()),
                }
            }
            Kind::CtxRemove => cs::remove_page_here(dir, &slug).map(|_| {
                let l = self.ledger(id);
                l.pages.remove(&slug);
                l.proposed.remove(&slug);
            }),
            Kind::CtxLog => {
                let date = e.body["date"].as_str().unwrap_or("").to_string();
                match text {
                    Some(t) => cs::merge_log(dir, &date, t).map(|()| {
                        self.ledger(id).logs.insert(date, hash_text(t, 16));
                    }),
                    None => Ok(()),
                }
            }
            Kind::CtxForget => {
                let ids: Vec<String> = e.body["hashes"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect();
                let n = cs::drop_texts(&self.cfg.paths, &ids);
                self.log(&format!("{id}'s curator asked for {} texts to be forgotten; {n} were held here and are gone", ids.len()));
                Ok(())
            }
            _ => Ok(()),
        };
        self.ledger(id).epoch = epoch;
        if let Err(err) = r {
            self.log(&format!("a {} for {id} wasn't written into the copy: {err}", e.kind.as_str()));
        }
    }

    fn forget(&mut self, text: &str, ignore_case: bool) {
        let ids = cs::texts_with(&self.cfg.paths, text, ignore_case);
        if ids.is_empty() {
            return;
        }
        let n = cs::drop_texts(&self.cfg.paths, &ids);
        self.log(&format!("forgot {n} texts sync kept"));
        let curated: Vec<String> = self.held.keys().filter(|id| self.curates(id)).cloned().collect();
        for id in curated {
            let epoch = self.held[&id].space.view().epoch;
            if let Some(e) = self.write(&id, Kind::CtxForget, json!({ "epoch": epoch, "hashes": ids })) {
                for (peer, key, _) in self.connected_in(&id) {
                    self.send_entries(&id, peer, &key.id(), std::slice::from_ref(&e));
                }
            }
        }
    }
}
