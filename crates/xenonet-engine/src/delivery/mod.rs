//! Live delivery (core/delivery/): into a session now, by the fastest way
//! in that's allowed, under the policy's guards.

pub mod channel;
pub mod opencode;
pub mod policy;
pub mod resume;
pub mod router;
pub mod selector;
pub mod setup;
pub mod tmux;
pub mod usage;
