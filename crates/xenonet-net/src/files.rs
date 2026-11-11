//! A host's network (docs/design/network.md, "The wire"): the network,
//! this host's id in it, and the gateway's address and pinned key
//! (`NetFile`), the roster, how the link last stood, which the doctor
//! reads, and the services this host published, all kept through
//! `Persist`; and the files in `<state>/network/`: this host's key and, on
//! the gateway's host, the network's queues. Who may start and wake sessions here is the network's
//! chain's (each member's `settings` entry).

use crate::persist::Persist;
use crate::roster::{NetworkPolicy, Roster};
use crate::R;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub const KEY: &str = "host.key";
pub const GATEWAY_DB: &str = "gateway.sqlite";

/// The gateway as this host pinned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub host: String,
    pub key_id: String,
    pub sign: String,
    pub seal: String,
    /// Where hosts reach it: a host name (through a tunnel), or an
    /// address and port on the LAN. None until it has one.
    pub address: Option<String>,
}

/// The network as this host keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetFile {
    pub id: String,
    pub name: String,
    /// This host's id.
    pub host: String,
    pub founder: String,
    pub gateway: Pin,
    /// Where this host's gateway listens on the LAN, when it's the gateway
    /// in LAN-only mode (never 0.0.0.0).
    pub lan: Option<String>,
    pub titles_only: bool,
    /// Whether this host sends its load (CPU, GPU, memory and what the
    /// parts are) to the window hosts' consoles that watch its map. On
    /// unless the person turns it off.
    pub share_load: bool,
    pub joined: i64,
}

/// What kind of address a gateway has: through a tunnel (a name), on the
/// LAN or a tailnet (an IP address), or this machine alone.
pub fn address_kind(address: &str) -> &'static str {
    let host = host_of(address);
    if host == "localhost" || host.starts_with("127.") || host == "::1" {
        "loopback"
    } else if host.parse::<std::net::IpAddr>().is_ok() {
        "lan"
    } else {
        "tunnel"
    }
}

/// The host part of an address, without a scheme, port or path.
pub fn host_of(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, r)| r);
    let rest = rest.split('/').next().unwrap_or("");
    if let Some(v6) = rest.strip_prefix('[') {
        return v6.split(']').next().unwrap_or("").to_ascii_lowercase();
    }
    match rest.rsplit_once(':') {
        Some((h, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !h.contains(':') => h.to_ascii_lowercase(),
        _ => rest.to_ascii_lowercase(),
    }
}

/// The URL a host dials for an address: http on the LAN and this machine,
/// https through a tunnel, or the scheme given.
pub fn base_url(address: &str) -> String {
    let a = address.trim().trim_end_matches('/');
    if a.contains("://") {
        return a.to_string();
    }
    if address_kind(a) == "tunnel" {
        format!("https://{a}")
    } else {
        format!("http://{a}")
    }
}

/// The network as this host keeps it, and the folder with its key and
/// queues.
#[derive(Clone)]
pub struct Dir {
    pub path: PathBuf,
    pub persist: Arc<dyn Persist>,
}

impl std::fmt::Debug for Dir {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dir").field("path", &self.path).finish_non_exhaustive()
    }
}

impl Dir {
    pub fn new(path: impl Into<PathBuf>, persist: Arc<dyn Persist>) -> Dir {
        Dir { path: path.into(), persist }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn read_network(&self) -> R<Option<NetFile>> {
        self.persist.network()
    }

    /// `<state>/identity`, beside this folder: the person's keys on this
    /// machine.
    pub fn identity_dir(&self) -> PathBuf {
        self.path.parent().map_or_else(|| self.path.join("identity"), |p| p.join("identity"))
    }

    pub fn write_network(&self, n: &NetFile) -> R<()> {
        self.persist.put_network(n)
    }

    /// The network's roster, checked whole under `policy`.
    pub fn read_roster(&self, network: &str, policy: NetworkPolicy) -> R<Roster> {
        Ok(Roster::from_entries_with(policy, self.persist.roster(network)?)?)
    }

    pub fn write_roster(&self, network: &str, r: &Roster) -> R<()> {
        self.persist.put_roster(network, r.entries())
    }

    pub fn write_link(&self, network: &str, v: &Value) -> R<()> {
        self.persist.put_link(network, v)
    }

    pub fn read_link(&self, network: &str) -> Option<Value> {
        self.persist.link(network).ok().flatten()
    }

    /// Forgets the network: what's kept of it, and every file here, the
    /// key with them.
    pub fn forget(&self) -> R<()> {
        if let Some(n) = self.persist.network()? {
            self.persist.forget(&n.id)?;
        }
        for name in [KEY, GATEWAY_DB] {
            for f in [name.to_string(), format!("{name}-wal"), format!("{name}-shm")] {
                match std::fs::remove_file(self.file(&f)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_their_kinds() {
        assert_eq!(address_kind("gw.example.net"), "tunnel");
        assert_eq!(address_kind("192.168.1.5:48200"), "lan");
        assert_eq!(address_kind("100.101.102.103:48200"), "lan");
        assert_eq!(address_kind("127.0.0.1:48113"), "loopback");
        assert_eq!(address_kind("localhost:1"), "loopback");
        assert_eq!(address_kind("[::1]:5"), "loopback");
        assert_eq!(base_url("gw.example.net"), "https://gw.example.net");
        assert_eq!(base_url("192.168.1.5:48200"), "http://192.168.1.5:48200");
        assert_eq!(base_url("http://gw.example.net:8080/"), "http://gw.example.net:8080");
        assert_eq!(host_of("https://GW.example.net/x"), "gw.example.net");
    }
}
