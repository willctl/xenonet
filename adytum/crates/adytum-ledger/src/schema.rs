use sha2::{Digest, Sha256};

/// The ledger's tables. Changing a word here changes [`version`], and every
/// machine with an older ledger is set up again.
pub(crate) const TABLES: &[&str] = &[
    "CREATE TABLE cells (
        name TEXT PRIMARY KEY,
        id TEXT NOT NULL,
        driver TEXT NOT NULL,
        wall TEXT NOT NULL,
        spec_json TEXT NOT NULL,
        state TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        settled_at INTEGER
    ) WITHOUT ROWID",
    "CREATE TABLE ops (
        id INTEGER PRIMARY KEY,
        name TEXT NOT NULL,
        kind TEXT NOT NULL CHECK (kind IN ('create', 'remove', 'start', 'stop')),
        started_at INTEGER NOT NULL,
        settled_at INTEGER,
        outcome TEXT
    )",
    "CREATE INDEX ops_open ON ops (id) WHERE settled_at IS NULL",
    "CREATE TABLE holds (
        name TEXT NOT NULL,
        eidolon TEXT NOT NULL,
        PRIMARY KEY (name, eidolon)
    ) WITHOUT ROWID",
];

/// What `PRAGMA user_version` holds in an up-to-date ledger: the first 31
/// bits of the SHA-256 of the tables' text, never 0 (SQLite's value for a
/// database nothing has versioned).
pub(crate) fn version() -> i32 {
    let mut h = Sha256::new();
    for t in TABLES {
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
