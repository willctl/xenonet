//! core/classify.js: the optional pass over the projects folder by Codex,
//! or by a decision model, when you ask. XenoNet sends an outline of the
//! folders it looked at (paths inside the projects folder, well-known files,
//! counts, file types, sessions, its own call) and keeps only the picks
//! that would change something. Running Codex and asking the decider stay
//! with the twin.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use xenonet_base::jsstr;
use xenonet_base::text::{js_string, js_trim, js_trim_start};

/// The shape Codex answers in (--output-schema).
pub fn picks_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "picks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "path": { "type": "string" }, "as": { "type": "string", "enum": ["project", "group"] }, "reason": { "type": "string" } },
                    "required": ["path", "as", "reason"],
                    "additionalProperties": false,
                },
            },
        },
        "required": ["picks"],
        "additionalProperties": false,
    })
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn field(v: &Value, k: &str) -> Value {
    get(v, k).cloned().unwrap_or(Value::Null)
}

/// The outline sent to Codex, from a scan's `folders`. Repos are settled,
/// so they go as context only: a path and a session count.
pub fn outline_folders(folders: &[Value]) -> Vec<Value> {
    folders
        .iter()
        .map(|f| {
            let mut m = Map::new();
            let put = |m: &mut Map<String, Value>, k: &str, v: Option<&Value>| {
                // A key whose value is undefined is left out, as JSON has it.
                if let Some(v) = v {
                    m.insert(k.into(), v.clone());
                }
            };
            if xenonet_base::text::truthy(get(f, "repo")) {
                put(&mut m, "path", get(f, "rel"));
                m.insert("xenonet".into(), Value::String("repo".into()));
                m.insert("markers".into(), json!([]));
                m.insert("files".into(), Value::Null);
                m.insert("folders".into(), Value::Null);
                m.insert("types".into(), json!({}));
                put(&mut m, "sessions", get(f, "sessions"));
            } else {
                put(&mut m, "path", get(f, "rel"));
                put(&mut m, "xenonet", get(f, "call"));
                put(&mut m, "markers", get(f, "markers"));
                put(&mut m, "files", get(f, "files"));
                put(&mut m, "folders", get(f, "folders"));
                put(&mut m, "types", get(f, "kinds"));
                put(&mut m, "sessions", get(f, "sessions"));
            }
            Value::Object(m)
        })
        .collect()
}

/// What tells two outlines apart: the folders, which are repos, and their
/// markers. File counts and sessions change all the time and don't count.
pub fn fingerprint(outline: &[Value]) -> String {
    let shape: Vec<Value> = outline.iter().map(|f| json!([field(f, "path"), get(f, "xenonet").and_then(Value::as_str) == Some("repo"), field(f, "markers")])).collect();
    let text = xenonet_base::json::stringify(&Value::Array(shape), 0);
    let hex = jsstr::hex(&Sha256::digest(text.as_bytes()));
    hex[..16].to_string()
}

const PROMPT: &str = "You are sorting the folders in a developer's projects folder, for a tool that draws a map of the projects their Claude Code and Codex sessions work in. Every project you add becomes a node on that map, so add one only when the developer would want to see it there.

Each entry below is one folder, by its path inside the projects folder. It lists the well-known files the folder holds (markers), how many files and subfolders it has, its commonest file types, how many coding sessions ran in it or below it, and how the tool sees it now (\"xenonet\"):
- \"repo\": it has its own version control, so it is a project. Don't pick it.
- \"project\": one project.
- \"group\": a folder that holds separate projects; each is judged on its own.
- \"folder\": neither, just a folder inside a project or a group.

Pick only the folders the tool clearly has wrong:
- \"project\": something the developer builds and comes back to as a unit (an app, a library, a service, a tool), even without a repo of its own. Signs of one: a manifest together with a README, a lockfile, tests, a Dockerfile or compose file, an AGENTS.md or CLAUDE.md, or sessions run in it. Not a project: examples and samples, tutorials worked through, scratch or one-off scripts, generated output or media, and a part of a bigger project next to it or above it (its infrastructure, docs, assets or tests). A manifest alone doesn't make a project.
- \"group\": a top-level \"project\" that is really a collection of separate projects, so its subfolders should count on their own.
Leave out repos and anything you aren't sure about. Most folders need no pick, and none at all is a fine answer. Give each pick a reason of under 15 words. Do not run any commands.

Folders:
";

pub fn picks_prompt(outline: &[Value]) -> String {
    format!("{PROMPT}{}", xenonet_base::json::stringify(&Value::Array(outline.to_vec()), 0))
}

// String(text).trim() without a fence around it: ```json … ```.
fn unfenced(text: &str) -> &str {
    let mut s = js_trim(text);
    if let Some(rest) = s.strip_prefix("```") {
        let rest = rest.strip_prefix("json").unwrap_or(rest);
        s = js_trim_start(rest);
        if let Some(body) = s.strip_suffix("```") {
            return body.trim_end_matches(xenonet_base::text::is_js_space);
        }
        return s;
    }
    if let Some(body) = s.strip_suffix("```") {
        return body.trim_end_matches(xenonet_base::text::is_js_space);
    }
    s
}

/// Codex's answer, keeping only picks for folders it was sent that would
/// change something.
pub fn parse_picks(text: &str, outline: &[Value]) -> Result<Vec<Value>, String> {
    let data = xenonet_base::json::parse(unfenced(text)).map_err(|_| "Codex didn't answer in the expected shape".to_string())?;
    let mut by_path: Vec<(Value, &Value)> = Vec::new();
    for f in outline {
        let p = field(f, "path");
        match by_path.iter_mut().find(|(k, _)| *k == p) {
            Some(slot) => slot.1 = f,
            None => by_path.push((p, f)),
        }
    }
    let mut picks: Vec<(Value, Value)> = Vec::new();
    for p in get(&data, "picks").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
        let path = if p.is_object() { field(p, "path") } else { Value::Null };
        let Some(&(_, f)) = by_path.iter().find(|(k, _)| *k == path && !(p.is_null() && k.is_null())) else { continue };
        let call = get(f, "xenonet").and_then(Value::as_str);
        let as_ = get(p, "as").and_then(Value::as_str);
        let fpath = field(f, "path");
        if call == Some("repo") || !matches!(as_, Some("project" | "group")) || call == as_ || picks.iter().any(|(k, _)| *k == fpath) {
            continue;
        }
        let reason = get(p, "reason").filter(|v| !v.is_null()).map_or_else(String::new, js_string);
        let reason = jsstr::head(js_trim(&reason), 200);
        picks.push((fpath.clone(), json!({ "path": fpath, "as": as_, "reason": reason })));
    }
    Ok(picks.into_iter().map(|(_, v)| v).collect())
}

/// How sure a decision model must be of a pick for it to count.
pub const CONFIDENT: f64 = 0.75;
const PER_REQUEST: usize = 40;

pub fn criteria() -> Value {
    json!({
        "project": "One project: something the developer builds and comes back to as a unit (an app, a library, a service, a tool), even without a repo of its own. Signs: a manifest with a README, a lockfile, tests, a Dockerfile, an AGENTS.md or CLAUDE.md, or coding sessions run in it.",
        "group": "A collection of separate projects, whose subfolders should each count on their own.",
        "folder": "Neither: examples or samples, tutorials, scratch or one-off scripts, generated output or media, or a part of a bigger project beside or above it (its docs, assets, tests or infrastructure).",
    })
}

/// What the decider is told it knows.
pub const DECIDER_ABOUT: &str = "Folders in a developer's projects folder, by path inside it, with the well-known files each holds (markers), how many files and subfolders, its commonest file types, how many coding sessions ran in it, and how the tool sees it now (xenonet). Repos are settled and listed only as context.";

/// The questions for a decision model, a few requests' worth: one choice a
/// folder (project, group or folder), repos left out.
pub fn jev_questions(outline: &[Value]) -> Vec<Value> {
    let asked: Vec<&Value> = outline.iter().filter(|f| get(f, "xenonet").and_then(Value::as_str) != Some("repo")).collect();
    asked
        .chunks(PER_REQUEST)
        .enumerate()
        .map(|(b, part)| {
            let mut questions = Map::new();
            for (j, f) in part.iter().enumerate() {
                let instructions = format!(
                    "For a map of a developer's projects folder, what is the folder \"{}\"? The tool sees it now as \"{}\". Most folders are what the tool sees; answer otherwise only when the folder clearly is.",
                    get(f, "path").map_or_else(|| "undefined".into(), js_string),
                    get(f, "xenonet").map_or_else(|| "undefined".into(), js_string)
                );
                questions.insert(format!("f{}", b * PER_REQUEST + j), json!({ "type": "choice", "instructions": instructions, "criteria": criteria() }));
            }
            json!({ "folders": part, "questions": questions })
        })
        .collect()
}

/// The picks a decider's answers make: those it's sure of that would
/// change something.
pub fn picks_from_answers(answers: &Value, folders: &[Value], offset: f64) -> Vec<Value> {
    let mut picks = Vec::new();
    for (j, f) in folders.iter().enumerate() {
        let key = format!("f{}", xenonet_base::json::number_string(offset + j as f64));
        let Some(a) = get(answers, &key).filter(|a| xenonet_base::text::truthy(Some(a))) else { continue };
        let choice = get(a, "choice").and_then(Value::as_str);
        let confidence = get(a, "confidence").and_then(Value::as_f64);
        if !matches!(choice, Some("project" | "group")) || choice == get(f, "xenonet").and_then(Value::as_str) || !confidence.is_some_and(|c| c >= CONFIDENT) {
            continue;
        }
        let pct = xenonet_base::units::js_round(confidence.unwrap() * 100.0);
        picks.push(json!({ "path": field(f, "path"), "as": choice, "reason": format!("the decision model was {}% sure", xenonet_base::json::number_string(pct)) }));
    }
    picks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_that_change_something() {
        let outline =
            vec![json!({ "path": "a", "xenonet": "folder", "markers": [] }), json!({ "path": "r", "xenonet": "repo", "markers": [] }), json!({ "path": "g", "xenonet": "group" })];
        let text = "```json\n{\"picks\":[{\"path\":\"a\",\"as\":\"project\",\"reason\":\"  a manifest and tests  \"},{\"path\":\"r\",\"as\":\"project\"},{\"path\":\"g\",\"as\":\"group\"},{\"path\":\"a\",\"as\":\"group\"}]}\n```";
        assert_eq!(parse_picks(text, &outline).unwrap(), vec![json!({ "path": "a", "as": "project", "reason": "a manifest and tests" })]);
        assert!(parse_picks("nope", &outline).is_err());
        assert_eq!(fingerprint(&outline).len(), 16);
        let q = jev_questions(&outline);
        assert_eq!(q.len(), 1);
        assert_eq!(q[0]["questions"].as_object().unwrap().keys().collect::<Vec<_>>(), ["f0", "f1"]);
        let picks = picks_from_answers(
            &json!({ "f0": { "choice": "project", "confidence": 0.8 }, "f1": { "choice": "group", "confidence": 0.9 } }),
            &[outline[0].clone(), outline[2].clone()],
            0.0,
        );
        assert_eq!(picks, vec![json!({ "path": "a", "as": "project", "reason": "the decision model was 80% sure" })]);
    }
}
