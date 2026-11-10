//! xenonet-authz's tests, in one binary: the made-up world they share, the
//! decision matrix, tokens, presence, the property tests and the
//! compile-fail tests.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod common;
mod matrix;
mod presence;
mod props;
mod tokens;
mod ui;
