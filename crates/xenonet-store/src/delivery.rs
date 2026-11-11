//! What delivery keeps, in the store: its policy's settings (the pause
//! switch and the rest) and the sessions you consented to wake, the wake
//! budget's times, so a restart doesn't hand every sender a fresh one, its
//! audit log (the last `AUDIT_LINES`), the headless runs this machine
//! started, so a restart keeps counting the ones under way, and the
//! sessions started under the person's delegation, so their prompts are
//! judged against it after a restart too.

use crate::{Db, StoreError, R};
use rusqlite::{params, OptionalExtension};
use serde_json::{Map, Value};
use xenonet_text::json::{parse, stringify};

/// The audit log's lines kept, the oldest going first.
pub const AUDIT_LINES: i64 = 10_000;

pub(crate) const TABLES: &[&str] = &[
    // The policy's settings, each a JSON value by its name: `paused`, and
    // whatever else it keeps but consent.
    "CREATE TABLE delivery_settings (k TEXT PRIMARY KEY, v TEXT NOT NULL) STRICT",
    // The sessions you consented to wake, with the terms of each.
    "CREATE TABLE consent (session TEXT PRIMARY KEY, terms TEXT NOT NULL) STRICT",
    // When each wake the budget counts was taken, by its budget's key.
    "CREATE TABLE wake_budget (key TEXT NOT NULL, at REAL NOT NULL) STRICT",
    "CREATE INDEX wake_budget_by_key ON wake_budget (key)",
    // The audit log, its lines in order.
    "CREATE TABLE delivery_audit (n INTEGER PRIMARY KEY, line TEXT NOT NULL) STRICT",
    // The headless runs started here, newest last.
    "CREATE TABLE runs (
        id TEXT PRIMARY KEY,
        key TEXT NOT NULL,
        app TEXT NOT NULL,
        pid REAL,
        at REAL NOT NULL,
        end_at REAL,
        exit REAL,
        ms REAL,
        timed_out INTEGER NOT NULL
    ) STRICT",
    // The sessions started under the person's delegation: the folder each
    // started in, and the machine it was started from (NULL: this one).
    "CREATE TABLE delegated_runs (
        session TEXT PRIMARY KEY,
        cwd TEXT NOT NULL,
        from_machine TEXT,
        at REAL NOT NULL
    ) STRICT",
];

/// A headless run, as the store keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunRow {
    pub id: String,
    pub key: String,
    pub app: String,
    pub pid: Option<f64>,
    pub at: f64,
    pub end: Option<f64>,
    pub exit: Option<f64>,
    pub ms: Option<f64>,
    pub timed_out: bool,
}

/// A session started under the person's delegation.
#[derive(Clone, Debug, PartialEq)]
pub struct Delegated {
    pub cwd: String,
    /// The machine it was started from; None for this one.
    pub from: Option<String>,
    pub at: f64,
}

// Rows of a key and its JSON text.
type Pairs = Vec<(String, String)>;

fn value_of(text: &str) -> R<Value> {
    parse(text).map_err(|_| StoreError::new("bad-entry", format!("a delivery value isn't JSON: {text}")))
}

impl Db {
    /// The policy's data as it was last kept (`{ paused, consent, … }`), or
    /// None when nothing was.
    pub fn delivery_policy(&self) -> R<Option<Map<String, Value>>> {
        let (settings, consent): (Pairs, Pairs) = self.read(|c| {
            let mut q = c.prepare_cached("SELECT k, v FROM delivery_settings ORDER BY rowid")?;
            let s = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            let mut q = c.prepare_cached("SELECT session, terms FROM consent ORDER BY rowid")?;
            let k = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok((s, k))
        })?;
        if settings.is_empty() && consent.is_empty() {
            return Ok(None);
        }
        let mut out = Map::new();
        for (k, v) in settings {
            out.insert(k, value_of(&v)?);
        }
        let mut terms = Map::new();
        for (s, t) in consent {
            terms.insert(s, value_of(&t)?);
        }
        out.insert("consent".into(), Value::Object(terms));
        Ok(Some(out))
    }

    /// The policy's data, in place of what was kept: its consent as rows,
    /// the rest as settings, in one write.
    pub fn put_delivery_policy(&self, data: &Map<String, Value>) -> R<()> {
        self.write(|c| {
            c.execute_batch("DELETE FROM delivery_settings; DELETE FROM consent;")?;
            for (k, v) in data {
                if k == "consent" {
                    for (session, terms) in v.as_object().into_iter().flatten() {
                        c.execute("INSERT INTO consent (session, terms) VALUES (?1, ?2)", params![session, stringify(terms, 0)])?;
                    }
                } else {
                    c.execute("INSERT INTO delivery_settings (k, v) VALUES (?1, ?2)", params![k, stringify(v, 0)])?;
                }
            }
            Ok(())
        })
    }

    /// The wake budget's times, by its key.
    pub fn wake_budget(&self) -> R<Vec<(String, Vec<f64>)>> {
        let rows: Vec<(String, f64)> = self.read(|c| {
            let mut q = c.prepare_cached("SELECT key, at FROM wake_budget ORDER BY rowid")?;
            let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        let mut out: Vec<(String, Vec<f64>)> = Vec::new();
        for (k, at) in rows {
            match out.iter_mut().find(|(have, _)| *have == k) {
                Some((_, times)) => times.push(at),
                None => out.push((k, vec![at])),
            }
        }
        Ok(out)
    }

    /// The wake budget's times, in place of what was kept.
    pub fn put_wake_budget(&self, budget: &[(String, Vec<f64>)]) -> R<()> {
        self.write(|c| {
            c.execute_batch("DELETE FROM wake_budget")?;
            for (k, times) in budget {
                for at in times {
                    c.execute("INSERT INTO wake_budget (key, at) VALUES (?1, ?2)", params![k, at])?;
                }
            }
            Ok(())
        })
    }

    /// A line of the audit log; past `AUDIT_LINES`, the oldest go.
    pub fn add_delivery_audit(&self, line: &Value) -> R<()> {
        self.write(|c| {
            c.execute("INSERT INTO delivery_audit (line) VALUES (?1)", params![stringify(line, 0)])?;
            let n = c.last_insert_rowid();
            c.execute("DELETE FROM delivery_audit WHERE n <= ?1", params![n - AUDIT_LINES])?;
            Ok(())
        })
    }

    /// The audit log's lines kept, oldest first.
    pub fn delivery_audit(&self) -> R<Vec<Value>> {
        let rows: Vec<String> = self.read(|c| {
            let mut q = c.prepare_cached("SELECT line FROM delivery_audit ORDER BY n")?;
            let rows = q.query_map([], |r| r.get(0))?.collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })?;
        rows.iter().map(|t| value_of(t)).collect()
    }

    /// The runs kept, oldest first.
    pub fn runs(&self) -> R<Vec<RunRow>> {
        self.read(|c| {
            let mut q = c.prepare_cached("SELECT id, key, app, pid, at, end_at, exit, ms, timed_out FROM runs ORDER BY rowid")?;
            let rows = q
                .query_map([], |r| {
                    Ok(RunRow {
                        id: r.get(0)?,
                        key: r.get(1)?,
                        app: r.get(2)?,
                        pid: r.get(3)?,
                        at: r.get(4)?,
                        end: r.get(5)?,
                        exit: r.get(6)?,
                        ms: r.get(7)?,
                        timed_out: r.get::<_, i64>(8)? != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// The runs to keep, in place of what was kept, in one write.
    pub fn put_runs(&self, runs: &[RunRow]) -> R<()> {
        self.write(|c| {
            c.execute_batch("DELETE FROM runs")?;
            for r in runs {
                c.execute(
                    "INSERT INTO runs (id, key, app, pid, at, end_at, exit, ms, timed_out) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![r.id, r.key, r.app, r.pid, r.at, r.end, r.exit, r.ms, i64::from(r.timed_out)],
                )?;
            }
            Ok(())
        })
    }

    /// A session started under the person's delegation. Those started more
    /// than `keep_ms` before `at` go: their runs are long over.
    pub fn put_delegated(&self, session: &str, d: &Delegated, keep_ms: f64) -> R<()> {
        self.write(|c| {
            c.execute(
                "INSERT INTO delegated_runs (session, cwd, from_machine, at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (session) DO UPDATE SET cwd = excluded.cwd, from_machine = excluded.from_machine, at = excluded.at",
                params![session, d.cwd, d.from, d.at],
            )?;
            c.execute("DELETE FROM delegated_runs WHERE at < ?1", params![d.at - keep_ms])?;
            Ok(())
        })
    }

    /// The delegation a session was started under, if it was.
    pub fn delegated(&self, session: &str) -> R<Option<Delegated>> {
        self.read(|c| {
            Ok(c.query_row("SELECT cwd, from_machine, at FROM delegated_runs WHERE session = ?1", params![session], |r| {
                Ok(Delegated { cwd: r.get(0)?, from: r.get(1)?, at: r.get(2)? })
            })
            .optional()?)
        })
    }
}
