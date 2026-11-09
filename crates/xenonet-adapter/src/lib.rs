//! How a platform plugs into XenoNet (docs/design/direction.md, "Adapters
//! and surfaces"). The core knows no vendor: what it knows of Claude Code,
//! Codex, Grok Build, Hermes and the rest comes through here.
//!
//! - `apps`: the registry of apps XenoNet knows sessions of, each entry
//!   the facts about one (its folder, hook events, capabilities).
//! - `products`: the actor products (Hermes Agent, Grok Bot, Muse, dots).
//! - `platform`: the `Platform` trait an adapter implements, and the
//!   `Registry` the service builds from the adapters it was given.
//! - `observe`: the observer an adapter brings for an actor product, and
//!   what every observer shares (keeping actors, attribution, an app).
//! - `reader`: what an adapter brings to the map: a reader of its
//!   transcripts for the live tailer, or of its own records.
//! - `check`: a doctor row an adapter brings; `processes`, `delivery` and
//!   `connect`: what an adapter gives for its app's processes, its ways
//!   in, and its config format.
//!
//! The binary installs the adapters it was built with (`install`), and
//! the core asks `platforms()`.

pub mod apps;
pub mod check;
pub mod connect;
pub mod delivery;
pub mod observe;
pub mod platform;
pub mod processes;
pub mod products;
pub mod reader;

pub use check::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
pub use platform::{install, platforms, Hook, Platform, Registry};
