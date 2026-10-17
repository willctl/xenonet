//! xenonet-actors' attribution for the twin of core/actors/attribution.js:
//! which agent processes a product's processes started, and which session
//! each holds. The store, the map, procs and the actors are called through
//! the JS objects the twin was given.

use crate::args::JsArg;
use crate::js::Json;
use crate::machine::{start, AskCalls, Machine};
use crate::peer::{rethrow, settle as settled, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_actors::attribution::{self, Candidate, Paths, Pending, Proc, Spawns};
use xenonet_engine::peer::{list, Calls, Thrown};
use xenonet_engine::sql::Arg;

#[napi(js_name = "attributionConstants")]
pub fn attribution_constants() -> Json {
    Json(json!({ "spawnWaitMs": attribution::SPAWN_WAIT_MS, "livedMs": attribution::LIVED_MS }))
}

fn pairs(a: Arg) -> Vec<Proc> {
    list(a).into_iter().map(|p| Proc::of(list(p).get(1).cloned().unwrap_or(Arg::Undef).to_json())).collect()
}

fn thrown(e: Error) -> Thrown {
    Thrown { message: e.reason.clone(), code: None, handle: None, stdout: None }
}

/// The census's agent processes a product's processes started: `rows`
/// the table's [pid, row] pairs, `of(row)` what the census says a row is,
/// `is_root(row)` whether it's one of the product's.
#[napi(js_name = "attrStartedUnder")]
pub fn started_under(rows: JsArg, of: Function<JsArg, JsArg>, is_root: Function<JsArg, bool>) -> Result<Json> {
    let table = pairs(rows.0);
    let row = |i: usize| JsArg(crate::args::value_to_arg(&table[i].row));
    let mut failed: Option<Error> = None;
    let mut agent_of = |i: usize| match of.call(row(i)) {
        Ok(v) => Ok(v.0.field("agent").as_str().map(str::to_string)),
        Err(e) => {
            let t = thrown(Error::new(e.status, e.reason.clone()));
            failed = Some(e);
            Err(t)
        }
    };
    let mut fail2: Option<Error> = None;
    let mut root = |i: usize| match is_root.call(row(i)) {
        Ok(b) => Ok(b),
        Err(e) => {
            let t = thrown(Error::new(e.status, e.reason.clone()));
            fail2 = Some(e);
            Err(t)
        }
    };
    let out = attribution::started_under(&table, &mut agent_of, &mut root);
    if let Some(e) = failed.or(fail2) {
        return Err(e);
    }
    let out = out.map_err(|t| Error::new(Status::GenericFailure, t.message))?;
    Ok(Json(Value::Array(out.into_iter().map(|s| json!({ "pid": xenonet_base::json::number(s.pid), "start": xenonet_base::json::number(s.start), "agent": s.agent, "root": xenonet_base::json::number(s.root) })).collect())))
}

fn pending_of(a: &Arg) -> Pending {
    let n = |k: &str| a.field(k).as_f64().unwrap_or(f64::NAN);
    Pending {
        actor: xenonet_engine::store::js_text(&a.field("actor")),
        pid: n("pid"),
        start: n("start"),
        agent: xenonet_engine::store::js_text(&a.field("agent")),
        cwd: a.field("cwd").to_json(),
        seen: n("seen"),
        gone_at: a.field("goneAt").as_f64(),
    }
}

/// What a Spawns keeps.
#[napi]
pub struct SpawnsCore {
    pub(crate) state: Arc<Mutex<Spawns>>,
}

#[napi]
impl SpawnsCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        SpawnsCore { state: Arc::new(Mutex::new(Spawns::default())) }
    }

    /// [key, record] pairs.
    #[napi]
    pub fn pending(&self) -> Json {
        Json(Value::Array(self.state.lock().unwrap().pending.iter().map(|(k, p)| json!([k, p.json()])).collect()))
    }
    #[napi]
    pub fn done(&self) -> Vec<String> {
        self.state.lock().unwrap().done.items().to_vec()
    }
    #[napi]
    pub fn claimed(&self) -> Vec<String> {
        self.state.lock().unwrap().claimed.items().to_vec()
    }
    #[napi]
    pub fn claim(&self, id: String) {
        self.state.lock().unwrap().claimed.add(id);
    }
    #[napi(js_name = "clearPending")]
    pub fn clear_pending(&self) {
        self.state.lock().unwrap().pending.clear();
    }

    /// Agent processes the actor started, each taken once, with the folder
    /// it works in (procs.cwds, for those without one).
    #[napi]
    pub fn saw(&self, actor: String, items: JsArg, at: f64) -> Machine {
        let state = self.state.clone();
        let list: Vec<Candidate> = list(items.0).iter().map(Candidate::of).collect();
        start(move |asker| {
            attribution::saw(&state, &mut AskCalls(asker), &Paths::default(), &actor, &list, at)?;
            Ok(Arg::Undef)
        })
    }

    #[napi(js_name = "sessionOf")]
    pub fn session_of(&self, env: Env, deps: Object, s: JsArg) -> Result<Option<String>> {
        let out = attribution::session_of(&self.state, &mut SyncCalls::new(&env, &deps), &Paths::default(), &pending_of(&s.0));
        let out = out.map_err(|t| rethrow(&env, t));
        settled(&env);
        out
    }

    /// The processes settled now. `alive(record)`, when given, says
    /// whether one still runs.
    #[napi]
    pub fn settle(&self, env: Env, deps: Object, at: f64, alive: Option<Function<JsArg, JsArg>>) -> Result<Json> {
        let mut calls = SyncCalls::new(&env, &deps);
        let mut check = |_: &mut dyn Calls, p: &Pending| -> xenonet_engine::peer::Result<bool> {
            let f = alive.as_ref().unwrap();
            f.call(JsArg(crate::args::value_to_arg(&p.json()))).map(|v| v.0.truthy()).map_err(|e| Thrown::new(e.reason.clone()))
        };
        let out = attribution::settle(&self.state, &mut calls, &Paths::default(), at, if alive.is_some() { Some(&mut check) } else { None });
        let out = out.map(|v| Json(Value::Array(v.iter().map(|s| s.json()).collect()))).map_err(|t| rethrow(&env, t));
        settled(&env);
        out
    }
}

/// ReportedSpawns' look: the spawned reports that wait, matched to the
/// sessions started under their actors' processes. Its collaborators:
/// actors, procs, store, map, and census() (procs.census() read with what
/// each row is).
#[napi(js_name = "reportedLook")]
pub fn reported_look(spawns: &SpawnsCore, at: f64) -> Machine {
    let state = spawns.state.clone();
    start(move |asker| {
        attribution::reported_look(&state, &mut AskCalls(asker), &Paths::default(), at)?;
        Ok(Arg::Undef)
    })
}
