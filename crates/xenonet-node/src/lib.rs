//! The Node-API addon behind the JS twins (docs/design/rust-core.md): each
//! twin in `js/core/` keeps its JS module's exports and calls in here, and
//! this calls the crates. Nothing here decides anything a crate doesn't:
//! it converts arguments, calls back into what a test passed in, and
//! converts the answers.

mod actor_helpers;
mod actor_spend;
mod actors_core;
mod actors;
mod args;
mod attribution;
mod base;
mod blocks;
mod drift;
mod js;
mod lines;
mod machine;
mod peer;
mod procs;
mod schema;
mod secrets;
mod state;
mod store;
