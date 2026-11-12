//! A host's networks (docs/design/network.md, "The wire"): each network
//! and this host's id in it (`NetFile`), the roster, how this machine
//! stands in it, which the doctor reads, and the services this host
//! published, all kept through `Persist`; and the files in
//! `<state>/network/`: this machine's key, one for every network it's in
//! (one endpoint), and a folder for each network by its id, with where
//! its members were last found. Who may start and wake sessions here is
//! the network's chain's (each member's `settings` entry).

use crate::persist::Persist;
use crate::roster::{NetworkPolicy, Roster};
use crate::R;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub const KEY: &str = "host.key";

/// The network as this host keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetFile {
    pub id: String,
    pub name: String,
    /// This host's id.
    pub host: String,
    pub founder: String,
    pub titles_only: bool,
    /// Whether this host sends its load (CPU, GPU, memory and what the
    /// parts are) to the window hosts' consoles. On unless the person
    /// turns it off.
    pub share_load: bool,
    pub joined: i64,
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

    /// A file of one network's own, in its folder, which is made.
    pub fn net_file(&self, network: &str, name: &str) -> R<PathBuf> {
        if !network.starts_with("net_") || !network[4..].bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(crate::NetError::internal(format!("{network} isn't a network's id")));
        }
        let dir = self.path.join(network);
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join(name))
    }

    /// The first network this machine is in, if it's in one.
    pub fn read_network(&self) -> R<Option<NetFile>> {
        self.persist.network()
    }

    /// Every network this machine is in, first joined first.
    pub fn read_networks(&self) -> R<Vec<NetFile>> {
        self.persist.networks()
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

    /// Forgets `network`: what's kept of it and its folder, and the key
    /// with them when it was the `last` this machine was in.
    pub fn forget(&self, network: &str, last: bool) -> R<()> {
        self.persist.forget(network)?;
        let gone = |path: PathBuf| match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        let dir = self.net_file(network, "")?;
        let _ = std::fs::remove_dir_all(dir);
        if last {
            gone(self.file(KEY))?;
        }
        Ok(())
    }
}
