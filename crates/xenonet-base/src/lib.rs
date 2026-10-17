//! The small pieces every XenoNet crate uses: JSON as JS reads and writes it
//! (`json`), times as the transcripts write
//! them (`time`), paths as XenoNet compares and shows them (`paths`), and
//! what a call to a model cost at API prices (`pricing`).

pub mod fs;
pub mod json;
pub mod ordered;
pub mod paths;
pub mod pricing;
pub mod text;
pub mod time;
