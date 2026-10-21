//! The network in the service (docs/design/network.md): this machine's
//! node (xenonet-net), its link to the gateway, and, when this host is a
//! LAN-only gateway, the listener on the address the person picked.
//!
//! The engine reaches the network through `net()` (`xenonet_base::network::
//! Net`) and plugs its `Sink` in with `sinks().set(…)`. Until it does, a
//! sink that logs takes what arrives.

use super::front_door::listen_lan;
use serde_json::Value;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};
use xenonet_base::network::Net;
use xenonet_base::sync::Locked;
use xenonet_net::node::{Node, Options};
use xenonet_net::sink::{LogFn, LogSink, SinkSlot};
use xenonet_net::{NetError, R};

pub struct NetService {
    node: Arc<Node>,
    lan: Mutex<Option<(SocketAddr, tokio::task::JoinHandle<()>)>>,
}

impl NetService {
    /// The node, from `<state>/network`, with a sink that logs until the
    /// engine sets its own. Nothing runs until `start`.
    pub fn open(opts: Options) -> R<Arc<NetService>> {
        let log = opts.log.clone();
        let sinks = Arc::new(SinkSlot::new(Arc::new(LogSink::new(log))));
        let node = Node::open(opts, sinks)?;
        Ok(Arc::new(NetService { node, lan: Mutex::new(None) }))
    }

    /// The service's own options: its state folder, version and platform.
    pub fn options(state_dir: &str, version: &str, log: LogFn) -> Options {
        Options::new(Path::new(state_dir).join("network"), version, crate::borrowed::platform(), log)
    }

    pub fn node(&self) -> &Arc<Node> {
        &self.node
    }

    /// What the engine sends through.
    pub fn net(&self) -> Arc<dyn Net> {
        self.node.net()
    }

    /// Where the engine plugs its `Sink` in.
    pub fn sinks(&self) -> &Arc<SinkSlot> {
        self.node.sink()
    }

    /// Starts the link, and the LAN listener when this host is a LAN-only
    /// gateway.
    pub async fn start(&self) -> R<()> {
        self.node.start()?;
        let lan = self.node.shared().network().filter(|_| self.node.shared().is_gateway()).and_then(|n| n.lan);
        if let Some(lan) = lan {
            let addr = parse_lan(&lan)?;
            self.listen(addr).await?;
        }
        Ok(())
    }

    /// Listens for the network on a LAN address, in place of one before.
    pub async fn listen(&self, addr: SocketAddr) -> R<()> {
        if self.lan.locked().as_ref().is_some_and(|(a, _)| *a == addr) {
            return Ok(());
        }
        self.unlisten();
        let task = listen_lan(addr, self.node.clone()).await.map_err(|e| NetError::bad(format!("XenoNet can't listen on {addr}: {e}")))?;
        *self.lan.locked() = Some((addr, task));
        Ok(())
    }

    pub fn unlisten(&self) {
        if let Some((_, t)) = self.lan.locked().take() {
            t.abort();
        }
    }

    pub fn lan(&self) -> Option<SocketAddr> {
        self.lan.locked().as_ref().map(|(a, _)| *a)
    }

    pub fn stop(&self) {
        self.unlisten();
        self.node.stop();
    }

    pub fn state(&self) -> Value {
        self.node.state()
    }
}

/// A LAN address as the person gives it: an IP address this machine has
/// and a port, never 0.0.0.0.
pub fn parse_lan(text: &str) -> R<SocketAddr> {
    let addr: SocketAddr = text.trim().parse().map_err(|_| NetError::bad("a LAN address is an IP address and a port, such as 192.168.1.5:47850"))?;
    if addr.ip().is_unspecified() || addr.port() == 0 {
        return Err(NetError::bad("a LAN address is one this machine has, with a port: never 0.0.0.0"));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_addresses_are_this_machines() {
        assert_eq!(parse_lan("192.168.1.5:47850").unwrap().to_string(), "192.168.1.5:47850");
        assert!(parse_lan("0.0.0.0:47850").is_err());
        assert!(parse_lan("[::]:47850").is_err());
        assert!(parse_lan("192.168.1.5:0").is_err());
        assert!(parse_lan("gw.example.net:1").is_err());
    }
}
