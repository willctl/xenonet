//! The network's feeds in the store (xenonet-log's `feed`): this machine's
//! own feed in each network, as its author, and what it holds and has
//! applied of every other member's.
//!
//! - **The author's side.** `feed_writers` keeps where this machine writes:
//!   its incarnation, how many kept and loose entries it has written in it,
//!   the incarnation's link and the last kept entry it wrote. The store is
//!   in FULL sync, so an incarnation it said it took is on disk. `feed_out`
//!   keeps the kept entries it wrote, each with its header, which go again
//!   to a member until that member's ack (`feed_acks`) passes them. A
//!   compact entry, kept in the same transaction as its writer's place,
//!   drops the entries of its range it doesn't keep live: their headers
//!   stay, the header chain a member behind it is sent (`feed_compacted`).
//!   Loose entries aren't kept here: a message the engine sent waits for
//!   its receipt in the engine's own tables.
//! - **The receiver's side.** `Feeds` is the `FeedStore` a `Feed` offers
//!   entries through: the head of each author's feed (`feed_heads`, frozen
//!   by a fork), the kept entries applied and whether each is still open
//!   (`feed_in`, which also keeps the ids a compacted range's header chain
//!   put at places it skipped), the loose ones taken with their signed time
//!   (`feed_loose`, pruned past `LOOSE_DEDUPE_MS`), each numbered in the
//!   store's order, and the kept entries held behind a gap (`feed_held`).
//!   Its calls join the transaction their thread has open, so the entries a
//!   `Feed` applies and what the engine writes for them are kept together
//!   or not at all: each is applied exactly once.

use crate::{next_stored, Db, StoreError, R};
use rusqlite::{params, Connection, OptionalExtension};
use xenonet_log::feed::{Compacted, FeedEntry, FeedStore, Fork, Head, Header, Kind, Link, Open, Pos};

pub(crate) const TABLES: &[&str] = &[
    // Where this machine writes its feed in each network.
    "CREATE TABLE feed_writers (
        network TEXT PRIMARY KEY,
        author TEXT NOT NULL,
        inc INTEGER NOT NULL,
        kept INTEGER NOT NULL,
        loose INTEGER NOT NULL,
        link_inc INTEGER NOT NULL,
        link_seq INTEGER NOT NULL,
        link_id BLOB,
        head_inc INTEGER NOT NULL,
        head_seq INTEGER NOT NULL,
        head_id BLOB
    ) STRICT",
    // The kept entries this machine wrote: each one's header, and the
    // entry as it signed it until a compact entry drops it.
    "CREATE TABLE feed_out (
        network TEXT NOT NULL,
        inc INTEGER NOT NULL,
        seq INTEGER NOT NULL,
        kind TEXT NOT NULL,
        header TEXT NOT NULL,
        entry TEXT,
        at INTEGER NOT NULL,
        PRIMARY KEY (network, inc, seq)
    ) STRICT",
    // How far each member said it holds this machine's feed.
    "CREATE TABLE feed_acks (
        network TEXT NOT NULL,
        member TEXT NOT NULL,
        inc INTEGER NOT NULL,
        high INTEGER NOT NULL,
        PRIMARY KEY (network, member)
    ) STRICT",
    // The kept entries applied from each author's feed, by place, and
    // whether each is still open, with the entry as its author signed it,
    // which this machine serves as a holder; `stored` and `entry` are empty
    // for an id a header chain put at a place this machine skipped.
    "CREATE TABLE feed_in (
        author TEXT NOT NULL,
        network TEXT NOT NULL,
        inc INTEGER NOT NULL,
        seq INTEGER NOT NULL,
        kind TEXT NOT NULL,
        hash BLOB NOT NULL,
        until INTEGER,
        open INTEGER NOT NULL,
        stored INTEGER UNIQUE,
        entry TEXT,
        PRIMARY KEY (author, network, inc, seq)
    ) STRICT",
    // The loose entries taken from each author's feed, by id, with the
    // time they were signed at, by which they're pruned.
    "CREATE TABLE feed_loose (
        author TEXT NOT NULL,
        network TEXT NOT NULL,
        hash BLOB NOT NULL,
        kind TEXT NOT NULL,
        at INTEGER NOT NULL,
        stored INTEGER NOT NULL UNIQUE,
        PRIMARY KEY (author, network, hash)
    ) STRICT",
    "CREATE INDEX feed_loose_at ON feed_loose (at)",
    // Where this machine stands with each author's feed, and the fork that
    // froze it, if one did.
    "CREATE TABLE feed_heads (
        author TEXT NOT NULL,
        network TEXT NOT NULL,
        inc INTEGER NOT NULL,
        seq INTEGER NOT NULL,
        fork_inc INTEGER,
        fork_seq INTEGER,
        fork_held BLOB,
        fork_offered BLOB,
        PRIMARY KEY (author, network)
    ) STRICT",
    // Kept entries that came before the ones ahead of them.
    "CREATE TABLE feed_held (
        author TEXT NOT NULL,
        network TEXT NOT NULL,
        inc INTEGER NOT NULL,
        seq INTEGER NOT NULL,
        entry TEXT NOT NULL,
        PRIMARY KEY (author, network, inc, seq)
    ) STRICT",
];

/// Where this machine writes its feed in a network (xenonet-log's
/// `Writer::resume` takes it back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriterAt {
    pub inc: u64,
    pub kept: u64,
    pub loose: u64,
    /// What the incarnation's first kept entry names.
    pub link: Link,
    /// The last kept entry written, the link until the first.
    pub head: Link,
}

/// A member's feed that a fork froze here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frozen {
    pub author: String,
    pub network: String,
    pub fork: Fork,
}

fn int(n: u64) -> R<i64> {
    i64::try_from(n).map_err(|_| StoreError::new("bad-entry", format!("{n} is past what the store counts")))
}

fn num(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}

fn hash_of(v: Vec<u8>) -> [u8; 32] {
    let mut h = [0u8; 32];
    if v.len() == 32 {
        h.copy_from_slice(&v);
    }
    h
}

fn link_of(inc: i64, seq: i64, id: Option<Vec<u8>>) -> Link {
    Link { at: Pos::new(num(inc), num(seq)), id: id.map(hash_of) }
}

fn bad(what: &str, message: &str) -> StoreError {
    StoreError::new("bad-entry", format!("a feed {what} doesn't read: {message}"))
}

fn entry_of(text: &str) -> R<FeedEntry> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| bad("entry", &e.to_string()))?;
    FeedEntry::parse(&v).map_err(|e| bad("entry", &e.message))
}

fn header_of(text: &str) -> R<Header> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| bad("header", &e.to_string()))?;
    Header::parse(&v).map_err(|e| bad("header", &e.message))
}

fn write_head(c: &Connection, author: &str, network: &str, head: &Head) -> R<()> {
    let (fi, fs, fh, fo) = match &head.frozen {
        Some(f) => (Some(int(f.at.inc)?), Some(int(f.at.seq)?), Some(f.held.to_vec()), Some(f.offered.to_vec())),
        None => (None, None, None, None),
    };
    c.execute(
        "INSERT INTO feed_heads (author, network, inc, seq, fork_inc, fork_seq, fork_held, fork_offered) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (author, network) DO UPDATE SET inc = excluded.inc, seq = excluded.seq, fork_inc = excluded.fork_inc,
           fork_seq = excluded.fork_seq, fork_held = excluded.fork_held, fork_offered = excluded.fork_offered",
        params![author, network, int(head.latest.inc)?, int(head.latest.seq)?, fi, fs, fh, fo],
    )?;
    Ok(())
}

type HeadRow = (i64, i64, Option<i64>, Option<i64>, Option<Vec<u8>>, Option<Vec<u8>>);

fn head_of(row: HeadRow) -> Head {
    let (inc, seq, fi, fs, fh, fo) = row;
    let frozen = match (fi, fs, fh, fo) {
        (Some(i), Some(s), Some(h), Some(o)) => Some(Fork { at: Pos::new(num(i), num(s)), held: hash_of(h), offered: hash_of(o) }),
        _ => None,
    };
    Head { latest: Pos::new(num(inc), num(seq)), frozen }
}

/// Drops from `feed_out` the entries a compact entry covers and doesn't
/// keep live, keeping their headers.
fn compact_out(c: &Connection, network: &str, e: &FeedEntry) -> R<()> {
    let Some(body) = e.compact() else { return Ok(()) };
    let mut q = c.prepare("SELECT seq FROM feed_out WHERE network = ?1 AND inc = ?2 AND seq <= ?3 AND entry IS NOT NULL")?;
    let seqs = q.query_map(params![network, int(body.inc)?, int(body.upto)?], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
    for s in seqs {
        if !body.keeps(Pos::new(body.inc, num(s))) {
            c.execute("UPDATE feed_out SET entry = NULL WHERE network = ?1 AND inc = ?2 AND seq = ?3", params![network, int(body.inc)?, s])?;
        }
    }
    Ok(())
}

impl Db {
    /// Where this machine writes its feed in `network`, if it has begun.
    pub fn feed_writer(&self, network: &str) -> R<Option<WriterAt>> {
        self.read(|c| {
            Ok(c.query_row("SELECT inc, kept, loose, link_inc, link_seq, link_id, head_inc, head_seq, head_id FROM feed_writers WHERE network = ?1", params![network], |r| {
                Ok(WriterAt {
                    inc: num(r.get(0)?),
                    kept: num(r.get(1)?),
                    loose: num(r.get(2)?),
                    link: link_of(r.get(3)?, r.get(4)?, r.get(5)?),
                    head: link_of(r.get(6)?, r.get(7)?, r.get(8)?),
                })
            })
            .optional()?)
        })
    }

    /// Keeps where this machine writes in `network`, and with it the kept
    /// entry it just wrote, if it wrote one: both or neither. A compact
    /// entry drops, in the same transaction, what it doesn't keep.
    pub fn put_feed_writer(&self, network: &str, author: &str, at: WriterAt, kept: Option<&FeedEntry>) -> R<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO feed_writers (network, author, inc, kept, loose, link_inc, link_seq, link_id, head_inc, head_seq, head_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (network) DO UPDATE SET author = excluded.author, inc = excluded.inc, kept = excluded.kept,
                   loose = excluded.loose, link_inc = excluded.link_inc, link_seq = excluded.link_seq, link_id = excluded.link_id,
                   head_inc = excluded.head_inc, head_seq = excluded.head_seq, head_id = excluded.head_id",
                params![
                    network,
                    author,
                    int(at.inc)?,
                    int(at.kept)?,
                    int(at.loose)?,
                    int(at.link.at.inc)?,
                    int(at.link.at.seq)?,
                    at.link.id.map(|h| h.to_vec()),
                    int(at.head.at.inc)?,
                    int(at.head.at.seq)?,
                    at.head.id.map(|h| h.to_vec()),
                ],
            )?;
            if let Some(e) = kept {
                c.execute(
                    "INSERT INTO feed_out (network, inc, seq, kind, header, entry, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![network, int(e.inc)?, int(e.seq)?, e.kind.as_str(), e.header().to_json().to_string(), e.to_json().to_string(), int(e.at)?],
                )?;
                compact_out(c, network, e)?;
            }
            Ok(())
        })
    }

    /// The kept entries this machine wrote in `network` after `after` and
    /// still holds, in order, at most `limit`.
    pub fn feed_out_after(&self, network: &str, after: Pos, limit: usize) -> R<Vec<FeedEntry>> {
        let rows: Vec<String> = self.read(|c| {
            let mut q = c.prepare("SELECT entry FROM feed_out WHERE network = ?1 AND entry IS NOT NULL AND (inc > ?2 OR (inc = ?2 AND seq > ?3)) ORDER BY inc, seq LIMIT ?4")?;
            let rows = q.query_map(params![network, int(after.inc)?, int(after.seq)?, i64::try_from(limit).unwrap_or(i64::MAX)], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| entry_of(t)).collect()
    }

    /// The answer to a member that wants this machine's feed in `network`
    /// from after `after`, when that's below the floor its latest compact
    /// entry set: the compact entry, the live entries from there on, and
    /// the headers of the range from there to the compact entry's `upto`.
    /// None when nothing the member lacks was compacted.
    pub fn feed_compacted(&self, network: &str, after: Pos) -> R<Option<Compacted>> {
        let text: Option<String> = self.read(|c| {
            Ok(c.query_row(
                "SELECT entry FROM feed_out WHERE network = ?1 AND kind = ?2 AND entry IS NOT NULL ORDER BY inc DESC, seq DESC LIMIT 1",
                params![network, Kind::Compact.as_str()],
                |r| r.get(0),
            )
            .optional()?)
        })?;
        let Some(compact) = text.map(|t| entry_of(&t)).transpose()? else { return Ok(None) };
        let Some(body) = compact.compact() else { return Ok(None) };
        if after.inc > body.inc || after >= Pos::new(body.inc, body.upto) {
            return Ok(None);
        }
        let lo = if after.inc == body.inc { after.seq + 1 } else { 1 };
        let rows: Vec<(String, Option<String>)> = self.read(|c| {
            let mut q = c.prepare("SELECT header, entry FROM feed_out WHERE network = ?1 AND inc = ?2 AND seq >= ?3 AND seq <= ?4 ORDER BY seq")?;
            let rows = q.query_map(params![network, int(body.inc)?, int(lo)?, int(body.upto)?], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        let mut headers = Vec::with_capacity(rows.len());
        let mut live = Vec::new();
        for (h, e) in rows {
            headers.push(header_of(&h)?);
            if let Some(e) = e {
                live.push(entry_of(&e)?);
            }
        }
        Ok(Some(Compacted { compact, live, headers }))
    }

    /// How far `member` said it holds this machine's feed in `network`.
    pub fn feed_ack(&self, network: &str, member: &str) -> R<Option<Pos>> {
        self.read(|c| {
            Ok(c.query_row("SELECT inc, high FROM feed_acks WHERE network = ?1 AND member = ?2", params![network, member], |r| Ok(Pos::new(num(r.get(0)?), num(r.get(1)?))))
                .optional()?)
        })
    }

    /// `member` holds this machine's feed up to `at`: kept when it's past
    /// what it said before, since acks only move on.
    pub fn put_feed_ack(&self, network: &str, member: &str, at: Pos) -> R<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO feed_acks (network, member, inc, high) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (network, member) DO UPDATE SET inc = excluded.inc, high = excluded.high
                 WHERE excluded.inc > feed_acks.inc OR (excluded.inc = feed_acks.inc AND excluded.high > feed_acks.high)",
                params![network, member, int(at.inc)?, int(at.seq)?],
            )?;
            Ok(())
        })
    }

    /// Every member's feed in `network` that a fork froze here.
    pub fn frozen_feeds(&self, network: &str) -> R<Vec<Frozen>> {
        self.read(|c| {
            let mut q =
                c.prepare("SELECT author, inc, seq, fork_inc, fork_seq, fork_held, fork_offered FROM feed_heads WHERE network = ?1 AND fork_inc IS NOT NULL ORDER BY author")?;
            let rows = q
                .query_map(params![network], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows.into_iter().filter_map(|(author, row)| head_of(row).frozen.map(|fork| Frozen { author, network: network.to_string(), fork })).collect())
        })
    }

    /// Where this machine stands with each author's feed in `network`.
    pub fn feed_heads(&self, network: &str) -> R<Vec<(String, Pos)>> {
        self.read(|c| {
            let mut q = c.prepare("SELECT author, inc, seq FROM feed_heads WHERE network = ?1 ORDER BY author")?;
            let rows = q.query_map(params![network], |r| Ok((r.get::<_, String>(0)?, Pos::new(num(r.get(1)?), num(r.get(2)?)))))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The kept entries this machine applied of `author`'s feed in
    /// `network` from `from` on, in order, at most `limit`, each with
    /// whether it is still open: what it serves as a holder. A place a
    /// header chain filled has no entry, and ends the run.
    pub fn feed_in_from(&self, author: &str, network: &str, from: Pos, limit: usize) -> R<Vec<(FeedEntry, bool)>> {
        let rows: Vec<(Option<String>, i64)> = self.read(|c| {
            let mut q = c.prepare("SELECT entry, open FROM feed_in WHERE author = ?1 AND network = ?2 AND (inc > ?3 OR (inc = ?3 AND seq >= ?4)) ORDER BY inc, seq LIMIT ?5")?;
            let rows = q
                .query_map(params![author, network, int(from.inc)?, int(from.seq)?, i64::try_from(limit).unwrap_or(i64::MAX)], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter().map_while(|(e, open)| e.map(|e| (e, open))).map(|(e, open)| Ok((entry_of(&e)?, open == 1))).collect()
    }

    /// How many kept entries are held behind a gap in `network`.
    pub fn feed_held_count(&self, network: &str) -> R<u64> {
        self.read(|c| Ok(num(c.query_row("SELECT count(*) FROM feed_held WHERE network = ?1", params![network], |r| r.get(0))?)))
    }

    /// The receiver's side of the feeds, for a `Feed` to offer entries
    /// through, in the caller's transaction.
    pub fn feeds(&self) -> Feeds<'_> {
        Feeds(self)
    }
}

/// What this machine holds of other members' feeds, as a `FeedStore`.
pub struct Feeds<'a>(&'a Db);

impl FeedStore for Feeds<'_> {
    type Error = StoreError;

    fn head(&self, author: &str, network: &str) -> R<Head> {
        self.0.read(|c| {
            let row: Option<HeadRow> = c
                .query_row("SELECT inc, seq, fork_inc, fork_seq, fork_held, fork_offered FROM feed_heads WHERE author = ?1 AND network = ?2", params![author, network], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
                })
                .optional()?;
            Ok(row.map(head_of).unwrap_or_default())
        })
    }

    fn set_head(&mut self, author: &str, network: &str, head: Head) -> R<()> {
        self.0.write(|c| write_head(c, author, network, &head))
    }

    fn kept(&self, author: &str, network: &str, at: Pos) -> R<Option<[u8; 32]>> {
        self.0.read(|c| {
            let h: Option<Vec<u8>> = c
                .query_row("SELECT hash FROM feed_in WHERE author = ?1 AND network = ?2 AND inc = ?3 AND seq = ?4", params![author, network, int(at.inc)?, int(at.seq)?], |r| {
                    r.get(0)
                })
                .optional()?;
            Ok(h.map(hash_of))
        })
    }

    fn keep(&mut self, e: &FeedEntry) -> R<()> {
        self.0.write(|c| {
            let stored = next_stored(c)?;
            let until = e.until().map(int).transpose()?;
            c.execute(
                "INSERT INTO feed_in (author, network, inc, seq, kind, hash, until, open, stored, entry) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9)",
                params![e.author, e.group, int(e.inc)?, int(e.seq)?, e.kind.as_str(), e.hash().to_vec(), until, stored as i64, e.to_json().to_string()],
            )?;
            Ok(())
        })
    }

    fn know(&mut self, h: &Header) -> R<()> {
        self.0.write(|c| {
            c.execute(
                "INSERT OR IGNORE INTO feed_in (author, network, inc, seq, kind, hash, until, open, stored) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 0, NULL)",
                params![h.author, h.group, int(h.inc)?, int(h.seq)?, h.kind.as_str(), h.hash().to_vec()],
            )?;
            Ok(())
        })
    }

    fn open_upto(&self, author: &str, network: &str, upto: Pos) -> R<Vec<Open>> {
        let rows: Vec<(i64, i64, String, Option<i64>)> = self.0.read(|c| {
            let mut q = c.prepare(
                "SELECT inc, seq, kind, until FROM feed_in WHERE author = ?1 AND network = ?2 AND open = 1 AND (inc < ?3 OR (inc = ?3 AND seq <= ?4)) ORDER BY inc, seq",
            )?;
            let rows =
                q.query_map(params![author, network, int(upto.inc)?, int(upto.seq)?], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.into_iter()
            .map(|(inc, seq, kind, until)| {
                let kind = Kind::parse(&kind).ok_or_else(|| bad("kind", &kind))?;
                Ok(Open { at: Pos::new(num(inc), num(seq)), kind, until: until.map(num) })
            })
            .collect()
    }

    fn close(&mut self, author: &str, network: &str, at: Pos) -> R<()> {
        self.0.write(|c| {
            c.execute("UPDATE feed_in SET open = 0 WHERE author = ?1 AND network = ?2 AND inc = ?3 AND seq = ?4", params![author, network, int(at.inc)?, int(at.seq)?])?;
            Ok(())
        })
    }

    fn has_loose(&self, author: &str, network: &str, id: &[u8; 32]) -> R<bool> {
        self.0.read(|c| {
            Ok(c.query_row("SELECT 1 FROM feed_loose WHERE author = ?1 AND network = ?2 AND hash = ?3", params![author, network, id.to_vec()], |_| Ok(())).optional()?.is_some())
        })
    }

    fn take_loose(&mut self, e: &FeedEntry) -> R<()> {
        self.0.write(|c| {
            let stored = next_stored(c)?;
            c.execute(
                "INSERT INTO feed_loose (author, network, hash, kind, at, stored) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![e.author, e.group, e.hash().to_vec(), e.kind.as_str(), int(e.at)?, stored as i64],
            )?;
            Ok(())
        })
    }

    fn prune_loose(&mut self, before: u64) -> R<()> {
        self.0.write(|c| {
            c.execute("DELETE FROM feed_loose WHERE at < ?1", params![int(before)?])?;
            Ok(())
        })
    }

    fn held(&self, author: &str, network: &str, at: Pos) -> R<Option<FeedEntry>> {
        let text: Option<String> = self.0.read(|c| {
            Ok(c.query_row("SELECT entry FROM feed_held WHERE author = ?1 AND network = ?2 AND inc = ?3 AND seq = ?4", params![author, network, int(at.inc)?, int(at.seq)?], |r| {
                r.get(0)
            })
            .optional()?)
        })?;
        text.map(|t| entry_of(&t)).transpose()
    }

    fn held_in(&self, author: &str, network: &str, inc: u64, upto: u64) -> R<Vec<FeedEntry>> {
        let rows: Vec<String> = self.0.read(|c| {
            let mut q = c.prepare("SELECT entry FROM feed_held WHERE author = ?1 AND network = ?2 AND inc = ?3 AND seq <= ?4 ORDER BY seq")?;
            let rows = q.query_map(params![author, network, int(inc)?, int(upto)?], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| entry_of(t)).collect()
    }

    fn hold(&mut self, e: FeedEntry) -> R<()> {
        self.0.write(|c| {
            c.execute(
                "INSERT OR REPLACE INTO feed_held (author, network, inc, seq, entry) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![e.author, e.group, int(e.inc)?, int(e.seq)?, e.to_json().to_string()],
            )?;
            Ok(())
        })
    }

    fn unhold(&mut self, author: &str, network: &str, at: Pos) -> R<()> {
        self.0.write(|c| {
            c.execute("DELETE FROM feed_held WHERE author = ?1 AND network = ?2 AND inc = ?3 AND seq = ?4", params![author, network, int(at.inc)?, int(at.seq)?])?;
            Ok(())
        })
    }

    fn held_firsts(&self, author: &str, network: &str, after: u64) -> R<Vec<FeedEntry>> {
        let rows: Vec<String> = self.0.read(|c| {
            let mut q = c.prepare("SELECT entry FROM feed_held WHERE author = ?1 AND network = ?2 AND inc > ?3 AND seq = 1 ORDER BY inc")?;
            let rows = q.query_map(params![author, network, int(after)?], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| entry_of(t)).collect()
    }

    fn drop_held_before(&mut self, author: &str, network: &str, inc: u64) -> R<()> {
        self.0.write(|c| {
            c.execute("DELETE FROM feed_held WHERE author = ?1 AND network = ?2 AND inc < ?3", params![author, network, int(inc)?])?;
            Ok(())
        })
    }
}
