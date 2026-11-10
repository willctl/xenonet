//! What a registered actor reports, taken into the registry a part at a
//! time (crates/xenonet-actors/src/reports.rs), and the checks on it: an
//! event or a target read as its type (xenonet-model's ReportEvent and
//! ReportTarget), a status line (detail.rs) and the processes below the
//! first app in a verdict's chain (verify.ownOf).

use crate::runtime::Runtime;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::sync::Arc;
use xenonet_actors::registration::Fail;
use xenonet_actors::reports;
use xenonet_api::cores::{CoreError, Reports, R};
use xenonet_engine::engine::Thrown;
use xenonet_engine::sql::Bind;
use xenonet_model::actor::{ReportEvent, ReportTarget};
use xenonet_os::sync::Locked;
use xenonet_schema::typed;
use xenonet_text::time::now_ms;

/// What a report reads besides the registry: the actor's processes as
/// given, how to read a process's lineage, the indexes already dropped,
/// and the targets to take away.
struct ReportHost<'a> {
    w: &'a Runtime,
    own: Option<Vec<u32>>,
    skip: Vec<usize>,
    remove: Vec<Value>,
}

fn thrown(t: Thrown) -> Fail {
    Fail::Error(xenonet_actors::store::Error { message: t.message, code: t.code })
}

impl ReportHost<'_> {
    /// The registry, or the refusal a call on it gets without actors.
    fn actors(&self, method: &str) -> Result<&crate::actors::ActorSide, Fail> {
        self.w.actors.as_ref().ok_or_else(|| thrown(Thrown::coded(format!("actors.{method} is not a function"), "TypeError")))
    }

    fn with<T>(
        &self,
        method: &str,
        f: impl FnOnce(&mut xenonet_actors::registry::Actors, &mut crate::actors::ActorHost<'_>) -> xenonet_actors::registry::Result<T>,
    ) -> Result<T, Fail> {
        let a = self.actors(method)?;
        a.with(self.w, f).map_err(thrown)
    }

    fn read<T: Default>(&self, f: impl FnOnce(&xenonet_actors::registry::Actors) -> T) -> T {
        self.w.actors.as_ref().map_or_else(T::default, |a| a.read(f))
    }

    fn store<T>(&self, f: impl FnOnce(&mut xenonet_engine::store::EngineStore) -> xenonet_engine::sql::Result<T>) -> Result<T, Fail> {
        self.w.store.with(f).map_err(|e| Fail::Error(crate::actors::store_error(e)))
    }
}

impl reports::Host for ReportHost<'_> {
    fn can(&mut self, id: &str, capability: &str) -> bool {
        self.read(|r| r.can(id, &Value::String(capability.into())))
    }
    fn place_of(&mut self, path: &Value) -> Value {
        self.with("placeOf", |r, h| Ok(r.place_of(path, h))).unwrap_or(Value::Null)
    }
    fn report(&mut self, id: &str, report: &Value) -> Result<(), Fail> {
        self.with("report", |r, h| r.report(id, report, h))
    }
    fn record(&mut self, id: &str, record: &Value) -> Result<(), Fail> {
        self.with("record", |r, h| r.record(id, record, h)).map(|_| ())
    }
    fn targets_of(&mut self, id: &str) -> Vec<Value> {
        self.read(|r| r.targets_of(id))
    }
    fn untarget(&mut self, id: &str, node: &str, relation: &str) -> Result<(), Fail> {
        self.with("untarget", |r, _| r.untarget(id, node, relation))
    }
    fn target(&mut self, id: &str, node: &str, relation: &str, at: f64) -> Result<(), Fail> {
        self.with("target", |r, _| r.target(id, node, relation, at))
    }
    fn pins(&mut self, session: &str) -> Result<Vec<Value>, Fail> {
        self.store(|s| s.pins(session)).map(|rows| rows.iter().map(xenonet_engine::store::row_json).collect())
    }
    fn has_actor_ref(&mut self, id: &str, r: &Value, since: f64) -> Result<bool, Fail> {
        // The ref as the event gave it: a text, or a number kept as text.
        let r = match Bind::scalar(r) {
            Some(Bind::Text(t)) => t,
            Some(Bind::Null) => return Ok(false),
            _ => xenonet_text::text::display(r),
        };
        self.store(|s| s.has_actor_ref(id, &r, since))
    }
    fn map_ready(&mut self) -> bool {
        self.w.actors.is_some() && self.w.map.ready()
    }
    fn map_has(&mut self, id: &str) -> bool {
        self.w.map.has(id).unwrap_or(false)
    }
    fn own_known(&mut self) -> bool {
        self.own.as_ref().is_some_and(|o| !o.is_empty())
    }
    fn owns(&mut self, pid: &Value) -> Option<bool> {
        let own = self.own.as_ref().filter(|o| !o.is_empty())?;
        Some(pid.as_f64().is_some_and(|p| own.iter().any(|q| f64::from(*q) == p)))
    }
    fn lineage(&mut self, pid: &Value) -> Option<Vec<Value>> {
        let line = self.w.checks.lineage(pid.as_f64())?;
        Some(line.into_iter().map(xenonet_text::json::number).collect())
    }
    fn skipped(&mut self, index: usize) -> bool {
        self.skip.contains(&index)
    }
    fn await_session(&mut self, id: &str, app: &Value, place: &Value, at: &Value) -> Result<(), Fail> {
        let a = self.actors("awaitSession")?;
        let own: Vec<f64> = self.own.clone().unwrap_or_default().into_iter().map(f64::from).collect();
        let at = xenonet_text::text::to_number(Some(at));
        a.reg.locked().await_session(id, app.clone(), place.clone(), own, at, now_ms());
        Ok(())
    }
    fn remove_list(&mut self) -> Vec<Value> {
        self.remove.clone()
    }
}

fn error(f: Fail) -> CoreError {
    crate::registration::actor_error(f)
}

/// An event (`i` 0) or a target (1) read as its type: the value as a loose
/// object gives it back (its type's keys first, then the rest), or the
/// first issue.
fn parse(i: usize, v: &Value) -> Result<Value, String> {
    let shape = match i {
        0 => typed::read::<ReportEvent>(v).map(|e| serde_json::to_value(e).unwrap_or_default()),
        _ => typed::read::<ReportTarget>(v).map(|t| serde_json::to_value(t).unwrap_or_default()),
    }
    .map_err(|issue| issue.worded("it"))?;
    let Value::Object(o) = v else { return Ok(v.clone()) };
    let mut out = Map::new();
    for k in shape.as_object().into_iter().flat_map(Map::keys) {
        if let Some(x) = o.get(k) {
            out.insert(k.clone(), x.clone());
        }
    }
    for (k, x) in o {
        if !out.contains_key(k) {
            out.insert(k.clone(), x.clone());
        }
    }
    Ok(Value::Object(out))
}

pub struct ReportsDriver(pub Arc<Runtime>);

#[async_trait]
impl Reports for ReportsDriver {
    fn take_heartbeat(&self, id: &str, body: &Value) -> R<()> {
        let mut h = ReportHost { w: &self.0, own: None, skip: Vec::new(), remove: Vec::new() };
        reports::take_heartbeat(&mut h, id, body, now_ms()).map_err(error)
    }
    async fn take_events(&self, id: &str, list: Vec<Value>, own: Option<Vec<u32>>, skip: Vec<usize>) -> R<Value> {
        let w = self.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut h = ReportHost { w: &w, own, skip, remove: Vec::new() };
            reports::take_events(&mut h, &id, &list, now_ms()).map(|t| t.to_json()).map_err(error)
        })
        .await
        .map_err(|e| CoreError::new(e.to_string()))?
    }
    async fn take_targets(&self, id: &str, targets: Vec<Value>, remove: Vec<Value>, replace: bool, own: Option<Vec<u32>>, skip: Vec<usize>) -> R<Value> {
        let w = self.0.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut h = ReportHost { w: &w, own, skip, remove };
            reports::take_targets(&mut h, &id, &targets, replace, now_ms()).map(|t| t.to_json()).map_err(error)
        })
        .await
        .map_err(|e| CoreError::new(e.to_string()))?
    }
    fn status_line(&self, status: &Value) -> Value {
        match xenonet_actors::detail::status_line(Some(status)) {
            Ok(s) => serde_json::json!({ "ok": true, "status": s }),
            Err(reason) => serde_json::json!({ "ok": false, "reason": reason }),
        }
    }
    fn parse_event(&self, event: &Value) -> Result<Value, String> {
        parse(0, event)
    }
    fn parse_target(&self, target: &Value) -> Result<Value, String> {
        parse(1, target)
    }
    fn own_of(&self, chain: &Value) -> Vec<u32> {
        let chain = chain.as_array().map_or(&[][..], Vec::as_slice);
        crate::verify::own_of(chain).iter().filter_map(|p| p.get("pid").and_then(Value::as_f64)).filter(|p| p.is_finite() && *p >= 0.0).map(|p| p as u32).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_and_targets_are_checked_as_their_types() {
        assert_eq!(parse(0, &json!({ "at": 5, "kind": "edited", "extra": 1 })).unwrap(), json!({ "kind": "edited", "at": 5, "extra": 1 }));
        let e = parse(0, &json!({ "kind": "edited", "paths": [1] })).unwrap_err();
        assert!(e.starts_with("paths.0 "), "{e}");
        assert_eq!(parse(0, &json!({})).unwrap_err(), "it missing field `kind`");
        assert!(parse(0, &json!(7)).unwrap_err().starts_with("it "));
        assert_eq!(parse(1, &json!({ "path": "/w/x" })).unwrap(), json!({ "path": "/w/x" }));
    }
}
