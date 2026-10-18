//! What a registered actor reports, taken into the registry a part at a
//! time (core/actors/reports.js over crates/xenonet-actors/src/reports.rs),
//! and the checks on it: an event or a target against the schema
//! (core/schema/actor.js's ReportEvent and ReportTarget, as npm run
//! schemas writes them), a status line (core/actors/detail.js) and the
//! processes below the first app in a verdict's chain (verify.ownOf).

use crate::actors::{collab_call, collab_get, collab_kind};
use crate::js::{arg, at, json, no_method, now_ms, obj};
use crate::world::World;
use async_trait::async_trait;
use serde_json::{Map, Value};
use std::sync::{Arc, OnceLock};
use xenonet_actors::registration::Fail;
use xenonet_actors::reports;
use xenonet_engine::peer::{Calls, Result};
use xenonet_engine::sql::Arg;
use xenonet_schema::check::{compile, Checker, Data};
use xenonet_service::cores::{CoreError, Reports, R};

/// What a report reads besides the registry: the actor's processes as
/// given, how to read a process's lineage, the indexes already dropped,
/// and the targets to take away.
struct ReportCalls<'a> {
    w: &'a World,
    own: Option<Vec<u32>>,
    skip: Vec<usize>,
    remove: Vec<Arg>,
}

impl ReportCalls<'_> {
    fn own_set(&self) -> Option<&Vec<u32>> {
        self.own.as_ref().filter(|o| !o.is_empty())
    }
}

impl Calls for ReportCalls<'_> {
    fn call(&mut self, path: &str, method: &str, args: Vec<Arg>) -> Result<Arg> {
        let x = |i| at(&args, i);
        match (path, method) {
            ("own", "has") => Ok(Arg::Bool(self.own_set().is_some_and(|o| x(0).as_f64().is_some_and(|p| o.iter().any(|q| f64::from(*q) == p))))),
            ("skip", "has") => Ok(Arg::Bool(x(0).as_f64().is_some_and(|i| self.skip.iter().any(|s| *s as f64 == i)))),
            ("", "lineage") => Ok(self.w.checks.lineage(&x(0))),
            ("", "removeList") => Ok(Arg::List(self.remove.clone())),
            ("", "awaitSession") => {
                let own = Arg::List(self.own.clone().unwrap_or_default().into_iter().map(|p| Arg::Num(f64::from(p))).collect());
                let o = obj(vec![("app", x(1)), ("place", x(2)), ("own", own), ("at", x(3))]);
                collab_call(self.w, "actors", "awaitSession", &[x(0), o])
            }
            (p, m) if p == "actors" || p.starts_with("actors.") => collab_call(self.w, p, m, &args),
            _ => Err(no_method(path, method)),
        }
    }
    fn get(&mut self, path: &str) -> Result<Arg> {
        Ok(match path {
            "own.size" => Arg::Num(self.own.as_ref().map_or(0, Vec::len) as f64),
            "skip.size" => Arg::Num(self.skip.len() as f64),
            p => collab_get(self.w, p),
        })
    }
    fn kind(&mut self, path: &str) -> Result<String> {
        Ok(match path {
            "own" => {
                if self.own.is_some() {
                    "object"
                } else {
                    "null"
                }
            }
            "own.size" | "skip.size" => "number",
            "skip" => "object",
            "lineage" | "awaitSession" | "removeList" => "function",
            p => collab_kind(self.w, p),
        }
        .into())
    }
    fn now(&mut self) -> f64 {
        now_ms()
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
fn parse(i: usize, v: &Value) -> std::result::Result<Value, String> {
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

impl ReportsDriver {
    fn calls(&self, own: Option<Vec<u32>>, skip: Vec<usize>, remove: Vec<Arg>) -> ReportCalls<'_> {
        ReportCalls { w: &self.0, own, skip, remove }
    }
}

fn items(list: &[Value]) -> Vec<Arg> {
    list.iter().map(arg).collect()
}

#[async_trait]
impl Reports for ReportsDriver {
    fn take_heartbeat(&self, id: &str, body: &Value) -> R<()> {
        let mut c = self.calls(None, Vec::new(), Vec::new());
        reports::take_heartbeat(&mut c, &Arg::text(id), &arg(body), &Arg::Num(now_ms())).map_err(error)
    }
    async fn take_events(&self, id: &str, list: Vec<Value>, own: Option<Vec<u32>>, skip: Vec<usize>) -> R<Value> {
        let w = self.0.clone();
        let id = Arg::text(id);
        tokio::task::spawn_blocking(move || {
            let mut c = ReportCalls { w: &w, own, skip, remove: Vec::new() };
            reports::take_events(&mut c, &id, &items(&list), &Arg::Num(now_ms())).map(|t| json(&t.to_arg())).map_err(error)
        })
        .await
        .map_err(|e| CoreError::new(e.to_string()))?
    }
    async fn take_targets(&self, id: &str, targets: Vec<Value>, remove: Vec<Value>, replace: bool, own: Option<Vec<u32>>, skip: Vec<usize>) -> R<Value> {
        let w = self.0.clone();
        let id = Arg::text(id);
        tokio::task::spawn_blocking(move || {
            let mut c = ReportCalls { w: &w, own, skip, remove: items(&remove) };
            reports::take_targets(&mut c, &id, &items(&targets), replace, &Arg::Num(now_ms())).map(|t| json(&t.to_arg())).map_err(error)
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
    fn parse_event(&self, event: &Value) -> std::result::Result<Value, String> {
        parse(0, event)
    }
    fn parse_target(&self, target: &Value) -> std::result::Result<Value, String> {
        parse(1, target)
    }
    fn own_of(&self, chain: &Value) -> Vec<u32> {
        let chain = crate::js::list(arg(chain));
        xenonet_engine::verify::own_of(&chain).iter().filter_map(|p| p.field("pid").as_f64()).filter(|p| p.is_finite() && *p >= 0.0).map(|p| p as u32).collect()
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
