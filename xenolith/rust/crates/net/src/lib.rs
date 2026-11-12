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

use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use xenolith_core::keys::Keys;
use xenolith_core::paths::AppPaths;
use xenolith_core::sync::Settings;
use xenonet_net::admission::{Admission, Doors};
use xenonet_net::dial::{Alpn, Mesh};
use xenonet_net::endpoint::{IrohDial, LanLookup, Relay, Relays};
use xenonet_net::feed::Group;
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
}

/// The running sync: a handle the service keeps. Dropping it stops the worker and closes the
/// endpoint, and waits for both.
pub struct Engine {
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
        let worker = engine::spawn(cfg.clone(), tx.clone(), rx, net.clone(), status.clone(), open_tx, shared.clone())?;
        let endpoint = spawn_endpoint(cfg, tx.clone(), net.clone(), open_rx, shared, stop.clone())?;
        Ok(Engine { jobs: tx, net, status, stop, threads: vec![worker, endpoint] })
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
                let admission = Arc::new(Admission::with(Doors { members: vec![protocol().alpn], join: None }));
                let dial = match IrohDial::bind(&key, &settings, admission.clone()).await {
                    Ok(d) => Arc::new(d),
                    Err(e) => return (cfg.log)(&format!("sync: the endpoint didn't open on {bind}: {}", e.message)),
                };
                let mesh = Arc::new(Mesh::new(dial.clone(), admission));
                let machine = engine::machine(&cfg, jobs.clone(), mesh.me().0, shared);
                let links = Links::over(protocol(), mesh, machine);
                (cfg.log)(&format!("sync: the endpoint {} is open on {bind}", hex(links.mesh().me().0)));
                let _ = net.set(Net { links: links.clone(), dial });
                let _ = jobs.send(engine::Job::Opened);
                let keep = tokio::spawn(links.clone().keep_up());
                tokio::select! {
                    () = links.clone().accept() => {}
                    () = stop.notified() => {}
                }
                keep.abort();
                links.mesh().close().await;
            });
        })
        .map_err(|e| format!("sync: no thread for the endpoint: {e}"))
}
