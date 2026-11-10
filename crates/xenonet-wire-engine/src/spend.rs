//! What each actor's work cost today (core/actors/spend.js's actorSpend
//! over crates/xenonet-actors/src/spend.rs): the live state's costs, the
//! map's sessions and who started them, from the local day's start.

use crate::runtime::day_start;
use crate::runtime::Runtime;
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};
use xenonet_actors::spend::{reported_today, spend_by_actor};
use xenonet_base::json::number;
use xenonet_base::pricing::PRICES_CHECKED;
use xenonet_base::time::now_ms;
use xenonet_service::cores::{Spend, R};

pub struct SpendDriver {
    world: Arc<Runtime>,
    // What Hermes and OpenCode reported at first sight and since today
    // began (spend::reported_today), kept between requests.
    reported: Mutex<Value>,
}

impl SpendDriver {
    pub fn new(world: Arc<Runtime>) -> Self {
        Self { world, reported: Mutex::new(Value::Null) }
    }
}

impl Spend for SpendDriver {
    fn actor_spend(&self) -> R<Value> {
        let w = &self.world;
        let Some(a) = &w.actors else { return Ok(Value::Null) };
        let now = now_ms();
        let (names, starts) = a.read(|r| r.sessions_started(now)).map_err(|f| crate::actors::core_error(crate::actors::thrown(f)))?;
        let spend = w.state().session_costs(now)?;
        let sessions: Vec<Value> = w.state().nodes().or_else(|e| if e.is_pending() { Ok(Vec::new()) } else { Err(e) })?.into_iter().filter(|n| n["kind"] == "session").collect();
        let since = day_start(now);
        // Each actor's usage, as its snapshot says it.
        let mut usages: Vec<Value> = Vec::new();
        for (id, _) in &names {
            let snap = a.with(w, |r, h| r.snapshot(id, "public", Some(now), None, h)).map_err(crate::actors::core_error)?;
            if snap.get("usage").is_some_and(Value::is_object) {
                usages.push(serde_json::json!({ "id": id, "product": snap.get("product").cloned().unwrap_or(Value::Null), "usage": snap["usage"] }));
            }
        }
        let reported = {
            let mut st = self.reported.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let (next, items) = reported_today(&st, since, &usages, &sessions);
            *st = next;
            items
        };
        let by = spend_by_actor(&spend, &sessions, &starts, &names, Some(since), &reported);
        let mut out = Map::new();
        out.insert("at".into(), number(now));
        out.insert("since".into(), number(since));
        out.insert("checked".into(), Value::String(PRICES_CHECKED.into()));
        if let Value::Object(m) = by {
            out.extend(m);
        }
        Ok(Value::Object(out))
    }
}
