//! The one store (docs/design/direction.md, phase 4): every domain's state
//! in `<state>/xenonet.sqlite`, in place of the dozen files and databases
//! each kept apart.
//!
//! - **One writer.** One connection, behind a lock, so every write is in
//!   one order; in WAL mode, so a read never waits for a write to end.
//! - **No migrations.** The database's `user_version` is its schema's
//!   hash (`schema::version`). A database with another version, or an
//!   `engine.sqlite` from before the store, is left on disk as it is and
//!   refused, with why: the machine is set up again, as every machine is
//!   after a phase of the rewrite lands.
//! - **One store order.** `meta.stored` counts every chain and feed entry
//!   the store takes, and each entry keeps its number: what was stored
//!   before a revocation and what after is a comparison of two numbers.
//!
//! SQLite's own types stay inside the crate: what the store answers is its
//! own, so a domain moving in doesn't spread rusqlite through the core.

mod chains;
pub mod schema;

pub use chains::Put;

use rusqlite::{Connection, OpenFlags, Transaction};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The store's file in the state folder.
pub const FILE: &str = "xenonet.sqlite";
/// What the engine kept before the store, in the same folder. One here
/// means data from an earlier build, which the rewrite doesn't carry.
pub const OLD_ENGINE: &str = "engine.sqlite";

/// Why the store didn't do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreError {
    /// `set-up-again` (the folder holds data this build doesn't read),
    /// `conflict` (another entry is stored where this one would go),
    /// `bad-entry` (a stored entry doesn't read as its chain's) or
    /// `sqlite`.
    pub code: &'static str,
    pub message: String,
}

impl StoreError {
    fn new(code: &'static str, message: impl Into<String>) -> StoreError {
        StoreError { code, message: message.into() }
    }

    /// Whether the machine has to be set up again before it has a store.
    pub fn set_up_again(&self) -> bool {
        self.code == "set-up-again"
    }
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> StoreError {
        StoreError::new("sqlite", e.to_string())
    }
}

pub type R<T> = Result<T, StoreError>;

/// How a state folder's store stands, for the doctor: without opening it
/// for writing, and without changing anything there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Standing {
    /// No store yet: the service makes one.
    None,
    /// A store this build reads.
    Ready,
    /// Data this build doesn't read, left as it is: why, and that the
    /// machine is to be set up again.
    SetUpAgain(String),
}

/// The store a state folder holds, as `Db::open` would find it.
pub fn standing(state_dir: &Path) -> Standing {
    if let Some(why) = old_data(state_dir) {
        return Standing::SetUpAgain(why);
    }
    let file = state_dir.join(FILE);
    if !file.exists() {
        return Standing::None;
    }
    match Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(StoreError::from).and_then(|c| found_version(&c)) {
        Ok(v) if v == schema::version() => Standing::Ready,
        Ok(v) => Standing::SetUpAgain(other_version(&file, v)),
        Err(e) => Standing::SetUpAgain(format!("{} can't be read: {}", file.display(), e.message)),
    }
}

// An engine.sqlite from before the store: what to say of it.
fn old_data(state_dir: &Path) -> Option<String> {
    let old = state_dir.join(OLD_ENGINE);
    old.exists().then(|| format!("{} is from an earlier XenoNet, which this one doesn't read: it's left as it is; set this machine up again", old.display()))
}

fn other_version(file: &Path, v: i32) -> String {
    format!("{} was made by another XenoNet (version {v}, this one's is {}): it's left as it is; set this machine up again", file.display(), schema::version())
}

fn found_version(c: &Connection) -> R<i32> {
    Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// The store, open.
pub struct Db {
    conn: Mutex<Connection>,
    path: Option<PathBuf>,
}

impl fmt::Debug for Db {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Db").field("path", &self.path).finish_non_exhaustive()
    }
}

impl Db {
    /// `<state_dir>/xenonet.sqlite`, made with the schema when there's
    /// none. Refused, with nothing on disk changed, when the folder holds
    /// an engine.sqlite from before the store, or the store there is
    /// another schema's.
    pub fn open(state_dir: &Path) -> R<Db> {
        if let Some(why) = old_data(state_dir) {
            return Err(StoreError::new("set-up-again", why));
        }
        let file = state_dir.join(FILE);
        if let Ok(c) = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY) {
            let v = found_version(&c)?;
            if v != schema::version() && !(v == 0 && is_empty(&c)?) {
                return Err(StoreError::new("set-up-again", other_version(&file, v)));
            }
        }
        std::fs::create_dir_all(state_dir).map_err(|e| StoreError::new("sqlite", format!("{}: {e}", state_dir.display())))?;
        let conn = Connection::open(&file)?;
        // WAL, so reads go on while the one writer writes; FULL, so an
        // entry the store said it took survives the machine losing power.
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        conn.execute_batch("PRAGMA synchronous = FULL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")?;
        Db::ready(conn, Some(file))
    }

    /// A store in memory, gone when it's dropped: for tests and the demo.
    pub fn memory() -> R<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        Db::ready(conn, None)
    }

    // The schema made in an empty database, in one transaction with its
    // version, so a store is either whole or not there.
    fn ready(mut conn: Connection, path: Option<PathBuf>) -> R<Db> {
        if found_version(&conn)? == 0 {
            let tx = conn.transaction()?;
            for t in schema::TABLES {
                tx.execute_batch(t)?;
            }
            tx.pragma_update(None, "user_version", schema::version())?;
            tx.commit()?;
        }
        Ok(Db { conn: Mutex::new(conn), path })
    }

    /// Where the store is; None for one in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// How many chain and feed entries the store has taken.
    pub fn stored(&self) -> R<u64> {
        self.read(|c| Ok(c.query_row("SELECT v FROM meta WHERE k = 'stored'", [], |r| r.get::<_, i64>(0))? as u64))
    }

    /// A read, under the one lock.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        let c = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&c)
    }

    /// A write, in a transaction of its own, under the one lock: it all
    /// happens, or none of it.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Transaction<'_>) -> R<T>) -> R<T> {
        let mut c = self.conn.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tx = c.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

/// The next number in the store's order, taken inside the write that
/// stores its entry, so the count and the entry go in together.
pub(crate) fn next_stored(tx: &Transaction<'_>) -> R<u64> {
    Ok(tx.query_row("UPDATE meta SET v = v + 1 WHERE k = 'stored' RETURNING v", [], |r| r.get::<_, i64>(0))? as u64)
}

// Whether a database has nothing in it: a file SQLite made, or one left
// empty, which the store may make its tables in.
fn is_empty(c: &Connection) -> R<bool> {
    Ok(c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get::<_, i64>(0))? == 0)
}

#[cfg(test)]
mod tests;
