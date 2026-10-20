//! The small pieces every XenoNet crate uses: JSON as JS reads and writes it
//! (`json`), times as the transcripts write them (`time`), paths as XenoNet
//! compares and shows them (`paths`) and as Node's `path` builds them
//! (`nodepath`), what a call to a model cost at API prices (`pricing`), the
//! client that reaches a running XenoNet (`client`, over `http`), whether a
//! process runs (`process`), sorting as JS sorts (`sort`), locks that outlive
//! a panic (`sync`), and the apps XenoNet knows (`apps`).

pub mod apps;
pub mod client;
pub mod dirs;
pub mod fs;
pub mod http;
pub mod ids;
pub mod json;
pub mod jsstr;
pub mod log;
pub mod nodepath;
pub mod ordered;
pub mod paths;
pub mod pricing;
pub mod private_file;
pub mod process;
pub mod sort;
pub mod sync;
pub mod text;
pub mod time;
pub mod units;
