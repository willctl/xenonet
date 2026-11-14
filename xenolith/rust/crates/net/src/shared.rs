//! Shared mode: sync/2 over the XenoNet on this machine (phase 6, commit 16; review revisions 16,
//! 17d, 18, 22). XenoNet carries sync/2 for this Xenolith over its own endpoint (`forward(alpn)`,
//! xenonet-net's `forward`), so the members of the networks this machine is in reach it with no
//! pairing and no endpoint of its own.
//!
//! - **Per peer, not once at start** (revision 18). This runs beside the own endpoint, not instead
//!   of it: standalone peers (shares to a key) go over the own endpoint, network members over the
//!   forward. It watches XenoNet's runtime folder and the grant's file (a push watch), and attaches
//!   when the socket and a grant are there, again whenever XenoNet restarts.
//! - **The socket's server** (revision 17d) must be this account's: the socket file is this
//!   account's and no one else may use it, and the process that answers on it runs as this
//!   account (`getpeereid`). Only then does the grant go to it. On Windows XenoNet's socket is a
//!   named pipe, not built yet, so shared mode is off there.
//! - **What XenoNet says** of a connection is taken for its remote endpoint id alone. Who that is
//!   comes from the chains XenoNet hands out, verified here from genesis (xenolith_core::xenonet),
//!   and from the peer's signed hello and grant (`engine::Side::hello_ext_in`).
//! - Like the own endpoint's, this thread never waits on a file once it's attached: what reads or
//!   writes one is the worker's, through a channel.

// Most of this is for Unix sockets: on Windows shared mode only says it isn't built.
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]

use crate::engine::{self, Job};
use crate::{Config, protocol};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use xenonet_net::admission::{Admission, Doors};
use xenonet_net::dial::{Dial as _, Mesh};
use xenonet_net::forward::ForwardDial;
use xenonet_net::link2::Links;

/// XenoNet's own port, whose socket shared mode looks for when config.json names none.
pub const XENONET_PORT: u16 = 47840;

/// The forward, while attached.
pub(crate) struct Fwd {
    pub links: Arc<Links>,
    /// The session, for what this Xenolith says to XenoNet's window (`say`).
    pub dial: Arc<ForwardDial>,
}

/// Where XenoNet's forward socket is: config.json's `sync.xenonet.socket`, else
/// `<XenoNet's runtime folder>/forward-47840.sock` (XENONET_RUNTIME_DIR, else XenoNet's state
/// folder's `runtime`).
pub fn socket_path(cfg: &Config) -> PathBuf {
    if let Some(s) = &cfg.settings.xenonet_socket {
        return PathBuf::from(s);
    }
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let runtime = env("XENONET_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| {
        let home = xenonet_os::home_dir(&|k| env(k));
        let state =
            env("XENONET_STATE_DIR").unwrap_or_else(|| xenonet_os::dirs::app_dir("state", env("XDG_STATE_HOME").as_deref(), env("LOCALAPPDATA").as_deref(), xenonet_os::platform(), &home, "XenoNet"));
        Path::new(&state).join("runtime")
    });
    runtime.join(format!("forward-{XENONET_PORT}.sock"))
}

/// The socket, connected, once it shows it's this account's XenoNet's.
#[cfg(unix)]
fn connect_checked(path: PathBuf) -> xenonet_net::forward::Connect {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use xenonet_net::NetError;
    Arc::new(move || {
        let path = path.clone();
        Box::pin(async move {
            let me = xenonet_os::paths::own_uid();
            let m = std::fs::symlink_metadata(&path).map_err(|e| NetError::new(503, "unreachable", format!("XenoNet's forward socket: {e}")))?;
            if !m.file_type().is_socket() || m.uid() != me || m.mode() & 0o077 != 0 {
                return Err(NetError::new(403, "not-xenonet", format!("{} isn't a socket only this account may use: no grant goes to it", path.display())));
            }
            let s = tokio::net::UnixStream::connect(&path).await.map_err(|e| NetError::new(503, "unreachable", format!("XenoNet's forward socket: {e}")))?;
            match s.peer_cred() {
                Ok(c) if c.uid() == me => {}
                _ => return Err(NetError::new(403, "not-xenonet", "the process on XenoNet's forward socket runs as another account: no grant goes to it")),
            }
            let (r, w) = s.into_split();
            Ok((Box::new(r) as xenonet_net::dial::Reader, Box::new(w) as xenonet_net::dial::Writer))
        })
    })
}

/// Watches the folders the socket and the grant are in: any change there wakes `wake`.
fn watch(dirs: &[PathBuf], wake: tokio::sync::mpsc::UnboundedSender<()>) -> Vec<Box<dyn notify::Watcher + Send>> {
    use notify::Watcher as _;
    let mut out: Vec<Box<dyn notify::Watcher + Send>> = vec![];
    let tell = move |ev: notify::Result<notify::Event>| {
        if ev.is_ok() {
            let _ = wake.send(());
        }
    };
    if let Ok(mut w) = notify::recommended_watcher(tell) {
        for d in dirs.iter().filter(|d| d.is_dir()) {
            let _ = w.watch(d, notify::RecursiveMode::NonRecursive);
        }
        out.push(Box::new(w));
    }
    out
}

/// The thread that attaches to XenoNet's forward and runs sync/2 over it, until stopped.
pub(crate) fn spawn(
    cfg: Arc<Config>,
    jobs: mpsc::Sender<Job>,
    fwd: Arc<RwLock<Option<Fwd>>>,
    shared: Arc<engine::Shared>,
    stop: Arc<tokio::sync::Notify>,
) -> Result<std::thread::JoinHandle<()>, String> {
    std::thread::Builder::new()
        .name("xenolith-sync-xenonet".into())
        .spawn(move || {
            if cfg.settings.xenonet_off {
                shared.set_xenonet(json!({ "mode": "off" }));
                return;
            }
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => return (cfg.log)(&format!("sync: no runtime for shared mode: {e}")),
            };
            rt.block_on(run(cfg, jobs, fwd, shared, stop));
        })
        .map_err(|e| format!("sync: no thread for shared mode: {e}"))
}

#[cfg(not(unix))]
async fn run(_cfg: Arc<Config>, _jobs: mpsc::Sender<Job>, _fwd: Arc<RwLock<Option<Fwd>>>, shared: Arc<engine::Shared>, _stop: Arc<tokio::sync::Notify>) {
    shared.set_xenonet(json!({ "mode": "unbuilt", "why": "shared mode over XenoNet's named pipe isn't built for Windows yet" }));
}

#[cfg(unix)]
async fn run(cfg: Arc<Config>, jobs: mpsc::Sender<Job>, fwd: Arc<RwLock<Option<Fwd>>>, shared: Arc<engine::Shared>, stop: Arc<tokio::sync::Notify>) {
    let path = socket_path(&cfg);
    let (wake_tx, mut wake) = tokio::sync::mpsc::unbounded_channel();
    let keys = xenolith_core::keys::dir(&cfg.paths);
    let _ = std::fs::create_dir_all(&keys);
    let dirs: Vec<PathBuf> = [path.parent().map(Path::to_path_buf), Some(keys)].into_iter().flatten().collect();
    let mut whole = dirs.iter().all(|d| d.is_dir());
    // Held so the watches last: dropping one ends it.
    let mut _watchers = watch(&dirs, wake_tx.clone());
    let mut said = String::new();
    // A refused attach (XenoNet's network endpoint not up yet) is tried again sooner than a
    // minute, a second at first and twice as long each time.
    let mut wait = Duration::from_secs(1);
    loop {
        // A folder that wasn't there at first (XenoNet not installed yet) is watched once it is.
        if !whole && dirs.iter().all(|d| d.is_dir()) {
            _watchers = watch(&dirs, wake_tx.clone());
            whole = true;
        }
        // Ok: the engine stopped while attached.
        let ended = match attach(&cfg, &path, &jobs, &fwd, &shared, &stop, &mut wake).await {
            Ok(()) => return,
            Err(e) => e,
        };
        if ended.attached {
            wait = Duration::from_secs(1);
        }
        let why = ended.why;
        if why != said {
            (cfg.log)(&format!("sync: shared mode waits: {why}"));
        }
        said = why.clone();
        shared.set_xenonet(json!({ "mode": "waiting", "socket": path.to_string_lossy(), "why": why }));
        if ended.again {
            continue;
        }
        // A change in either folder, or the wait, which is a minute at most in case a watch
        // missed one.
        tokio::select! {
            _ = wake.recv() => {}
            () = tokio::time::sleep(wait) => {}
            () = stop.notified() => return,
        }
        wait = (wait * 2).min(Duration::from_secs(60));
        tokio::time::sleep(Duration::from_millis(100)).await;
        while wake.try_recv().is_ok() {}
    }
}

/// Why an attachment ended or didn't start, whether it had attached, and whether to try again at
/// once (the grant changed).
#[cfg(unix)]
struct Ended {
    why: String,
    attached: bool,
    again: bool,
}

#[cfg(unix)]
fn waits(why: impl Into<String>) -> Ended {
    Ended { why: why.into(), attached: false, again: false }
}

/// One attachment: from the grant to the end of the session. Ok once the engine stops; why it
/// ended or didn't start otherwise.
#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
async fn attach(
    cfg: &Arc<Config>,
    path: &Path,
    jobs: &mpsc::Sender<Job>,
    fwd: &Arc<RwLock<Option<Fwd>>>,
    shared: &Arc<engine::Shared>,
    stop: &Arc<tokio::sync::Notify>,
    wake: &mut tokio::sync::mpsc::UnboundedReceiver<()>,
) -> Result<(), Ended> {
    if !path.exists() {
        return Err(waits(format!("XenoNet's forward socket isn't there ({}): is XenoNet running?", path.display())));
    }
    // No grant yet: collected over the socket, once you approve it at XenoNet's window.
    let grant = match xenolith_core::xenonet::read_grant(&cfg.paths) {
        Some(g) => g,
        None => match collect(cfg, path, shared, stop, Some(&mut *wake)).await? {
            Some(g) => g,
            None => return Ok(()),
        },
    };
    let dial = match ForwardDial::attach(connect_checked(path.to_path_buf()), &grant, protocol().alpn).await {
        Ok(d) => d,
        // The grant kept here no longer stands (renewed elsewhere, revoked, past its end): a new
        // one is collected for this context key, which asks you again only if your approval went.
        Err(e) if matches!(e.code, "no-grant" | "revoked" | "expired" | "other-alpn") => {
            let Some(g) = collect(cfg, path, shared, stop, Some(&mut *wake)).await? else { return Ok(()) };
            ForwardDial::attach(connect_checked(path.to_path_buf()), &g, protocol().alpn).await.map_err(|e| waits(format!("XenoNet refused the attach: {} ({})", e.message, e.code)))?
        }
        Err(e) => return Err(waits(format!("XenoNet refused the attach: {} ({})", e.message, e.code))),
    };
    let grant = xenolith_core::xenonet::read_grant(&cfg.paths).unwrap_or(grant);
    let dial = Arc::new(dial);
    // Near its end, the grant is renewed in the background: the new file wakes the loop below,
    // which attaches with it.
    if dial.until() - xenolith_core::text::now_ms() < RENEW_MS {
        let (cfg, path, shared, stop) = (cfg.clone(), path.to_path_buf(), shared.clone(), stop.clone());
        tokio::spawn(async move {
            let _ = collect(&cfg, &path, &shared, &stop, None).await;
        });
    }
    let me = dial.me().0;
    let chains = dial.chains().await.map_err(|e| waits(e.message))?;
    let _ = jobs.send(Job::Chains { chains, me });
    let admission = Arc::new(Admission::with(Doors { members: vec![protocol().alpn], join: None }));
    let mesh = Arc::new(Mesh::new(dial.clone(), admission));
    let links = Links::over(protocol(), mesh, engine::machine(cfg, jobs.clone(), me, shared.clone(), true));
    *fwd.write().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Fwd { links: links.clone(), dial: dial.clone() });
    let _ = jobs.send(Job::Attached);
    (cfg.log)(&format!("sync: shared mode is on, over XenoNet's endpoint {}", crate::hex(me)));
    shared.set_xenonet(json!({ "mode": "attached", "socket": path.to_string_lossy(), "endpoint": crate::hex(me) }));
    let keep = tokio::spawn(links.clone().keep_up());
    let rechain = {
        let (dial, jobs, shared) = (dial.clone(), jobs.clone(), shared.clone());
        tokio::spawn(async move {
            loop {
                shared.rechain.notified().await;
                match dial.chains().await {
                    Ok(chains) => {
                        let _ = jobs.send(Job::Chains { chains, me });
                    }
                    Err(_) => return,
                }
                // At most once a second, however many connections ask.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        })
    };
    // What XenoNet's window tells this Xenolith (drafts to make, entries the person signed, what
    // each network sees of the projects) goes to the worker, which reads and writes the chains.
    let told = {
        let (dial, jobs) = (dial.clone(), jobs.clone());
        tokio::spawn(async move {
            while let Some(body) = dial.told().await {
                if jobs.send(Job::Told(body)).is_err() {
                    return;
                }
            }
        })
    };
    let accept = links.clone().accept();
    tokio::pin!(accept);
    let ended = dial.ended();
    tokio::pin!(ended);
    // Stopped, or why not.
    let why: Option<Ended> = loop {
        tokio::select! {
            () = &mut accept => break Some(waits("XenoNet's forward ended")),
            () = &mut ended => break Some(waits("XenoNet's forward ended (XenoNet stopped, or the grant was revoked)")),
            () = stop.notified() => break None,
            // A new grant (renewed, or another key) is attached with at once.
            Some(()) = wake.recv() => {
                if xenolith_core::xenonet::read_grant(&cfg.paths).as_deref() != Some(grant.as_str()) {
                    break Some(Ended { why: "the grant changed: attaching with the new one".into(), attached: true, again: true });
                }
            }
        }
    };
    keep.abort();
    rechain.abort();
    told.abort();
    *fwd.write().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    let _ = jobs.send(Job::Detached);
    links.mesh().close().await;
    let _ = dial.close().await;
    match why {
        None => Ok(()),
        Some(mut e) => {
            e.attached = true;
            Err(e)
        }
    }
}

/// How long before its end a grant is renewed.
pub const RENEW_MS: i64 = 10 * 86_400_000;

/// A grant collected from XenoNet over the socket for this Xenolith's context key, and kept in
/// its owner-only file: it waits for you to approve it at XenoNet's window, and goes from
/// XenoNet's service to this process alone, never through a command's output or a log. None once
/// the engine stops. A grant put in its file meanwhile (`xenolith sync grant`) is taken instead.
#[cfg(unix)]
async fn collect(
    cfg: &Arc<Config>,
    path: &Path,
    shared: &Arc<engine::Shared>,
    stop: &Arc<tokio::sync::Notify>,
    wake: Option<&mut tokio::sync::mpsc::UnboundedReceiver<()>>,
) -> Result<Option<String>, Ended> {
    use xenonet_id::sign::{Purpose, Signer as _};
    let key = cfg.keys.context.public();
    let signer = cfg.clone();
    let (asked, at) = (shared.clone(), path.to_string_lossy().into_owned());
    let waiting = move || {
        asked.set_xenonet(json!({ "mode": "asking", "socket": at, "why": "this Xenolith asked XenoNet for a grant to sync over this machine's networks: approve it in XenoNet's window (Network settings, Shared context)" }));
    };
    let connect = connect_checked(path.to_path_buf());
    let got = tokio::select! {
        r = ForwardDial::collect(&connect, protocol().alpn, &key, move |m| signer.keys.context.sign(Purpose::FORWARD_COLLECT, m).ok(), waiting) => r,
        () = stop.notified() => return Ok(None),
        g = put(cfg, wake) => return Ok(Some(g)),
    };
    let text = got.map_err(|e| match e.code {
        "denied" => waits("you turned down this Xenolith's grant in XenoNet's window; it asks again when its service restarts"),
        _ => waits(format!("XenoNet gave no grant: {} ({})", e.message, e.code)),
    })?;
    xenolith_core::xenonet::write_grant(&cfg.paths, &text).map_err(waits)?;
    (cfg.log)("sync: XenoNet gave this Xenolith a grant to sync over its network");
    Ok(Some(text))
}

// A grant that appears in its file while a collect waits.
#[cfg(unix)]
async fn put(cfg: &Config, wake: Option<&mut tokio::sync::mpsc::UnboundedReceiver<()>>) -> String {
    let Some(wake) = wake else { return std::future::pending().await };
    loop {
        if wake.recv().await.is_none() {
            return std::future::pending().await;
        }
        if let Some(g) = xenolith_core::xenonet::read_grant(&cfg.paths) {
            return g;
        }
    }
}

/// What /status says of shared mode, before anything happened.
pub fn idle() -> Value {
    json!({ "mode": "starting" })
}
