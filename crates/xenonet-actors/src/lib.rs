//! Actors (docs/design/actors/interface.md): the programs you allow in
//! XenoNet: the registry (`registry`), registering them (`registration`),
//! what they report (`reports`), waking them (`wake`), their remote
//! connections (`connections`), their tokens (`tokens`) and the files that
//! keep them (`token_file`). Each module asks what it needs of the world
//! through a trait of its own, and keeps its rows through a store trait of
//! its own (`store` has what those share).

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
pub mod store;
pub mod token_file;
pub mod tokens;
pub mod wake;
pub mod you;

pub use xenonet_adapter::products;
