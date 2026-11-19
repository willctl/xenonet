//! The browser facet against fakes: a fake driver, a fake BiDi server and a
//! host folder standing in for a cell.
#![cfg(unix)]
#![expect(clippy::unwrap_used, reason = "a failing test should say where; helpers here are not #[test] fns")]

mod browser;
mod facet;
mod plan;
mod support;
