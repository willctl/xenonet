//! A WebDriver BiDi client that speaks WebSocket over any byte stream.
//!
//! Eidolon's browser runs in a VM and its BiDi endpoint is never forwarded
//! to the host, so the host reaches it as a stream (the stdin and stdout of
//! a command run in the cell). The client therefore takes any
//! `AsyncRead + AsyncWrite` stream and opens no sockets of its own.
//!
//! - [`frame`]: WebSocket frames as pure functions over bytes.
//! - [`handshake`] and [`ws`]: the upgrade and whole messages, for either end.
//! - [`Session`]: BiDi commands, answers and events, with typed helpers for
//!   what the browser facet needs ([`commands`]).

pub mod commands;
mod error;
pub mod frame;
pub mod handshake;
mod session;
pub mod ws;

pub use commands::{ContextInfo, Navigation, Node, Wait};
pub use error::Error;
pub use session::{Event, Options, Session};
