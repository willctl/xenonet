//! A host's network files in `<state>/network/` (docs/design/network.md,
//! "The wire"): `network.json` (the network, this host's id, and the
//! gateway's address and pinned key), `roster.json` (the chain),
//! `link.json`, how the link last stood, which the doctor reads, and
//! `services.json`, the services this host published to the network.
//! Who may start and wake sessions here is the network's chain's (each
//! member's `settings` entry), not this file's: a network.json from before
//! it held them is read past them, and they're dropped when it's written.

use crate::roster::{NetworkPolicy, Roster};
use crate::{NetError, R};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const NETWORK: &str = "network.json";
pub const ROSTER: &str = "roster.json";
pub const LINK: &str = "link.json";
pub const KEY: &str = "host.key";
pub const HOST_DB: &str = "net.sqlite";
pub const GATEWAY_DB: &str = "gateway.sqlite";
pub const SERVICES: &str = "services.json";

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

/// `network.json`.
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

/// The fields a network.json from before the chain held settings has, which
/// this one drops.
pub const DROPPED: [&str; 3] = ["start", "wake", "delegated"];

impl NetFile {
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "host": self.host,
            "founder": self.founder,
            "gateway": {
                "host": self.gateway.host,
                "keyId": self.gateway.key_id,
                "sign": self.gateway.sign,
                "seal": self.gateway.seal,
                "address": self.gateway.address,
            },
            "lan": self.lan,
            "titlesOnly": self.titles_only,
            "shareLoad": self.share_load,
            "joined": self.joined,
        })
    }

    pub fn from_json(v: &Value) -> R<NetFile> {
        let bad = || NetError::internal("network.json isn't readable");
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string).ok_or_else(bad);
        let g = v.get("gateway").ok_or_else(bad)?;
        Ok(NetFile {
            id: s(v, "id")?,
            name: s(v, "name")?,
            host: s(v, "host")?,
            founder: s(v, "founder")?,
            gateway: Pin {
                host: s(g, "host")?,
                key_id: s(g, "keyId")?,
                sign: s(g, "sign")?,
                seal: s(g, "seal")?,
                address: g.get("address").and_then(Value::as_str).map(str::to_string),
            },
            lan: v.get("lan").and_then(Value::as_str).map(str::to_string),
            titles_only: v.get("titlesOnly").and_then(Value::as_bool).unwrap_or(false),
            share_load: v.get("shareLoad").and_then(Value::as_bool).unwrap_or(true),
            joined: v.get("joined").and_then(Value::as_i64).unwrap_or(0),
        })
    }
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

/// The folder's files.
#[derive(Debug, Clone)]
pub struct Dir {
    pub path: PathBuf,
}

impl Dir {
    pub fn new(path: impl Into<PathBuf>) -> Dir {
        Dir { path: path.into() }
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    pub fn read_network(&self) -> R<Option<NetFile>> {
        match read_json(&self.file(NETWORK))? {
            Some(v) => NetFile::from_json(&v).map(Some),
            None => Ok(None),
        }
    }

    /// Whether network.json still has the settings a network.json from
    /// before the chain held them had: the node drops them, and logs it.
    pub fn has_dropped_fields(&self) -> bool {
        read_json(&self.file(NETWORK)).ok().flatten().is_some_and(|v| DROPPED.iter().any(|k| v.get(k).is_some()))
    }

    /// `<state>/identity`, beside this folder: the person on this machine.
    pub fn identity_dir(&self) -> PathBuf {
        self.path.parent().map_or_else(|| self.path.join("identity"), |p| p.join("identity"))
    }

    pub fn write_network(&self, n: &NetFile) -> R<()> {
        write_json(&self.file(NETWORK), &n.to_json())
    }

    /// The roster, checked whole under `policy`.
    pub fn read_roster(&self, policy: NetworkPolicy) -> R<Roster> {
        match read_json(&self.file(ROSTER))? {
            Some(v) => Ok(Roster::from_json_with(policy, &v)?),
            None => Ok(Roster::with_policy(policy)),
        }
    }

    pub fn write_roster(&self, r: &Roster) -> R<()> {
        write_json(&self.file(ROSTER), &r.to_json())
    }

    pub fn write_link(&self, v: &Value) -> R<()> {
        write_json(&self.file(LINK), v)
    }

    pub fn read_link(&self) -> Option<Value> {
        read_json(&self.file(LINK)).ok().flatten()
    }

    /// Forgets the network: every file here goes, the key with it.
    pub fn forget(&self) -> R<()> {
        for name in [NETWORK, ROSTER, LINK, KEY, HOST_DB, GATEWAY_DB, SERVICES] {
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

pub fn read_json(file: &Path) -> R<Option<Value>> {
    match std::fs::read_to_string(file) {
        Ok(t) => serde_json::from_str(&t).map(Some).map_err(|_| NetError::internal(format!("{} isn't JSON", file.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Writes JSON whole or not at all: to a file beside it, then renamed.
pub fn write_json(file: &Path, v: &Value) -> R<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    xenonet_os::fs::write_atomic(file, format!("{}\n", serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())).as_bytes())?;
    Ok(())
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

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writers_at_once_never_fail_or_tear_a_file() {
        let dir = scratch("race");
        let file = dir.join("network.json");
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let file = file.clone();
                std::thread::spawn(move || {
                    for i in 0..60 {
                        write_json(&file, &json!({ "t": t, "i": i, "pad": "x".repeat(4000) })).expect("every write lands");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(read_json(&file).unwrap().is_some(), "whole JSON");
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(left, vec!["network.json".to_string()], "no temp files left");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let dir = scratch("fail");
        let file = dir.join("network.json");
        std::fs::create_dir(&file).unwrap(); // a rename onto a folder fails
        assert!(write_json(&file, &json!({})).is_err());
        let left = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(left, 1, "only the folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
