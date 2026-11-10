//! The routes' bodies checked, and the vendor readers' drift report, from the
//! service's drift log and what the map's readers read.

use crate::runtime::vendor::Vendor;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use xenonet_api::cores::{DriftReport, Schemas, R};
use xenonet_model::actor;
use xenonet_schema::drift::{source_name, DriftView, ADDITIONS};
use xenonet_schema::lenient::{load_vendor, vendor_sources};
use xenonet_schema::typed;
use xenonet_text::json::number;

/// A body checked by reading it as its type (crates/xenonet-model): the
/// schema's id names the type.
pub struct SchemaCore;

impl Schemas for SchemaCore {
    fn check(&self, schema: &str, value: &Value) -> Result<(), String> {
        let issue = match schema {
            "actor/console/v1" => typed::read::<actor::Console>(value).err(),
            "actor/registration/v1" => typed::read::<actor::Registration>(value).err(),
            "actor/report/v1" => typed::read::<actor::Report>(value).err(),
            other => return Err(format!("the body can't be checked: there's no schema called {other}")),
        };
        // `${path || "the body"} ${message}`, as the routes word it.
        issue.map_or(Ok(()), |i| Err(i.worded("the body")))
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
    fn report(&self, view: DriftView, all: bool, home: &str, now: f64) -> R<Value> {
        let (file, looked_at, records) = view;
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
        out.insert("file".into(), file.map_or(Value::Null, |f| Value::String(if home.is_empty() { f } else { xenonet_os::paths::tilde(&f, home) })));
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
    use std::path::PathBuf;
    use std::sync::Mutex;
    use xenonet_api::wire::VendorReaders;
    use xenonet_schema::drift::DriftLog;
    use xenonet_schema::vendor::Readers;

    fn checkout() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
    }

    #[test]
    fn bodies_checked_as_the_routes_word_it() {
        let s = SchemaCore;
        assert_eq!(s.check("actor/registration/v1", &json!({ "name": "Kite", "capabilities": ["run-commands"] })), Ok(()));
        let e = s.check("actor/registration/v1", &json!({ "name": 3 })).unwrap_err();
        assert!(e.starts_with("name invalid type: integer `3`"), "{e}");
        let e = s.check("actor/registration/v1", &json!({})).unwrap_err();
        assert_eq!(e, "the body missing field `name`");
        assert_eq!(
            s.check("actor/report/v1", &json!({ "events": [{ "kind": "edited", "paths": [1] }] })),
            Err("events.0.paths.0 invalid type: integer `1`, expected a string".into())
        );
        assert!(s.check("no/such/v1", &json!({})).is_err());
    }

    #[test]
    fn the_report_names_every_source() {
        let vendor = Vendor::new(checkout(), Arc::new(Mutex::new(DriftLog::default())));
        let r = Drift { vendor }.report(DriftLog::default().view(), false, "", 5.0).unwrap();
        assert_eq!(r["sources"].as_array().unwrap().len(), vendor_sources(&checkout()).len());
        assert_eq!((r["new"].as_u64(), r["changes"].as_u64(), r["all"].as_bool()), (Some(0), Some(0), Some(false)));
    }

    // The actors' observers check Hermes's files through the service's
    // readers, the ones the map's readers use, so the report counts them.
    #[test]
    fn the_report_counts_what_the_observers_read() {
        let slot = VendorReaders::default();
        let log = Arc::new(Mutex::new(DriftLog::default()));
        let vendor = Vendor::with_readers(slot.get_or_make(|| Readers::new(checkout(), log.clone())));
        // Wired after the map, the observers get its readers.
        let observers = slot.get_or_make(|| Readers::new(PathBuf::from("elsewhere"), Arc::default()));
        assert_eq!(observers.dir(), checkout());
        let stamp = "nous/hermes-agent/install-stamp";
        observers.check(stamp, &json!({ "version": "0.4.0" }), Some("0.4.0"), None, 5.0);
        let r = Drift { vendor }.report(log.lock().unwrap().view(), true, "", 5.0).unwrap();
        let hermes = r["sources"].as_array().unwrap().iter().find(|s| s["source"] == stamp).unwrap();
        assert_eq!((hermes["read"]["records"].as_u64(), &hermes["read"]["versions"]), (Some(1), &json!(["0.4.0"])));
    }

    // The service's GET /api/drift takes the log's view and asks for the
    // report while the readers record drift in the same log. Holding the
    // log's lock across the report used to deadlock the two.
    #[test]
    fn the_report_and_busy_readers_never_wait_on_each_other() {
        let log = Arc::new(Mutex::new(DriftLog::default()));
        let vendor = Vendor::new(checkout(), log.clone());
        let source = vendor_sources(&checkout()).into_iter().next().expect("a source");
        let (done, finished) = std::sync::mpsc::channel();
        let reading = {
            let (vendor, source, done) = (vendor.clone(), source, done.clone());
            std::thread::spawn(move || {
                for i in 0..2000 {
                    vendor.check(&source, json!({ "odd": i }), None, None);
                }
                let _ = done.send("reader");
            })
        };
        let reporting = std::thread::spawn(move || {
            let drift = Drift { vendor };
            for _ in 0..200 {
                let view = log.lock().unwrap_or_else(std::sync::PoisonError::into_inner).view();
                drift.report(view, true, "", 5.0).unwrap();
            }
            let _ = done.send("report");
        });
        for _ in 0..2 {
            finished.recv_timeout(std::time::Duration::from_secs(60)).expect("the reader and the report both finish");
        }
        reading.join().unwrap();
        reporting.join().unwrap();
    }
}
