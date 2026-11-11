//! The chains' entries this machine holds: a network's roster, a person's
//! devices and delegations. The chain (xenonet-log) decides whether an
//! entry may be taken; the store keeps what it took, each with its number
//! in the store's order, and gives the entries back to rebuild the chain.
//! A chain's name is its kind and id, such as `person:prs_…`.

use crate::{next_stored, Db, StoreError, R};
use rusqlite::{params, OptionalExtension};
use xenonet_log::chain::{Entry, Policy};

/// What storing an entry did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Put {
    /// Stored now, as this number in the store's order.
    Stored(u64),
    /// The same entry was there already, stored as this number.
    Had(u64),
}

impl Put {
    /// Its number in the store's order.
    pub fn stored(self) -> u64 {
        match self {
            Put::Stored(n) | Put::Had(n) => n,
        }
    }
}

impl Db {
    /// Keeps `e` as entry `e.n` of `chain`. The same entry again is
    /// `Had`; another at that height is a conflict, and the stored one
    /// stays.
    pub fn put_entry<P: Policy>(&self, chain: &str, e: &Entry<P>) -> R<Put> {
        let hash = e.hash();
        let text = e.to_json().to_string();
        let n = i64::try_from(e.n).map_err(|_| StoreError::new("bad-entry", format!("{chain}'s entry {} is past what the store counts", e.n)))?;
        self.write(|tx| {
            let had: Option<(String, i64)> =
                tx.query_row("SELECT hash, stored FROM chain_entries WHERE chain = ?1 AND n = ?2", params![chain, n], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            match had {
                Some((h, stored)) if h == hash => Ok(Put::Had(stored as u64)),
                Some(_) => Err(StoreError::new("conflict", format!("{chain} has another entry {n} already"))),
                None => {
                    let stored = next_stored(tx)?;
                    tx.execute("INSERT INTO chain_entries (chain, n, hash, entry, stored) VALUES (?1, ?2, ?3, ?4, ?5)", params![chain, n, hash, text, stored as i64])?;
                    Ok(Put::Stored(stored))
                }
            }
        })
    }

    /// `chain`'s entries, from entry 0 up, each with its number in the
    /// store's order.
    pub fn entries<P: Policy>(&self, chain: &str) -> R<Vec<(Entry<P>, u64)>> {
        let rows: Vec<(String, i64)> = self.read(|c| {
            let mut q = c.prepare("SELECT entry, stored FROM chain_entries WHERE chain = ?1 ORDER BY n")?;
            let rows = q.query_map(params![chain], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter()
            .map(|(text, stored)| {
                let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| StoreError::new("bad-entry", format!("{chain}: {e}")))?;
                let e = Entry::<P>::parse(&v).map_err(|e| StoreError::new("bad-entry", format!("{chain}: {}", e.message)))?;
                Ok((e, stored as u64))
            })
            .collect()
    }

    /// Where entry `n` of `chain` stands in the store's order, if it's
    /// held.
    pub fn entry_stored(&self, chain: &str, n: u64) -> R<Option<u64>> {
        let Ok(n) = i64::try_from(n) else { return Ok(None) };
        self.read(|c| Ok(c.query_row("SELECT stored FROM chain_entries WHERE chain = ?1 AND n = ?2", params![chain, n], |r| r.get::<_, i64>(0)).optional()?.map(|s| s as u64)))
    }
}
