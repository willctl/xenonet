//! xenonet-schema for the twin of core/schema/lenient.js. A compiled
//! schema is a handle, `SchemaSpec`, to a node of it; a type change found
//! carries one too, which vendor.js keeps to look for the change again.

use crate::js::{Json, Jv};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use xenonet_schema::lenient::{self, Finding, Key, Schema, SpecRef, Step};

#[napi(js_name = "SchemaSpec")]
pub struct JsSpec {
    pub(crate) schema: Arc<Schema>,
    at: SpecRef,
}

/// A major of a vendor source's schema as JS sees it: { id, major, range, spec }.
pub fn major_jv(env: &Env, m: lenient::Major) -> Result<Jv> {
    let at = SpecRef::Node(m.schema.root);
    Ok(Jv::Obj(vec![("id", Jv::Json(Value::String(m.id))), ("major", Jv::Num(f64::from(m.major))), ("range", Jv::Json(m.range)), ("spec", spec_raw(env, &m.schema, at)?)]))
}

fn key_json(k: &Key) -> Value {
    match k {
        Key::Name(s) => Value::String(s.clone()),
        Key::Index(i) => Value::from(*i),
    }
}

fn keys_json(keys: &[Key]) -> Value {
    Value::Array(keys.iter().map(key_json).collect())
}

fn keys_of(v: &Value) -> Vec<Key> {
    v.as_array()
        .map(|a| {
            a.iter()
                .map(|k| match k {
                    Value::Number(n) => Key::Index(n.as_u64().unwrap_or(u64::MAX) as usize),
                    other => Key::Name(other.as_str().map_or_else(|| xenonet_base::text::js_string(other), str::to_string)),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn spec_raw(env: &Env, schema: &Arc<Schema>, at: SpecRef) -> Result<Jv> {
    let instance = JsSpec { schema: schema.clone(), at }.into_instance(env)?;
    Ok(Jv::Raw(unsafe { ToNapiValue::to_napi_value(env.raw(), instance)? }))
}

fn opt_str(s: &Option<String>) -> Jv {
    Jv::Json(s.clone().map_or(Value::Null, Value::String))
}

fn variant(v: &Option<Vec<Value>>) -> Jv {
    Jv::Json(v.clone().map_or(Value::Null, Value::Array))
}

fn finding(env: &Env, schema: &Arc<Schema>, f: &Finding) -> Result<Jv> {
    let kind = |k: &str| ("kind", Jv::Json(Value::String(k.into())));
    Ok(Jv::Obj(match f {
        Finding::MissingField { path, variant: v } => vec![kind("missing-field"), ("path", opt_str(path)), ("variant", variant(v))],
        Finding::TypeChange { path, variant: v, value, data, keys, segments, spec } => vec![
            kind("type-change"),
            ("path", opt_str(path)),
            ("variant", variant(v)),
            ("value", Jv::Json(value.clone())),
            ("data", Jv::Bool(*data)),
            ("keys", Jv::Json(keys_json(keys))),
            ("segments", Jv::Json(serde_json::json!(segments))),
            ("spec", spec_raw(env, schema, spec.clone())?),
        ],
        Finding::NewField { path, variant: v, value, data } => {
            vec![kind("new-field"), ("path", opt_str(path)), ("variant", variant(v)), ("value", Jv::Json(value.clone())), ("data", Jv::Bool(*data))]
        }
        Finding::NewVariant { path, variant: v, value } => vec![kind("new-variant"), ("path", opt_str(path)), ("variant", variant(v)), ("value", Jv::Json(value.clone()))],
    }))
}

#[napi]
impl JsSpec {
    /// Whether it takes a value of this sort at all.
    #[napi]
    pub fn accepts(&self, v: Json) -> bool {
        self.schema.accepts(&self.at, &v.0)
    }

    /// { findings, strip, drop }, as lenient.js's inspect() gives them.
    #[napi]
    pub fn inspect(&self, env: &Env, value: Json) -> Result<Jv> {
        let r = self.schema.inspect_at(&self.at, &value.0);
        let findings = r.findings.iter().map(|f| finding(env, &self.schema, f)).collect::<Result<Vec<_>>>()?;
        Ok(Jv::Obj(vec![("findings", Jv::Arr(findings)), ("strip", Jv::Json(Value::Array(r.strip.iter().map(|k| keys_json(k)).collect()))), ("drop", Jv::Bool(r.drop))]))
    }
}

#[napi(js_name = "compileSchema")]
pub fn compile_schema(root: Json) -> JsSpec {
    let schema = Arc::new(lenient::compile(&root.0));
    let at = SpecRef::Node(schema.root);
    JsSpec { schema, at }
}

/// The majors of a vendor source's schema, newest first: [{ id, major, range, spec }].
#[napi(js_name = "loadVendor")]
pub fn load_vendor(env: &Env, source: String, dir: String) -> Result<Jv> {
    let majors = lenient::load_vendor(&source, Path::new(&dir));
    let out = majors.into_iter().map(|m| major_jv(env, m)).collect::<Result<Vec<_>>>()?;
    Ok(Jv::Arr(out))
}

#[napi(js_name = "vendorSources")]
pub fn vendor_sources(dir: String) -> Vec<String> {
    lenient::vendor_sources(Path::new(&dir))
}

/// Where, as a drift record says it, from the steps to a place, outermost
/// first, as [kind, key].
#[napi(js_name = "pathOf")]
pub fn path_of(steps: Json) -> Option<String> {
    let steps: Vec<Step> = steps
        .0
        .as_array()
        .map(|a| {
            a.iter()
                .map(|s| {
                    let key = &s[1];
                    let name = || key.as_str().map_or_else(|| xenonet_base::text::js_string(key), str::to_string);
                    match s[0].as_str() {
                        Some("item") => Step::Item(key.as_u64().unwrap_or(0) as usize),
                        Some("entry") => Step::Entry(name()),
                        _ => Step::Prop(name()),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    lenient::path_of(&steps)
}

/// The keys to each place `segments` lead to that `spec` doesn't take.
#[napi]
pub fn recheck(value: Json, segments: Vec<String>, spec: &JsSpec) -> Json {
    Json(Value::Array(spec.schema.recheck(&value.0, &segments, &spec.at).iter().map(|k| keys_json(k)).collect()))
}

#[napi(js_name = "stripRecord")]
pub fn strip_record(record: Json, paths: Json) -> Json {
    let paths: Vec<Vec<Key>> = paths.0.as_array().map(|a| a.iter().map(keys_of).collect()).unwrap_or_default();
    Json(lenient::strip(&record.0, &paths))
}
