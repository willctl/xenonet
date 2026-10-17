//! The observers of the actor products XenoNet observes
//! (core/actors/observers.js): Hermes Agent's, Grok Bot's and Muse's. Each
//! is on unless you've turned its product's observer off, and reads nothing
//! while it's off. There are none in the demo and test instances, whose
//! actors are made up.
//!
//! They look together every 5 seconds, one after the other, so they share
//! one read of the process table; after them, the registered actors'
//! spawned reports that named no session look for the session that starts
//! under their processes, with the same read.
//!
//! Its collaborators: "list" (the observers), "reported" (the reports'
//! matcher), "actors" and "clock".

use serde_json::Value;
use xenonet_engine::peer::{opt_call, Calls, Result};
use xenonet_engine::sql::Arg;

/// One look of every observer, then the waiting reports.
pub fn look(c: &mut dyn Calls) -> Result<()> {
    let n = c.get("list.length")?.as_f64().unwrap_or(0.0) as usize;
    for i in 0..n {
        c.call(&format!("list.{i}"), "tick", vec![])?;
    }
    // Before XenoNet's first look at the map, no session could be named. A
    // look that fails leaves the reports as they were told.
    if opt_call(c, "actors.map", "ready", vec![])? == Arg::Bool(false) {
        return Ok(());
    }
    let at = c.call("clock", "now", vec![])?;
    let _ = c.call("reported", "look", vec![at]);
    Ok(())
}

/// What the doctor shows of each: its own status, and whether it's on.
pub fn status(c: &mut dyn Calls) -> Result<Value> {
    let n = c.get("list.length")?.as_f64().unwrap_or(0.0) as usize;
    let mut out = Vec::new();
    for i in 0..n {
        let path = format!("list.{i}");
        let mut s = c.call(&path, "status", vec![])?.to_json();
        let on = c.call(&path, "observing", vec![])?.to_json();
        if let Value::Object(m) = &mut s {
            m.insert("on".into(), on);
        }
        out.push(s);
    }
    Ok(Value::Array(out))
}
