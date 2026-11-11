//! The network across machines and the person on this machine, in the
//! store (docs/design/network.md): the networks this machine is in, each
//! by its id, with how its link last stood and the services published to
//! it; the people's chains this machine holds, its own among them; the
//! presence blocks it has taken, so one counts once across restarts; and
//! the certificates of the roots it mints tokens with. A network's roster
//! and each person's chain are chains like any other: their entries are in
//! `chain_entries`, as `network:<id>` and `person:<id>`, numbered in the
//! store's order. The keys stay files.

use crate::{next_stored, Db, StoreError, R};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use xenonet_log::chain::{Entry, Policy};

pub(crate) const TABLES: &[&str] = &[
    // Each network this machine is in: this machine's id in it, who
    // founded it, the gateway as this machine pinned it, and its settings.
    "CREATE TABLE networks (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        host TEXT NOT NULL,
        founder TEXT NOT NULL,
        gateway_host TEXT NOT NULL,
        gateway_key_id TEXT NOT NULL,
        gateway_sign TEXT NOT NULL,
        gateway_seal TEXT NOT NULL,
        gateway_address TEXT,
        lan TEXT,
        titles_only INTEGER NOT NULL,
        share_load INTEGER NOT NULL,
        joined INTEGER NOT NULL
    ) STRICT",
    // How each network's link last stood, as the link writes it for the
    // doctor.
    "CREATE TABLE link_status (
        network TEXT PRIMARY KEY REFERENCES networks (id) ON DELETE CASCADE,
        status TEXT NOT NULL
    ) STRICT",
    // Where this machine stands in each network's gateway stream: the
    // gateway's epoch and the last seq read in it.
    "CREATE TABLE link_cursor (
        network TEXT PRIMARY KEY REFERENCES networks (id) ON DELETE CASCADE,
        epoch TEXT NOT NULL,
        seq INTEGER NOT NULL
    ) STRICT",
    // The services this machine published to each network, in the order
    // they were published.
    "CREATE TABLE published_services (
        network TEXT NOT NULL REFERENCES networks (id) ON DELETE CASCADE,
        n INTEGER NOT NULL,
        service TEXT NOT NULL,
        PRIMARY KEY (network, n)
    ) STRICT",
    // The people whose chains this machine holds: its own person (one at
    // most), and the others a network's chain needs checked.
    "CREATE TABLE persons (
        id TEXT PRIMARY KEY,
        own INTEGER NOT NULL
    ) STRICT",
    "CREATE UNIQUE INDEX persons_own ON persons (own) WHERE own = 1",
    // The presence blocks taken: each device's nonce, when it was taken.
    "CREATE TABLE presence_seen (
        device TEXT NOT NULL,
        nonce TEXT NOT NULL,
        at INTEGER NOT NULL,
        PRIMARY KEY (device, nonce)
    ) STRICT",
    // The token roots' certificates, in the order they were made.
    "CREATE TABLE token_roots (
        n INTEGER PRIMARY KEY,
        certificate TEXT NOT NULL
    ) STRICT",
];

/// A network, as this machine keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Network {
    pub id: String,
    pub name: String,
    /// This machine's id in it.
    pub host: String,
    pub founder: String,
    pub gateway: Gateway,
    /// Where this machine's gateway listens on the LAN, when it's the
    /// gateway in LAN-only mode.
    pub lan: Option<String>,
    pub titles_only: bool,
    pub share_load: bool,
    pub joined: i64,
}

/// The network's gateway, as this machine pinned it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gateway {
    pub host: String,
    pub key_id: String,
    pub sign: String,
    pub seal: String,
    pub address: Option<String>,
}

/// A person whose chain this machine holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub id: String,
    /// This machine's own person.
    pub own: bool,
}

/// A presence block taken: the device that signed it, its nonce, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub device: String,
    pub nonce: String,
    pub at: i64,
}

fn network_chain(id: &str) -> String {
    format!("network:{id}")
}

fn person_chain(id: &str) -> String {
    format!("person:{id}")
}

fn json_text(v: &Value) -> String {
    v.to_string()
}

fn json_of(text: &str, what: &str) -> R<Value> {
    serde_json::from_str(text).map_err(|e| StoreError::new("bad-entry", format!("{what} doesn't read: {e}")))
}

// A chain's entries from `entries` that the store doesn't hold, kept in
// the transaction that's open, each with the next number in the store's
// order. Another entry where one is held is a conflict, and nothing of the
// write stays.
fn keep_entries<P: Policy>(c: &Connection, chain: &str, entries: &[Entry<P>]) -> R<()> {
    for e in entries {
        let n = i64::try_from(e.n).map_err(|_| StoreError::new("bad-entry", format!("{chain}'s entry {} is past what the store counts", e.n)))?;
        let hash = e.hash();
        let had: Option<String> = c.query_row("SELECT hash FROM chain_entries WHERE chain = ?1 AND n = ?2", params![chain, n], |r| r.get(0)).optional()?;
        match had {
            Some(h) if h == hash => {}
            Some(_) => return Err(StoreError::new("conflict", format!("{chain} has another entry {n} already"))),
            None => {
                let stored = next_stored(c)?;
                c.execute(
                    "INSERT INTO chain_entries (chain, n, hash, entry, stored) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![chain, n, hash, e.to_json().to_string(), stored as i64],
                )?;
            }
        }
    }
    Ok(())
}

impl Db {
    /// The networks this machine is in.
    pub fn networks(&self) -> R<Vec<Network>> {
        self.read(|c| {
            let mut q = c.prepare(
                "SELECT id, name, host, founder, gateway_host, gateway_key_id, gateway_sign, gateway_seal, gateway_address, lan, titles_only, share_load, joined FROM networks ORDER BY rowid",
            )?;
            let rows = q
                .query_map([], |r| {
                    Ok(Network {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        host: r.get(2)?,
                        founder: r.get(3)?,
                        gateway: Gateway { host: r.get(4)?, key_id: r.get(5)?, sign: r.get(6)?, seal: r.get(7)?, address: r.get(8)? },
                        lan: r.get(9)?,
                        titles_only: r.get::<_, i64>(10)? != 0,
                        share_load: r.get::<_, i64>(11)? != 0,
                        joined: r.get(12)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// A network, made or changed.
    pub fn put_network(&self, n: &Network) -> R<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO networks (id, name, host, founder, gateway_host, gateway_key_id, gateway_sign, gateway_seal, gateway_address, lan, titles_only, share_load, joined)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT (id) DO UPDATE SET name = excluded.name, host = excluded.host, founder = excluded.founder,
                   gateway_host = excluded.gateway_host, gateway_key_id = excluded.gateway_key_id, gateway_sign = excluded.gateway_sign,
                   gateway_seal = excluded.gateway_seal, gateway_address = excluded.gateway_address, lan = excluded.lan,
                   titles_only = excluded.titles_only, share_load = excluded.share_load, joined = excluded.joined",
                params![
                    n.id,
                    n.name,
                    n.host,
                    n.founder,
                    n.gateway.host,
                    n.gateway.key_id,
                    n.gateway.sign,
                    n.gateway.seal,
                    n.gateway.address,
                    n.lan,
                    i64::from(n.titles_only),
                    i64::from(n.share_load),
                    n.joined
                ],
            )?;
            Ok(())
        })
    }

    /// A network left: its row, its roster's entries, how its link stood,
    /// the services published to it and its feeds, all at once.
    pub fn forget_network(&self, id: &str) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM chain_entries WHERE chain = ?1", params![network_chain(id)])?;
            c.execute("DELETE FROM link_status WHERE network = ?1", params![id])?;
            c.execute("DELETE FROM link_cursor WHERE network = ?1", params![id])?;
            for t in ["feed_writers", "feed_out", "feed_acks", "feed_in", "feed_loose", "feed_heads", "feed_held"] {
                c.execute(&format!("DELETE FROM {t} WHERE network = ?1"), params![id])?;
            }
            c.execute("DELETE FROM published_services WHERE network = ?1", params![id])?;
            c.execute("DELETE FROM networks WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    /// A network's roster: its chain's entries, from entry 0.
    pub fn roster<P: Policy>(&self, network: &str) -> R<Vec<Entry<P>>> {
        Ok(self.entries::<P>(&network_chain(network))?.into_iter().map(|(e, _)| e).collect())
    }

    /// Keeps the entries of a network's roster the store doesn't hold yet,
    /// in one write.
    pub fn put_roster<P: Policy>(&self, network: &str, entries: &[Entry<P>]) -> R<()> {
        let chain = network_chain(network);
        self.write(|c| keep_entries(c, &chain, entries))
    }

    /// How a network's link last stood.
    pub fn link_status(&self, network: &str) -> R<Option<Value>> {
        let text: Option<String> = self.read(|c| Ok(c.query_row("SELECT status FROM link_status WHERE network = ?1", params![network], |r| r.get(0)).optional()?))?;
        text.map(|t| json_of(&t, "the link's status")).transpose()
    }

    /// How a network's link stands now. Nothing is kept for a network this
    /// machine isn't in.
    pub fn put_link_status(&self, network: &str, status: &Value) -> R<()> {
        let text = json_text(status);
        self.write(|c| {
            c.execute(
                "INSERT INTO link_status (network, status) SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM networks WHERE id = ?1)
                 ON CONFLICT (network) DO UPDATE SET status = excluded.status",
                params![network, text],
            )?;
            Ok(())
        })
    }

    /// Where this machine stands in a network's gateway stream: the
    /// gateway's epoch and the last seq read in it.
    pub fn link_cursor(&self, network: &str) -> R<Option<(String, u64)>> {
        self.read(|c| {
            Ok(c.query_row("SELECT epoch, seq FROM link_cursor WHERE network = ?1", params![network], |r| {
                Ok((r.get::<_, String>(0)?, u64::try_from(r.get::<_, i64>(1)?).unwrap_or(0)))
            })
            .optional()?)
        })
    }

    /// Where this machine stands in a network's gateway stream now.
    pub fn put_link_cursor(&self, network: &str, epoch: &str, seq: u64) -> R<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO link_cursor (network, epoch, seq) SELECT ?1, ?2, ?3 WHERE EXISTS (SELECT 1 FROM networks WHERE id = ?1)
                 ON CONFLICT (network) DO UPDATE SET epoch = excluded.epoch, seq = excluded.seq",
                params![network, epoch, i64::try_from(seq).unwrap_or(i64::MAX)],
            )?;
            Ok(())
        })
    }

    /// The services published to a network, in order.
    pub fn services(&self, network: &str) -> R<Vec<Value>> {
        let rows: Vec<String> = self.read(|c| {
            let mut q = c.prepare("SELECT service FROM published_services WHERE network = ?1 ORDER BY n")?;
            let rows = q.query_map(params![network], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| json_of(t, "a published service")).collect()
    }

    /// The services published to a network, in place of what was.
    pub fn put_services(&self, network: &str, services: &[Value]) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM published_services WHERE network = ?1", params![network])?;
            for (n, s) in services.iter().enumerate() {
                c.execute("INSERT INTO published_services (network, n, service) VALUES (?1, ?2, ?3)", params![network, n as i64, json_text(s)])?;
            }
            Ok(())
        })
    }

    /// The people whose chains this machine holds.
    pub fn persons(&self) -> R<Vec<Person>> {
        self.read(|c| {
            let mut q = c.prepare("SELECT id, own FROM persons ORDER BY own DESC, id")?;
            let rows = q.query_map([], |r| Ok(Person { id: r.get(0)?, own: r.get::<_, i64>(1)? != 0 }))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// A person's chain: its entries, from entry 0.
    pub fn person<P: Policy>(&self, id: &str) -> R<Vec<Entry<P>>> {
        Ok(self.entries::<P>(&person_chain(id))?.into_iter().map(|(e, _)| e).collect())
    }

    /// Keeps a person's chain: the entries the store doesn't hold yet, in
    /// one write. `own` makes it this machine's own person, in place of the
    /// one before, whose chain goes.
    pub fn put_person<P: Policy>(&self, id: &str, own: bool, entries: &[Entry<P>]) -> R<()> {
        let chain = person_chain(id);
        self.write(|c| {
            if own {
                let before: Option<String> = c.query_row("SELECT id FROM persons WHERE own = 1", [], |r| r.get(0)).optional()?;
                if let Some(old) = before.filter(|old| old != id) {
                    c.execute("DELETE FROM chain_entries WHERE chain = ?1", params![person_chain(&old)])?;
                    c.execute("DELETE FROM persons WHERE id = ?1", params![old])?;
                }
            }
            c.execute("INSERT INTO persons (id, own) VALUES (?1, ?2) ON CONFLICT (id) DO UPDATE SET own = MAX(persons.own, excluded.own)", params![id, i64::from(own)])?;
            keep_entries(c, &chain, entries)
        })
    }

    /// Forgets the other people's chains, as when the network that needed
    /// them was left; the machine's own person stays.
    pub fn forget_others(&self) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM chain_entries WHERE chain IN (SELECT 'person:' || id FROM persons WHERE own = 0)", [])?;
            c.execute("DELETE FROM persons WHERE own = 0", [])?;
            Ok(())
        })
    }

    /// The presence blocks taken.
    pub fn presence_seen(&self) -> R<Vec<Seen>> {
        self.read(|c| {
            let mut q = c.prepare("SELECT device, nonce, at FROM presence_seen ORDER BY at, device, nonce")?;
            let rows = q.query_map([], |r| Ok(Seen { device: r.get(0)?, nonce: r.get(1)?, at: r.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The presence blocks taken, in place of what was.
    pub fn put_presence_seen(&self, seen: &[Seen]) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM presence_seen", [])?;
            for s in seen {
                c.execute("INSERT OR REPLACE INTO presence_seen (device, nonce, at) VALUES (?1, ?2, ?3)", params![s.device, s.nonce, s.at])?;
            }
            Ok(())
        })
    }

    /// The token roots' certificates, in the order they were made.
    pub fn token_roots(&self) -> R<Vec<Value>> {
        let rows: Vec<String> = self.read(|c| {
            let mut q = c.prepare("SELECT certificate FROM token_roots ORDER BY n")?;
            let rows = q.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| json_of(t, "a token root's certificate")).collect()
    }

    /// The token roots' certificates, in place of what was.
    pub fn put_token_roots(&self, certificates: &[Value]) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM token_roots", [])?;
            for (n, cert) in certificates.iter().enumerate() {
                c.execute("INSERT INTO token_roots (n, certificate) VALUES (?1, ?2)", params![n as i64, json_text(cert)])?;
            }
            Ok(())
        })
    }
}
