//! xenonet-authz's tests, in one binary: the made-up world they share, the
//! decision matrix, actors' answers, intents, tokens, presence, spaces, the property
//! tests and the compile-fail tests.

#![expect(clippy::unwrap_used, reason = "a test fails where its helpers unwrap")]

mod answers;
mod common;
mod delegate;
mod intents;
mod matrix;
mod presence;
mod props;
mod spaces;
mod tokens;
mod ui;
