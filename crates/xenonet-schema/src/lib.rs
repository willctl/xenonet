//! XenoNet's schemas, as the files in schemas/ have them (the vendors'
//! hand-kept under schemas/vendor/, XenoNet's own written from
//! crates/xenonet-model): vendor data read leniently against them
//! (`lenient`), which keys are fields and which are data (`fields`), the
//! vendor readers (`vendor`) and the drift log (`drift`); and XenoNet's own
//! documents read as their types (`typed`), where the type is the check.

pub mod drift;
pub mod fields;
pub mod lenient;
pub mod typed;
pub mod vendor;
