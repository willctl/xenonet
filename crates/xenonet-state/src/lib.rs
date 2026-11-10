//! The map's data and what XenoNet remembers: state.json, its versions and
//! its settings (`statefile`, `migrate`, `store`), the projects and the
//! accounts sessions ran as, the map itself (`graph`, refreshed by `state`),
//! and what the transcripts add up to (`stats`, `search`, `digest`); the
//! decision model's requests and the checks of its answers (`decide`); and
//! the runtime that keeps the map current and serves it as the service's
//! cores (`runtime`).

pub mod classify;
pub mod decide;
pub mod digest;
pub mod export_labels;
pub mod frames;
pub mod graph;
pub mod identities;
pub mod migrate;
pub mod needs_you;
pub mod projects;
pub mod redact;
pub mod rename;
pub mod runtime;
pub mod search;
pub mod state;
pub mod statefile;
pub mod stats;
pub mod store;
pub mod workspace;
