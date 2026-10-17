//! xenonet-state for the twins of core/migrate.js and core/statefile.js.

use crate::js::{Json, MaybeJson};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use xenonet_state::{migrate, statefile};

/// The steps that take a document from `from` to `to`, by their place in
/// `steps` ([from, to] each); throws, in the JS's words, when none do.
#[napi(js_name = "planMigration")]
pub fn plan_migration(steps: Vec<Vec<f64>>, from: MaybeJson, to: MaybeJson) -> Result<Vec<u32>> {
    let steps: Vec<migrate::Step> = steps.iter().map(|s| (s.first().copied().unwrap_or(f64::NAN), s.get(1).copied().unwrap_or(f64::NAN))).collect();
    migrate::plan(&steps, from.0.as_ref(), to.0.as_ref()).map(|v| v.into_iter().map(|i| i as u32).collect()).map_err(|e| Error::new(Status::GenericFailure, e))
}

#[napi(js_name = "stateVersion")]
pub fn state_version() -> f64 {
    statefile::STATE_VERSION
}

/// state.json's step from version 1 to 2.
#[napi(js_name = "stateUp1To2")]
pub fn state_up_1_to_2(s: Json, root_key: Option<String>) -> Json {
    Json(statefile::up_1_to_2(&s.0, root_key.as_deref()))
}
