//! Xenolith between machines, on XenoNet's transport library (xenonet-net): its own iroh endpoint
//! and `xenolith/sync/2`, link/2's engine with Xenolith's own admission, hello and groups (spaces).
//! It runs in xenolith-service alone, so the hook, the stdio server and the tray (xenolith-core
//! only) never start or link it.
//!
//! - **The endpoint** is keyed by this Xenolith's transport key, bound the way the person set it
//!   (`sync` in config.json; xenolith_core::sync::Settings): no relay and no mDNS unless named, and
//!   never n0's (review revision 19). It opens only once a space is shared with another Xenolith or
//!   followed from one (revision 18: no endpoint without a peer).
//! - **Admission.** Each space's members are its chain's key subjects, each with the endpoint id
//!   its share names; a member admits its curator, from the chain or, before the chain is here,
//!   from its follow record. A connection from any other endpoint is refused before anything is
//!   read (revision 15), and a hello whose context key isn't the one bound to its endpoint ends the
//!   connection (`hello`).
//! - **Threads.** One thread runs the endpoint on a tokio runtime of its own, and never blocks:
//!   everything that reads or writes a file goes to the worker thread (`engine`) through a channel
//!   (revision 36), and the worker sends what goes back through the links, which take it from any
//!   thread. A third thread watches the spaces' folders for changes (notify), so a curator's pages
//!   and a member's notes and edits go out as they happen, not once a minute.
//! - Nothing here runs until the upgrade from 1.x has run (xenolith_core::upgrade): the service
//!   starts it only then.

mod engine;
pub mod hello;
pub mod join;

use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use xenolith_core::keys::Keys;
use xenolith_core::paths::AppPaths;
use xenolith_core::sync::Settings;
use xenonet_net::admission::{Admission, Doors};
use xenonet_net::dial::{Alpn, Close, Link, Mesh};
use xenonet_net::endpoint::{IrohDial, LanLookup, Relay, Relays};
use xenonet_net::feed::Group;
use xenonet_net::join::{Answer, Door, Invite, Served};
use xenonet_net::link2::{Links, Protocol};

/// sync/2, shaped like link/2: spaces' feeds and their chains.
pub fn protocol() -> Protocol {
    Protocol { alpn: Alpn::named(xenolith_core::sync::ALPN), group: Group::Space, name: "sync/2" }
}

/// What the sync needs from the service.
pub struct Config {
    pub paths: AppPaths,
    /// The shared context folder.
    pub root: PathBuf,
    pub keys: Arc<Keys>,
    pub settings: Settings,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

/// The endpoint, once it's open.
pub(crate) struct Net {
    pub links: Arc<Links>,
    pub dial: Arc<IrohDial>,
    /// The join/1 tickets: join/1 is answered while one is open.
    pub door: Arc<Door>,
    /// Wakes the task that closes tickets past their time.
    pub expiry: Arc<tokio::sync::Notify>,
    /// The endpoint's runtime, for a join started from another thread.
    pub rt: tokio::runtime::Handle,
}

/// How long an address a join was given is kept in the endpoint's lookup.
const ADDR_TTL: std::time::Duration = std::time::Duration::from_secs(365 * 24 * 3600);

/// The running sync: a handle the service keeps. Dropping it stops the worker and closes the
/// endpoint, and waits for both.
pub struct Engine {
    cfg: Arc<Config>,
    pendings: join::Pendings,
    jobs: mpsc::Sender<engine::Job>,
    net: Arc<OnceLock<Net>>,
    status: Arc<Mutex<Value>>,
    stop: Arc<tokio::sync::Notify>,
    threads: Vec<std::thread::JoinHandle<()>>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.jobs.send(engine::Job::Stop);
        self.stop.notify_one();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Engine {
    /// Starts the worker and the watcher, and the endpoint once there's a peer.
    pub fn start(cfg: Config) -> Result<Engine, String> {
        let (tx, rx) = mpsc::channel();
        let net: Arc<OnceLock<Net>> = Arc::new(OnceLock::new());
        let status = Arc::new(Mutex::new(json!({ "endpoint": null })));
        let cfg = Arc::new(cfg);
        let (open_tx, open_rx) = mpsc::channel::<()>();
        let shared = Arc::new(engine::Shared::default());
        let stop = Arc::new(tokio::sync::Notify::new());
        let pendings = join::Pendings::default();
        let worker = engine::spawn(cfg.clone(), tx.clone(), rx, net.clone(), status.clone(), open_tx, shared.clone())?;
        let endpoint = spawn_endpoint(cfg.clone(), tx.clone(), net.clone(), open_rx, shared, stop.clone(), pendings.clone())?;
        Ok(Engine { cfg, pendings, jobs: tx, net, status, stop, threads: vec![worker, endpoint] })
    }

    /// The endpoint, opened now if it wasn't (an invite or a join needs it with no peer yet).
    fn net_now(&self) -> Result<&Net, String> {
        if self.cfg.settings.off {
            return Err("this Xenolith's endpoint is off (config.json's sync.endpoint): turn it on to invite or join".into());
        }
        let _ = self.jobs.send(engine::Job::OpenNow);
        let t = std::time::Instant::now();
        loop {
            if let Some(n) = self.net.get() {
                return Ok(n);
            }
            if t.elapsed() > std::time::Duration::from_secs(15) {
                return Err("the endpoint didn't open (the service's log says why)".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    /// An invite to `addresses` (spaces this Xenolith curates), for one Xenolith to join at
    /// `level`, for `ttl_ms` (ten minutes at most). `owner_key` is the owner key the request
    /// presented: the Shares are signed with it once the joiner's code checks. Returns the invite,
    /// the code (shown once, never kept), where this endpoint is reached and what it offers.
    pub fn invite(&self, addresses: &[xenolith_core::spaces::SpaceId], level: xenolith_core::chains::Level, owner_key: &str, ttl_ms: i64) -> Result<Value, String> {
        if level < xenolith_core::chains::Level::Read || level > xenolith_core::chains::Level::Write {
            return Err("an invite shares at read, append or write".into());
        }
        let (offer, _) = join::offer_of(&self.cfg, addresses)?;
        let real = xenolith_core::owner::read(&self.cfg.paths).ok_or("inviting needs the owner key")?;
        self.cfg.keys.owner.signer(&real, owner_key)?;
        let net = self.net_now()?;
        let code = join::make_code()?;
        let now = xenolith_core::text::now_ms();
        let expires = now + ttl_ms.clamp(1000, join::TICKET_MS);
        let digest = offer.digest();
        let label = format!("invite-{}", xenolith_core::text::random_hex(4));
        let id = net.door.open(&digest, &label, false, &code, expires).map_err(|e| e.message)?;
        if let Ok(mut p) = self.pendings.lock() {
            p.insert(id, join::Pending { offer: offer.clone(), addresses: addresses.to_vec(), level, owner_key: owner_key.to_string() });
        }
        net.expiry.notify_one();
        let invite = Invite { network: digest, inviter: net.links.mesh().me(), relay: self.cfg.settings.relays.first().cloned(), ticket: id };
        let text = invite.encode_for(&join::TARGET).ok_or("that offer can't go in an invite")?;
        let at: Vec<String> = net.dial.direct_addrs().iter().map(ToString::to_string).collect();
        (self.cfg.log)(&format!("sync: an invite to {} is open until {expires}", addresses.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")));
        Ok(json!({ "invite": text, "code": join::display(&code), "at": at, "expiresAt": expires, "level": level.as_str(), "offer": offer.to_json() }))
    }

    /// Joins with `invite` and `code`: dials the inviter the invite names (at `choice.at`, or by
    /// its relay), proves the code, checks the answer and follows what `choice` names
    /// (`join::accept`).
    pub fn join(&self, invite: &str, code: &str, choice: &join::Choice) -> Result<Value, String> {
        let inv = Invite::parse_for(&join::TARGET, invite).map_err(|e| e.message)?;
        let code = join::normalize(code).ok_or(join::TARGET.code_shape)?;
        let net = self.net_now()?;
        let me = net.links.mesh().me();
        if inv.inviter == me {
            return Err("that invite is this Xenolith's own".into());
        }
        net.dial.addresses().put(inv.inviter, &choice.at, inv.relay.as_deref(), ADDR_TTL).map_err(|e| e.message)?;
        let endpoint = hex(me.0);
        let add = join::add_for(&self.cfg, &endpoint);
        let (tx, rx) = mpsc::channel();
        let (mesh, asked) = (net.links.mesh().clone(), inv.clone());
        net.rt.spawn(async move {
            let r = tokio::time::timeout(std::time::Duration::from_secs(45), xenonet_net::join::ask_to(&join::TARGET, &mesh, &asked, &code, &add)).await;
            let _ = tx.send(r.unwrap_or_else(|_| Err(xenonet_net::NetError::new(504, "unreachable", "the inviter didn't answer: check the address it's at"))));
        });
        let answer = rx.recv_timeout(std::time::Duration::from_secs(60)).map_err(|_| "the join didn't finish".to_string())?;
        match answer {
            Ok(Answer::Joined(v)) => {
                let out = join::accept(&self.cfg, &inv, &endpoint, &v, choice)?;
                (self.cfg.log)(&format!("sync: joined {} spaces from {}", out["followed"].as_array().map_or(0, Vec::len), join::inviter_of(&inv)));
                self.changed();
                Ok(out)
            }
            Ok(Answer::Waiting(why)) => Err(why),
            // With no invite open, the inviter doesn't offer join/1 at all.
            Err(e) if e.code == "unreachable" && e.message.contains("protocol") => Err("the inviter has no invite open: that one was used or retired, or it's past its ten minutes (no-ticket)".into()),
            Err(e) => Err(format!("{} ({})", e.message, e.code)),
        }
    }

    /// A share or a follow changed what this Xenolith syncs: look again.
    pub fn changed(&self) {
        let _ = self.jobs.send(engine::Job::Changed);
    }

    /// A forget ran: drop the texts that hold `text` from what sync keeps, and tell the members of
    /// the spaces this Xenolith curates to drop them too (`ctx.forget`).
    pub fn forget(&self, text: &str, ignore_case: bool) {
        let _ = self.jobs.send(engine::Job::Forget { text: text.to_string(), ignore_case });
    }

    /// What /status shows of the sync.
    pub fn status(&self) -> Value {
        let mut s = self.status.lock().map(|s| s.clone()).unwrap_or(Value::Null);
        if let Some(n) = self.net.get() {
            s["endpoint"] = json!({
                "id": hex(n.links.mesh().me().0),
                "addrs": n.dial.direct_addrs().iter().map(ToString::to_string).collect::<Vec<_>>(),
                "peers": n.links.peers().iter().map(|(p, st)| json!({ "endpoint": hex(p.0), "path": st.path, "spaces": st.networks })).collect::<Vec<_>>(),
            });
        }
        s
    }
}

pub(crate) fn hex(b: [u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub(crate) fn unhex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, c) in out.iter_mut().enumerate() {
        *c = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// The endpoint's thread: waits for the first peer, then binds and serves until stopped.
fn spawn_endpoint(
    cfg: Arc<Config>,
    jobs: mpsc::Sender<engine::Job>,
    net: Arc<OnceLock<Net>>,
    open: mpsc::Receiver<()>,
    shared: Arc<engine::Shared>,
    stop: Arc<tokio::sync::Notify>,
    pendings: join::Pendings,
) -> Result<std::thread::JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("xenolith-sync-endpoint".into())
        .spawn(move || {
            if open.recv().is_err() || cfg.settings.off {
                return;
            }
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return (cfg.log)(&format!("sync: no runtime for the endpoint: {e}")),
            };
            rt.block_on(async move {
                let key = match engine::host_key(&cfg.keys) {
                    Ok(k) => k,
                    Err(e) => return (cfg.log)(&format!("sync: {e}")),
                };
                let s = &cfg.settings;
                let relays = if s.relays.is_empty() { Relays::None } else { Relays::Listed(s.relays.iter().map(|u| Relay { url: u.clone(), auth_token: None }).collect()) };
                let bind = s.bind.as_deref().and_then(|b| b.parse().ok()).unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], xenolith_core::sync::DEFAULT_PORT)));
                let settings = xenonet_net::endpoint::Settings { relays, lan: if s.mdns { LanLookup::On } else { LanLookup::Off }, bind: Some(bind), roots: vec![] };
                // join/1 is answered only while an invite is open (the door's tickets).
                let admission = Arc::new(Admission::with(Doors { members: vec![protocol().alpn], join: Some(join::TARGET.alpn) }));
                let dial = match IrohDial::bind(&key, &settings, admission.clone()).await {
                    Ok(d) => Arc::new(d),
                    Err(e) => return (cfg.log)(&format!("sync: the endpoint didn't open on {bind}: {}", e.message)),
                };
                let mesh = Arc::new(Mesh::new(dial.clone(), admission));
                let machine = engine::machine(&cfg, jobs.clone(), mesh.me().0, shared);
                let door = Arc::new(Door::to(join::TARGET, mesh.clone()));
                let links = Links::over(protocol(), mesh, machine);
                let inviting = Arc::new(join::Inviting { pendings: pendings.clone(), jobs: std::sync::Mutex::new(jobs.clone()) });
                links.hand_others(serve_joins(door.clone(), inviting, cfg.log.clone()));
                let expiry = Arc::new(tokio::sync::Notify::new());
                let expire = tokio::spawn(close_expired(door.clone(), pendings, expiry.clone(), cfg.log.clone()));
                (cfg.log)(&format!("sync: the endpoint {} is open on {bind}", hex(links.mesh().me().0)));
                let _ = net.set(Net { links: links.clone(), dial, door, expiry, rt: tokio::runtime::Handle::current() });
                let _ = jobs.send(engine::Job::Opened);
                let keep = tokio::spawn(links.clone().keep_up());
                tokio::select! {
                    () = links.clone().accept() => {}
                    () = stop.notified() => {}
                }
                keep.abort();
                expire.abort();
                links.mesh().close().await;
            });
        })
        .map_err(|e| format!("sync: no thread for the endpoint: {e}"))
}

/// What the endpoint does with a join/1 connection: one attempt, answered by the door.
fn serve_joins(door: Arc<Door>, inviting: Arc<join::Inviting>, log: Arc<dyn Fn(&str) + Send + Sync>) -> xenonet_net::link2::Others {
    Arc::new(move |link: Arc<Link>| {
        if link.alpn() != join::TARGET.alpn {
            link.close(Close::Done);
            return;
        }
        let (door, inviting, log) = (door.clone(), inviting.clone(), log.clone());
        tokio::spawn(async move {
            let peer = hex(link.peer().0);
            let served = door.serve(&link, inviting.as_ref(), xenolith_core::text::now_ms()).await;
            // The joiner closes the connection once it has read the answer: closing it here first
            // could drop the answer still in flight.
            let _ = tokio::time::timeout(xenonet_net::join::STEP, link.closed()).await;
            match served {
                Ok(Served::Joined { .. }) => log(&format!("sync: {peer} joined with an invite")),
                Ok(Served::Refused(why)) => log(&format!("sync: a join from {peer} was refused: {why}")),
                Ok(Served::Waiting { .. }) => {}
                Err(e) => log(&format!("sync: a join from {peer} ended: {}", e.message)),
            }
            link.close(Close::Done);
        });
    })
}

/// Closes each invite once it's past its ten minutes, and forgets what it offered.
async fn close_expired(door: Arc<Door>, pendings: join::Pendings, wake: Arc<tokio::sync::Notify>, log: Arc<dyn Fn(&str) + Send + Sync>) {
    loop {
        let now = xenolith_core::text::now_ms();
        for _ in door.expire(now) {
            log("sync: an invite ran out unused");
        }
        let open: std::collections::BTreeSet<_> = door.open_tickets().iter().map(|t| t.id).collect();
        if let Ok(mut p) = pendings.lock() {
            p.retain(|id, _| open.contains(id));
        }
        match door.next_expiry() {
            Some(at) => {
                let wait = std::time::Duration::from_millis(u64::try_from(at - now).unwrap_or(0).max(1));
                tokio::select! {
                    () = tokio::time::sleep(wait) => {}
                    () = wake.notified() => {}
                }
            }
            None => wake.notified().await,
        }
    }
}
