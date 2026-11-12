//! Actors (docs/design/actors/interface.md): the programs you allow in
//! XenoNet: the registry (`registry`), registering them (`registration`),
//! what they report (`reports`), waking them (`wake`), their remote
//! connections (`connections`), the cloud assistants among them and what
//! they read (`assistant`), and their tokens (`tokens`, whose files are
//! xenonet-api's `token_file`); and the secret stores (`secret_store`, and
//! `secrets`, which runs your credential store's program or keeps them in
//! memory). Each module asks what it needs of the world
//! through a trait of its own, and keeps its rows through a store trait of
//! its own (`store` has what those share).

pub mod account_links;
pub mod adapters;
pub mod assistant;
pub mod connections;
pub mod demo;
pub mod detail;
pub mod handoff;
pub mod names;
pub mod registration;
pub mod registry;
pub mod remote_presence;
pub mod reports;
pub mod secret_store;
pub mod secrets;
pub mod spend;
pub mod store;
pub mod tokens;
pub mod wake;
pub mod you;

pub use xenonet_adapter::products;

/// A pattern, in the regex crate's syntax, compiled once, the first time
/// it's used. White space in a pattern that reads what an actor sent is
/// `[\s\x{FEFF}--\x{85}]`, what trim() takes off.
macro_rules! re {
    ($p:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        R.get_or_init(|| $crate::compile($p))
    }};
}
pub(crate) use re;

/// A pattern `re!` was given: one written in the source, so it compiles.
pub(crate) fn compile(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern).expect("a fixed pattern: callers pass them from the source")
}
