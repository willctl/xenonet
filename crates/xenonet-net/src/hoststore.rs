//! A host's own store, `<state>/network/net.sqlite` (docs/design/
//! network.md, section 9): the outbox, the frame ids seen, and the epoch
//! and seq of the gateway's stream.
//!
//! The outbox keeps a frame until the gateway accepts it. The gateway then
//! keeps it, for a week, until the host it's for has read it; a frame it
//! lets go is a `gap`, both to that host and to the sender, and the
//! engine (through `Sink::gap`) sends again the messages that have no
//! `stored` receipt, which the receiver drops as repeats by `mid`. So the
//! outbox needn't wait for receipts itself: the engine keeps its own sent
//! messages, and the network says when to send them again.
//!
//! Presence, claims and relay frames never wait here: a full set goes when
//! the link is back. Past 10,000 frames or 50 MB, or a week, the oldest go,
//! and the doctor says so.

use crate::gwstore::{Caps, WEEK_MS};
use crate::R;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

pub struct HostStore {
    db: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS outbox (n INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, host TEXT NOT NULL, kind TEXT NOT NULL, line TEXT NOT NULL, size INTEGER NOT NULL, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS seen (id TEXT PRIMARY KEY, at INTEGER NOT NULL);
";

/// How full the outbox is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Size {
    pub frames: u64,
    pub bytes: u64,
    pub oldest: Option<i64>,
}

fn u(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl HostStore {
    pub fn open(file: &Path) -> R<HostStore> {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let db = Connection::open(file)?;
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "synchronous", "NORMAL")?;
        db.execute_batch(SCHEMA)?;
        Ok(HostStore { db })
    }

    pub fn meta(&self, k: &str) -> R<Option<String>> {
        Ok(self.db.query_row("SELECT v FROM meta WHERE k = ?1", [k], |r| r.get(0)).optional()?)
    }

    pub fn set_meta(&self, k: &str, v: &str) -> R<()> {
        self.db.execute("INSERT INTO meta (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v", [k, v])?;
        Ok(())
    }

    /// The gateway's epoch this host last saw, and the last seq in it.
    pub fn cursor(&self) -> R<(Option<String>, u64)> {
        let seq = self.meta("seq")?.and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((self.meta("epoch")?, seq))
    }

    pub fn set_cursor(&self, epoch: &str, seq: u64) -> R<()> {
        self.set_meta("epoch", epoch)?;
        self.set_meta("seq", &seq.to_string())
    }

    /// Puts a frame in the outbox; the oldest go past the caps: how many.
    pub fn push(&self, id: &str, host: &str, kind: &str, line: &str, now: i64, caps: &Caps) -> R<u64> {
        self.db.execute(
            "INSERT OR IGNORE INTO outbox (id, host, kind, line, size, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, host, kind, line, i64::try_from(line.len()).unwrap_or(i64::MAX), now],
        )?;
        let mut dropped = self.db.execute("DELETE FROM outbox WHERE at < ?1", [now - caps.keep_ms as i64])? as u64;
        loop {
            let s = self.size()?;
            if s.frames <= caps.frames && s.bytes <= caps.bytes {
                break;
            }
            if self.db.execute("DELETE FROM outbox WHERE n = (SELECT MIN(n) FROM outbox)", [])? == 0 {
                break;
            }
            dropped += 1;
        }
        if dropped > 0 {
            let before = self.meta("dropped")?.and_then(|d| d.parse::<u64>().ok()).unwrap_or(0);
            self.set_meta("dropped", &(before + dropped).to_string())?;
            self.set_meta("droppedAt", &now.to_string())?;
        }
        Ok(dropped)
    }

    /// The oldest frames, in order, up to `max_bytes` (at least one) and
    /// `max_frames`: (id, line).
    pub fn batch(&self, max_bytes: u64, max_frames: u64) -> R<Vec<(String, String)>> {
        let mut st = self.db.prepare("SELECT id, line, size FROM outbox ORDER BY n LIMIT ?1")?;
        let rows = st.query_map([i64::try_from(max_frames).unwrap_or(i64::MAX)], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, u(r.get(2)?))))?;
        let mut out = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let (id, line, size) = row?;
            if !out.is_empty() && bytes + size > max_bytes {
                break;
            }
            bytes += size;
            out.push((id, line));
        }
        Ok(out)
    }

    /// The gateway took these (or refused them for good): they go.
    pub fn done(&self, ids: &[String]) -> R<()> {
        let tx = self.db.unchecked_transaction()?;
        for id in ids {
            tx.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn size(&self) -> R<Size> {
        let (frames, bytes, oldest): (i64, i64, Option<i64>) =
            self.db.query_row("SELECT COUNT(*), COALESCE(SUM(size), 0), MIN(at) FROM outbox", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(Size { frames: u(frames), bytes: u(bytes), oldest })
    }

    /// Frames for `host` go: it was removed.
    pub fn forget_host(&self, host: &str) -> R<()> {
        self.db.execute("DELETE FROM outbox WHERE host = ?1", [host])?;
        Ok(())
    }

    /// Records a frame id: true when it was seen before, so the frame is a
    /// repeat.
    pub fn seen(&self, id: &str, now: i64) -> R<bool> {
        Ok(self.db.execute("INSERT OR IGNORE INTO seen (id, at) VALUES (?1, ?2)", params![id, now])? == 0)
    }

    /// Ids older than a week are forgotten.
    pub fn prune(&self, now: i64) -> R<()> {
        self.db.execute("DELETE FROM seen WHERE at < ?1", [now - WEEK_MS as i64])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outbox_keeps_frames_until_they_are_done_within_its_caps() {
        let dir = std::env::temp_dir().join(format!("xenonet-net-hoststore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s = HostStore::open(&dir.join("net.sqlite")).unwrap();
        let caps = Caps { frames: 3, bytes: 1 << 20, keep_ms: WEEK_MS };
        for i in 0..4 {
            s.push(&format!("f{i}"), "laptop", "message", "12345", i, &caps).unwrap();
        }
        assert_eq!(s.size().unwrap(), Size { frames: 3, bytes: 15, oldest: Some(1) });
        assert_eq!(s.meta("dropped").unwrap().as_deref(), Some("1"));
        let b = s.batch(10, 100).unwrap();
        assert_eq!(b.iter().map(|(i, _)| i.as_str()).collect::<Vec<_>>(), ["f1", "f2"]);
        assert_eq!(s.batch(1, 100).unwrap().len(), 1, "at least one");
        s.done(&["f1".into(), "f2".into()]).unwrap();
        assert_eq!(s.size().unwrap().frames, 1);
        assert!(!s.seen("x", 1).unwrap());
        assert!(s.seen("x", 1).unwrap());
        s.set_cursor("e1", 9).unwrap();
        assert_eq!(s.cursor().unwrap(), (Some("e1".into()), 9));
        let _ = std::fs::remove_dir_all(dir);
    }
}
