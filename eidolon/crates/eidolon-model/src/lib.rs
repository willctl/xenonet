//! Eidolon's vocabulary: typed ids (`eid_`, `fct_`), walls, facet specs,
//! holders, the seat record and the refusals the policy returns. No IO
//! besides the system's random numbers, and no word about who the entity
//! is: an eidolon is only an id, a wall, some holders and some facets.
//!
//! The manifest's file lists and chunk names belong to `eidolon-pack`.

mod error;
mod ids;
mod refusal;
mod seat;
mod spec;

pub use error::ModelError;
pub use ids::{EidolonId, FacetId, Wall};
pub use refusal::{Refusal, RefusalCode};
pub use seat::{verify, SeatRecord, SignedSeat, State};
pub use spec::{EidolonSpec, FacetKind, FacetSpec, Holder, HolderKey};
