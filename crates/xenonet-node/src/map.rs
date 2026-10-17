//! xenonet-state's small modules and the map (graph, state), for their twins.

use crate::js::{Json, Jv, MaybeJson};
use napi_derive::napi;
use serde_json::{json, Value};
use std::cmp::Ordering;
use xenonet_state::{export_labels, frames, needs_you, rename};

// ---------- frames.js ----------

#[napi(js_name = "framesConstants")]
pub fn frames_constants() -> Json {
    let (min, max, default) = frames::FILM_SECONDS;
    Json(json!({
        "posterSizes": frames::poster_sizes(),
        "defaultSize": frames::DEFAULT_SIZE,
        "filmFrames": frames::film_frames(),
        "defaultFrame": frames::DEFAULT_FRAME,
        "filmSeconds": { "min": min, "max": max, "default": default },
        "filmCardsMs": frames::FILM_CARDS_MS,
    }))
}

/// The poster size's place in POSTER_SIZES.
#[napi(js_name = "posterSize")]
pub fn poster_size(id: Option<String>) -> u32 {
    frames::poster_size(id.as_deref()) as u32
}

#[napi(js_name = "filmSeconds")]
pub fn film_seconds(total: f64) -> f64 {
    frames::film_seconds(total)
}

#[napi(js_name = "filmPlaySeconds")]
pub fn film_play_seconds(total: f64) -> f64 {
    frames::film_play_seconds(total)
}

#[napi(js_name = "fitFrame")]
pub fn fit_frame(frame: Json, area: Json) -> Json {
    Json(frames::fit_frame(&frame.0, &area.0))
}

// ---------- rename.js ----------

#[napi(js_name = "maxTitle")]
pub fn max_title() -> u32 {
    rename::MAX_TITLE as u32
}

#[napi(js_name = "cleanTitle")]
pub fn clean_title(title: String) -> String {
    rename::clean_title(&title)
}

#[napi(js_name = "renamable")]
pub fn renamable(n: MaybeJson) -> bool {
    rename::renamable(n.0.as_ref())
}

// ---------- export-labels.js ----------

#[napi(js_name = "labelledKinds")]
pub fn labelled_kinds() -> Vec<&'static str> {
    export_labels::LABELLED_KINDS.to_vec()
}

#[napi(js_name = "exportLabel")]
pub fn export_label(node: MaybeJson, labels: MaybeJson) -> Option<String> {
    export_labels::export_label(node.0.as_ref(), labels.0.as_ref())
}

// ---------- needs-you.js ----------

#[napi(js_name = "needsYouKinds")]
pub fn needs_you_kinds() -> Vec<&'static str> {
    needs_you::KINDS.to_vec()
}

#[napi(js_name = "waitsWords")]
pub fn waits_words(it: Json) -> &'static str {
    needs_you::waits_words(&it.0)
}

#[napi(js_name = "needsYouQueue")]
pub fn needs_you_queue(input: Json) -> Json {
    Json(Value::Array(needs_you::needs_you_queue(&input.0)))
}

#[napi(js_name = "needsYouOrder")]
pub fn needs_you_order(a: Json, b: Json) -> i32 {
    match needs_you::order(&a.0, &b.0) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

#[napi(js_name = "queueOfSummary")]
pub fn queue_of_summary(summary: MaybeJson) -> Json {
    Json(Value::Array(needs_you::queue_of_summary(summary.0.as_ref())))
}

#[napi(js_name = "countOf")]
pub fn count_of(items: Json) -> Jv {
    Jv::Num(needs_you::count_of(items.0.as_array().map_or(&[], Vec::as_slice)))
}
