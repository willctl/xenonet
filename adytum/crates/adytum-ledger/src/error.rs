use std::fmt;

use adytum_driver::DriverError;
use adytum_model::{CellName, ModelError, Refusal};

use crate::OpId;

/// Why the ledger did not do what it was asked.
#[derive(Debug)]
pub enum LedgerError {
    /// The file holds a ledger this build does not read. It is left as it
    /// is; the message says what to do.
    SetUpAgain(String),
    /// A rule refused before anything was recorded or made.
    Refused(Refusal),
    /// The driver failed.
    Driver(DriverError),
    /// A cell with this name is already recorded.
    Exists(CellName),
    /// The ledger has no record of a cell with this name, and Adytum never
    /// touches a cell it did not record.
    NotRecorded(CellName),
    /// This operation was already settled.
    AlreadySettled(OpId),
    /// A stored value does not read back as what it should be.
    Corrupt(String),
    /// The database failed.
    Sqlite(String),
    /// The ledger's folder could not be made.
    Io(String),
    /// An id could not be made.
    Model(ModelError),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SetUpAgain(why) => write!(f, "adytum: {why}"),
            Self::Refused(r) => write!(f, "{r}"),
            Self::Driver(e) => write!(f, "{e}"),
            Self::Exists(n) => write!(f, "adytum: the ledger already has a cell named {n}. Pick another name or remove that cell first."),
            Self::NotRecorded(n) => write!(f, "adytum: the ledger has no record of a cell named {n}, so Adytum leaves it alone. Run `adytum ls` to see the cells it made."),
            Self::AlreadySettled(op) => write!(f, "adytum: operation {} was already settled. This is a bug; report it.", op.0),
            Self::Corrupt(what) => write!(f, "adytum: the ledger holds a value it cannot read ({what}). Set Adytum up again."),
            Self::Sqlite(e) => write!(f, "adytum: the ledger database failed: {e}. Check the disk and the folder's permissions, then try again."),
            Self::Io(e) => write!(f, "adytum: could not prepare the ledger's folder: {e}. Check that it can be written."),
            Self::Model(e) => write!(f, "adytum: {e}"),
        }
    }
}

impl std::error::Error for LedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(r) => Some(r),
            Self::Driver(e) => Some(e),
            Self::Model(e) => Some(e),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for LedgerError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e.to_string())
    }
}

impl From<DriverError> for LedgerError {
    fn from(e: DriverError) -> Self {
        Self::Driver(e)
    }
}

impl From<Refusal> for LedgerError {
    fn from(r: Refusal) -> Self {
        Self::Refused(r)
    }
}

impl From<ModelError> for LedgerError {
    fn from(e: ModelError) -> Self {
        Self::Model(e)
    }
}
