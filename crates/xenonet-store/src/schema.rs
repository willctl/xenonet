//! The store's tables, and its version: the hash of the statements that
//! make them. A database made by another schema has another version, and
//! is refused rather than migrated (docs/design/direction.md: no
//! backwards compatibility in the rewrite), so a change here is a new
//! version by itself. Each domain adds its tables here as it moves into
//! the store.

use sha2::{Digest, Sha256};

/// The store's own statements, before the domains'.
const TABLES: &[&str] = &[
    // What the store knows of itself. `stored` counts every chain and feed
    // entry it has taken, the order they were stored in.
    "CREATE TABLE meta (k TEXT PRIMARY KEY, v INTEGER NOT NULL) STRICT",
    "INSERT INTO meta (k, v) VALUES ('stored', 0)",
    // The chains' entries this machine holds, each by its chain and height,
    // with its hash, its JSON and where it stands in the store's order.
    "CREATE TABLE chain_entries (
        chain TEXT NOT NULL,
        n INTEGER NOT NULL,
        hash TEXT NOT NULL,
        entry TEXT NOT NULL,
        stored INTEGER NOT NULL UNIQUE,
        PRIMARY KEY (chain, n)
    ) STRICT",
];

/// The statements that make an empty store, in order: the store's own,
/// then each domain's.
pub(crate) fn tables() -> impl Iterator<Item = &'static str> {
    TABLES
        .iter()
        .chain(crate::engine::TABLES)
        .chain(crate::net::TABLES)
        .chain(crate::feed::TABLES)
        .chain(crate::state::TABLES)
        .chain(crate::delivery::TABLES)
        .chain(crate::a2a::TABLES)
        .chain(crate::projects::TABLES)
        .chain(crate::agents::TABLES)
        .chain(crate::intents::TABLES)
        .copied()
}

/// The version an up-to-date store has in `PRAGMA user_version`: the first
/// 31 bits of the SHA-256 of its statements, never 0, which is SQLite's
/// for a database nothing has set one on.
pub fn version() -> i32 {
    let mut h = Sha256::new();
    for t in tables() {
        h.update(t.as_bytes());
        h.update([0]);
    }
    let d = h.finalize();
    let v = i32::from_be_bytes([d[0], d[1], d[2], d[3]]) & i32::MAX;
    if v == 0 {
        1
    } else {
        v
    }
}
