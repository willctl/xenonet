//! What a registered actor reports, taken into the registry a part at a
//! time (crates/xenonet-actors/src/reports.rs), and the checks on it: an
//! event or a target against the schema (schemas/actor/report/v1.json's
//! ReportEvent and ReportTarget, as npm run schemas writes them), a status
//! line (detail.rs) and the processes below the first app in a verdict's
//! chain (verify.ownOf).

use crate::js::{arg, items, json, now_ms, Locked};
use crate::world::World;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::sync::{Arc, OnceLock};
use xenonet_actors::registration::Fail;
use xenonet_actors::reports;
use xenonet_engine::peer::Thrown;
use xenonet_engine::sql::Arg;
use xenonet_schema::check::{compile, Checker, Data};
use xenonet_service::cores::{CoreError, Reports, R};

/// What a report reads besides the registry: the actor's processes as
/// given, how to read a process's lineage, the indexes already dropped,
/// and the targets to take away.
struct ReportHost<'a> {
    w: &'a World,
    own: Option<Vec<u32>>,
    skip: Vec<usize>,
    remove: Vec<Value>,
}

fn thrown(t: Thrown) -> Fail {
    Fail::Error(crate::actors::store_error(t))
}

impl ReportHost<'_> {
    /// The registry, or the refusal a call on it gets without actors.
    fn actors(&self, method: &str) -> Result<&crate::actors::ActorSide, Fail> {
        self.w.actors.as_ref().ok_or_else(|| thrown(crate::js::no_method("actors", method)))
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

    fn store(&self, name: &str, args: &[Arg]) -> Result<Value, Fail> {
        crate::store::call(&self.w.store, name, args).map(|a| json(&a)).map_err(thrown)
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
        self.store("pins", &[Arg::text(session)]).map(items)
    }
    fn has_actor_ref(&mut self, id: &str, r: &Value, since: f64) -> Result<bool, Fail> {
        self.store("hasActorRef", &[Arg::text(id), arg(r), Arg::Num(since)]).map(|v| crate::js::truthy_json(&v))
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
        let line = self.w.checks.lineage(&arg(pid));
        if line.is_nullish() {
            return None;
        }
        Some(items(json(&line)))
    }
    fn skipped(&mut self, index: usize) -> bool {
        self.skip.contains(&index)
    }
    fn await_session(&mut self, id: &str, app: &Value, place: &Value, at: &Value) -> Result<(), Fail> {
        let a = self.actors("awaitSession")?;
        let own: Vec<f64> = self.own.clone().unwrap_or_default().into_iter().map(f64::from).collect();
        let at = xenonet_base::text::js_number(Some(at));
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

/// ReportEvent and ReportTarget, as the report's schema has them.
struct Schemas {
    checker: Checker,
    shapes: [Vec<String>; 2],
}

fn schemas() -> Option<&'static Schemas> {
    static S: OnceLock<Option<Schemas>> = OnceLock::new();
    S.get_or_init(|| {
        let doc: Value = xenonet_base::json::parse(include_str!("../../../schemas/actor/report/v1.json")).ok()?;
        let p = doc.get("properties")?;
        let event = p.get("events")?.get("items")?.clone();
        let target = p.get("targets")?.get("items")?.clone();
        let keys = |v: &Value| v.get("properties").and_then(Value::as_object).map(|o| o.keys().cloned().collect()).unwrap_or_default();
        let shapes = [keys(&event), keys(&target)];
        let checker = compile(&[event, target]).ok()?;
        Some(Schemas { checker, shapes })
    })
    .as_ref()
}

/// A path as zod's issues join it: keys and indexes with dots.
fn dotted(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('[') {
            let end = r.find(']').unwrap_or(r.len());
            let inner = &r[..end];
            let key = xenonet_base::json::parse(inner).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| inner.to_string());
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&key);
            rest = r.get(end + 1..).unwrap_or("");
        } else {
            let r = rest.strip_prefix('.').unwrap_or(rest);
            let end = r.find(['.', '[']).unwrap_or(r.len());
            if !out.is_empty() {
                out.push('.');
            }
            out.push_str(&r[..end]);
            rest = &r[end..];
        }
    }
    out
}

/// `X.safeParse(v)`: the value as a loose object gives it back (its
/// shape's keys first, then the rest), or the first issue.
fn parse(i: usize, v: &Value) -> Result<Value, String> {
    let Some(s) = schemas() else { return Err("it can't be checked: the report's schema isn't there".into()) };
    let checked = s.checker.check(s.checker.root(i), &Data::from(v), false);
    if let Some(first) = checked.issues.first() {
        let path = dotted(&first.path);
        return Err(format!("{} {}", if path.is_empty() { "it" } else { &path }, first.message));
    }
    let Value::Object(o) = v else { return Ok(v.clone()) };
    let mut out = Map::new();
    for k in &s.shapes[i] {
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

pub struct ReportsDriver(pub Arc<World>);

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
        let chain = crate::js::list(arg(chain));
        crate::verify::own_of(&chain).iter().filter_map(|p| p.field("pid").as_f64()).filter(|p| p.is_finite() && *p >= 0.0).map(|p| p as u32).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn events_and_targets_are_checked_as_zod_checks_them() {
        assert_eq!(parse(0, &json!({ "at": 5, "kind": "edited", "extra": 1 })).unwrap(), json!({ "kind": "edited", "at": 5, "extra": 1 }));
        let e = parse(0, &json!({ "kind": "edited", "ref": "no spaces allowed" })).unwrap_err();
        assert!(e.starts_with("ref "), "{e}");
        let e = parse(0, &json!({ "kind": "edited", "paths": [1] })).unwrap_err();
        assert!(e.starts_with("paths.0 "), "{e}");
        assert!(parse(0, &json!({})).unwrap_err().starts_with("kind "));
        assert!(parse(0, &json!(7)).unwrap_err().starts_with("it "));
        assert_eq!(parse(1, &json!({ "path": "/w/x" })).unwrap(), json!({ "path": "/w/x" }));
        assert_eq!(dotted("a[3].b[\"x y\"]"), "a.3.b.x y");
    }
}
