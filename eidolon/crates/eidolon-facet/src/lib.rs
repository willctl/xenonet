//! The facet trait and the plain folder kind.
//!
//! A facet lives inside a cell (a VM), but packing works on a host folder.
//! So a facet moves its files between the cell and a host staging folder
//! through a [`CellAccess`], which the CLI implements with Adytum's driver.
//!
//! - [`Facet`]: quiet, take, place, resume, and what to leave out.
//! - [`FolderFacet`]: a plain folder.
//! - [`seal_facets`] and [`open_facets`]: all of an eidolon's facets in one
//!   step each way.
//! - [`abort_at`]: dies at a named step, for the kill tests only.
//! - [`LocalCell`]: a host folder standing in for a cell, for tests.

mod abort;
mod cell;
mod error;
mod facet;
mod flow;

pub use abort::{abort_at, ABORT_VAR};
pub use cell::{CellAccess, LocalCell, RunOutput};
pub use error::{FacetError, Result};
pub use facet::{Facet, FolderFacet};
pub use flow::{open_facets, seal_facets};
