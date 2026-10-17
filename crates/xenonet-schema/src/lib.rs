//! XenoNet's schemas (schemas/, written from core/schema/): vendor data
//! read leniently against them (`lenient`), and which keys are fields and
//! which are data (`fields`). The vendor readers and the drift log follow.
//! Data checked against them as zod checks it (`check`), with their
//! patterns run as JS runs them (`jsregex`).

pub mod check;
pub mod drift;
pub mod fields;
pub mod jsregex;
pub mod lenient;
pub mod vendor;
