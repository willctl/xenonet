//! Adytum's record of what it made. A record is written before a cell is
//! created or removed and settled after; at every start [`reconcile`]
//! compares the ledger with what the driver reports and finishes or removes
//! the difference, so a crash cannot leak a cell.
//!
//! The ledger keeps its own file, `<ADYTUM_HOME>/adytum.sqlite`, and adds
//! nothing to XenoNet's store, because that would change XenoNet's schema
//! hash and force every XenoNet machine to set up again.
//!
//! It copies the store's design: one connection behind a lock, WAL, and
//! `PRAGMA user_version` set to a hash of the schema text. A file with
//! another version is refused and left alone ([`LedgerError::SetUpAgain`]);
//! nothing is migrated.
//!
//! Several [`Ledger`] handles on one file are safe: every write is one
//! `BEGIN IMMEDIATE` transaction, so SQLite orders them. Run [`reconcile`]
//! only at start, before other work on the same ledger begins, since it
//! treats every open operation as interrupted.

mod error;
mod reconcile;
mod schema;

pub use error::LedgerError;
pub use reconcile::{create_cell, reconcile, remove_cell, Report};

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use adytum_model::{Cell, CellId, CellName, CellSpec, CellState};
use adytum_policy::{CellFacts, World};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior};

/// The ledger's file in Adytum's home folder.
pub const FILE: &str = "adytum.sqlite";

/// One recorded operation's number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OpId(pub i64);

/// What an operation does to a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpKind {
    /// Makes the cell.
    Create,
    /// Removes the cell.
    Remove,
    /// Starts the cell.
    Start,
    /// Stops the cell.
    Stop,
}

impl OpKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Remove => "remove",
            Self::Start => "start",
            Self::Stop => "stop",
        }
    }

    fn parse(text: &str) -> Result<Self, LedgerError> {
        match text {
            "create" => Ok(Self::Create),
            "remove" => Ok(Self::Remove),
            "start" => Ok(Self::Start),
            "stop" => Ok(Self::Stop),
            other => Err(LedgerError::Corrupt(format!("operation kind {other:?}"))),
        }
    }
}

/// How an operation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It worked.
    Done,
    /// It did not, and why.
    Failed(String),
}

/// An operation written down and not yet settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOp {
    /// Its number.
    pub id: OpId,
    /// The cell it acts on.
    pub name: CellName,
    /// What it does.
    pub kind: OpKind,
    /// When it was written, in seconds since 1970.
    pub started_at: i64,
}

/// The ledger, open.
pub struct Ledger {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger").field("path", &self.path).finish_non_exhaustive()
    }
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn enum_text<T: serde::Serialize>(value: &T) -> Result<String, LedgerError> {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => Ok(s),
        _ => Err(LedgerError::Corrupt("an enum that is not text".to_owned())),
    }
}

fn enum_parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, LedgerError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).map_err(|_| LedgerError::Corrupt(format!("{text:?}")))
}

fn version_of(c: &Connection) -> Result<i32, LedgerError> {
    Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

fn is_empty(c: &Connection) -> Result<bool, LedgerError> {
    let n: i64 = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
    Ok(n == 0)
}

fn other_version(file: &Path, v: i32) -> LedgerError {
    LedgerError::SetUpAgain(format!(
        "{} was made by another Adytum (version {v}, this one's is {}): it is left as it is; set Adytum up again on this machine.",
        file.display(),
        schema::version()
    ))
}

impl Ledger {
    /// `<dir>/adytum.sqlite`, made with the schema when there is none. A
    /// file with another version is refused and left as it is.
    pub fn open(dir: &Path) -> Result<Self, LedgerError> {
        let file = dir.join(FILE);
        // Probe read-only first: opening for write in WAL mode would change
        // a file we are about to refuse.
        if let Ok(c) = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let v = version_of(&c)?;
            if v != schema::version() && !(v == 0 && is_empty(&c)?) {
                return Err(other_version(&file, v));
            }
        }
        std::fs::create_dir_all(dir).map_err(|e| LedgerError::Io(format!("{}: {e}", dir.display())))?;
        let conn = Connection::open(&file)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        // FULL, so a record said to be written survives the machine losing
        // power: the whole point is that the record comes first.
        conn.execute_batch("PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        Self::ready(conn, Some(file))
    }

    /// A ledger in memory, gone when dropped: for tests.
    pub fn memory() -> Result<Self, LedgerError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Self::ready(conn, None)
    }

    fn ready(mut conn: Connection, path: Option<PathBuf>) -> Result<Self, LedgerError> {
        // Take the write lock before looking, so two handles opening a new
        // file together make the schema once.
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if version_of(&tx)? == 0 {
            if !is_empty(&tx)? {
                return Err(LedgerError::SetUpAgain("the file holds a database that is not Adytum's ledger: it is left as it is; set Adytum up again on this machine.".to_owned()));
            }
            for t in schema::TABLES {
                tx.execute_batch(t)?;
            }
            tx.pragma_update(None, "user_version", schema::version())?;
        }
        tx.commit()?;
        Ok(Self { conn: Mutex::new(conn), path })
    }

    /// Where the ledger is; `None` for one in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    // One write: a transaction that takes SQLite's write lock at once, so
    // handles on the same file queue instead of failing half-way.
    fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> Result<T, LedgerError>) -> Result<T, LedgerError> {
        let mut guard = self.lock();
        let tx = guard.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Writes down that a cell is about to be made: the operation, and the
    /// cell in state `Creating`, in one transaction. Refused if a cell with
    /// that name is already recorded.
    pub fn intend_create(&self, spec: &CellSpec) -> Result<OpId, LedgerError> {
        let id = CellId::new_random()?;
        let spec_json = serde_json::to_string(spec).map_err(|e| LedgerError::Corrupt(e.to_string()))?;
        let driver = enum_text(&spec.driver)?;
        let state = enum_text(&CellState::Creating)?;
        let at = now();
        self.write(|tx| {
            let taken: Option<i64> = tx.query_row("SELECT 1 FROM cells WHERE name = ?1", [spec.name.as_str()], |r| r.get(0)).optional()?;
            if taken.is_some() {
                return Err(LedgerError::Exists(spec.name.clone()));
            }
            tx.execute(
                "INSERT INTO cells (name, id, driver, wall, spec_json, state, created_at, settled_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
                params![spec.name.as_str(), id.to_string(), driver, spec.wall.as_str(), spec_json, state, at],
            )?;
            tx.execute("INSERT INTO ops (name, kind, started_at) VALUES (?1, 'create', ?2)", params![spec.name.as_str(), at])?;
            Ok(OpId(tx.last_insert_rowid()))
        })
    }

    /// Writes down that a recorded cell is about to be removed, started or
    /// stopped. A remove puts the cell in state `Removing`. Refused for a
    /// cell the ledger never recorded. Use [`Ledger::intend_create`] for a
    /// create.
    pub fn intend(&self, kind: OpKind, name: &CellName) -> Result<OpId, LedgerError> {
        if kind == OpKind::Create {
            return Err(LedgerError::Corrupt("a create needs a spec: use intend_create".to_owned()));
        }
        let at = now();
        self.write(|tx| {
            let known: Option<i64> = tx.query_row("SELECT 1 FROM cells WHERE name = ?1", [name.as_str()], |r| r.get(0)).optional()?;
            if known.is_none() {
                return Err(LedgerError::NotRecorded(name.clone()));
            }
            if kind == OpKind::Remove {
                tx.execute("UPDATE cells SET state = ?2 WHERE name = ?1", params![name.as_str(), enum_text(&CellState::Removing)?])?;
            }
            tx.execute("INSERT INTO ops (name, kind, started_at) VALUES (?1, ?2, ?3)", params![name.as_str(), kind.as_str(), at])?;
            Ok(OpId(tx.last_insert_rowid()))
        })
    }

    /// Closes an operation and updates its cell. A create that worked leaves
    /// the cell `Stopped`; one that failed deletes the row. A remove that
    /// worked deletes the row (and its holds); one that failed leaves the
    /// cell `Broken`. Start and stop set `Running` and `Stopped` when they
    /// worked and change nothing when they failed.
    pub fn settle(&self, op: OpId, outcome: Outcome) -> Result<(), LedgerError> {
        let at = now();
        self.write(|tx| {
            let row: Option<(String, String, Option<i64>)> =
                tx.query_row("SELECT name, kind, settled_at FROM ops WHERE id = ?1", [op.0], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
            let (name, kind, settled) = row.ok_or_else(|| LedgerError::Corrupt(format!("operation {} is not in the ledger", op.0)))?;
            if settled.is_some() {
                return Err(LedgerError::AlreadySettled(op));
            }
            let kind = OpKind::parse(&kind)?;
            let outcome_text = match &outcome {
                Outcome::Done => "done".to_owned(),
                Outcome::Failed(why) => format!("failed: {why}"),
            };
            tx.execute("UPDATE ops SET settled_at = ?2, outcome = ?3 WHERE id = ?1", params![op.0, at, outcome_text])?;
            let done = outcome == Outcome::Done;
            let set = |state: CellState| -> Result<(), LedgerError> {
                tx.execute("UPDATE cells SET state = ?2, settled_at = ?3 WHERE name = ?1", params![name, enum_text(&state)?, at])?;
                Ok(())
            };
            let delete = || -> Result<(), LedgerError> {
                tx.execute("DELETE FROM holds WHERE name = ?1", [&name])?;
                tx.execute("DELETE FROM cells WHERE name = ?1", [&name])?;
                Ok(())
            };
            match (kind, done) {
                (OpKind::Create | OpKind::Stop, true) => set(CellState::Stopped),
                (OpKind::Start, true) => set(CellState::Running),
                (OpKind::Create, false) | (OpKind::Remove, true) => delete(),
                (OpKind::Remove, false) => set(CellState::Broken),
                (OpKind::Start | OpKind::Stop, false) => Ok(()),
            }
        })
    }

    /// Sets a recorded cell's state to what the driver reports. Used when
    /// reconciling; a name the ledger does not know is left alone.
    pub(crate) fn set_state(&self, name: &CellName, state: CellState) -> Result<(), LedgerError> {
        let text = enum_text(&state)?;
        self.write(|tx| {
            tx.execute("UPDATE cells SET state = ?2 WHERE name = ?1", params![name.as_str(), text])?;
            Ok(())
        })
    }

    /// Deletes a recorded cell's row and holds, whatever its state. Used
    /// when reconciling a cell the driver no longer lists.
    pub(crate) fn forget(&self, name: &CellName) -> Result<(), LedgerError> {
        self.write(|tx| {
            tx.execute("DELETE FROM holds WHERE name = ?1", [name.as_str()])?;
            tx.execute("DELETE FROM cells WHERE name = ?1", [name.as_str()])?;
            Ok(())
        })
    }

    /// Every recorded cell, by name.
    pub fn cells(&self) -> Result<Vec<Cell>, LedgerError> {
        let guard = self.lock();
        let mut stmt = guard.prepare("SELECT id, spec_json, state FROM cells ORDER BY name")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, spec_json, state) = row?;
            let id = id.parse::<CellId>().map_err(|e| LedgerError::Corrupt(e.to_string()))?;
            let spec = serde_json::from_str::<CellSpec>(&spec_json).map_err(|e| LedgerError::Corrupt(e.to_string()))?;
            out.push(Cell { id, spec, state: enum_parse(&state)? });
        }
        Ok(out)
    }

    /// The facts `adytum_policy::check_create` reads: one entry per recorded
    /// cell (including one being made, so its name is taken), with
    /// `holds_open_eidolon` true when a hold is recorded.
    pub fn world(&self) -> Result<World, LedgerError> {
        let cells = self.cells()?;
        let held: Vec<String> = {
            let guard = self.lock();
            let mut stmt = guard.prepare("SELECT DISTINCT name FROM holds")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<Result<_, _>>()?
        };
        Ok(World {
            cells: cells
                .into_iter()
                .map(|c| CellFacts {
                    holds_open_eidolon: held.iter().any(|h| h == c.spec.name.as_str()),
                    name: c.spec.name,
                    wall: c.spec.wall,
                    network: c.spec.network,
                    mount_sources: c.spec.mounts.into_iter().map(|m| m.source).collect(),
                    secrets: c.spec.secrets.into_iter().map(|s| s.name).collect(),
                })
                .collect(),
        })
    }

    /// Records that the cell holds an open eidolon. Eidolon writes this; the
    /// clone guard reads it. Refused for a cell the ledger does not know.
    pub fn hold(&self, name: &CellName, eidolon: &str) -> Result<(), LedgerError> {
        self.write(|tx| {
            let known: Option<i64> = tx.query_row("SELECT 1 FROM cells WHERE name = ?1", [name.as_str()], |r| r.get(0)).optional()?;
            if known.is_none() {
                return Err(LedgerError::NotRecorded(name.clone()));
            }
            tx.execute("INSERT OR IGNORE INTO holds (name, eidolon) VALUES (?1, ?2)", params![name.as_str(), eidolon])?;
            Ok(())
        })
    }

    /// Records that the cell no longer holds this eidolon.
    pub fn release(&self, name: &CellName, eidolon: &str) -> Result<(), LedgerError> {
        self.write(|tx| {
            tx.execute("DELETE FROM holds WHERE name = ?1 AND eidolon = ?2", params![name.as_str(), eidolon])?;
            Ok(())
        })
    }

    /// Operations written down and not settled, oldest first.
    pub fn open_ops(&self) -> Result<Vec<OpenOp>, LedgerError> {
        let guard = self.lock();
        let mut stmt = guard.prepare("SELECT id, name, kind, started_at FROM ops WHERE settled_at IS NULL ORDER BY id")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (id, name, kind, started_at) = row?;
            let name = name.parse::<CellName>().map_err(|e| LedgerError::Corrupt(e.to_string()))?;
            out.push(OpenOp { id: OpId(id), name, kind: OpKind::parse(&kind)?, started_at });
        }
        Ok(out)
    }
}
