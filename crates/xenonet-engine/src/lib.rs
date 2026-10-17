//! The engine (core/engine/, core/delivery/ and core/procs.js): which
//! process is on the other end of a connection (`procs`), whether a request
//! made in an agent's or an actor's name comes from it (`verify`), and the
//! rest as they're ported.
//!
//! What a module does with the outside world (running a command, reading
//! the clock, the store) goes through a trait of its own, so the same code
//! runs in the binary and behind a JS twin, where the twin does the reading.

pub mod js;
pub mod jsre;
pub mod procs;
pub mod products;
pub mod sql;
pub mod store;
