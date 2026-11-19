//! Adytum's vocabulary: typed ids (`cell_`, `img_`, `wall_`), the cell spec,
//! image pins, refusals, and their JSON. No IO beyond asking the system for
//! random bytes when a new id is made.

mod error;
mod ids;
mod names;
mod refusal;
mod spec;

pub use error::ModelError;
pub use ids::{CellId, ImageId};
pub use names::{CellName, NetworkName, Wall};
pub use refusal::{Refusal, RefusalCode};
pub use spec::{Arch, Cell, CellSpec, CellState, DriverKind, ImagePin, Mount, Port, Resources, SecretRef};
