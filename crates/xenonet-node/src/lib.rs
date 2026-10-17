//! The Node-API addon behind the JS twins (docs/design/rust-core.md): each
//! twin in `js/core/` keeps its JS module's exports and calls in here, and
//! this calls the crates. Nothing here decides anything a crate doesn't:
//! it converts arguments, calls back into what a test passed in, and
//! converts the answers.

mod account_links;
mod actor_helpers;
mod actor_spend;
mod actors_core;
mod actors;
mod args;
mod attest;
mod attribution;
mod connect;
mod connections;
mod delivery;
mod engine;
mod base;
mod blocks;
mod drift;
mod hermes_files;
mod js;
mod lines;
mod live;
mod machine;
mod map;
mod native;
mod observer;
mod observers_core;
mod peer;
mod procs;
mod readers;
mod registration;
mod remote_presence;
mod reports;
mod schema;
mod schemas;
mod secrets;
mod sources;
mod state;
mod stats;
mod store;
mod verify;
mod wake;
