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
//! - `dirs`: where the desktop apps keep their folders and binaries, which
//!   the adapters and the core both look up.
//! - `open`: the actions a node offers and the URLs they open, through
//!   each app's `Opener`.
//! - `watch`: where a platform's sessions and transcripts are written,
//!   what a change to them means, and the watcher that pushes those
//!   changes (`notify`, with a 100 ms debounce) instead of a poll.
//! - `repo`: what the core knows of a checkout (`Checkouts`), which the
//!   binary installs from the adapter of its version control (git's).
//! - `formats`: formats several crates read: Codex's apply_patch text
//!   (`patch`) and OpenCode's ids (`opencode`).
//!
//! The binary installs the adapters it was built with (`install`), and
//! the core asks `platforms()`.

pub mod apps;
pub mod check;
pub mod connect;
pub mod delivery;
pub mod dirs;
pub mod formats;
pub mod observe;
pub mod open;
pub mod platform;
pub mod processes;
pub mod products;
pub mod reader;
pub mod repo;
pub mod watch;

pub use check::{Check, CheckCtx, CheckRow, Slot, Stop, Verdict};
pub use platform::{install, platforms, Hook, Platform, Registry};
