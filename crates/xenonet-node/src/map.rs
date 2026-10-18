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
pub fn digest_summarize(events: Json, since: f64, until: f64, title_of: Function<'_, Json, MaybeJson>, live: MaybeJson) -> Result<Json> {
    let mut ask = asker(&title_of);
    digest::summarize(events.0.as_array().map_or(&[], Vec::as_slice), since, until, &mut ask, live.0.as_ref()).map(Json)
}

#[napi(js_name = "digestDayLog")]
pub fn digest_day_log(
    events: Json,
    now: f64,
    title_of: Option<Function<'_, Json, MaybeJson>>,
    home_of: Function<'_, Json, MaybeJson>,
    name_of: Function<'_, Json, MaybeJson>,
    live: MaybeJson,
    repos: Json,
) -> Result<Json> {
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

// ---------- store.js ----------

use xenonet_state::{store, workspace};

/// { parts, repaired }: the parts of the document prepare() fills in, as
/// they should be, and which of them weren't objects.
#[napi(js_name = "prepareState")]
pub fn prepare_state(state: Json) -> Json {
    let mut m = state.0.as_object().cloned().unwrap_or_default();
    let repaired = store::prepare(&mut m);
    let parts: serde_json::Map<String, Value> = store::PARTS.iter().map(|k| (k.to_string(), m[*k].clone())).collect();
    Json(json!({ "parts": parts, "repaired": repaired }))
}

/// [[id, place or null], …]: what setLayout does to the layout.
#[napi(js_name = "layoutChanges")]
pub fn layout_changes(positions: MaybeJson) -> Json {
    Json(Value::Array(store::layout_changes(positions.0.as_ref()).into_iter().map(|(id, p)| json!([id, p])).collect()))
}

#[napi(js_name = "rulesToClear")]
pub fn rules_to_clear(keys: Vec<String>, under_key: Option<String>) -> Vec<String> {
    store::rules_to_clear(&keys, under_key.as_deref())
}

#[napi(js_name = "picksFolderKey")]
pub fn picks_folder_key(root_key: MaybeJson) -> Result<String> {
    store::folder_key(root_key.0.as_ref()).map_err(|m| Error::new(Status::GenericFailure, m))
}

// ---------- workspace.js ----------

#[napi(js_name = "savedRoot")]
pub fn saved_root(file: String) -> Option<String> {
    workspace::saved_root(&file)
}

#[napi(js_name = "workspaceRoot")]
pub fn workspace_root(env_dir: Option<String>, saved: MaybeJson, home: String) -> Json {
    Json(workspace::root(env_dir.as_deref(), saved.0.as_ref(), &home))
}

#[napi(js_name = "workspaceLocked")]
pub fn workspace_locked(env_dir: Option<String>, home: String) -> Option<String> {
    workspace::locked(env_dir.as_deref(), &home)
}

#[napi(js_name = "workspaceChosen")]
pub fn workspace_chosen(dir: Option<String>, default: String) -> Option<String> {
    workspace::chosen(dir.as_deref(), &default)
}

#[napi(js_name = "resolveFolder")]
pub fn resolve_folder(input: MaybeJson, home: String) -> Option<String> {
    workspace::resolve_folder(input.0.as_ref(), &home)
}

#[napi(js_name = "validateFolder")]
pub fn validate_folder(input: MaybeJson, home: String, platform: String) -> Json {
    Json(workspace::validate(input.0.as_ref(), &home, &platform))
}

#[napi(js_name = "checkStart")]
pub fn check_start(input: MaybeJson, home: String, projects_dir: String, platform: String, vcs_markers: Vec<String>) -> Json {
    let markers: Vec<&str> = vcs_markers.iter().map(String::as_str).collect();
    Json(workspace::check_start(input.0.as_ref(), &home, &projects_dir, &platform, &markers))
}

#[napi(js_name = "checkFinish")]
pub fn check_finish(start: Json, scan: MaybeJson, time_ms: f64) -> Json {
    Json(workspace::check_finish(&start.0, scan.0.as_ref().filter(|v| !v.is_null()), time_ms))
}

#[napi(js_name = "instanceDirVar")]
pub fn instance_dir_var(kind: String) -> &'static str {
    workspace::instance_dir_var(&kind)
}

#[napi(js_name = "standsFor")]
pub fn stands_for(asked: Json, dev_port: Json, dev_name: String) -> Option<String> {
    workspace::stands_for(asked.0.as_array().map_or(&[], Vec::as_slice), &dev_port.0, &dev_name)
}

// ---------- graph.js ----------

use xenonet_state::graph;

#[napi(js_name = "activityOf")]
pub fn activity_of(updated_at: MaybeJson, now: f64, windows: MaybeJson) -> &'static str {
    graph::activity_of(updated_at.0.as_ref(), now, windows.0.as_ref())
}

/// buildGraph(input): which project orbits which is projects.rs's
/// relations, and a project whose name JS couldn't read throws its
/// TypeError, as projectRelations did.
#[napi(js_name = "buildGraph")]
pub fn build_graph(env: &Env, input: Json) -> Result<Json> {
    let projects = input.0.get("projects").and_then(Value::as_array).cloned().unwrap_or_default();
    match xenonet_state::projects::relations(&projects) {
        Ok((parent_of, _)) => Ok(Json(graph::build_graph(&input.0, &parent_of.into_iter().collect()))),
        Err(e) => crate::sources::thrown(env, e),
    }
}

// ---------- state.js ----------

use xenonet_state::state;

fn js_set(v: &Value) -> std::collections::HashSet<String> {
    v.as_array().map_or_else(Default::default, |a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
}

#[napi(js_name = "sourceStatus")]
pub fn source_status(name: String, ok: bool, ms: MaybeJson, value: MaybeJson, error: MaybeJson) -> Json {
    Json(state::source_status(&name, ok, ms.0.as_ref(), value.0.as_ref(), error.0.as_ref()))
}

#[napi(js_name = "visibleResource")]
pub fn visible_resource(server: String, hidden: Json) -> bool {
    state::visible_resource(&server, &js_set(&hidden.0))
}

#[napi(js_name = "mergeSessions")]
pub fn merge_sessions(prev: Json, codex_ok: bool, codex: Json, claude_ok: bool, claude: Json, observed: Json, demo_titles: Json) -> Json {
    let list = |v: &Json| v.0.as_array().cloned().unwrap_or_default();
    let titles: std::collections::HashMap<String, Value> = demo_titles
        .0
        .as_array()
        .map_or_else(Default::default, |a| a.iter().filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1).cloned().unwrap_or(Value::Null)))).collect());
    Json(Value::Array(state::merge_sessions(&list(&prev), (codex_ok, &list(&codex)), (claude_ok, &list(&claude)), &list(&observed), &titles)))
}

#[napi(js_name = "sessionDirs")]
pub fn session_dirs(sessions: Json) -> Json {
    Json(Value::Array(state::session_dirs(sessions.0.as_array().map_or(&[], Vec::as_slice))))
}

#[napi(js_name = "projectsNote")]
pub fn projects_note(scan: Json) -> String {
    state::projects_note(&scan.0)
}

#[napi(js_name = "projectRules")]
pub fn project_rules(codex: bool, root: String, picks: MaybeJson, yours: Json) -> Json {
    Json(state::project_rules(codex, &root, picks.0.as_ref(), &yours.0))
}

#[napi(js_name = "ruleProblem")]
pub fn rule_problem(as_: MaybeJson, full: String, projects_dir: String, is_dir: bool) -> Option<String> {
    state::rule_problem(as_.0.as_ref(), &full, &projects_dir, is_dir)
}

#[napi(js_name = "projectDetection")]
pub fn project_detection(root: String, scan: MaybeJson, rules: Json, codex: MaybeJson, current: MaybeJson) -> Json {
    Json(state::project_detection(&root, scan.0.as_ref(), &rules.0, codex.0.as_ref(), current.0.as_ref()))
}

#[napi(js_name = "homeOf")]
pub fn home_of(id: String, kind_of: Function<'_, String, Option<String>>, parent_of: Function<'_, String, Option<String>>) -> Option<String> {
    state::home_of(&id, &mut |i| kind_of.call(i.to_string()).ok().flatten(), &mut |i| parent_of.call(i.to_string()).ok().flatten())
}

#[napi(js_name = "titleOf")]
pub fn title_of(title: MaybeJson, name: MaybeJson, id: Json) -> Json {
    Json(state::title_of(title.0.as_ref(), name.0.as_ref(), &id.0))
}

#[napi(js_name = "cleanIdentityName")]
pub fn clean_identity_name(name: MaybeJson) -> Option<String> {
    state::clean_identity_name(name.0.as_ref())
}

#[napi(js_name = "knownIdentity")]
pub fn known_identity(id: MaybeJson, identities: Json, known: Json) -> Option<String> {
    state::known_identity(id.0.as_ref(), identities.0.as_array().map_or(&[], Vec::as_slice), &known.0)
}

#[napi(js_name = "identityDetection")]
pub fn identity_detection(enabled: bool, at: MaybeJson, identities: MaybeJson, saved: Json, node: Function<'_, String, MaybeJson>) -> Json {
    Json(state::identity_detection(enabled, at.0.as_ref(), identities.0.as_ref(), &saved.0, &mut |id| node.call(id.to_string()).ok().and_then(|n| n.0).filter(|v| !v.is_null())))
}

#[napi(js_name = "pinProblem")]
pub fn pin_problem(kind: Option<String>) -> Option<&'static str> {
    state::pin_problem(kind.as_deref())
}

#[napi(js_name = "liveSnapshotFor")]
pub fn live_snapshot_for(snap: Json, hidden: Json, acks: Json, signed_in: MaybeJson, node: Function<'_, String, MaybeJson>) -> Json {
    Json(state::live_snapshot(snap.0, &js_set(&hidden.0), &acks.0, signed_in.0.as_ref(), &mut |id| node.call(id.to_string()).ok().and_then(|n| n.0).filter(|v| !v.is_null())))
}

#[napi(js_name = "trayStatus")]
pub fn tray_status(snap: Json, title_of: Function<'_, Json, MaybeJson>, url: String) -> Json {
    Json(state::summary(&snap.0, &mut |id| title_of.call(Json(id.clone())).ok().and_then(|t| t.0).unwrap_or(Value::Null), &url))
}

#[napi(js_name = "handoffNotes")]
pub fn handoff_notes(notes: Json, page: MaybeJson) -> Json {
    Json(state::handoff_notes(&notes.0, page.0.as_ref()))
}

#[napi(js_name = "handoffNeedsTranscript")]
pub fn handoff_needs_transcript(notes: Json) -> bool {
    state::handoff_needs_transcript(&notes.0)
}

#[napi(js_name = "identitiesNote")]
pub fn identities_note(identities: Json) -> String {
    state::identities_note(identities.0.as_array().map_or(&[], Vec::as_slice))
}

#[napi(js_name = "readerStatus")]
pub fn reader_status(name: String, state_: String, runs: u32, what: String) -> Json {
    Json(state::reader_status(&name, &state_, runs as usize, &what))
}

#[napi(js_name = "trayUrl")]
pub fn tray_url(host: String, port: Json) -> String {
    state::url(&host, &port.0)
}

#[napi(js_name = "renameNote")]
pub fn rename_note(desktop: bool, desktop_error: MaybeJson) -> Json {
    Json(state::rename_note(desktop, desktop_error.0.as_ref()))
}

#[napi(js_name = "StateMemory")]
#[derive(Default)]
pub struct JsStateMemory {
    m: state::Memory,
}

#[napi]
impl JsStateMemory {
    #[napi(constructor)]
    pub fn new() -> Self {
        JsStateMemory { m: state::Memory::new() }
    }
    #[napi]
    pub fn updated(&mut self, sessions: Json) -> Json {
        Json(Value::Array(self.m.updated(sessions.0.as_array().map_or(&[], Vec::as_slice))))
    }
    #[napi]
    pub fn changed(&mut self, graph: Json) -> bool {
        self.m.changed(&graph.0)
    }
    #[napi(js_name = "noteSignins")]
    pub fn note_signins(&mut self, accounts: Json, now: f64) -> Json {
        Json(Value::Array(self.m.note_signins(&accounts.0, now)))
    }
    #[napi(getter, js_name = "signedIn")]
    pub fn signed_in(&self) -> Json {
        Json(self.m.signed_in().unwrap_or(Value::Null))
    }
    /// `home` is [id, name, repo] for the session's home, or null.
    #[napi(js_name = "shipIt")]
    pub fn ship_it(&mut self, ev: Json, home: MaybeJson, now: f64) -> Json {
        let home = home.0.filter(Value::is_array);
        let h = home.as_ref().and_then(|h| Some((h.get(0)?.as_str()?, h.get(1)?, h.get(2)?)));
        Json(self.m.ship_it(&ev.0, h, now).unwrap_or(Value::Null))
    }
}
