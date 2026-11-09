//! XenoNet's schemas, as the files in schemas/ have them (the vendors'
//! hand-kept under schemas/vendor/, XenoNet's own written from
//! crates/xenonet-model): vendor data read leniently against them
//! (`lenient`), which keys are fields and which are data (`fields`), the
//! vendor readers (`vendor`) and the drift log (`drift`), and data checked
//! against a schema (`check`, with JS's regular expressions in `jsregex`).

pub mod check;
pub mod drift;
pub mod fields;
pub mod jsregex;
pub mod lenient;
pub mod vendor;
