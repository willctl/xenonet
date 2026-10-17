//! XenoNet's schemas (schemas/, written from core/schema/): vendor data
//! read leniently against them (`lenient`), and which keys are fields and
//! which are data (`fields`). The vendor readers and the drift log follow.

pub mod fields;
pub mod lenient;
