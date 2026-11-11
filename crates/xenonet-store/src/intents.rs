//! Intents (docs/design/goals.md): work the person asks for on one device,
//! worked on the machine it names under one of their delegations, and
//! what came of it. Every machine of the network keeps the intents it
//! hears of, from the maker's feed, and the reports on them from the
//! target's; what may change one is the runtime's to decide
//! (xenonet-authz), never the store's.
//!
//! An intent moves through `made` → `assigned` → `running` → `done`,
//! `failed`, `cancelled` or `expired`, and only forward: a move names the
//! states it may come from (`move_intent`), so a report that arrives late
//! never reopens one. Its goal, its last note and its outcome are words the
//! store keeps and shows; the runtime reads them as information, never
//! as authority.
//!
//! Beside them, the intents an actor's grant may read and report on, by
//! its token's first revocation id (`grant_intents`).

use crate::{Db, R};
use rusqlite::{params, OptionalExtension, Row};

pub(crate) const TABLES: &[&str] = &[
    // An intent: its id (the hash of the act that made it), the network it
    // travels in, the person and device that signed it, the machine that
    // device is on (the maker), the delegation it runs under, the project
    // and app it's worked in, the machine it's worked on (the target), its
    // goal and when it ends; how it stands, the agent working it, its last
    // note and its outcome; the nonce and the person's signature it was
    // made with, so the target decides it again after a restart; and when
    // it was made and ended.
    "CREATE TABLE intents (
        id TEXT PRIMARY KEY,
        network TEXT NOT NULL,
        person TEXT NOT NULL,
        device TEXT NOT NULL,
        machine TEXT NOT NULL,
        delegation TEXT NOT NULL,
        project TEXT NOT NULL,
        app TEXT NOT NULL,
        target TEXT NOT NULL,
        goal TEXT NOT NULL,
        until INTEGER NOT NULL,
        state TEXT NOT NULL,
        agent TEXT,
        note TEXT,
        outcome TEXT,
        nonce TEXT NOT NULL,
        authority TEXT NOT NULL,
        made_at INTEGER NOT NULL,
        ended_at INTEGER
    ) STRICT",
    "CREATE INDEX intents_by_target ON intents (target, state)",
    // The intents an actor's grant may read and report on, by its token's
    // first revocation id.
    "CREATE TABLE grant_intents (
        token TEXT NOT NULL,
        intent TEXT NOT NULL,
        PRIMARY KEY (token, intent)
    ) STRICT",
];

/// The states an intent ends in.
pub const ENDED: [&str; 4] = ["done", "failed", "cancelled", "expired"];

/// An intent as the store keeps it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentRow {
    pub id: String,
    pub network: String,
    pub person: String,
    pub device: String,
    /// The maker: the machine of the device that signed it, by its host id.
    pub machine: String,
    pub delegation: String,
    pub project: String,
    pub app: String,
    /// The machine it's worked on, by its host id.
    pub target: String,
    pub goal: String,
    pub until: i64,
    pub state: String,
    pub agent: Option<String>,
    pub note: Option<String>,
    pub outcome: Option<String>,
    pub nonce: String,
    /// The person's signature as it came (`{ person: { person, device,
    /// sig }, personHead }`).
    pub authority: String,
    pub made_at: i64,
    pub ended_at: Option<i64>,
}

impl IntentRow {
    /// Neither done, failed, cancelled nor expired.
    pub fn open(&self) -> bool {
        !ENDED.contains(&self.state.as_str())
    }
}

const COLUMNS: &str = "id, network, person, device, machine, delegation, project, app, target, goal, until, state, agent, note, outcome, nonce, authority, made_at, ended_at";

fn row_of(r: &Row<'_>) -> rusqlite::Result<IntentRow> {
    Ok(IntentRow {
        id: r.get(0)?,
        network: r.get(1)?,
        person: r.get(2)?,
        device: r.get(3)?,
        machine: r.get(4)?,
        delegation: r.get(5)?,
        project: r.get(6)?,
        app: r.get(7)?,
        target: r.get(8)?,
        goal: r.get(9)?,
        until: r.get(10)?,
        state: r.get(11)?,
        agent: r.get(12)?,
        note: r.get(13)?,
        outcome: r.get(14)?,
        nonce: r.get(15)?,
        authority: r.get(16)?,
        made_at: r.get(17)?,
        ended_at: r.get(18)?,
    })
}

impl Db {
    /// An intent heard of for the first time; Ok(false) when the store has
    /// one by that id already, which stays as it is (an intent is made
    /// once: its id is its act's hash).
    pub fn put_intent(&self, i: &IntentRow) -> R<bool> {
        self.write(|c| {
            let n = c.execute(
                &format!("INSERT OR IGNORE INTO intents ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)"),
                params![
                    i.id,
                    i.network,
                    i.person,
                    i.device,
                    i.machine,
                    i.delegation,
                    i.project,
                    i.app,
                    i.target,
                    i.goal,
                    i.until,
                    i.state,
                    i.agent,
                    i.note,
                    i.outcome,
                    i.nonce,
                    i.authority,
                    i.made_at,
                    i.ended_at
                ],
            )?;
            Ok(n == 1)
        })
    }

    pub fn intent(&self, id: &str) -> R<Option<IntentRow>> {
        self.read(|c| Ok(c.query_row(&format!("SELECT {COLUMNS} FROM intents WHERE id = ?1"), params![id], row_of).optional()?))
    }

    /// Every intent this machine keeps, the newest first.
    pub fn intents(&self) -> R<Vec<IntentRow>> {
        self.read(|c| {
            let mut q = c.prepare_cached(&format!("SELECT {COLUMNS} FROM intents ORDER BY made_at DESC, id"))?;
            let rows = q.query_map([], row_of)?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Moves an intent to `to` from one of `from`, at `at` (its end, when
    /// `to` is one); Ok(false) when it isn't in one of them, which leaves
    /// it as it is.
    pub fn move_intent(&self, id: &str, from: &[&str], to: &str, at: i64) -> R<bool> {
        self.write(|c| {
            let ended = ENDED.contains(&to).then_some(at);
            let mut moved = 0;
            for f in from {
                moved += c.execute("UPDATE intents SET state = ?1, ended_at = COALESCE(?2, ended_at) WHERE id = ?3 AND state = ?4", params![to, ended, id, f])?;
            }
            Ok(moved > 0)
        })
    }

    /// The agent working an intent, once its run started.
    pub fn set_intent_agent(&self, id: &str, agent: &str) -> R<()> {
        self.write(|c| {
            c.execute("UPDATE intents SET agent = ?1 WHERE id = ?2", params![agent, id])?;
            Ok(())
        })
    }

    /// An intent's last note.
    pub fn set_intent_note(&self, id: &str, note: &str) -> R<()> {
        self.write(|c| {
            c.execute("UPDATE intents SET note = ?1 WHERE id = ?2", params![note, id])?;
            Ok(())
        })
    }

    /// What came of an intent.
    pub fn set_intent_outcome(&self, id: &str, outcome: &str) -> R<()> {
        self.write(|c| {
            c.execute("UPDATE intents SET outcome = ?1 WHERE id = ?2", params![outcome, id])?;
            Ok(())
        })
    }

    /// The intents still open whose time is over at `now`.
    pub fn intents_past(&self, now: i64) -> R<Vec<IntentRow>> {
        self.read(|c| {
            let mut q = c.prepare_cached(&format!("SELECT {COLUMNS} FROM intents WHERE until < ?1 AND state NOT IN ('done', 'failed', 'cancelled', 'expired')"))?;
            let rows = q.query_map(params![now], row_of)?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The intents a grant may read and report on, in place of those
    /// before.
    pub fn set_grant_intents(&self, token: &str, intents: &[String]) -> R<()> {
        self.write(|c| {
            c.execute("DELETE FROM grant_intents WHERE token = ?1", params![token])?;
            for i in intents {
                c.execute("INSERT OR IGNORE INTO grant_intents (token, intent) VALUES (?1, ?2)", params![token, i])?;
            }
            Ok(())
        })
    }

    /// The intents a grant may read and report on; None when it names
    /// none.
    pub fn grant_intents(&self, token: &str) -> R<Option<Vec<String>>> {
        let ids: Vec<String> = self.read(|c| {
            let mut q = c.prepare_cached("SELECT intent FROM grant_intents WHERE token = ?1 ORDER BY intent")?;
            let rows = q.query_map(params![token], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        Ok((!ids.is_empty()).then_some(ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(id: &str) -> IntentRow {
        IntentRow {
            id: id.into(),
            network: "net_a".into(),
            person: "prs_a".into(),
            device: "xns1_d".into(),
            machine: "laptop".into(),
            delegation: "dlg_0123456789abcdef".into(),
            project: "prj_a".into(),
            app: "claude".into(),
            target: "desk".into(),
            goal: "fix the build".into(),
            until: 2_000,
            state: "made".into(),
            agent: None,
            note: None,
            outcome: None,
            nonce: "bm9uY2U".into(),
            authority: "{}".into(),
            made_at: 1_000,
            ended_at: None,
        }
    }

    #[test]
    fn an_intent_is_kept_once_and_moves_only_forward() {
        let db = Db::memory().unwrap();
        assert!(db.put_intent(&made("int_a")).unwrap());
        assert!(!db.put_intent(&IntentRow { goal: "another goal".into(), ..made("int_a") }).unwrap(), "made once, by its id");
        assert_eq!(db.intent("int_a").unwrap().unwrap().goal, "fix the build");
        assert!(db.move_intent("int_a", &["made"], "assigned", 1_100).unwrap());
        assert!(!db.move_intent("int_a", &["made"], "assigned", 1_100).unwrap(), "not from where it is");
        assert!(db.move_intent("int_a", &["assigned"], "running", 1_200).unwrap());
        db.set_intent_agent("int_a", "agt_a").unwrap();
        db.set_intent_note("int_a", "half way").unwrap();
        assert!(db.move_intent("int_a", &["assigned", "running"], "cancelled", 1_300).unwrap());
        assert!(!db.move_intent("int_a", &["assigned", "running"], "done", 1_400).unwrap(), "a late done doesn't reopen it");
        let i = db.intent("int_a").unwrap().unwrap();
        assert_eq!((i.state.as_str(), i.agent.as_deref(), i.note.as_deref(), i.ended_at, i.open()), ("cancelled", Some("agt_a"), Some("half way"), Some(1_300), false));
        assert!(db.put_intent(&made("int_b")).unwrap());
        assert_eq!(db.intents_past(2_001).unwrap().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["int_b"], "the ended one isn't");
        assert_eq!(db.intents().unwrap().len(), 2);
    }

    #[test]
    fn a_grant_names_the_intents_it_reports_on() {
        let db = Db::memory().unwrap();
        assert_eq!(db.grant_intents("t1").unwrap(), None);
        db.set_grant_intents("t1", &["int_b".into(), "int_a".into()]).unwrap();
        assert_eq!(db.grant_intents("t1").unwrap(), Some(vec!["int_a".to_string(), "int_b".to_string()]));
        db.set_grant_intents("t1", &[]).unwrap();
        assert_eq!(db.grant_intents("t1").unwrap(), None);
    }
}
