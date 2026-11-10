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

use crate::registry::{Actors, Host, Result};
use serde_json::{json, Map, Value};
use xenonet_model::ids::YOU;
use xenonet_text::json::number;
use xenonet_text::units::round;

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

// The demo's actors and their scripts are data (demo.json), where a node
// of the demo's world is `{ "$": [kind, what] }`, which `Resolve` says.
// Kestrel, a cloud agent, sends no heartbeat, so it never turns away; at
// its script's end its app closed, and nothing here can say whether it
// stopped.
fn demo_data(part: &str, r: &mut dyn Resolve) -> Value {
    static DATA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    let data = DATA.get_or_init(|| serde_json::from_str(include_str!("demo.json")).expect("the demo's actors are JSON"));
    resolved(&data[part], r)
}

// A value with each node of the demo's world in it resolved.
fn resolved(v: &Value, r: &mut dyn Resolve) -> Value {
    match v {
        Value::Object(o) => match o.get("$").and_then(Value::as_array).map(Vec::as_slice) {
            Some([Value::String(kind), Value::String(what)]) if o.len() == 1 => r.resolve(kind, what),
            _ => Value::Object(o.iter().map(|(k, x)| (k.clone(), resolved(x, r))).collect()),
        },
        Value::Array(a) => Value::Array(a.iter().map(|x| resolved(x, r)).collect()),
        other => other.clone(),
    }
}

/// The demo's actors, as upsert takes them.
pub fn demo_actors(r: &mut dyn Resolve) -> Value {
    demo_data("actors", r)
}

/// Each actor's script, looped. An agent's step is what its source would
/// say: `presence` and `status`, `targets` it acts on, or an `event` as
/// facts. Yours is what the window reports.
pub fn demo_scripts(r: &mut dyn Resolve) -> Value {
    demo_data("scripts", r)
}

/// The demo's scripts at play.
pub struct Demo {
    scripts: Value,
    at: Vec<(String, usize)>,
    turn: usize,
}

// A step's object with `at` set, last.
fn with_at(v: &Value, at: f64) -> Value {
    let mut a = v.clone();
    if let Value::Object(f) = &mut a {
        f.shift_remove("at");
        f.insert("at".into(), number(at));
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
        let slot = self.at.iter_mut().find(|(k, _)| k == id).expect("each turn's actor has a script");
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
            return actors.you_report(console, when, host);
        }
        if step.get("presence").is_some() || step.get("status").is_some() {
            let mut o = Map::new();
            if let Some(inst) = instance(id) {
                o.insert("instance".into(), json!(inst));
            }
            for k in ["presence", "status"] {
                if let Some(v) = step.get(k) {
                    o.insert(k.into(), v.clone());
                }
            }
            o.insert("at".into(), number(when));
            actors.report(id, &Value::Object(o), host)?;
        }
        for t in step.get("targets").and_then(Value::as_array).cloned().unwrap_or_default() {
            let node = xenonet_text::text::template(t.get(0));
            let relation = xenonet_text::text::template(t.get(1));
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
                actors.you_report(&json!({}), when, host)?;
            } else if let Some(inst) = instance(&id) {
                actors.report(&id, &json!({ "instance": inst, "at": number(when) }), host)?;
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
        crate::store::transaction(store.as_ref(), || -> Result<()> {
            for d in defs.as_array().into_iter().flatten() {
                actors.upsert(&with_at(d, now - history), host)?;
            }
            let longest = demo.scripts.as_object().map_or(0, |o| o.values().map(|s| s.as_array().map_or(0, Vec::len)).max().unwrap_or(0));
            let turns = ((longest * 2 * TURNS.len()) as f64 / 3.0).ceil() as usize;
            for k in 0..turns {
                let when = round(now - history + ((history - MIN) * k as f64) / (turns as f64 - 1.0));
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
