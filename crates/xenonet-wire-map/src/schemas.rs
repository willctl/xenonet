//! core/schema/index.js's check, for the routes: XenoNet's JSON Schemas
//! (schemas/, which npm run schemas writes from core/schema) checked as
//! zod checks them (xenonet-schema's check.rs), and core/vendor.js's drift
//! report, from the service's drift log and what the map's readers read.

use crate::vendor::Vendor;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use xenonet_base::json::number;
use xenonet_schema::check::{compile, Checker, Data};
use xenonet_schema::drift::{source_name, DriftLog, ADDITIONS};
use xenonet_schema::lenient::{load_vendor, vendor_sources};
use xenonet_service::cores::{DriftReport, Schemas, R};

/// Every document under schemas/ but the vendors' (which the lenient
/// readers load), by id: the file's path without .json.
fn documents(dir: &Path) -> Vec<(String, Value)> {
    fn walk(dir: &Path, rel: &str, out: &mut Vec<(String, Value)>) {
        let Ok(list) = xenonet_base::paths::read_dir_sorted(dir) else { return };
        for e in list {
            let name = e.file_name().to_string_lossy().into_owned();
            let at = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() {
                if at != "vendor" {
                    walk(&e.path(), &at, out);
                }
            } else if let Some(id) = at.strip_suffix(".json") {
                let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
                let Ok(doc) = xenonet_base::json::parse(&text) else { continue };
                out.push((id.to_string(), doc));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out);
    out
}

// The documents compiled once, with each one's root by id.
type Compiled = Result<(Checker, HashMap<String, usize>), String>;

pub struct SchemaCore {
    dir: PathBuf,
    compiled: OnceLock<Compiled>,
}

impl SchemaCore {
    pub fn new(dir: PathBuf) -> SchemaCore {
        SchemaCore { dir, compiled: OnceLock::new() }
    }

    fn compiled(&self) -> &Compiled {
        self.compiled.get_or_init(|| {
            let docs = documents(&self.dir);
            if docs.is_empty() {
                return Err(format!("XenoNet's schemas aren't in {}", self.dir.display()));
            }
            let list: Vec<Value> = docs.iter().map(|(_, d)| d.clone()).collect();
            let checker = compile(&list)?;
            let roots = docs.iter().enumerate().map(|(i, (id, _))| (id.clone(), checker.root(i))).collect();
            Ok((checker, roots))
        })
    }
}

impl Schemas for SchemaCore {
    fn check(&self, schema: &str, value: &Value) -> Result<(), String> {
        let (checker, roots) = self.compiled().as_ref().map_err(|e| format!("the body can't be checked: {e}"))?;
        let Some(&root) = roots.get(schema) else { return Err(format!("the body can't be checked: there's no schema called {schema}")) };
        let checked = checker.check(root, &Data::from(value), false);
        match checked.issues.first() {
            None => Ok(()),
            // `${path || "the body"} ${message}`, as the routes word it.
            Some(i) => Err(format!("{} {}", if i.path.is_empty() { "the body" } else { &i.path }, i.message).trim().to_string()),
        }
    }
}

/// driftReport({ log, all, home, now }), with what this XenoNet's readers
/// have read.
pub struct Drift {
    pub vendor: Arc<Vendor>,
}

fn first_seen(r: &Value) -> f64 {
    r.get("firstSeen").and_then(Value::as_f64).unwrap_or(f64::NAN)
}

impl DriftReport for Drift {
    fn report(&self, log: &mut DriftLog, all: bool, home: &str, now: f64) -> R<Value> {
        let (file, looked_at, records) = log.view();
        let newer = |r: &Value| looked_at.is_none_or(|l| first_seen(r) > l);
        let dir = self.vendor.dir();
        let sources: Vec<Value> = vendor_sources(dir)
            .into_iter()
            .map(|source| {
                let mine: Vec<&Value> = records.iter().filter(|r| r.get("source").and_then(Value::as_str) == Some(source.as_str())).collect();
                let schemas: Vec<Value> = load_vendor(&source, dir).into_iter().map(|m| json!({ "id": m.id, "major": m.major, "range": m.range })).collect();
                json!({
                    "source": source,
                    "name": source_name(&source),
                    "schemas": schemas,
                    "read": self.vendor.read_of(&source).unwrap_or(Value::Null),
                    "drift": mine.iter().filter(|r| all || newer(r)).map(|r| (*r).clone()).collect::<Vec<_>>(),
                    "known": mine.len(),
                })
            })
            .collect();
        let is_addition = |r: &&Value| r.get("kind").and_then(Value::as_str).is_some_and(|k| ADDITIONS.contains(&k));
        let fresh: Vec<&Value> = sources.iter().flat_map(|s| s["drift"].as_array().into_iter().flatten()).filter(|r| newer(r)).collect();
        let (new, changes) = (fresh.len(), fresh.iter().filter(|r| !is_addition(r)).count());
        let known: u64 = sources.iter().map(|s| s["known"].as_u64().unwrap_or(0)).sum();
        let mut out = Map::new();
        out.insert("at".into(), number(now));
        out.insert("file".into(), file.map_or(Value::Null, |f| Value::String(if home.is_empty() { f } else { xenonet_base::paths::tilde(&f, home) })));
        out.insert("since".into(), looked_at.map_or(Value::Null, number));
        out.insert("all".into(), Value::Bool(all));
        out.insert("sources".into(), Value::Array(sources));
        out.insert("new".into(), json!(new));
        out.insert("changes".into(), json!(changes));
        out.insert("known".into(), json!(known));
        Ok(Value::Object(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn checkout() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
    }

    #[test]
    fn bodies_checked_as_the_routes_word_it() {
        let s = SchemaCore::new(checkout());
        assert_eq!(s.check("actor/registration/v1", &json!({ "name": "Kite", "capabilities": ["run-commands"] })), Ok(()));
        let e = s.check("actor/registration/v1", &json!({ "name": 3 })).unwrap_err();
        assert!(e.starts_with("name "), "{e}");
        assert!(s.check("no/such/v1", &json!({})).is_err());
    }

    #[test]
    fn the_report_names_every_source() {
        let vendor = Vendor::new(checkout(), Arc::new(Mutex::new(DriftLog::default())));
        let mut log = DriftLog::default();
        let r = Drift { vendor }.report(&mut log, false, "", 5.0).unwrap();
        assert_eq!(r["sources"].as_array().unwrap().len(), vendor_sources(&checkout()).len());
        assert_eq!((r["new"].as_u64(), r["changes"].as_u64(), r["all"].as_bool()), (Some(0), Some(0), Some(false)));
    }
}
