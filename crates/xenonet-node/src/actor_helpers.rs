//! xenonet-actors' small modules for their twins: products, detail, names,
//! key, limits and handoff.

use crate::js::{Json, Jv, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_actors::{detail, handoff, key, limits, names};
use xenonet_engine::js::{template, truthy};
use xenonet_engine::products;

// ----- products -----

#[napi(js_name = "actorProducts")]
pub fn actor_products() -> Json {
    Json(Value::Array(
        products::PRODUCTS
            .iter()
            .map(|p| json!({ "id": p.id, "slug": p.slug, "name": p.name, "short": p.short, "vendor": p.vendor, "observed": p.observed, "registers": p.registers, "remote": p.remote }))
            .collect(),
    ))
}

#[napi(js_name = "actorProductWhyNot")]
pub fn actor_product_why_not() -> Json {
    Json(Value::Array(products::WHY_NOT.iter().map(|(k, v)| json!([k, v])).collect()))
}

/// The id of the product with this id, or null.
#[napi(js_name = "actorProductOf")]
pub fn actor_product_of(id: String) -> Option<&'static str> {
    products::product_of(&id).map(|p| p.id)
}

#[napi(js_name = "actorProductOfSlug")]
pub fn actor_product_of_slug(slug: String) -> Option<&'static str> {
    products::product_of_slug(&slug)
}

// ----- detail -----

#[napi(js_name = "detailStatusMax")]
pub fn detail_status_max() -> u32 {
    detail::STATUS_MAX as u32
}

#[napi(js_name = "detailApps")]
pub fn detail_apps() -> Json {
    Json(Value::Object(detail::APPS.iter().map(|(k, v)| (k.to_string(), json!(v))).collect()))
}

#[napi(js_name = "detailClip")]
pub fn detail_clip(s: String, n: f64) -> String {
    detail::clip(&s, if n.is_finite() && n > 0.0 { n as usize } else { 0 })
}

#[napi(js_name = "detailSessionWords")]
pub fn detail_session_words(app: MaybeJson, place: MaybeJson) -> String {
    detail::session_words(app.0.as_ref(), place.0.as_ref())
}

#[napi(js_name = "detailPresenceWords")]
pub fn detail_presence_words(from: MaybeJson, to: MaybeJson) -> &'static str {
    detail::presence_words(from.0.as_ref(), to.0.as_ref().and_then(Value::as_str))
}

#[napi(js_name = "detailWrite")]
pub fn detail_write(kind: MaybeJson, facts: MaybeJson, at: MaybeJson) -> Result<Option<String>> {
    let kind = kind.0.as_ref().and_then(Value::as_str).unwrap_or("").to_string();
    detail::write_detail(&kind, facts.0.as_ref(), at.0.as_ref()).map_err(|e| Error::new(Status::GenericFailure, e))
}

#[napi(js_name = "detailConsoleStatus")]
pub fn detail_console_status(what: MaybeJson, at: MaybeJson) -> Option<String> {
    detail::console_status(what.0.as_ref().and_then(Value::as_str), at.0.as_ref())
}

#[napi(js_name = "detailLooksLikeKey")]
pub fn detail_looks_like_key(s: String) -> bool {
    detail::looks_like_key(&s)
}

#[napi(js_name = "detailPlaceName")]
pub fn detail_place_name(name: String, kind: MaybeJson) -> String {
    detail::place_name(&name, kind.0.as_ref().and_then(Value::as_str))
}

#[napi(js_name = "detailStatusLine")]
pub fn detail_status_line(text: MaybeJson) -> Jv {
    match detail::status_line(text.0.as_ref()) {
        Ok(s) => Jv::Obj(vec![("ok", Jv::Bool(true)), ("status", Jv::Json(s.map_or(Value::Null, Value::String)))]),
        Err(reason) => Jv::Obj(vec![("ok", Jv::Bool(false)), ("reason", Jv::Json(json!(reason)))]),
    }
}

// ----- names -----

#[napi(js_name = "namesKey")]
pub fn names_key(s: String) -> String {
    names::name_key(&s)
}

#[napi(js_name = "namesReadsAs")]
pub fn names_reads_as(s: String) -> Vec<String> {
    names::reads_as(&s)
}

#[napi(js_name = "namesCheck")]
pub fn names_check(raw: MaybeJson, product: MaybeJson) -> Jv {
    let product = product.0.as_ref().and_then(Value::as_str).map(str::to_string);
    match names::check_name(raw.0.as_ref().and_then(Value::as_str), product.as_deref()) {
        Ok(name) => Jv::Obj(vec![("ok", Jv::Bool(true)), ("name", Jv::Json(Value::String(name)))]),
        Err(reason) => Jv::Obj(vec![("ok", Jv::Bool(false)), ("reason", Jv::Json(Value::String(reason)))]),
    }
}

#[napi(js_name = "namesProductWordRefusal")]
pub fn names_product_word_refusal(text: String, what: MaybeJson) -> Option<String> {
    names::product_word_refusal(&text, &template(what.0.as_ref()))
}

#[napi(js_name = "namesReservedSlugs")]
pub fn names_reserved_slugs() -> Vec<&'static str> {
    names::RESERVED_SLUGS.to_vec()
}

#[napi(js_name = "namesSlugFrom")]
pub fn names_slug_from(name: String) -> String {
    names::slug_from(&name)
}

#[napi(js_name = "namesSlugRefusal")]
pub fn names_slug_refusal(slug: MaybeJson, product: MaybeJson) -> Option<String> {
    let product = product.0.as_ref().and_then(Value::as_str).map(str::to_string);
    names::slug_refusal(slug.0.as_ref().and_then(Value::as_str), product.as_deref())
}

#[napi(js_name = "namesSuggestSlug")]
pub fn names_suggest_slug(base: String, product: MaybeJson, taken: Function<String, bool>) -> Result<Option<String>> {
    let product = product.0.as_ref().and_then(Value::as_str).map(str::to_string);
    names::suggest_slug(&base, product.as_deref(), &mut |s| taken.call(s.to_string()))
}

#[napi(js_name = "namesHermesSlug")]
pub fn names_hermes_slug(profile: MaybeJson) -> String {
    names::hermes_slug(profile.0.as_ref())
}

// ----- key -----

#[napi]
pub struct WindowKey {
    inner: key::WindowKey,
}

#[napi]
impl WindowKey {
    #[napi(constructor)]
    pub fn new() -> Result<Self> {
        key::WindowKey::new().map(|inner| WindowKey { inner }).map_err(|e| Error::new(Status::GenericFailure, format!("no random bytes: {e}")))
    }

    #[napi]
    pub fn reveal(&self) -> String {
        self.inner.reveal().to_string()
    }

    #[napi]
    pub fn matches(&self, given: String) -> bool {
        self.inner.matches(&given)
    }

    #[napi(getter)]
    pub fn hidden(&self) -> &'static str {
        key::HIDDEN
    }
}

// ----- limits -----

#[napi]
pub struct Bucket {
    inner: Mutex<limits::Bucket>,
}

#[napi]
impl Bucket {
    #[napi(constructor)]
    pub fn new(rate: f64, burst: f64, now: f64) -> Self {
        Bucket { inner: Mutex::new(limits::Bucket::new(rate, burst, now)) }
    }

    #[napi]
    pub fn take(&self, n: f64, now: f64) -> f64 {
        self.inner.lock().unwrap().take(n, now)
    }

    #[napi(js_name = "fullIn")]
    pub fn full_in(&self, now: f64) -> f64 {
        self.inner.lock().unwrap().full_in(now)
    }
}

#[napi]
pub struct Buckets {
    inner: Mutex<limits::Buckets>,
}

#[napi]
impl Buckets {
    #[napi(constructor)]
    pub fn new(max: f64) -> Self {
        Buckets {
            inner: Mutex::new(limits::Buckets::new(if max.is_finite() && max > 0.0 {
                max as usize
            } else if max > 0.0 {
                usize::MAX
            } else {
                0
            })),
        }
    }

    #[napi]
    pub fn take(&self, key: String, rate: f64, burst: f64, n: f64, now: f64) -> f64 {
        self.inner.lock().unwrap().take(&key, rate, burst, n, now)
    }

    #[napi]
    pub fn forget(&self, prefix: String) {
        self.inner.lock().unwrap().forget(&prefix)
    }

    #[napi(getter)]
    pub fn size(&self) -> u32 {
        self.inner.lock().unwrap().size() as u32
    }
}

// ----- handoff -----

/// `host`: the URL's host, as Node's URL reads it ("" when it isn't one).
#[napi(js_name = "actorHandoff")]
pub fn actor_handoff(product: MaybeJson, url: MaybeJson, host: String, token: MaybeJson) -> Json {
    let product = product.0.as_ref().and_then(Value::as_str).map(str::to_string);
    let token = truthy(token.0.as_ref()).then(|| template(token.0.as_ref()));
    let h = handoff::handoff(product.as_deref(), &template(url.0.as_ref()), &host, token.as_deref());
    Json(json!({
        "app": h.app,
        "steps": h.steps.iter().map(|(text, copy)| json!({ "text": text, "copy": copy })).collect::<Vec<_>>(),
        "message": h.message,
    }))
}

#[napi(js_name = "actorWakeHelp")]
pub fn actor_wake_help(product: MaybeJson) -> Json {
    Json(handoff::wake_help(product.0.as_ref().and_then(Value::as_str)).map_or(Value::Null, |(steps, message)| json!({ "steps": steps, "message": message })))
}
