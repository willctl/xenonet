use eidolon_model::{FacetId, FacetKind, Refusal};
use eidolon_pack::PackError;
use std::fmt;

/// Why a facet could not be taken, placed, sealed or opened.
#[derive(Debug)]
pub enum FacetError {
    /// Packing or unpacking failed.
    Pack(PackError),
    /// A rule said no (a leave-out pattern that escapes its facet).
    Refused(Refusal),
    /// A file operation on the host failed.
    Io { what: &'static str, source: std::io::Error },
    /// The cell could not move files or run a command.
    Cell(String),
    /// A spec of one kind given to a facet of another.
    WrongKind { expected: FacetKind },
    /// The manifest has no part for this facet.
    MissingFacet(FacetId),
    /// A guest path that is not absolute, or reaches up with `..`.
    BadGuestPath(String),
    /// The system gave no random numbers.
    Random,
}

impl FacetError {
    pub(crate) fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> FacetError {
        move |source| FacetError::Io { what, source }
    }
}

impl From<PackError> for FacetError {
    fn from(e: PackError) -> FacetError {
        FacetError::Pack(e)
    }
}

impl From<Refusal> for FacetError {
    fn from(e: Refusal) -> FacetError {
        FacetError::Refused(e)
    }
}

impl fmt::Display for FacetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FacetError::Pack(e) => write!(f, "{e}"),
            FacetError::Refused(e) => write!(f, "{e}"),
            FacetError::Io { what, source } => write!(f, "Could not {what}: {source}"),
            FacetError::Cell(why) => write!(f, "The cell failed: {why}"),
            FacetError::WrongKind { expected } => write!(f, "That facet is not a {expected:?} facet. Use the facet kind its spec names."),
            FacetError::MissingFacet(id) => write!(f, "The manifest has nothing for facet {id}. Open the eidolon from a manifest that carries this facet."),
            FacetError::BadGuestPath(p) => write!(f, "The cell path {p:?} must be absolute and free of `..`."),
            FacetError::Random => f.write_str("The system gave no random numbers. Try again."),
        }
    }
}

impl std::error::Error for FacetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FacetError::Pack(e) => Some(e),
            FacetError::Refused(e) => Some(e),
            FacetError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, FacetError>;
