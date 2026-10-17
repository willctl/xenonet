//! XenoNet's schemas (schemas/, written from core/schema/): vendor data
//! read leniently against them (`lenient`), and which keys are fields and
//! which are data (`fields`). The vendor readers and the drift log follow.
//! Data checked against them as zod checks it (`check`, with JS's regular
//! expressions in `jsregex`), the registry of their ids and documents
//! (`registry`), and the files in schemas/ (`files`).

pub mod check;
pub mod drift;
pub mod fields;
pub mod files;
pub mod jsregex;
pub mod lenient;
pub mod registry;
pub mod vendor;
