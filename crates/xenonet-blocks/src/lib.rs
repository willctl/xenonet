//! XenoNet's building blocks (docs/design/blocks.md), each with a lifecycle
//! and a health report: what the subsystems are made of.

pub mod clock;
pub mod gatherer;
pub mod lifecycle;
pub mod normalizer;
pub mod scanner;
pub mod store;
pub mod tailer;

/// Stops what a block was doing: the caller's own code failed, so the
/// block goes no further.
#[derive(Debug)]
pub struct Halt;
