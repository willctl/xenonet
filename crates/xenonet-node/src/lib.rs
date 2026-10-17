//! The Node-API addon behind the JS twins (docs/design/rust-core.md): each
//! twin in `js/core/` keeps its JS module's exports and calls in here, and
//! this calls the crates. Nothing here decides anything a crate doesn't:
//! it converts arguments, calls back into what a test passed in, and
//! converts the answers.

mod actors;
mod app;
mod base;
mod blocks;
mod drift;
mod js;
mod lines;
mod schema;
mod state;
