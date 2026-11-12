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
//! never reopens one. One for any machine (`target` is `@any`) is given
//! to one machine once (`assign_intent`), and never to another after: a
//! stuck one ends with the person's cancel.
//!
//! Beside them: the takes machines sent for intents for any machine, in
//! the order the store took them, which the arbiter goes by; and the
//! person's signed confirmations of this machine's checkouts, which its
//! takes carry. Its goal, its last note and its outcome are words the
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
    // made with, so the target decides it again after a restart; when it
    // was made and ended; the arbiter, whose assign alone gives it to a
    // machine; and for one for any machine, the machine it was given to.
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
        ended_at INTEGER,
        arbiter TEXT NOT NULL,
        assigned TEXT
    ) STRICT",
    "CREATE INDEX intents_by_target ON intents (target, state)",
    // A machine's take of an intent for any machine: what it sent as its
    // evidence (the person's confirmation of its checkout), in the order
    // the store took them (n).
    "CREATE TABLE intent_takes (
        n INTEGER PRIMARY KEY,
        intent TEXT NOT NULL,
        machine TEXT NOT NULL,
        evidence TEXT NOT NULL,
        at INTEGER NOT NULL,
        UNIQUE (intent, machine)
    ) STRICT",
    // An assign that came before its intent did (the arbiter's feed and the
    // maker's travel apart): held until the intent comes, then decided.
    "CREATE TABLE intent_early_assigns (
        intent TEXT NOT NULL,
        host TEXT NOT NULL,
        target TEXT NOT NULL,
        at INTEGER NOT NULL,
        PRIMARY KEY (intent, host)
    ) STRICT",
    // The person's word, signed by their device here, that this machine's
    // checkout of a project is theirs: by project.
    "CREATE TABLE checkout_confirmations (
        project TEXT PRIMARY KEY,
        device TEXT NOT NULL,
        at INTEGER NOT NULL,
        sig TEXT NOT NULL
    ) STRICT",
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
    /// The machine it's worked on, by its host id, or `@any`
    /// (`xenonet_authz::ANY_MACHINE`) for any its delegation names.
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
    /// The machine whose assign alone gives it to a machine.
    pub arbiter: String,
    /// For one for any machine: the machine it was given to.
    pub assigned: Option<String>,
}

/// What an intent for any machine names in place of one.
pub const ANY: &str = "@any";

impl IntentRow {
    /// Neither done, failed, cancelled nor expired.
    pub fn open(&self) -> bool {
        !ENDED.contains(&self.state.as_str())
    }

    /// Whether it's for any machine its delegation names.
    pub fn any(&self) -> bool {
        self.target == ANY
    }

    /// The machine it's worked on: the one it names, or the one an intent
    /// for any machine was given to (None until then).
    pub fn worked_on(&self) -> Option<&str> {
        if self.any() {
            self.assigned.as_deref()
        } else {
            Some(&self.target)
        }
    }
}

/// A machine's take of an intent for any machine, as it came.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Take {
    pub machine: String,
    pub evidence: String,
    pub at: i64,
}

/// The person's confirmation of this machine's checkout of a project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confirmation {
    pub project: String,
    pub device: String,
    pub at: i64,
    pub sig: String,
}

const COLUMNS: &str =
    "id, network, person, device, machine, delegation, project, app, target, goal, until, state, agent, note, outcome, nonce, authority, made_at, ended_at, arbiter, assigned";

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
        arbiter: r.get(19)?,
        assigned: r.get(20)?,
    })
}

impl Db {
    /// An intent heard of for the first time; Ok(false) when the store has
    /// one by that id already, which stays as it is (an intent is made
    /// once: its id is its act's hash).
    pub fn put_intent(&self, i: &IntentRow) -> R<bool> {
        self.write(|c| {
            let n = c.execute(
                &format!("INSERT OR IGNORE INTO intents ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)"),
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
                    i.ended_at,
                    i.arbiter,
                    i.assigned
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

    /// Gives an intent for any machine to `machine`, at `at`: Ok(false)
    /// when it isn't one, was given already (to it or another: never
    /// again), or isn't `made` any more.
    pub fn assign_intent(&self, id: &str, machine: &str, at: i64) -> R<bool> {
        let _ = at;
        self.write(|c| {
            let n =
                c.execute("UPDATE intents SET state = 'assigned', assigned = ?1 WHERE id = ?2 AND target = ?3 AND assigned IS NULL AND state = 'made'", params![machine, id, ANY])?;
            Ok(n == 1)
        })
    }

    /// `machine`'s take of intent `id`, kept once; Ok(false) when it took
    /// it before.
    pub fn add_take(&self, id: &str, machine: &str, evidence: &str, at: i64) -> R<bool> {
        self.write(|c| {
            let n = c.execute("INSERT OR IGNORE INTO intent_takes (intent, machine, evidence, at) VALUES (?1, ?2, ?3, ?4)", params![id, machine, evidence, at])?;
            Ok(n == 1)
        })
    }

    /// The takes of intent `id`, in the order the store took them.
    pub fn takes(&self, id: &str) -> R<Vec<Take>> {
        self.read(|c| {
            let mut q = c.prepare_cached("SELECT machine, evidence, at FROM intent_takes WHERE intent = ?1 ORDER BY n")?;
            let rows = q.query_map(params![id], |r| Ok(Take { machine: r.get(0)?, evidence: r.get(1)?, at: r.get(2)? }))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// An assign of intent `id` from `host` that came before the intent:
    /// held, the first from each host.
    pub fn hold_assign(&self, id: &str, host: &str, target: &str, at: i64) -> R<()> {
        self.write(|c| {
            c.execute("INSERT OR IGNORE INTO intent_early_assigns (intent, host, target, at) VALUES (?1, ?2, ?3, ?4)", params![id, host, target, at])?;
            Ok(())
        })
    }

    /// The assigns held for intent `id`, each (host, target), and gone
    /// from the store.
    pub fn take_held_assigns(&self, id: &str) -> R<Vec<(String, String)>> {
        self.write(|c| {
            let rows = {
                let mut q = c.prepare_cached("SELECT host, target FROM intent_early_assigns WHERE intent = ?1 ORDER BY at, host")?;
                let rows = q.query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
                rows
            };
            c.execute("DELETE FROM intent_early_assigns WHERE intent = ?1", params![id])?;
            Ok(rows)
        })
    }

    /// The person's confirmation of this machine's checkout of a project,
    /// in place of one before.
    pub fn put_confirmation(&self, c: &Confirmation) -> R<()> {
        self.write(|db| {
            db.execute(
                "INSERT INTO checkout_confirmations (project, device, at, sig) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (project) DO UPDATE SET device = ?2, at = ?3, sig = ?4",
                params![c.project, c.device, c.at, c.sig],
            )?;
            Ok(())
        })
    }

    pub fn confirmation(&self, project: &str) -> R<Option<Confirmation>> {
        self.read(|c| {
            Ok(c.query_row("SELECT project, device, at, sig FROM checkout_confirmations WHERE project = ?1", params![project], |r| {
                Ok(Confirmation { project: r.get(0)?, device: r.get(1)?, at: r.get(2)?, sig: r.get(3)? })
            })
            .optional()?)
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
            arbiter: "laptop".into(),
            assigned: None,
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
    fn an_intent_for_any_machine_is_given_to_one_once_and_takes_keep_their_order() {
        let db = Db::memory().unwrap();
        let any = IntentRow { target: ANY.into(), ..made("int_any") };
        assert!(db.put_intent(&any).unwrap());
        assert_eq!(db.intent("int_any").unwrap().unwrap().worked_on(), None);
        assert!(!db.assign_intent("int_a", "desk", 1).unwrap(), "no such intent");
        assert!(db.put_intent(&made("int_a")).unwrap());
        assert!(!db.assign_intent("int_a", "studio", 1).unwrap(), "one for a named machine isn't given away");
        assert!(db.add_take("int_any", "studio", "{}", 1_100).unwrap());
        assert!(db.add_take("int_any", "desk", "{}", 1_050).unwrap());
        assert!(!db.add_take("int_any", "studio", "{\"again\":1}", 1_200).unwrap(), "a take is kept once");
        assert_eq!(db.takes("int_any").unwrap().iter().map(|t| t.machine.as_str()).collect::<Vec<_>>(), ["studio", "desk"], "in the order taken");
        assert!(db.assign_intent("int_any", "studio", 1_300).unwrap());
        assert!(!db.assign_intent("int_any", "desk", 1_400).unwrap(), "no reassignment");
        let i = db.intent("int_any").unwrap().unwrap();
        assert_eq!((i.state.as_str(), i.worked_on()), ("assigned", Some("studio")));
        db.put_confirmation(&Confirmation { project: "prj_a".into(), device: "xns1_d".into(), at: 1, sig: "s1".into() }).unwrap();
        db.put_confirmation(&Confirmation { project: "prj_a".into(), device: "xns1_d".into(), at: 2, sig: "s2".into() }).unwrap();
        assert_eq!(db.confirmation("prj_a").unwrap().map(|c| (c.at, c.sig)), Some((2, "s2".into())));
        assert_eq!(db.confirmation("prj_b").unwrap(), None);
        db.hold_assign("int_c", "desk", "studio", 5).unwrap();
        db.hold_assign("int_c", "desk", "laptop", 6).unwrap();
        assert_eq!(db.take_held_assigns("int_c").unwrap(), [("desk".to_string(), "studio".to_string())], "the first from each host");
        assert_eq!(db.take_held_assigns("int_c").unwrap(), Vec::<(String, String)>::new(), "taken once");
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
