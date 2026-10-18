//! xenonet-schema's drift log and vendor readers, for the twins of
//! core/drift.js and core/vendor.js.

use crate::js::{Json, Jv};
use crate::schema::JsSpec;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use xenonet_schema::drift::{self, DriftLog, Piece};
use xenonet_schema::lenient::Major;
use xenonet_schema::vendor::{self, Checked, Sampling, VendorReader, NO_VERSION};

fn piece(v: &Value) -> Piece {
    let f = |k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let s = |k: &str| v.get(k).map_or("undefined".into(), xenonet_base::text::js_string);
    Piece { source: s("source"), kind: s("kind"), path: f("path"), variant: f("variant"), shape: f("shape"), type_: f("type"), vendor_version: f("vendorVersion") }
}

#[napi(object)]
pub struct DriftConstants {
    pub max: u32,
    pub file: String,
    pub kinds: Vec<String>,
    pub additions: Vec<String>,
}

#[napi(js_name = "driftConstants")]
pub fn drift_constants() -> DriftConstants {
    DriftConstants {
        max: drift::DRIFT_MAX as u32,
        file: drift::DRIFT_FILE.into(),
        kinds: drift::KINDS.iter().map(ToString::to_string).collect(),
        additions: drift::ADDITIONS.iter().map(ToString::to_string).collect(),
    }
}

#[napi(js_name = "sourceName")]
pub fn source_name(source: String) -> String {
    drift::source_name(&source).to_string()
}

#[napi(js_name = "isField")]
pub fn is_field(k: String) -> bool {
    xenonet_schema::fields::is_field(&k)
}

#[napi(js_name = "shapeOf")]
pub fn shape_of(value: Json, depth: f64, budget: f64, names: bool) -> String {
    drift::shape_of(&value.0, depth as i64, budget as i64, names)
}

#[napi(js_name = "variantShape")]
pub fn variant_shape(value: Json) -> String {
    drift::variant_shape(&value.0)
}

#[napi(js_name = "cleanVersion")]
pub fn clean_version(v: String) -> Option<String> {
    drift::clean_version(&v).map(str::to_string)
}

#[napi(js_name = "keyOf")]
pub fn key_of(p: Json) -> String {
    drift::key_of(&piece(&p.0))
}

#[napi]
pub fn describe(r: Json) -> String {
    drift::describe(&r.0)
}

#[napi(js_name = "readDriftFile")]
pub fn read_drift_file(file: String) -> Jv {
    match drift::read_drift_file(Path::new(&file)) {
        None => Jv::Json(Value::Null),
        Some((looked, records)) => Jv::Obj(vec![
            ("file", Jv::Json(Value::String(file))),
            ("lookedAt", Jv::Json(looked.map_or(Value::Null, xenonet_base::json::number))),
            ("records", Jv::Json(Value::Array(records))),
        ]),
    }
}

#[napi(js_name = "DriftLogCore")]
pub struct JsDriftLog {
    pub(crate) log: DriftLog,
}

#[napi]
impl JsDriftLog {
    #[napi(constructor)]
    pub fn new(max: f64) -> Self {
        JsDriftLog { log: DriftLog::new(if max.is_finite() { max as usize } else { usize::MAX }) }
    }
    #[napi]
    pub fn knows(&self, p: Json) -> bool {
        self.log.knows(&piece(&p.0))
    }
    #[napi]
    pub fn record(&mut self, p: Json, now: f64) -> Json {
        Json(self.log.record(&piece(&p.0), now))
    }
    #[napi]
    pub fn list(&self) -> Json {
        Json(Value::Array(self.log.list()))
    }
    #[napi]
    pub fn looked(&mut self, at: f64) {
        self.log.looked(at);
    }
    #[napi]
    pub fn open(&mut self, file: String) {
        self.log.open(&file);
    }
    /// Null, or why it couldn't write.
    #[napi]
    pub fn flush(&mut self) -> Option<String> {
        self.log.flush()
    }
    #[napi]
    pub fn view(&mut self) -> Jv {
        let (file, looked, records) = self.log.view();
        Jv::Obj(vec![
            ("file", Jv::Json(file.map_or(Value::Null, Value::String))),
            ("lookedAt", Jv::Json(looked.map_or(Value::Null, xenonet_base::json::number))),
            ("records", Jv::Json(Value::Array(records))),
        ])
    }
    #[napi]
    pub fn clear(&mut self) {
        self.log.clear();
    }
    #[napi(getter, js_name = "lookedAt")]
    pub fn looked_at(&self) -> Option<f64> {
        self.log.looked_at
    }
    #[napi(getter)]
    pub fn file(&self) -> Option<String> {
        self.log.file.clone()
    }
    #[napi(getter)]
    pub fn dirty(&self) -> bool {
        self.log.dirty
    }
}

// ----- versions -----

#[napi(js_name = "parseVersion")]
pub fn parse_version(v: String) -> Option<Jv> {
    vendor::parse_version(&v).map(|(a, b, c, pre)| Jv::Arr(vec![Jv::Num(a), Jv::Num(b), Jv::Num(c), Jv::Json(pre.map_or(Value::Null, Value::String))]))
}

fn version_of(v: &Value) -> vendor::Version {
    let n = |i: usize| v.get(i).and_then(Value::as_f64).unwrap_or(f64::NAN);
    (n(0), n(1), n(2), v.get(3).and_then(Value::as_str).map(str::to_string))
}

#[napi(js_name = "compareVersions")]
pub fn compare_versions(a: Json, b: Json) -> i32 {
    vendor::compare_versions(&version_of(&a.0), &version_of(&b.0)) as i32
}

#[napi(js_name = "validRange")]
pub fn valid_range(range: String) -> bool {
    vendor::valid_range(&range)
}

#[napi]
pub fn satisfies(version: String, range: String) -> bool {
    vendor::satisfies(&version, &range)
}

#[napi(js_name = "codexVersionOf")]
pub fn codex_version_of(user_agent: String) -> Option<String> {
    vendor::codex_version_of(&user_agent)
}

/// Where JSON text first goes wrong: [line, column], or null.
#[napi(js_name = "jsonErrorAt")]
pub fn json_error_at(text: String) -> Option<Vec<u32>> {
    vendor::json_error_at(&text).map(|(l, c)| vec![l as u32, c as u32])
}

/// JSON.parse for another app's file: the value, or { error } saying where
/// the text goes wrong without quoting it.
#[napi(js_name = "parseJson")]
pub fn parse_json(text: String) -> Jv {
    match xenonet_base::json::parse(&text) {
        Ok(v) => Jv::Obj(vec![("value", Jv::Json(v))]),
        Err(_) => {
            let message = match vendor::json_error_at(&text) {
                Some((line, column)) => format!("isn't valid JSON (line {line}, column {column})"),
                None => "isn't valid JSON".into(),
            };
            Jv::Obj(vec![("error", Jv::Json(Value::String(message)))])
        }
    }
}

// ----- readers -----

#[napi(js_name = "Sampling")]
pub struct JsSampling {
    s: Sampling,
}

#[napi]
impl JsSampling {
    #[napi(constructor)]
    pub fn new() -> Self {
        JsSampling { s: Sampling::default() }
    }
    #[napi(getter)]
    pub fn version(&self) -> Option<String> {
        self.s.version.clone()
    }
    #[napi(setter)]
    pub fn set_version(&mut self, v: Option<String>) {
        self.s.version = v;
    }
    /// A line of this kind came: whether to check it whole.
    #[napi]
    pub fn whole(&mut self, kind: String) -> bool {
        self.s.whole(&kind)
    }
}

#[napi(js_name = "VendorReaderCore")]
pub struct JsVendorReader {
    r: VendorReader,
    given: Option<Vec<Major>>,
}

fn checked(c: Checked) -> Jv {
    match c {
        Checked::Same => Jv::Obj(vec![("same", Jv::Bool(true))]),
        Checked::Stripped(v) => Jv::Obj(vec![("record", Jv::Json(v))]),
        Checked::Dropped => Jv::Obj(vec![("dropped", Jv::Bool(true))]),
    }
}

#[napi]
impl JsVendorReader {
    /// `given`: whether the caller gives the majors (addMajor) rather than
    /// the reader reading them from `dir`.
    #[napi(constructor)]
    pub fn new(source: String, dir: String, given: bool) -> Self {
        JsVendorReader { r: VendorReader::new(&source, PathBuf::from(dir), None), given: given.then(Vec::new) }
    }

    #[napi(js_name = "addMajor")]
    pub fn add_major(&mut self, id: String, major: u32, range: Json, spec: &JsSpec) {
        if let Some(g) = &mut self.given {
            g.push(Major { id, major, range: range.0, schema: spec.schema.clone() });
        }
    }

    fn ready(&mut self) -> &mut VendorReader {
        if let Some(g) = self.given.take() {
            let source = self.r.source.clone();
            let mut fresh = VendorReader::new(&source, PathBuf::new(), Some(g));
            std::mem::swap(&mut fresh.stats, &mut self.r.stats);
            fresh.calls = self.r.calls;
            self.r = fresh;
        }
        &mut self.r
    }

    #[napi]
    pub fn majors(&mut self, env: Env) -> Result<Jv> {
        let majors = self.ready().majors().to_vec();
        let out = majors.into_iter().map(|m| crate::schema::major_jv(&env, m)).collect::<Result<Vec<_>>>()?;
        Ok(Jv::Arr(out))
    }

    /// The major a version reads with, by its place, or null.
    #[napi]
    pub fn pick(&mut self, version: Option<String>, log: &mut JsDriftLog, now: f64) -> Option<u32> {
        self.ready().pick(version.as_deref(), &mut log.log, now).map(|i| i as u32)
    }

    #[napi]
    pub fn inspect(&mut self, record: Json, version: Option<String>, kind: Option<String>, log: &mut JsDriftLog, now: f64) -> Jv {
        checked(self.ready().inspect(&record.0, version.as_deref(), kind, &mut log.log, now))
    }

    #[napi]
    pub fn guard(&mut self, record: Json, version: Option<String>, kind: Option<String>, log: &mut JsDriftLog, now: f64) -> Jv {
        checked(self.ready().guard(&record.0, version.as_deref(), &kind, &mut log.log, now))
    }

    #[napi(js_name = "carefulAbout")]
    pub fn careful_about(&self, kind: Option<String>) -> bool {
        self.r.careful_about(&kind)
    }

    #[napi(js_name = "anyCareful")]
    pub fn any_careful(&self) -> bool {
        self.r.any_careful()
    }

    #[napi]
    pub fn sample(&mut self, record: Json, version: Option<String>, kind: Option<String>, every: f64, log: &mut JsDriftLog, now: f64) -> Jv {
        checked(self.ready().sample(&record.0, version.as_deref(), kind, every as u64, &mut log.log, now))
    }

    #[napi]
    pub fn stat(&self, name: String) -> f64 {
        let s = &self.r.stats;
        match name.as_str() {
            "records" => s.records as f64,
            "checked" => s.checked as f64,
            "dropped" => s.dropped as f64,
            "stripped" => s.stripped as f64,
            _ => f64::NAN,
        }
    }

    #[napi(js_name = "setStat")]
    pub fn set_stat(&mut self, name: String, v: f64) {
        let s = &mut self.r.stats;
        let v = v.max(0.0) as u64;
        match name.as_str() {
            "records" => s.records = v,
            "checked" => s.checked = v,
            "dropped" => s.dropped = v,
            "stripped" => s.stripped = v,
            _ => {}
        }
    }

    /// The versions seen, as [version or null, records checked].
    #[napi(js_name = "versionCounts")]
    pub fn version_counts(&self) -> Json {
        Json(Value::Array(self.r.stats.versions.iter().map(|(k, n)| json!([if k == NO_VERSION { Value::Null } else { Value::String(k.into()) }, n])).collect()))
    }

    /// The versions its records said, oldest first.
    #[napi]
    pub fn versions(&self) -> Vec<String> {
        self.r.versions()
    }

    #[napi(getter)]
    pub fn calls(&self) -> f64 {
        self.r.calls as f64
    }
}
