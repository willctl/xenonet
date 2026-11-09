//! What the actors keep in the engine's database, as each module asks for
//! it. Every module names its own store trait, with the rows it reads and
//! writes as JSON (a column as node:sqlite read it: integers as numbers);
//! this is what they share: how a call fails, and running several writes
//! as one transaction.

use std::fmt;

/// How a collaborator or the store failed: its message, and a code when it
/// has one (SQLite's, or a program's).
#[derive(Clone, Debug, PartialEq)]
pub struct Error {
    pub message: String,
    pub code: Option<String>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Error { message: message.into(), code: None }
    }
    pub fn coded(message: impl Into<String>, code: impl Into<String>) -> Self {
        Error { message: message.into(), code: Some(code.into()) }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Error::new(message)
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Error::new(message)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Transactions on the database a module's rows are in.
pub trait Tx {
    /// Whether one is under way already.
    fn in_transaction(&self) -> bool;
    fn begin(&self) -> Result<()>;
    fn commit(&self) -> Result<()>;
    fn rollback(&self);
}

/// `f` in a transaction of its own, unless one is under way already:
/// committed when it answers, rolled back when it fails.
pub fn transaction<T, E: From<Error>>(tx: &dyn Tx, f: impl FnOnce() -> std::result::Result<T, E>) -> std::result::Result<T, E> {
    if tx.in_transaction() {
        return f();
    }
    tx.begin()?;
    match f() {
        Ok(v) => match tx.commit() {
            Ok(()) => Ok(v),
            Err(e) => {
                tx.rollback();
                Err(e.into())
            }
        },
        Err(e) => {
            tx.rollback();
            Err(e)
        }
    }
}

/// A fake store's transactions, for tests: nothing to begin or end.
#[derive(Debug, Default)]
pub struct NoTx;

impl Tx for NoTx {
    fn in_transaction(&self) -> bool {
        false
    }
    fn begin(&self) -> Result<()> {
        Ok(())
    }
    fn commit(&self) -> Result<()> {
        Ok(())
    }
    fn rollback(&self) {}
}
