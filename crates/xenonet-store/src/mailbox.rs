//! What this machine keeps as a mailbox of its networks (xenonet-net's
//! `mailbox`): sealed envelopes for members that are away, in one queue
//! for each (network, recipient), oldest first, each by its entry's id,
//! with when it was kept. The queue's caps, what a full one lets go and
//! when a kept envelope is handed over are the network crate's; this is
//! where they're kept, so a mailbox that restarts hands over the rest.

use crate::{Db, R};
use rusqlite::params;

pub(crate) const TABLES: &[&str] = &[
    // The envelopes kept for each away member, in the order they came.
    "CREATE TABLE mailbox (
        n INTEGER PRIMARY KEY AUTOINCREMENT,
        network TEXT NOT NULL,
        recipient TEXT NOT NULL,
        author TEXT NOT NULL,
        id TEXT NOT NULL,
        kept INTEGER NOT NULL,
        envelope BLOB NOT NULL,
        UNIQUE (network, recipient, id)
    ) STRICT",
    "CREATE INDEX mailbox_kept ON mailbox (kept)",
];

/// One envelope a mailbox keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
    pub network: String,
    pub recipient: String,
    pub author: String,
    pub id: String,
    pub kept: u64,
    pub envelope: Vec<u8>,
}

type Row = (String, String, String, String, i64, Vec<u8>);

fn mail_of((network, recipient, author, id, kept, envelope): Row) -> Mail {
    Mail { network, recipient, author, id, kept: u64::try_from(kept).unwrap_or(0), envelope }
}

fn int(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

impl Db {
    /// Keeps `m` at the end of its queue, unless one with its id is there,
    /// after letting go the `evict` oldest of the queue: both or neither.
    /// Whether it was new.
    pub fn put_mail(&self, m: &Mail, evict: usize) -> R<bool> {
        self.write(|c| {
            let had: bool =
                c.query_row("SELECT EXISTS (SELECT 1 FROM mailbox WHERE network = ?1 AND recipient = ?2 AND id = ?3)", params![m.network, m.recipient, m.id], |r| r.get(0))?;
            if had {
                return Ok(false);
            }
            c.execute(
                "DELETE FROM mailbox WHERE n IN (SELECT n FROM mailbox WHERE network = ?1 AND recipient = ?2 ORDER BY n LIMIT ?3)",
                params![m.network, m.recipient, i64::try_from(evict).unwrap_or(i64::MAX)],
            )?;
            c.execute(
                "INSERT INTO mailbox (network, recipient, author, id, kept, envelope) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![m.network, m.recipient, m.author, m.id, int(m.kept), m.envelope],
            )?;
            Ok(true)
        })
    }

    /// The queue for `recipient` in `network`, oldest first, at most `limit`.
    pub fn mail_for(&self, network: &str, recipient: &str, limit: usize) -> R<Vec<Mail>> {
        self.read(|c| {
            let mut q = c.prepare("SELECT network, recipient, author, id, kept, envelope FROM mailbox WHERE network = ?1 AND recipient = ?2 ORDER BY n LIMIT ?3")?;
            let rows = q
                .query_map(params![network, recipient, i64::try_from(limit).unwrap_or(i64::MAX)], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))?
                .collect::<Result<Vec<Row>, _>>()?;
            Ok(rows.into_iter().map(mail_of).collect())
        })
    }

    /// The sizes of what the queue for `recipient` in `network` holds,
    /// oldest first.
    pub fn mail_sizes(&self, network: &str, recipient: &str) -> R<Vec<u64>> {
        self.read(|c| {
            let mut q = c.prepare("SELECT length(envelope) FROM mailbox WHERE network = ?1 AND recipient = ?2 ORDER BY n")?;
            let rows = q.query_map(params![network, recipient], |r| r.get::<_, i64>(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows.into_iter().map(|n| u64::try_from(n).unwrap_or(0)).collect())
        })
    }

    /// The recipient took these.
    pub fn mail_took(&self, network: &str, recipient: &str, ids: &[String]) -> R<()> {
        self.write(|c| {
            for id in ids {
                c.execute("DELETE FROM mailbox WHERE network = ?1 AND recipient = ?2 AND id = ?3", params![network, recipient, id])?;
            }
            Ok(())
        })
    }

    /// Lets go what was kept before `before` (ms): how many went.
    pub fn prune_mail(&self, before: u64) -> R<u64> {
        self.write(|c| Ok(c.execute("DELETE FROM mailbox WHERE kept < ?1", params![int(before)])? as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, to: &str, kept: u64) -> Mail {
        Mail { network: "net_a".into(), recipient: to.into(), author: "xns1_a".into(), id: id.into(), kept, envelope: vec![1; 10] }
    }

    #[test]
    fn a_queue_keeps_its_order_lets_its_oldest_go_and_each_id_once() {
        let db = Db::memory().unwrap();
        for (i, id) in ["e1", "e2", "e3"].into_iter().enumerate() {
            assert!(db.put_mail(&mail(id, "xns1_b", i as u64), 0).unwrap());
        }
        assert!(!db.put_mail(&mail("e2", "xns1_b", 9), 0).unwrap(), "once");
        assert!(db.put_mail(&mail("e4", "xns1_b", 3), 2).unwrap());
        db.put_mail(&mail("e1", "xns1_c", 3), 0).unwrap();
        let ids = |to: &str| db.mail_for("net_a", to, 10).unwrap().into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids("xns1_b"), ["e3", "e4"], "the two oldest went");
        assert_eq!(db.mail_sizes("net_a", "xns1_b").unwrap(), [10, 10]);
        db.mail_took("net_a", "xns1_b", &["e3".into()]).unwrap();
        assert_eq!(ids("xns1_b"), ["e4"]);
        assert_eq!(db.prune_mail(3).unwrap(), 0);
        assert_eq!(db.prune_mail(4).unwrap(), 2);
        assert_eq!(ids("xns1_c"), [] as [String; 0]);
    }
}
