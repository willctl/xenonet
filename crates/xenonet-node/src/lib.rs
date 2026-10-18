//! The Node-API addon behind the JS twins (docs/design/rust-core.md): each
//! twin in `js/core/` keeps its JS module's exports and calls in here, and
//! this calls the crates. Nothing here decides anything a crate doesn't:
//! it converts arguments, calls back into what a test passed in, and
//! converts the answers.
#![expect(unsafe_code, reason = "Node-API is a C interface: values cross it through raw handles")]
#![cfg_attr(
    test,
    expect(dead_code, reason = "napi-rs registers the #[napi] exports only outside test builds, so a test build sees every export, and what only they call, as unused")
)]

mod account_links;
mod actor_helpers;
mod actor_spend;
mod actors;
mod actors_core;
mod app;
mod args;
mod assistant;
mod attest;
mod attribution;
mod base;
mod blocks;
mod components;
mod connect;
mod connections;
mod delivery;
mod doctor;
mod drift;
mod engine;
mod hermes_files;
mod hosts;
mod integrations;
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
