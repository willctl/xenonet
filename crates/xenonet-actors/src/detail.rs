//! What actors' events say (core/actors/detail.js;
//! docs/design/actors/interface.md, sections 1 and 6). XenoNet writes every
//! detail from facts: ids, counts, and the names of projects, apps and
//! platforms, never text an actor sends. The one exception is a status,
//! which is the actor's own line, checked first (`status_line`). Details
//! name projects, never session titles, which come from conversations, so
//! a session is only ever "a Claude Code session in atlas-api".

use serde_json::Value;
use std::sync::OnceLock;
use xenonet_base::log::js_slice;
use xenonet_base::text::js_trim;
use xenonet_engine::js::{template, truthy};

/// The agent apps by their id.
pub fn app_name(app: &str) -> Option<&'static str> {
    match app {
        "claude" => Some("Claude Code"),
        "codex" => Some("Codex"),
        _ => None,
    }
}

pub const APPS: [(&str, &str); 2] = [("claude", "Claude Code"), ("codex", "Codex")];

// How a message left, by the fixed list of platforms a report may name, so
// no free text gets in.
fn platform(p: &str) -> Option<&'static str> {
    Some(match p {
        "telegram" => "on Telegram",
        "discord" => "on Discord",
        "slack" => "on Slack",
        "whatsapp" => "on WhatsApp",
        "signal" => "on Signal",
        "matrix" => "on Matrix",
        "mattermost" => "on Mattermost",
        "email" => "by email",
        "sms" => "by text message",
        "imessage" => "on iMessage",
        "teams" => "on Teams",
        _ => return None,
    })
}

fn finished(s: &str) -> Option<&'static str> {
    Some(match s {
        "session" => "Finished a session",
        "task" => "Finished a task",
        "job" => "A scheduled job ran",
        "turn" => "Finished a turn",
        "command" => "A command finished",
        _ => return None,
    })
}

fn failed(s: &str) -> Option<&'static str> {
    Some(match s {
        "session" => "A session failed",
        "task" => "A task failed",
        "job" => "A scheduled job failed",
        "turn" => "A turn failed",
        "command" => "A command failed",
        _ => return None,
    })
}

// n === 1 ? one : `${n} many`
fn plural(n: &Value, one: &str, many: &str) -> String {
    if n.as_f64() == Some(1.0) {
        one.to_string()
    } else {
        format!("{} {many}", template(Some(n)))
    }
}

// A count as a detail says it: a whole number above 0, or 1.
fn count(n: Option<&Value>) -> Value {
    match n {
        Some(v) if v.as_f64().is_some_and(|f| f.is_finite() && f.trunc() == f && f > 0.0) => v.clone(),
        _ => Value::from(1),
    }
}

/// A text cut to `n` UTF-16 units, the last of them an ellipsis.
pub fn clip(s: &str, n: usize) -> String {
    if s.encode_utf16().count() > n {
        format!("{}…", js_slice(s, n.saturating_sub(1)))
    } else {
        s.to_string()
    }
}

/// "a Claude Code session in atlas-api"
pub fn session_words(app: Option<&Value>, place: Option<&Value>) -> String {
    let app = app.and_then(Value::as_str).and_then(app_name);
    let place = if truthy(place) { format!(" in {}", template(place)) } else { String::new() };
    format!("a {}session{place}", app.map(|a| format!("{a} ")).unwrap_or_default())
}

// A file by its name alone, cut short: the folders above it are the
// project's to name. One that reads like a key is just "a file".
fn file_name(p: &Value) -> String {
    let s = template(Some(p));
    let last = s.split(['\\', '/']).rfind(|x| !x.is_empty()).unwrap_or("");
    let name = clip(&one_line(last), 60);
    if !name.is_empty() && looks_like_key(&name) {
        "a file".into()
    } else {
        name
    }
}

fn file_words(paths: Option<&Value>, n: Option<&Value>) -> Result<String, String> {
    let names: Vec<String> = match paths {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(file_name).filter(|s| !s.is_empty()).collect(),
        Some(_) => return Err("facts.paths isn't a list".into()),
    };
    if !names.is_empty() {
        return Ok(if names.len() == 1 { names[0].clone() } else { format!("{} and {}", names[0], plural(&Value::from(names.len() - 1), "1 more file", "more files")) });
    }
    Ok(match n.filter(|n| truthy(Some(n))) {
        Some(n) => plural(n, "a file", "files"),
        None => "files".into(),
    })
}

/// A presence that changed, from what it was.
pub fn presence_words(from: Option<&Value>, to: Option<&str>) -> &'static str {
    match to {
        Some("away") => return "Went away",
        Some("offline") => return "Went offline",
        Some("unknown") => return "Out of sight",
        _ => {}
    }
    let from_s = from.and_then(Value::as_str);
    if !truthy(from) || from_s == Some("offline") || from_s == Some("unknown") {
        return "Came online";
    }
    if from_s == Some("away") {
        return "Came back";
    }
    if to == Some("idle") {
        "Went idle"
    } else {
        "Started working"
    }
}

fn get<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(k))
}

fn str_of<'a>(v: Option<&'a Value>, k: &str) -> Option<&'a str> {
    get(v, k).and_then(Value::as_str)
}

/// What an event says, from its kind, its facts (count, paths, platform,
/// subject, app, to) and what the map says of its target (`where`: a
/// session's app and place, or a place's name). Status and presence events
/// say their own (`status_line`, `presence_words`).
pub fn write_detail(kind: &str, facts: Option<&Value>, at_where: Option<&Value>) -> Result<Option<String>, String> {
    let place = get(at_where, "place").filter(|p| !p.is_null());
    let at = if truthy(place) { format!(" in {}", template(place)) } else { String::new() };
    Ok(Some(match kind {
        "spawned" => {
            if truthy(get(facts, "to")) {
                let to = str_of(facts, "to").and_then(app_name).unwrap_or("another app");
                return Ok(Some(format!("Handed {} to {to}", session_words(get(facts, "app"), place))));
            }
            // A session not on the map yet says its app through the facts.
            if str_of(at_where, "kind") == Some("session") {
                let app = get(at_where, "app").filter(|a| !a.is_null()).or(get(facts, "app"));
                return Ok(Some(format!("Started {}", session_words(app, place))));
            }
            format!("Started {}", session_words(get(facts, "app"), place))
        }
        "edited" => format!("Edited {}{at}", file_words(get(facts, "paths"), get(facts, "count"))?),
        "ran" => format!("Ran {}{at}", plural(&count(get(facts, "count")), "a command", "commands")),
        "messaged" => {
            let how = str_of(facts, "platform").and_then(platform);
            format!("Sent {}{}", plural(&count(get(facts, "count")), "a message", "messages"), how.map(|h| format!(" {h}")).unwrap_or_default())
        }
        "said" => format!("Said something to {}{at}", plural(&count(get(facts, "count")), "an agent", "agents")),
        "finished" => format!("{}{at}", str_of(facts, "subject").and_then(finished).unwrap_or("Finished")),
        "failed" => format!("{}{at}", str_of(facts, "subject").and_then(failed).unwrap_or("Something failed")),
        _ => return Ok(None),
    }))
}

/// What the person at the console is doing, as XenoNet words it from what
/// the window reported (section 6): looking at a place or a session,
/// reading or opening a session, or a terminal opened in a folder.
pub fn console_status(what: Option<&str>, at_where: Option<&Value>) -> Option<String> {
    let it =
        if str_of(at_where, "kind") == Some("session") { Some(Value::String(session_words(get(at_where, "app"), get(at_where, "place")))) } else { get(at_where, "name").cloned() };
    if !truthy(it.as_ref()) {
        return None;
    }
    let it = template(it.as_ref());
    match what {
        Some("looked") => Some(format!("Looking at {it}")),
        Some("read") => Some(format!("Reading {it}")),
        Some("opened") => Some(format!("Opened {it}")),
        Some("terminal") => {
            let p = get(at_where, "place").filter(|p| !p.is_null()).or(get(at_where, "name"));
            Some(format!("Opened a terminal in {}", template(p)))
        }
        _ => None,
    }
}

// ---------- status lines ----------

fn controls(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}')
}

fn reorders(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

/// Control characters, and the ones that reorder text, out; any run of
/// spaces, tabs or line breaks becomes one space.
pub fn one_line(s: &str) -> String {
    let mut a = String::with_capacity(s.len());
    let mut run = false;
    for c in s.chars() {
        if controls(c) {
            if !run {
                a.push(' ');
            }
            run = true;
        } else {
            run = false;
            a.push(c);
        }
    }
    let b: String = a.chars().filter(|c| !reorders(*c)).collect();
    let mut out = String::with_capacity(b.len());
    let mut space = false;
    for c in b.chars() {
        if xenonet_base::text::is_js_space(c) {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            space = false;
            out.push(c);
        }
    }
    js_trim(&out).to_string()
}

fn key_prefix() -> &'static regex::Regex {
    static R: OnceLock<regex::Regex> = OnceLock::new();
    R.get_or_init(|| {
        regex::Regex::new(
            r"(?:^|[^A-Za-z0-9])(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)",
        )
        .expect("a fixed pattern")
    })
}

/// Words that start the way keys and tokens do, and a run of 32 or more
/// letters and digits that mixes both (hex and base64 keys do; words and
/// paths seldom do).
pub fn looks_like_key(s: &str) -> bool {
    if key_prefix().is_match(s) {
        return true;
    }
    let mut start = None;
    let b = s.as_bytes();
    for i in 0..=b.len() {
        let alnum = i < b.len() && b[i].is_ascii_alphanumeric();
        match (alnum, start) {
            (true, None) => start = Some(i),
            (false, Some(st)) => {
                let run = &b[st..i];
                if run.len() >= 32 && run.iter().any(u8::is_ascii_alphabetic) && run.iter().any(u8::is_ascii_digit) {
                    return true;
                }
                start = None;
            }
            _ => {}
        }
    }
    false
}

/// A place's name for a detail or a status, or what it is when its name
/// reads like a key (a folder named for a hash, say), which a status would
/// refuse.
pub fn place_name(name: &str, kind: Option<&str>) -> String {
    let line = one_line(name);
    if looks_like_key(&line) {
        match kind {
            Some("project" | "native") => "a project",
            Some("zone") => "a folder",
            _ => "a place",
        }
        .into()
    } else {
        line
    }
}

pub const STATUS_MAX: usize = 140;

/// A status as XenoNet keeps it: one line, at most 140 characters (cut),
/// or None for an empty one, which clears it. Err(reason) for one that
/// isn't text, or looks like it holds a key, which is refused rather than
/// cut.
pub fn status_line(text: Option<&Value>) -> Result<Option<String>, &'static str> {
    let text = match text {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s,
        Some(_) => return Err("a status is text"),
    };
    let line = one_line(text);
    if line.is_empty() {
        return Ok(None);
    }
    if looks_like_key(&line) {
        return Err("that status looks like it holds a key or a token, so XenoNet won't show it");
    }
    Ok(Some(clip(&line, STATUS_MAX)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn details_from_facts() {
        let w = json!({ "kind": "session", "app": "codex", "place": "atlas" });
        assert_eq!(write_detail("spawned", Some(&json!({})), Some(&w)).unwrap().unwrap(), "Started a Codex session in atlas");
        assert_eq!(write_detail("edited", Some(&json!({ "paths": ["/a/b/c.js", "d.js"] })), None).unwrap().unwrap(), "Edited c.js and 1 more file");
        assert_eq!(write_detail("ran", Some(&json!({ "count": 3 })), None).unwrap().unwrap(), "Ran 3 commands");
        assert_eq!(write_detail("messaged", Some(&json!({ "platform": "sms" })), None).unwrap().unwrap(), "Sent a message by text message");
    }

    #[test]
    fn statuses() {
        assert_eq!(status_line(Some(&json!("  a\n\tb  "))), Ok(Some("a b".into())));
        assert!(status_line(Some(&json!("key sk-abc"))).is_err());
        assert_eq!(clip(&"x".repeat(200), 140).chars().count(), 140);
        assert!(looks_like_key(&format!("deploy {}", "ab12".repeat(10))));
        assert!(!looks_like_key(&"a".repeat(40)));
    }
}
