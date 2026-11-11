//! XenoNet's data model: the ids, entities, relations, operations and the
//! action vocabulary as plain Rust types, with no IO. Every JSON document
//! XenoNet keeps, serves or carries between machines is one of these types
//! (serde), and the JSON Schemas in `schemas/` are written from them
//! (schemars, through `schema::documents`), so the type is the truth and
//! the file is output: `cargo test -p xenonet-model` fails when a file is
//! stale, and `XENONET_SCHEMAS_WRITE=1 cargo test -p xenonet-model` writes
//! them again.
//!
//! Beside the documents: the seam between the engine and the network
//! (`ops`: the bodies, and the `Net` and `Sink` traits each side
//! implements), and a call's tokens and what they cost at API prices
//! (`usage`, `usage::pricing`).
//!
//! The crate knows no vendor and no surface: a platform's name is a value
//! here, never a branch.

// A doc comment here is a schema's description, kept word for word, and a
// description writes <slug> or <host> where rustdoc reads an HTML tag.
#![cfg_attr(doc, allow(rustdoc::invalid_html_tags, rustdoc::broken_intra_doc_links, reason = "doc comments are the schemas' descriptions, kept as they are"))]

pub mod actions;
pub mod actor;
pub mod app;
pub mod assistant;
pub mod doctor;
pub mod drift;
pub mod engine;
pub mod graph;
pub mod host;
pub mod identity;
pub mod ids;
pub mod live;
pub mod network;
pub mod num;
pub mod ops;
pub mod project;
pub mod schema;
pub mod selector;
pub mod session;
pub mod starts;
pub mod state;
pub mod system;
pub mod theme;
pub mod tray;
pub mod usage;

pub use schema::{Any, Bounded, Document, Nullable, Opt, Record, Ref, Text, True};

/// Every schema document, as a writer of its file (`schema::documents()`
/// renders them): each module lists its own in its `DOCUMENTS`.
pub const DOCUMENTS: &[&[schema::Writer]] = &[
    actor::DOCUMENTS,
    app::DOCUMENTS,
    assistant::DOCUMENTS,
    doctor::DOCUMENTS,
    drift::DOCUMENTS,
    engine::DOCUMENTS,
    graph::DOCUMENTS,
    host::DOCUMENTS,
    identity::DOCUMENTS,
    live::DOCUMENTS,
    network::DOCUMENTS,
    project::DOCUMENTS,
    session::DOCUMENTS,
    state::DOCUMENTS,
    system::DOCUMENTS,
    theme::DOCUMENTS,
    tray::DOCUMENTS,
];
