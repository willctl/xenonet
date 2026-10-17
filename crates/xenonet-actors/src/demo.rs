//! The demo world's actors (core/actors/demo.js): you, and three made-up
//! agents at work on the demo's own projects and sessions:
//!
//!   Wren     a local agent (Hermes Agent), observed and attributed: it
//!            works on atlas-api and ledger and starts Claude Code sessions
//!            in them;
//!   Kestrel  a cloud agent (Grok Bot) that runs commands in pathfinder
//!            through its desktop app on this machine, seen only while the
//!            app runs;
//!   Moth     a custom actor that registered itself: it watches geofence
//!            and says things to the agents there.
//!
//! Each plays a script a step at a time, like the demo's sessions: a few
//! hours of it before now, so there's history, and then a step every few
//! seconds while the server runs. The scripts go through the registry's
//! own calls. The demo world's nodes (its projects, sessions and files,
//! and Wren's Claude account) are the world's to say: `Resolve` asks.

use crate::registry::{value_arg, Actors, Host, Result};
use serde_json::{json, Value};
use xenonet_base::ids::YOU;
use xenonet_base::units::js_round;
use xenonet_engine::sql::Arg;

const WREN: &str = "actor:wren";
const KESTREL: &str = "actor:kestrel";
const MOTH: &str = "actor:moth";
const MIN: f64 = 60e3;

pub const STEP_MS: f64 = 3000.0;
// Whose turn each tick is: the agents in turn, and you now and then.
const TURNS: [&str; 7] = [WREN, KESTREL, MOTH, WREN, KESTREL, MOTH, YOU];

// The instance each agent reports as: Hermes's gateway, Grok Bot's app, and
// the program Moth runs as.
fn instance(id: &str) -> Option<&'static str> {
    match id {
        WREN => Some("gateway"),
        KESTREL => Some("app"),
        MOTH => Some("main"),
        _ => None,
    }
}

/// The demo world's nodes, by what they are: ("project", "atlas-api") its
/// node id, ("session", title) its id or null, ("file", "atlas-api/…") its
/// path, ("identity", "ledger") the identity of that project's Claude
/// account.
pub trait Resolve {
    fn resolve(&mut self, kind: &str, what: &str) -> Value;
}

/// The demo's actors, as upsert takes them.
pub fn demo_actors(r: &mut dyn Resolve) -> Value {
    json!([
        {
            "id": WREN,
            "name": "Wren",
            "product": { "id": "hermes-agent", "vendor": "Nous Research", "version": "0.21.5" },
            "via": ["demo", "observed", "attributed"],
            "heartbeatMs": 60000,
            "capabilities": ["spawn-sessions", "edit-repos", "run-commands", "message-people", "schedule"],
            "identity": r.resolve("identity", "ledger"),
        },
        {
            "id": KESTREL,
            "name": "Kestrel",
            "product": { "id": "grok-bot", "vendor": "SpaceXAI", "version": null },
            "via": ["demo", "observed"],
            // A cloud agent sends no heartbeat, so it never turns away.
            "heartbeatMs": null,
            "capabilities": ["run-commands", "browse", "message-people"],
        },
        {
            "id": MOTH,
            "name": "Moth",
            "via": ["demo", "registered"],
            "heartbeatMs": 30000,
            "capabilities": ["message-agents", "message-people"],
        },
    ])
}

/// Each actor's script, looped. An agent's step is what its source would
/// say: `presence` and `status`, `targets` it acts on, or an `event` as
/// facts. Yours is what the window reports.
pub fn demo_scripts(r: &mut dyn Resolve) -> Value {
    let mut p = |rel: &str| r.resolve("project", rel);
    let (atlas, ledger, pathfinder, geofence) = (p("atlas-api"), p("ledger"), p("pathfinder"), p("labs/geofence"));
    let refunds = r.resolve("session", "Add partial refunds to the orders API");
    let close = r.resolve("session", "Draft the Q4 close checklist");
    let zones = r.resolve("session", "Draw delivery zones from postcodes");
    let review = r.resolve("session", "Review the partial refunds change");
    let file = r.resolve("file", "atlas-api/src/orders/refunds.ts");
    json!({
        WREN: [
            { "presence": "working" },
            { "status": "Triaging issues in atlas-api", "targets": [[atlas, "works-on"]] },
            { "event": { "kind": "spawned", "target": refunds, "via": "attributed" } },
            { "event": { "kind": "edited", "target": atlas, "paths": [file], "via": "attributed" } },
            { "event": { "kind": "ran", "target": atlas, "count": 3, "via": "attributed" } },
            { "event": { "kind": "finished", "target": atlas, "subject": "session" } },
            { "status": "Closing the month in ledger", "targets": [[ledger, "works-on"]] },
            { "event": { "kind": "spawned", "target": close, "via": "attributed" } },
            { "event": { "kind": "messaged", "platform": "telegram" } },
            { "presence": "idle", "status": "" },
        ],
        KESTREL: [
            { "presence": "idle" },
            { "presence": "working", "status": "Benchmarking the route solver", "targets": [[pathfinder, "works-on"]] },
            { "event": { "kind": "ran", "target": pathfinder, "count": 1 } },
            { "event": { "kind": "ran", "target": pathfinder, "count": 4 } },
            { "event": { "kind": "failed", "target": pathfinder, "subject": "command" } },
            { "event": { "kind": "ran", "target": pathfinder, "count": 2 } },
            { "event": { "kind": "finished", "target": pathfinder, "subject": "task" } },
            // Its app closed: nothing here can say whether it stopped.
            { "presence": "unknown", "status": "" },
        ],
        MOTH: [
            { "presence": "working" },
            { "status": "Watching geofence", "targets": [[geofence, "watches"]] },
            { "event": { "kind": "said", "target": zones, "count": 1 } },
            { "event": { "kind": "messaged", "platform": "email" } },
            { "presence": "idle" },
        ],
        YOU: [
            { "console": { "presence": "working", "did": [{ "what": "looked", "node": atlas }] } },
            { "console": { "did": [{ "what": "read", "node": review }] } },
            { "console": { "did": [{ "what": "started", "node": atlas, "app": "claude" }] } },
            { "console": { "did": [{ "what": "terminal", "node": ledger }] } },
            { "console": { "presence": "idle" } },
        ],
    })
}

/// The demo's scripts at play.
pub struct Demo {
    scripts: Value,
    at: Vec<(String, usize)>,
    turn: usize,
}

fn with_at(v: &Value, at: f64) -> Arg {
    let mut a = value_arg(v);
    if let Arg::Obj(f) = &mut a {
        f.retain(|(k, _)| k != "at");
        f.push(("at".into(), Arg::Num(at)));
    }
    a
}

impl Demo {
    // An actor you removed, or whose product's observer you turned off, is
    // left out until the demo starts again.
    fn playing(actors: &Actors, id: &str) -> bool {
        actors.has(id) && actors.enabled(id) && !actors.simulation_held(id)
    }

    fn play<H: Host + Resolve>(&mut self, actors: &mut Actors, id: &str, when: f64, host: &mut H) -> Result<Value> {
        let script = self.scripts[id].as_array().cloned().unwrap_or_default();
        let slot = self.at.iter_mut().find(|(k, _)| k == id).unwrap();
        let step = script[slot.1 % script.len()].clone();
        slot.1 += 1;
        if !Self::playing(actors, id) {
            return Ok(Value::Null);
        }
        // You turned yourself off the map: the made-up you keeps still until
        // the page reports again.
        if id == YOU && actors.you.off {
            return Ok(Value::Null);
        }
        if let Some(console) = step.get("console") {
            return actors.you_report(&value_arg(console), when, host);
        }
        if step.get("presence").is_some() || step.get("status").is_some() {
            let mut o = vec![("instance".to_string(), instance(id).map_or(Arg::Undef, Arg::text))];
            for k in ["presence", "status"] {
                if let Some(v) = step.get(k) {
                    o.push((k.into(), value_arg(v)));
                }
            }
            o.push(("at".into(), Arg::Num(when)));
            actors.report(id, &Arg::Obj(o), host)?;
        }
        for t in step.get("targets").and_then(Value::as_array).cloned().unwrap_or_default() {
            let node = xenonet_engine::js::template(t.get(0));
            let relation = xenonet_engine::js::template(t.get(1));
            actors.target(id, &node, &relation, when)?;
        }
        if let Some(e) = step.get("event") {
            actors.record(id, &with_at(e, when), host)?;
        }
        Ok(Value::Null)
    }

    // Those that are there say so between their steps: an observer looking
    // again, a heartbeat, the window's minute.
    fn beat<H: Host + Resolve>(actors: &mut Actors, when: f64, host: &mut H) -> Result<()> {
        for id in actors.ids() {
            if actors.simulation_held(&id) {
                continue;
            }
            let p = actors.presence(&id).unwrap_or_default();
            if !matches!(p.as_str(), "working" | "idle" | "away") || !actors.enabled(&id) {
                continue;
            }
            if id == YOU {
                actors.you_report(&Arg::Obj(vec![]), when, host)?;
            } else if let Some(inst) = instance(&id) {
                actors.report(&id, &Arg::Obj(vec![("instance".into(), Arg::text(inst)), ("at".into(), Arg::Num(when))]), host)?;
            }
        }
        Ok(())
    }

    /// Adds the demo's actors to `actors`, with the last `history` ms of
    /// their scripts behind them.
    pub fn add<H: Host + Resolve>(actors: &mut Actors, history: f64, host: &mut H) -> Result<Demo> {
        let now = host.now();
        let scripts = demo_scripts(host);
        let at: Vec<(String, usize)> = scripts.as_object().map(|o| o.keys().map(|k| (k.clone(), 0)).collect()).unwrap_or_default();
        let mut demo = Demo { scripts, at, turn: 0 };
        let defs = demo_actors(host);
        let store = actors.store.clone();
        store.transaction(|| -> Result<()> {
            for d in defs.as_array().unwrap() {
                actors.upsert(&with_at(d, now - history), host)?;
            }
            let longest = demo.scripts.as_object().map_or(0, |o| o.values().map(|s| s.as_array().map_or(0, Vec::len)).max().unwrap_or(0));
            let turns = ((longest * 2 * TURNS.len()) as f64 / 3.0).ceil() as usize;
            for k in 0..turns {
                let when = js_round(now - history + ((history - MIN) * k as f64) / (turns as f64 - 1.0));
                Self::beat(actors, when, host)?;
                demo.play(actors, TURNS[k % TURNS.len()], when, host)?;
            }
            Self::beat(actors, now, host)
        })?;
        Ok(demo)
    }

    /// The next turn, as the driver plays it every STEP_MS.
    pub fn step<H: Host + Resolve>(&mut self, actors: &mut Actors, host: &mut H) -> Result<Value> {
        let when = host.now();
        Self::beat(actors, when, host)?;
        let id = TURNS[self.turn % TURNS.len()];
        self.turn += 1;
        self.play(actors, id, when, host)
    }
}
