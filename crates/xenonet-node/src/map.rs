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

// ---------- redact.js ----------

use napi::bindgen_prelude::*;
use xenonet_state::{classify, digest, redact};

#[napi(js_name = "redactDayKey")]
pub fn redact_day_key(now: f64) -> String {
    redact::day_key(now)
}

#[napi(js_name = "starName")]
pub fn star_name(id: String, salt: String, day: String) -> String {
    redact::star_name(&id, &salt, &day)
}

/// [[id, name], …] in the order the names were given, for a Map.
#[napi(js_name = "starNames")]
pub fn star_names(ids: Json, salt: String, day: String) -> Json {
    let ids = ids.0.as_array().cloned().unwrap_or_default();
    Json(Value::Array(redact::star_names(&ids, &salt, &day).into_iter().map(|(id, n)| json!([id, n])).collect()))
}

#[napi(js_name = "posterSalt")]
pub fn poster_salt(dir: Option<String>) -> Result<String> {
    redact::poster_salt(dir.as_deref()).map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
}

#[napi(js_name = "archetypeOf")]
pub fn archetype_of(day: MaybeJson) -> &'static str {
    redact::archetype_of(day.0.as_ref())
}

#[napi(js_name = "constellationOf")]
pub fn constellation_of(day: MaybeJson, salt: String, now: f64, names: bool, nodes: Json) -> Json {
    Json(redact::constellation_of(day.0.as_ref(), &salt, now, names, nodes.0.as_array().map_or(&[], Vec::as_slice)))
}

#[napi(js_name = "labelsOf")]
pub fn labels_of(nodes: Json, day: MaybeJson, salt: String, now: f64) -> Json {
    Json(redact::labels_of(nodes.0.as_array().map_or(&[], Vec::as_slice), day.0.as_ref(), &salt, now))
}

// ---------- digest.js ----------

// A JS callback a core asks during the call: what it returns, or the
// exception it threw, left pending for JS.
fn asker<'a>(f: &'a Function<'a, Json, MaybeJson>) -> impl FnMut(&Value) -> Result<Value> + 'a {
    move |v: &Value| f.call(Json(v.clone())).map(|r| r.0.unwrap_or(Value::Null))
}

#[napi(js_name = "digestSummarize")]
pub fn digest_summarize(events: Json, since: f64, until: f64, title_of: Function<Json, MaybeJson>, live: MaybeJson) -> Result<Json> {
    let mut ask = asker(&title_of);
    digest::summarize(events.0.as_array().map_or(&[], Vec::as_slice), since, until, &mut ask, live.0.as_ref()).map(Json)
}

#[napi(js_name = "digestDayLog")]
pub fn digest_day_log(events: Json, now: f64, title_of: Option<Function<Json, MaybeJson>>, home_of: Function<Json, MaybeJson>, name_of: Function<Json, MaybeJson>, live: MaybeJson, repos: Json) -> Result<Json> {
    let mut title = title_of.as_ref().map(asker);
    let mut home = asker(&home_of);
    let mut name = asker(&name_of);
    let hooks = digest::DayHooks { title_of: title.as_mut().map(|f| f as &mut dyn FnMut(&Value) -> Result<Value>), home_of: Some(&mut home), name_of: Some(&mut name) };
    digest::day_log(events.0.as_array().map_or(&[], Vec::as_slice), now, hooks, live.0.as_ref(), repos.0.as_array().map_or(&[], Vec::as_slice)).map(Json)
}

#[napi(js_name = "summaryText")]
pub fn summary_text(sum: Json, label: String) -> String {
    digest::summary_text(&sum.0, &label)
}

#[napi(js_name = "handoffBrief")]
pub fn handoff_brief(node: Json, notes: Json, target: Option<String>) -> String {
    digest::handoff_brief(&node.0, &notes.0, target.as_deref())
}

#[napi(js_name = "narratePrompt")]
pub fn narrate_prompt(text: String) -> String {
    digest::narrate_prompt(&text)
}

#[napi(js_name = "codexExecArgs")]
pub fn codex_exec_args(dir: String, out: String, schema_file: Option<String>) -> Vec<String> {
    digest::codex_exec_args(&dir, &out, schema_file.as_deref())
}

// ---------- classify.js ----------

#[napi(js_name = "picksSchema")]
pub fn picks_schema() -> Json {
    Json(classify::picks_schema())
}

#[napi(js_name = "outlineFolders")]
pub fn outline_folders(folders: Json) -> Json {
    Json(Value::Array(classify::outline_folders(folders.0.as_array().map_or(&[], Vec::as_slice))))
}

#[napi(js_name = "fingerprint")]
pub fn fingerprint(outline: Json) -> String {
    classify::fingerprint(outline.0.as_array().map_or(&[], Vec::as_slice))
}

#[napi(js_name = "picksPrompt")]
pub fn picks_prompt(outline: Json) -> String {
    classify::picks_prompt(outline.0.as_array().map_or(&[], Vec::as_slice))
}

#[napi(js_name = "parsePicks")]
pub fn parse_picks(text: String, outline: Json) -> Result<Json> {
    classify::parse_picks(&text, outline.0.as_array().map_or(&[], Vec::as_slice)).map(|p| Json(Value::Array(p))).map_err(|m| Error::new(Status::GenericFailure, m))
}

#[napi(js_name = "confident")]
pub fn confident() -> f64 {
    classify::CONFIDENT
}

#[napi(js_name = "deciderAbout")]
pub fn decider_about() -> &'static str {
    classify::DECIDER_ABOUT
}

#[napi(js_name = "jevQuestions")]
pub fn jev_questions(outline: Json) -> Json {
    Json(Value::Array(classify::jev_questions(outline.0.as_array().map_or(&[], Vec::as_slice))))
}

#[napi(js_name = "picksFromAnswers")]
pub fn picks_from_answers(answers: Json, folders: Json, offset: f64) -> Json {
    Json(Value::Array(classify::picks_from_answers(&answers.0, folders.0.as_array().map_or(&[], Vec::as_slice), offset)))
}
