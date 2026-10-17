//! Actors (docs/design/actors/interface.md): the programs you allow in
//! XenoNet. So far, their tokens (`tokens`) and the files that keep them
//! (`token_file`); the registry, registration and what actors report follow
//! with the engine.

pub mod adapters;
pub mod detail;
pub mod handoff;
pub mod key;
pub mod limits;
pub mod names;
pub mod secret_store;
pub mod spend;
pub mod token_file;
pub mod tokens;

pub use xenonet_engine::products;
