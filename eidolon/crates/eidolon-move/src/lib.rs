//! An eidolon moved over any byte stream.
//!
//! The sender offers the names of the chunks its head needs; the receiver
//! answers with the ones its vault lacks; the sender sends those, still
//! sealed. Then the sender writes `pending.json`, hands over the next seat
//! and the eidolon key wrapped to the receiver's listed seal key, and the
//! receiver checks the seat, opens every chunk and acknowledges with a
//! signature. Only then does the sender delete its key.
//!
//! - [`local`]: what a machine keeps about one eidolon ([`Holding`]).
//! - [`wire`]: frames and messages.
//! - [`send`] and [`receive`]: the two sides.
//!
//! A move stopped at any point leaves at most one machine able to open the
//! eidolon. If the sender cannot tell whether the receiver took it, its copy
//! stays closed (`pending.json`) until the person settles it with
//! [`Holding::abandon_pending`] or [`Holding::confirm_moved`].

mod error;
pub mod local;
mod receive;
mod send;
#[cfg(test)]
mod tests;
pub mod wire;

pub use error::MoveError;
pub use local::{seat_first, Holding, Pending};
pub use receive::receive;
pub use send::send;

use std::time::Duration;

/// What one side of a move saw.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MoveStats {
    /// Distinct chunk names the head needs.
    pub chunks_offered: u64,
    /// Chunks that crossed the stream.
    pub chunks_sent: u64,
    /// Frame bytes this side wrote.
    pub bytes_sent: u64,
    /// Frame bytes this side read.
    pub bytes_received: u64,
    pub elapsed: Duration,
}
