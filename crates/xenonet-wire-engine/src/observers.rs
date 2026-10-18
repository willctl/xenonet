//! The observers of the actor products XenoNet observes
//! (core/actors/observers.js): Hermes Agent, Grok Bot and Muse, never in
//! the demo, whose actors are made up and whose machine isn't read.

use crate::world::World;
use serde_json::Value;

/// The observers, and their timer.
pub struct Observers {}

pub fn make(_w: &World) {}

/// What each observer reads, for the doctor.
pub fn status(_w: &World) -> Value {
    Value::Array(Vec::new())
}

pub fn start(_w: &World) {}

pub fn stop(_w: &World) {}
