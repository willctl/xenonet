//! Actors (docs/design/actors/interface.md): the programs you allow in
//! XenoNet. So far, their tokens (`tokens`) and the files that keep them
//! (`token_file`); the registry, registration and what actors report follow
//! with the engine.

pub mod account_links;
pub mod adapters;
pub mod connections;
pub mod demo;
pub mod detail;
pub mod handoff;
pub mod key;
pub mod limits;
pub mod names;
pub mod registration;
pub mod registry;
pub mod remote_presence;
pub mod reports;
pub mod secret_store;
pub mod spend;
pub mod token_file;
pub mod tokens;
pub mod wake;
pub mod you;

pub use xenonet_adapter::products;
