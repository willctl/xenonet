//! Actors (docs/design/actors/interface.md): the programs you allow in
//! XenoNet. So far, their tokens (`tokens`) and the files that keep them
//! (`token_file`); the registry, registration and what actors report follow
//! with the engine.

pub mod account_links;
pub mod adapters;
pub mod app_bundle;
pub mod attribution;
pub mod demo;
pub mod detail;
pub mod grok_bot;
pub mod handoff;
pub mod hermes;
pub mod hermes_files;
pub mod key;
pub mod limits;
pub mod muse;
pub mod names;
pub mod observer;
pub mod observers;
pub mod registration;
pub mod registry;
pub mod remote_presence;
pub mod reports;
pub mod secret_store;
pub mod spend;
pub mod token_file;
pub mod tokens;
pub mod you;

pub use xenonet_engine::products;
