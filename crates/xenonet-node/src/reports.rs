//! xenonet-actors' reports for the twin of core/actors/reports.js: what a
//! registered actor's heartbeats, events and targets come to. The registry
//! and the rest are called through the JS objects the twin was given.

use crate::args::JsArg;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{settle, SyncCalls};
use crate::registration::fail;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use xenonet_actors::registration::Fail;
use xenonet_actors::reports;
use xenonet_engine::peer::{list, Thrown};

fn thrown(f: Fail) -> Thrown {
    match f {
        Fail::Thrown(t) => t,
        Fail::Actor(e) => Thrown::new(e.message),
    }
}

/// A heartbeat taken: the registry told, or an ActorError for the twin.
#[napi(js_name = "reportsHeartbeat")]
pub fn heartbeat(env: Env, deps: Object<'_>, id: JsArg, body: JsArg, now: JsArg) -> Result<()> {
    let out = reports::take_heartbeat(&mut SyncCalls::new(&env, &deps), &id.0, &body.0, &now.0);
    match out {
        Ok(()) => {
            settle(&env);
            Ok(())
        }
        Err(f) => Err(fail(&env, f)),
    }
}

/// The events taken: { accepted, dropped }.
#[napi(js_name = "reportsEvents")]
pub fn events(id: JsArg, events: JsArg, now: JsArg) -> Machine {
    let events = list(events.0);
    start(move |asker| reports::take_events(&mut AskCalls(asker), &id.0, &events, &now.0).map(|t| t.to_arg()).map_err(thrown))
}

/// The targets taken: { accepted, dropped }.
#[napi(js_name = "reportsTargets")]
pub fn targets(id: JsArg, targets: JsArg, replace: bool, now: JsArg) -> Machine {
    let targets = list(targets.0);
    start(move |asker| reports::take_targets(&mut AskCalls(asker), &id.0, &targets, replace, &now.0).map(|t| t.to_arg()).map_err(thrown))
}
