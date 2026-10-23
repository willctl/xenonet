//! The gateway's store, `<state>/network/gateway.sqlite` (docs/design/
//! network.md, section 9 and "The wire"): tickets (their hashes), a queue
//! for each host, the frame ids of the last week, the epoch, the last
//! presence each host sent each other, and what each host said of itself
//! when it joined.
//!
//! A queue holds up to 10,000 frames or 50 MB, a week at most; past that,
//! the oldest goes, and the caller sends `gap`s. `gap` frames themselves
//! aren't counted, and a new one replaces one the host hasn't read.

use crate::ticket::MAX_WAITING;
use crate::{NetError, R};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub const WEEK_MS: f64 = 7.0 * 24.0 * 3600e3;

/// How much a queue holds.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub frames: u64,
    pub bytes: u64,
    pub keep_ms: f64,
}

impl Default for Caps {
    fn default() -> Self {
        Caps { frames: 10_000, bytes: 50 << 20, keep_ms: WEEK_MS }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketRow {
    /// base64url SHA-256 of the code.
    pub hash: String,
    pub host: String,
    pub window: bool,
    pub maker: String,
    pub made: i64,
    pub expires: i64,
}

/// A frame for a host's queue: its line has no seq yet.
#[derive(Clone, Copy)]
pub struct Item<'a> {
    pub host: &'a str,
    pub id: &'a str,
    pub kind: &'a str,
    pub sender: &'a str,
    pub line: &'a str,
}

/// A frame a full queue let go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub host: String,
    pub id: String,
    pub sender: String,
    pub kind: String,
}

/// What a host said of itself when it joined: its id, platform and version.
pub type HostInfo = (String, Option<String>, Option<String>);

pub struct GwStore {
    db: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS tickets (hash TEXT PRIMARY KEY, host TEXT NOT NULL, win INTEGER NOT NULL, maker TEXT NOT NULL, made INTEGER NOT NULL, expires INTEGER NOT NULL, used INTEGER);
CREATE TABLE IF NOT EXISTS queue (host TEXT NOT NULL, seq INTEGER NOT NULL, id TEXT NOT NULL, kind TEXT NOT NULL, sender TEXT NOT NULL, line TEXT NOT NULL, size INTEGER NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (host, seq));
CREATE TABLE IF NOT EXISTS seqs (host TEXT PRIMARY KEY, next INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS frame_ids (id TEXT PRIMARY KEY, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS presence (sender TEXT NOT NULL, host TEXT NOT NULL, line TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (sender, host));
CREATE TABLE IF NOT EXISTS hosts (id TEXT PRIMARY KEY, platform TEXT, version TEXT, desktop INTEGER, joined INTEGER NOT NULL);
";

fn n(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl GwStore {
    /// Opens the store, made with a new epoch when it's new.
    pub fn open(file: &Path) -> R<GwStore> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Connection::open(file)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.execute_batch(SCHEMA)?;
        let s = GwStore { db };
        if s.meta("epoch")?.is_none() {
            s.set_meta("epoch", &crate::codec::nonce()?)?;
        }
        Ok(s)
    }

    pub fn meta(&self, k: &str) -> R<Option<String>> {
        Ok(self.db.query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&self, k: &str, v: &str) -> R<()> {
        self.db.execute("INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v", [k, v])?;
        Ok(())
    }

    /// A random id made when the store was: a new one means the gateway
    /// moved or lost its store, and hosts resume from 0.
    pub fn epoch(&self) -> R<String> {
        self.meta("epoch")?.ok_or_else(|| NetError::internal("the gateway's store has no epoch"))
    }

    // ---------- tickets ----------

    pub fn waiting_tickets(&self, now: i64) -> R<Vec<TicketRow>> {
        let mut st = self.db.prepare("SELECT hash, host, win, maker, made, expires FROM tickets WHERE used IS NULL AND expires > ?1 ORDER BY made")?;
        let rows =
            st.query_map([now], |r| Ok(TicketRow { hash: r.get(0)?, host: r.get(1)?, window: r.get::<_, i64>(2)? != 0, maker: r.get(3)?, made: r.get(4)?, expires: r.get(5)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Keeps a ticket's hash; at most five wait at once.
    pub fn add_ticket(&self, t: &TicketRow, now: i64) -> R<()> {
        self.db.execute("DELETE FROM tickets WHERE expires <= ?1 OR used IS NOT NULL", [now])?;
        if self.waiting_tickets(now)?.len() >= MAX_WAITING {
            return Err(NetError::new(429, "tickets", format!("{MAX_WAITING} tickets wait already: use one, or wait ten minutes")));
        }
        self.db.execute(
            "INSERT INTO tickets (hash, host, win, maker, made, expires) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![t.hash, t.host, i64::from(t.window), t.maker, t.made, t.expires],
        )?;
        Ok(())
    }

    /// Uses a ticket: true when it was waiting, and now isn't.
    pub fn use_ticket(&self, hash: &str, now: i64) -> R<bool> {
        Ok(self.db.execute("UPDATE tickets SET used = ?2 WHERE hash = ?1 AND used IS NULL AND expires > ?2", params![hash, now])? == 1)
    }

    // ---------- frame ids ----------

    /// Records a frame id: true when it was seen before.
    pub fn seen(&self, id: &str, now: i64) -> R<bool> {
        Ok(self.db.execute("INSERT OR IGNORE INTO frame_ids (id, at) VALUES (?1, ?2)", params![id, now])? == 0)
    }

    pub fn was_seen(&self, id: &str) -> R<bool> {
        Ok(self.db.query_row("SELECT 1 FROM frame_ids WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
    }

    // ---------- queues ----------

    fn next_seq(&self, host: &str) -> R<u64> {
        let next: Option<i64> = self.db.query_row("SELECT next FROM seqs WHERE host = ?1", [host], |r| r.get(0)).optional()?;
        let seq = u(next.unwrap_or(1));
        self.db.execute("INSERT INTO seqs (host, next) VALUES (?1, ?2) ON CONFLICT(host) DO UPDATE SET next = excluded.next", params![host, n(seq + 1)])?;
        Ok(seq)
    }

    /// Puts a frame (its line without a seq) in `host`'s queue: its seq,
    /// and what the caps let go.
    pub fn enqueue(&self, item: Item<'_>, now: i64, caps: &Caps) -> R<(u64, Vec<Dropped>)> {
        let Item { host, id, kind, sender, line } = item;
        let tx = self.db.unchecked_transaction()?;
        let seq = self.next_seq(host)?;
        tx.execute(
            "INSERT INTO queue (host, seq, id, kind, sender, line, size, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![host, n(seq), id, kind, sender, line, n(line.len() as u64), now],
        )?;
        let mut dropped = Vec::new();
        loop {
            let (count, bytes) = self.size(host)?;
            if count <= caps.frames && bytes <= caps.bytes {
                break;
            }
            let oldest: Option<(i64, String, String, String)> = tx
                .query_row("SELECT seq, id, sender, kind FROM queue WHERE host = ?1 AND kind != 'gap' ORDER BY seq LIMIT 1", [host], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((s, id, sender, kind)) = oldest else { break };
            tx.execute("DELETE FROM queue WHERE host = ?1 AND seq = ?2", params![host, s])?;
            dropped.push(Dropped { host: host.into(), id, sender, kind });
        }
        tx.commit()?;
        Ok((seq, dropped))
    }

    /// A `gap` for `host`, in place of one it hasn't read.
    pub fn enqueue_gap(&self, host: &str, id: &str, line: &str, now: i64) -> R<u64> {
        let tx = self.db.unchecked_transaction()?;
        tx.execute("DELETE FROM queue WHERE host = ?1 AND kind = 'gap'", [host])?;
        let seq = self.next_seq(host)?;
        tx.execute(
            "INSERT INTO queue (host, seq, id, kind, sender, line, size, at) VALUES (?1, ?2, ?3, 'gap', '', ?4, ?5, ?6)",
            params![host, n(seq), id, line, n(line.len() as u64), now],
        )?;
        tx.commit()?;
        Ok(seq)
    }

    /// The frames after `seq` in `host`'s queue, in order: (seq, line).
    pub fn after(&self, host: &str, seq: u64, limit: u64) -> R<Vec<(u64, String)>> {
        let mut st = self.db.prepare("SELECT seq, line FROM queue WHERE host = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3")?;
        let rows = st.query_map(params![host, n(seq), n(limit)], |r| Ok((u(r.get(0)?), r.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The host has everything up to `seq`: those go.
    pub fn ack(&self, host: &str, seq: u64) -> R<()> {
        self.db.execute("DELETE FROM queue WHERE host = ?1 AND seq <= ?2", params![host, n(seq)])?;
        Ok(())
    }

    /// The last seq given out for `host`.
    pub fn last_seq(&self, host: &str) -> R<u64> {
        let next: Option<i64> = self.db.query_row("SELECT next FROM seqs WHERE host = ?1", [host], |r| r.get(0)).optional()?;
        Ok(u(next.unwrap_or(1)).saturating_sub(1))
    }

    /// A queue's frames and bytes, `gap`s aside.
    pub fn size(&self, host: &str) -> R<(u64, u64)> {
        let (count, bytes): (i64, i64) =
            self.db.query_row("SELECT COUNT(*), COALESCE(SUM(size), 0) FROM queue WHERE host = ?1 AND kind != 'gap'", [host], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok((u(count), u(bytes)))
    }

    /// Every queue's size: (host, frames, bytes).
    pub fn sizes(&self) -> R<Vec<(String, u64, u64)>> {
        let mut st = self.db.prepare("SELECT host, COUNT(*), COALESCE(SUM(size), 0) FROM queue WHERE kind != 'gap' GROUP BY host ORDER BY host")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, u(r.get(1)?), u(r.get(2)?))))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Frames older than `keep_ms` go, and ids older than a week are
    /// forgotten: what went, for the caller's `gap`s.
    pub fn prune(&self, now: i64, keep_ms: f64) -> R<Vec<Dropped>> {
        let before = now - keep_ms as i64;
        let mut st = self.db.prepare("SELECT host, id, sender, kind FROM queue WHERE at < ?1 AND kind != 'gap'")?;
        let old = st.query_map([before], |r| Ok(Dropped { host: r.get(0)?, id: r.get(1)?, sender: r.get(2)?, kind: r.get(3)? }))?.collect::<Result<Vec<_>, _>>()?;
        self.db.execute("DELETE FROM queue WHERE at < ?1", [before])?;
        self.db.execute("DELETE FROM frame_ids WHERE at < ?1", [now - WEEK_MS as i64])?;
        self.db.execute("DELETE FROM tickets WHERE expires <= ?1", [now])?;
        Ok(old)
    }

    /// A removed host's queue and presence go.
    pub fn forget_host(&self, host: &str) -> R<()> {
        self.db.execute("DELETE FROM queue WHERE host = ?1", [host])?;
        self.db.execute("DELETE FROM presence WHERE host = ?1 OR sender = ?1", [host])?;
        Ok(())
    }

    // ---------- presence ----------

    /// The last presence `sender` sent `host`.
    pub fn set_presence(&self, sender: &str, host: &str, line: &str, now: i64) -> R<()> {
        self.db.execute(
            "INSERT INTO presence (sender, host, line, at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(sender, host) DO UPDATE SET line = excluded.line, at = excluded.at",
            params![sender, host, line, now],
        )?;
        Ok(())
    }

    /// The last presence of each other host, sealed for `host`.
    pub fn presences(&self, host: &str) -> R<Vec<String>> {
        let mut st = self.db.prepare("SELECT line FROM presence WHERE host = ?1 ORDER BY sender")?;
        let rows = st.query_map([host], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- hosts ----------

    pub fn set_host(&self, id: &str, platform: Option<&str>, version: Option<&str>, desktop: bool, now: i64) -> R<()> {
        self.db.execute(
            "INSERT INTO hosts (id, platform, version, desktop, joined) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(id) DO UPDATE SET platform = excluded.platform, version = excluded.version, desktop = excluded.desktop",
            params![id, platform, version, i64::from(desktop), now],
        )?;
        Ok(())
    }

    /// What each host said of itself.
    pub fn hosts(&self) -> R<Vec<HostInfo>> {
        let mut st = self.db.prepare("SELECT id, platform, version FROM hosts ORDER BY id")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (GwStore, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("xenonet-net-gwstore-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        (GwStore::open(&dir.join("gateway.sqlite")).unwrap(), dir)
    }

    #[test]
    fn the_epoch_is_made_once() {
        let (s, dir) = store("epoch");
        let e = s.epoch().unwrap();
        drop(s);
        assert_eq!(GwStore::open(&dir.join("gateway.sqlite")).unwrap().epoch().unwrap(), e);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn tickets_wait_five_at_most_and_work_once() {
        let (s, dir) = store("tickets");
        let t = |i: u8| TicketRow { hash: format!("h{i}"), host: format!("host{i}"), window: false, maker: "studio".into(), made: 1, expires: 1000 };
        for i in 0..5 {
            s.add_ticket(&t(i), 10).unwrap();
        }
        assert_eq!(s.add_ticket(&t(5), 10).unwrap_err().status, 429);
        assert!(s.use_ticket("h0", 10).unwrap());
        assert!(!s.use_ticket("h0", 10).unwrap(), "once");
        assert!(!s.use_ticket("h1", 1000).unwrap(), "ten minutes");
        s.add_ticket(&t(5), 10).unwrap();
        assert_eq!(s.waiting_tickets(10).unwrap().len(), 5);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_full_queue_lets_the_oldest_go() {
        let (s, dir) = store("queue");
        let caps = Caps { frames: 3, bytes: 1 << 20, keep_ms: WEEK_MS };
        let mut all = Vec::new();
        for i in 0..5 {
            let (seq, dropped) = s.enqueue(Item { host: "laptop", id: &format!("f{i}"), kind: "message", sender: "studio", line: "{}" }, i, &caps).unwrap();
            assert_eq!(seq, u64::try_from(i).unwrap() + 1);
            all.extend(dropped);
        }
        assert_eq!(all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["f0", "f1"]);
        assert_eq!(s.size("laptop").unwrap(), (3, 6));
        s.enqueue_gap("laptop", "g1", "{gap}", 9).unwrap();
        s.enqueue_gap("laptop", "g2", "{gap}", 9).unwrap();
        let after = s.after("laptop", 0, 100).unwrap();
        assert_eq!(after.iter().map(|(q, _)| *q).collect::<Vec<_>>(), [3, 4, 5, 7], "one gap, the newest");
        s.ack("laptop", 4).unwrap();
        assert_eq!(s.after("laptop", 0, 100).unwrap().len(), 2);
        assert_eq!(s.last_seq("laptop").unwrap(), 7);
        // By bytes too.
        let caps = Caps { frames: 100, bytes: 10, keep_ms: WEEK_MS };
        let item = |id| Item { host: "desk", id, kind: "message", sender: "studio", line: "123456" };
        let (_, d) = s.enqueue(item("a"), 1, &caps).unwrap();
        assert_eq!(d, []);
        let (_, d) = s.enqueue(item("b"), 1, &caps).unwrap();
        assert_eq!(d.len(), 1);
        assert!(!s.seen("x", 1).unwrap() && s.seen("x", 2).unwrap());
        let old = s.prune(WEEK_MS as i64 + 10, WEEK_MS).unwrap();
        assert!(old.iter().any(|d| d.id == "f4"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
