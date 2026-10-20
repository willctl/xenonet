//! What each actor's work cost today (core/actors/spend.js's actorSpend
//! over crates/xenonet-actors/src/spend.rs): the live state's costs, the
//! map's sessions and who started them, from the local day's start.

use crate::js::{day_start, list, now_ms, number, text};
use crate::world::World;
use serde_json::{Map, Value};
use std::sync::Arc;
use xenonet_actors::spend::spend_by_actor;
use xenonet_base::pricing::PRICES_CHECKED;
use xenonet_service::cores::{Spend, R};

pub struct SpendDriver(pub Arc<World>);

impl Spend for SpendDriver {
    fn actor_spend(&self) -> R<Value> {
        let w = &self.0;
        let Some(a) = &w.actors else { return Ok(Value::Null) };
        let now = now_ms();
        let started = a.call(w, "sessionsStarted", &[crate::js::obj(vec![("now", xenonet_engine::sql::Arg::Num(now))])]).map_err(crate::actors::core_error)?;
        let names: Vec<(String, Value)> = list(started.field("names"))
            .into_iter()
            .map(|p| {
                let p = list(p);
                (p.first().map(text).unwrap_or_default(), p.get(1).map_or(Value::Null, crate::js::json))
            })
            .collect();
        let starts: Vec<Value> = list(started.field("starts")).iter().map(crate::js::json).collect();
        let spend = w.state().session_costs(now)?;
        let sessions: Vec<Value> = w.state().nodes().or_else(|e| if e.is_pending() { Ok(Vec::new()) } else { Err(e) })?.into_iter().filter(|n| n["kind"] == "session").collect();
        let since = day_start(now);
        let by = spend_by_actor(&spend, &sessions, &starts, &names, Some(since));
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
