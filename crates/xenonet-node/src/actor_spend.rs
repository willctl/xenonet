//! xenonet-actors' adapters and spend for their twins.

use crate::js::{Json, MaybeJson};
use napi_derive::napi;
use serde_json::{json, Value};
use std::collections::HashSet;
use xenonet_actors::{adapters, spend};

#[napi(js_name = "adapterTransports")]
pub fn adapter_transports() -> Json {
    Json(Value::Array(adapters::TRANSPORTS.iter().map(|t| json!({ "id": t.id, "name": t.name, "provider": t.provider })).collect()))
}

#[napi(js_name = "adapterList")]
pub fn adapter_list() -> Json {
    Json(Value::Array(
        adapters::ADAPTERS
            .iter()
            .map(|a| json!({ "key": a.key, "id": a.id, "product": a.product, "provider": a.provider, "transports": a.transports, "support": a.support, "unavailable": a.unavailable, "capabilities": a.capabilities }))
            .collect(),
    ))
}

/// The index of the adapter for `product` among these products, -2 for
/// the custom one, or -1 for none.
#[napi(js_name = "adapterFor")]
pub fn adapter_for(product: MaybeJson, products: Vec<MaybeJson>, custom: bool) -> i32 {
    let products: Vec<Option<Value>> = products.into_iter().map(|p| p.0).collect();
    match adapters::adapter_for(product.0.as_ref(), &products, custom.then_some(usize::MAX)) {
        Some(usize::MAX) => -2,
        Some(i) => i as i32,
        None => -1,
    }
}

/// null when the adapter's answer stands, else what to answer instead.
#[napi(js_name = "adapterVerified")]
pub fn adapter_verified(out: MaybeJson, failed: bool) -> Json {
    Json(adapters::verified(out.0.as_ref(), failed).unwrap_or(Value::Null))
}

#[napi(js_name = "spendUnattributed")]
pub fn spend_unattributed() -> &'static str {
    spend::UNATTRIBUTED
}

fn list(v: Json) -> Vec<Value> {
    match v.0 {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}

/// [[session, {actor, how}]], for a Map.
#[napi(js_name = "spendAttribute")]
pub fn spend_attribute(sessions: Json, starts: Json, known: Option<Vec<String>>, ids: Option<Json>) -> Json {
    let known: Option<HashSet<String>> = known.map(|k| k.into_iter().collect());
    let ids = ids.map(list);
    let out = spend::attribute(&list(sessions), &list(starts), known.as_ref(), ids.as_deref());
    Json(Value::Array(out.into_iter().map(|(id, w)| json!([id, { "actor": w.actor, "how": w.how }])).collect()))
}

/// `names`: [id, name] pairs.
#[napi(js_name = "spendByActor")]
pub fn spend_by_actor(spend_: Json, sessions: Json, starts: Json, names: Json, since: MaybeJson) -> Json {
    let names: Vec<(String, Value)> = list(names).into_iter().map(|p| (xenonet_engine::js::template(p.get(0)), p.get(1).cloned().unwrap_or(Value::Null))).collect();
    let since = since.0.filter(|v| !v.is_null()).map(|v| v.as_f64().unwrap_or(f64::NAN));
    Json(spend::spend_by_actor(&list(spend_), &list(sessions), &list(starts), &names, since))
}
