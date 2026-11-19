//! Tests for the browser tools, on a fake BiDi browser and a fake driver.

#![cfg(unix)]
#![expect(clippy::unwrap_used, clippy::panic, reason = "a failing test should say where; helpers here are not #[test] fns")]

mod http;
mod mcp;
mod support;
mod tools;
