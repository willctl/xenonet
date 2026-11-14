//! The networks in the service (docs/design/network.md): this machine's
//! nodes (xenonet-net), one per network it's in. No door serves the
//! network between machines: they reach each other on link/2.
//!
//! The engine reaches the networks through `net()` (`xenonet_model::ops::
//! Net`) and plugs its `Sink` in with `sinks().set(…)`. Until it does, a
//! sink that logs takes what arrives.

use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use xenonet_model::ops::Net;
use xenonet_net::node::{Node, Options};
use xenonet_net::nodes::Nodes;
use xenonet_net::persist::Persist;
use xenonet_net::sink::{LogFn, LogSink, SinkSlot};
use xenonet_net::R;

pub struct NetService {
    nodes: Arc<Nodes>,
}

impl NetService {
    /// The nodes, from `<state>/network` and the store, with a sink that
    /// logs until the engine sets its own. Nothing runs until `start`.
    pub fn open(opts: Options) -> R<Arc<NetService>> {
        let log = opts.log.clone();
        let sinks = Arc::new(SinkSlot::new(Arc::new(LogSink::new(log))));
        let nodes = Nodes::open(opts, sinks)?;
        Ok(Arc::new(NetService { nodes }))
    }

    /// The service's own options: its state folder, where the networks are
    /// kept, version and platform, and whether Desktop is installed here,
    /// which a join request says.
    pub fn options(state_dir: &str, persist: Arc<dyn Persist>, home: &str, version: &str, log: LogFn) -> Options {
        let mut o = Options::new(Path::new(state_dir).join("network"), persist, version, crate::borrowed::platform(), log);
        o.desktop = desktop_installed(home);
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

    /// Starts each network's node on link/2.
    pub fn start(self: &Arc<Self>) -> impl std::future::Future<Output = R<()>> {
        std::future::ready(self.nodes.start())
    }

    pub fn stop(&self) {
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

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn desktop_is_looked_for_where_its_installer_puts_it() {
        let home = std::env::temp_dir().join(format!("xenonet-desktop-test-{}", std::process::id()));
        let places = desktop_places(&home.to_string_lossy());
        assert!(places.contains(&std::path::PathBuf::from("/Applications/XenoNet.app")));
        std::fs::create_dir_all(home.join("Applications").join("XenoNet.app")).unwrap();
        assert!(desktop_installed(&home.to_string_lossy()), "in ~/Applications");
        let _ = std::fs::remove_dir_all(&home);
    }
}
