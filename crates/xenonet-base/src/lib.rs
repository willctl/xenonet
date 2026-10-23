//! The small pieces every XenoNet crate uses: JSON as JS reads and writes it
//! (`json`), times as the transcripts write them (`time`), paths as XenoNet
//! compares and shows them (`paths`) and as Node's `path` builds them
//! (`nodepath`), a call's tokens (`usage`) and what they cost at API prices
//! (`pricing`), the files an apply_patch text names (`patch`), OpenCode's
//! ids as session ids take them (`opencode`), the client that reaches a
//! running XenoNet (`client`, over `http`), whether a process runs
//! (`process`), sorting as JS sorts (`sort`), locks that outlive a panic
//! (`sync`), the apps XenoNet knows (`apps`), and the seam between the
//! engine and the network across machines (`network`).

pub mod apps;
pub mod client;
pub mod dirs;
pub mod fs;
pub mod http;
pub mod ids;
pub mod json;
pub mod jsstr;
pub mod log;
pub mod network;
pub mod nodepath;
pub mod opencode;
pub mod ordered;
pub mod patch;
pub mod paths;
pub mod pricing;
pub mod private_file;
pub mod process;
pub mod sort;
pub mod sync;
pub mod text;
pub mod time;
pub mod units;
pub mod usage;
