//! Xenolith's storage and the logic every program shares: where files live, pages and their
//! frontmatter, the index, the daily log, search, the inbox, cross-process locks, the request log
//! and the secret guard. A faithful port of the Node runtime (src/*.mjs): same files, same formats,
//! so Node and Rust programs can work on one wiki at the same time. What came after the port is
//! Rust only: spaces, who may use them (access), and the audit chain.

pub mod access;
pub mod activity;
pub mod asks;
pub mod askworker;
pub mod audit;
pub mod codex;
pub mod cron;
pub mod curator;
pub mod embed;
pub mod forget;
pub mod frontmatter;
pub mod held;
pub mod inbox;
pub mod lint;
pub mod lock;
pub mod models;
pub mod paths;
pub mod readcache;
pub mod reqlog;
pub mod search;
pub mod secrets;
pub mod settings;
pub mod spaces;
pub mod sync;
pub mod sys;
pub mod text;
pub mod waker;
pub mod wiki;

pub use wiki::{Error, Result};

/// The version of this build (kept equal to package.json's).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
