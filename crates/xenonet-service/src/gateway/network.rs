//! The networks in the service (docs/design/network.md): this machine's
//! nodes (xenonet-net), one per network it's in. No door serves the
//! network between machines: they reach each other on link/2.
//!
//! The engine reaches the networks through `net()` (`xenonet_model::ops::
//! Net`) and plugs its `Sink` in with `sinks().set(…)`. Until it does, a
//! sink that logs takes what arrives.

use serde_json::Value;
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use xenonet_model::ops::Net;
use xenonet_net::node::{Node, Options};
use xenonet_net::nodes::Nodes;
use xenonet_net::persist::Persist;
use xenonet_net::sink::{LogFn, LogSink, SinkSlot};
use xenonet_net::{NetError, R};
use xenonet_os::sync::Locked;

pub struct NetService {
    this: std::sync::Weak<NetService>,
    nodes: Arc<Nodes>,
    /// Follows each network's gateway when it moves, by network id.
    watchers: Mutex<BTreeMap<String, tokio::task::JoinHandle<()>>>,
}

impl NetService {
    /// The nodes, from `<state>/network` and the store, with a sink that
    /// logs until the engine sets its own. Nothing runs until `start`.
    pub fn open(opts: Options) -> R<Arc<NetService>> {
        let log = opts.log.clone();
        let sinks = Arc::new(SinkSlot::new(Arc::new(LogSink::new(log))));
        let nodes = Nodes::open(opts, sinks)?;
        Ok(Arc::new_cyclic(|this| NetService { this: this.clone(), nodes, watchers: Mutex::new(BTreeMap::new()) }))
    }

    /// When a network's gateway moves, its link starts again toward it: on
    /// the host that took over, with its gateway and LAN listener; on the
    /// one that gave it up, after a while, so the hosts linked to it hear
    /// first. One watcher a network.
    fn watch(&self) {
        let mut watchers = self.watchers.locked();
        let nodes = self.nodes.nodes();
        let ids: Vec<String> = nodes.iter().filter_map(|n| n.shared().network().map(|f| f.id)).collect();
        watchers.retain(|id, t| {
            let keep = ids.contains(id);
            if !keep {
                t.abort();
            }
            keep
        });
        for node in nodes {
            let Some(id) = node.shared().network().map(|f| f.id) else { continue };
            if watchers.contains_key(&id) {
                continue;
            }
            let weak = self.this.clone();
            let task = tokio::spawn(async move {
                loop {
                    node.shared().relinked().await;
                    let was = node.gateway().is_some();
                    if was && !node.shared().is_gateway() {
                        tokio::time::sleep(std::time::Duration::from_millis(node.shared().timing().handover_ms)).await;
                    }
                    if weak.upgrade().is_none() {
                        return;
                    }
                    if let Err(e) = node.start() {
                        node.shared().log("net.gateway", &format!("the link didn't start again after the gateway moved: {}", e.message));
                    }
                }
            });
            watchers.insert(id, task);
        }
    }

    /// The service's own options: its state folder, where the networks are
    /// kept, version and platform, and whether Desktop is installed here,
    /// which a join request says.
    pub fn options(state_dir: &str, persist: Arc<dyn Persist>, home: &str, version: &str, log: LogFn) -> Options {
        let mut o = Options::new(Path::new(state_dir).join("network"), persist, version, crate::borrowed::platform(), log);
        o.desktop = desktop_installed(home);
        // link/2 beside the gateway's link (phase 5). mDNS stays off until
        // the person answers its question at network setup.
        o.link = true;
        o
    }

    /// Every network's node.
    pub fn nodes(&self) -> &Arc<Nodes> {
        &self.nodes
    }

    /// The first network's node, or one in none.
    pub fn node(&self) -> Arc<Node> {
        self.nodes.first()
    }

    /// What the engine sends through.
    pub fn net(&self) -> Arc<dyn Net> {
        self.nodes.net()
    }

    /// Where the engine plugs its `Sink` in.
    pub fn sinks(&self) -> &Arc<SinkSlot> {
        self.nodes.sink()
    }

    /// Starts each network's link, the LAN listeners of the networks whose
    /// LAN-only gateway this host is, and link/2, and follows each gateway
    /// when it moves.
    pub fn start(self: &Arc<Self>) -> impl std::future::Future<Output = R<()>> {
        self.watch();
        std::future::ready(self.nodes.start())
    }

    /// A network was made, joined or left: each gateway's move is followed,
    /// and the LAN listeners are the gateways' here.
    pub fn networks_changed(&self) {
        self.watch();
    }

    pub fn stop(&self) {
        for (_, t) in std::mem::take(&mut *self.watchers.locked()) {
            t.abort();
        }

        self.nodes.stop();
    }

    pub fn state(&self) -> Value {
        self.nodes.state()
    }
}

/// The link as the engine's plug takes it (xenonet-api's `Cores::on_net`).
impl xenonet_api::cores::NetLink for NetService {
    fn net(&self) -> Arc<dyn Net> {
        NetService::net(self)
    }

    fn sinks(&self) -> &Arc<SinkSlot> {
        NetService::sinks(self)
    }

    fn governance(&self) -> Option<Arc<dyn xenonet_net::ground::Governance>> {
        Some(self.nodes.clone())
    }
}

impl Drop for NetService {
    fn drop(&mut self) {
        for (_, t) in std::mem::take(&mut *self.watchers.locked()) {
            t.abort();
        }
    }
}

/// Where XenoNet's Desktop is when it's installed here: the app its
/// installer puts in /Applications (or ~/Applications) on macOS, and in the
/// per-user or all-users Programs folder on Windows. There's no Desktop
/// for Linux.
fn desktop_places(home: &str) -> Vec<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        return vec!["/Applications/XenoNet.app".into(), Path::new(home).join("Applications").join("XenoNet.app")];
    }
    let mut out = Vec::new();
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            out.push(Path::new(&local).join("Programs").join("XenoNet").join("XenoNet.exe"));
        }
        if let Some(all) = std::env::var_os("ProgramFiles") {
            out.push(Path::new(&all).join("XenoNet").join("XenoNet.exe"));
        }
    }
    out
}

/// Whether Desktop is installed on this machine.
pub fn desktop_installed(home: &str) -> bool {
    desktop_places(home).iter().any(|p| p.exists())
}

/// A LAN address as the person gives it (xenonet-http's net routes read it
/// so too): an IP address this machine has and a port, never 0.0.0.0.
pub fn parse_lan(text: &str) -> R<SocketAddr> {
    xenonet_http::routes::net::parse_lan(text).map_err(NetError::bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "macos")]
    fn desktop_is_looked_for_where_its_installer_puts_it() {
        let home = std::env::temp_dir().join(format!("xenonet-desktop-test-{}", std::process::id()));
        let places = desktop_places(&home.to_string_lossy());
        assert!(places.contains(&std::path::PathBuf::from("/Applications/XenoNet.app")));
        std::fs::create_dir_all(home.join("Applications").join("XenoNet.app")).unwrap();
        assert!(desktop_installed(&home.to_string_lossy()), "in ~/Applications");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn lan_addresses_are_this_machines() {
        assert_eq!(parse_lan("192.168.1.5:47850").unwrap().to_string(), "192.168.1.5:47850");
        assert!(parse_lan("0.0.0.0:47850").is_err());
        assert!(parse_lan("[::]:47850").is_err());
        assert!(parse_lan("192.168.1.5:0").is_err());
        assert!(parse_lan("gw.example.net:1").is_err());
    }
}
