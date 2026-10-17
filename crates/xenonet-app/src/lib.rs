//! XenoNet's settings and the small pieces around them (core/config.js,
//! instances.js, version.js, environments.js, demo-root.js and the rest of
//! core/ that isn't a subsystem of its own), and Node's `path`, which they
//! and the crates above them build paths with.

pub mod command;
pub mod config;
pub mod decide;
pub mod demo_root;
pub mod environments;
pub mod fsx;
pub mod instances;
pub mod jstext;
pub mod nodepath;
pub mod open;
pub mod version;
