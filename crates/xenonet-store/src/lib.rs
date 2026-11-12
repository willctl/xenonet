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

pub mod a2a;
pub mod agents;
mod chains;
pub mod delivery;
pub mod engine;
pub mod feed;
pub mod intents;
pub mod mailbox;
pub mod net;
pub mod projects;
pub mod schema;
pub mod spaces;
pub mod sql;
pub mod state;

pub use chains::Put;

use rusqlite::{Connection, OpenFlags};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::ThreadId;

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
    // The one connection, and the thread whose transaction is open on it
    // if one is: a statement from another thread waits until that
    // transaction ends, so what a transaction writes is only its own.
    conn: Mutex<Turn>,
    free: Condvar,
    path: Option<PathBuf>,
    // Where the random ids the store mints come from, and who this
    // machine's projects are owned by (`projects`).
    ids: std::sync::RwLock<std::sync::Arc<xenonet_model::ids::Ids>>,
    owner: std::sync::RwLock<Option<projects::OwnerFn>>,
}

struct Turn {
    conn: Connection,
    holder: Option<ThreadId>,
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
        // What agents, sessions and actors wrote: the owner's alone, from
        // the start, so SQLite's own files beside it are too.
        private(&file);
        let conn = Connection::open(&file)?;
        let _ = xenonet_os::private_file::chmod_600(&file);
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
            for t in schema::tables() {
                tx.execute_batch(t)?;
            }
            tx.pragma_update(None, "user_version", schema::version())?;
            tx.commit()?;
        }
        // The statements the domains run often stay prepared.
        conn.set_prepared_statement_cache_capacity(256);
        Ok(Db {
            conn: Mutex::new(Turn { conn, holder: None }),
            free: Condvar::new(),
            path,
            ids: std::sync::RwLock::new(std::sync::Arc::new(xenonet_model::ids::Ids::random())),
            owner: std::sync::RwLock::new(None),
        })
    }

    /// Where the store is; None for one in memory.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// How many chain and feed entries the store has taken.
    pub fn stored(&self) -> R<u64> {
        self.read(|c| Ok(c.query_row("SELECT v FROM meta WHERE k = 'stored'", [], |r| r.get::<_, i64>(0))? as u64))
    }

    // The connection once it's this thread's turn: no other thread's
    // transaction is open on it.
    fn turn(&self) -> MutexGuard<'_, Turn> {
        let me = std::thread::current().id();
        let mut t = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        while t.holder.is_some_and(|h| h != me) {
            t = self.free.wait(t).unwrap_or_else(PoisonError::into_inner);
        }
        t
    }

    /// The connection for one call, on this thread's turn.
    pub(crate) fn with_conn<T>(&self, f: impl FnOnce(&mut Connection) -> T) -> T {
        f(&mut self.turn().conn)
    }

    /// A read, under the one lock.
    pub(crate) fn read<T>(&self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        self.with_conn(|c| f(c))
    }

    /// A write, in a transaction of its own (a savepoint, inside one
    /// that's open), under the one lock: it all happens, or none of it.
    pub(crate) fn write<T>(&self, f: impl FnOnce(&Connection) -> R<T>) -> R<T> {
        self.with_conn(|c| {
            if c.is_autocommit() {
                let tx = c.transaction()?;
                let out = f(&tx)?;
                tx.commit()?;
                Ok(out)
            } else {
                let sp = c.savepoint()?;
                let out = f(&sp)?;
                sp.commit()?;
                Ok(out)
            }
        })
    }

    /// Whether this thread has a transaction open on the store.
    pub fn in_transaction(&self) -> bool {
        let me = std::thread::current().id();
        self.conn.lock().unwrap_or_else(PoisonError::into_inner).holder == Some(me)
    }

    /// Opens a transaction for this thread, once no other thread has one
    /// open: until `commit` or `rollback`, other threads' calls wait.
    /// For a caller whose transaction spans calls of its own (the actors');
    /// `transaction` is the one to use otherwise.
    pub fn begin(&self) -> Result<(), sql::SqlError> {
        let mut t = self.turn();
        t.conn.execute_batch("BEGIN")?;
        t.holder = Some(std::thread::current().id());
        Ok(())
    }

    /// Ends this thread's transaction, keeping what it wrote: on an error
    /// it's rolled back, and nothing stays.
    pub fn commit(&self) -> Result<(), sql::SqlError> {
        self.end("COMMIT")
    }

    /// Ends this thread's transaction, keeping nothing it wrote.
    pub fn rollback(&self) {
        let _ = self.end("ROLLBACK");
    }

    fn end(&self, how: &str) -> Result<(), sql::SqlError> {
        let mut t = self.conn.lock().unwrap_or_else(PoisonError::into_inner);
        let out = t.conn.execute_batch(how).map_err(sql::SqlError::from);
        if out.is_err() && !t.conn.is_autocommit() {
            let _ = t.conn.execute_batch("ROLLBACK");
        }
        t.holder = None;
        drop(t);
        self.free.notify_all();
        out
    }

    /// `f` in a transaction of its own, unless this thread has one open
    /// already, in which case it's part of that one. The store isn't held
    /// while `f` runs: `f` makes its calls as any caller does, and another
    /// thread's calls wait for the transaction to end. On an error from
    /// `f`, or from the commit, nothing it wrote stays.
    pub fn transaction<T, E: From<sql::SqlError>>(&self, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
        if self.in_transaction() {
            return f();
        }
        self.begin()?;
        match f() {
            Ok(v) => self.commit().map(|()| v).map_err(E::from),
            Err(e) => {
                self.rollback();
                Err(e)
            }
        }
    }
}

// The store's file made empty and owner-only before SQLite opens it,
// which then makes its own files beside it with the same mode.
fn private(file: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(file);
    }
    #[cfg(not(unix))]
    let _ = file;
}

/// The next number in the store's order, taken inside the write that
/// stores its entry, so the count and the entry go in together.
pub(crate) fn next_stored(tx: &Connection) -> R<u64> {
    Ok(tx.query_row("UPDATE meta SET v = v + 1 WHERE k = 'stored' RETURNING v", [], |r| r.get::<_, i64>(0))? as u64)
}

// Whether a database has nothing in it: a file SQLite made, or one left
// empty, which the store may make its tables in.
fn is_empty(c: &Connection) -> R<bool> {
    Ok(c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get::<_, i64>(0))? == 0)
}

#[cfg(test)]
mod tests;
