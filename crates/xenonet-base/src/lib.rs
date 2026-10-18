//! The small pieces every XenoNet crate uses: JSON as JS reads and writes it
//! (`json`), times as the transcripts write
//! them (`time`), paths as XenoNet compares and shows them (`paths`), what
//! a call to a model cost at API prices (`pricing`), the client that
//! reaches a running XenoNet (`client`, over `http`), and locks that
//! outlive a panic (`sync`).

pub mod client;
pub mod dirs;
pub mod fs;
pub mod http;
pub mod ids;
pub mod json;
pub mod jsstr;
pub mod log;
pub mod ordered;
pub mod paths;
pub mod pricing;
pub mod private_file;
pub mod sync;
pub mod text;
pub mod time;
pub mod units;
