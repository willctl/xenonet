//! core/live.js: what the transcripts' lines say as they're written. Here,
//! the helpers its readers share (tool categories, text cut to fit, the
//! local day, whether a reply asks you something, edits as small diffs);
//! the tailer's own state is `telemetry`.
//!
//! Where JS would throw on an odd record (`.slice` on a number), a helper
//! gives the `TypeError` lines.rs gives, which the tailer counts as an
//! error, as live.js's catch did.

pub mod telemetry;

use crate::lines::{opt_prop, TypeError};
use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;
use xenonet_base::jsstr;
use xenonet_base::text::{is_js_space, js_string, js_trim, truthy};

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `String(v ?? "")`.
pub fn string_of(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(v) => js_string(v),
    }
}

/// The MCP server a tool's name says it's from (`mcp__<server>__<tool>`),
/// a plugin's server without its `plugin_<plugin>_`; None for another
/// tool.
pub fn mcp_server_of(tool_name: &str) -> Option<String> {
    // /^mcp__(.+?)__[^_].*$/: the server is the shortest run, without line
    // breaks, that's followed by "__", a character other than "_", and the
    // rest of the name without line breaks.
    let rest = tool_name.strip_prefix("mcp__")?;
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    for i in 1..chars.len() {
        if is_line_terminator(chars[i - 1].1) {
            return None;
        }
        let at = chars[i].0;
        if !rest[at..].starts_with("__") {
            continue;
        }
        let &(_, next) = chars.get(i + 2)?;
        if next == '_' {
            continue;
        }
        if chars[i + 3..].iter().any(|&(_, c)| is_line_terminator(c)) {
            continue;
        }
        let server = &rest[..at];
        if let Some(r) = server.strip_prefix("plugin_") {
            if let Some(k) = r.find('_').filter(|&k| k > 0) {
                return Some(r[k + 1..].to_string());
            }
        }
        return Some(server.to_string());
    }
    None
}

fn any_of(n: &str, names: &[&str]) -> bool {
    names.iter().any(|w| n.eq_ignore_ascii_case(w))
}

/// What kind of tool a name is: mcp, shell, edit, read, web, agent, code
/// or other. (live.js's regular expressions are ASCII, case-insensitive.)
pub fn categorize(name: &str) -> &'static str {
    if mcp_server_of(name).is_some_and(|s| !s.is_empty()) {
        return "mcp";
    }
    // local_shell_call: a Codex shell call as its rollout names it, with no
    // name of its own (stats lists it by its type).
    if any_of(name, &["Bash", "PowerShell", "shell", "exec_command", "local_shell", "local_shell_call", "unified_exec", "write_stdin"]) {
        return "shell";
    }
    if any_of(name, &["Edit", "Write", "MultiEdit", "NotebookEdit", "apply_patch"]) {
        return "edit";
    }
    if any_of(name, &["Read", "Glob", "Grep", "LS", "view_image"]) {
        return "read";
    }
    if any_of(name, &["WebFetch", "WebSearch", "web_search"]) {
        return "web";
    }
    let starts = |w: &str| name.len() >= w.len() && name.is_char_boundary(w.len()) && name[..w.len()].eq_ignore_ascii_case(w);
    if ["Agent", "Task", "spawn_agent", "collab"].iter().any(|w| starts(w)) {
        return "agent";
    }
    if name.eq_ignore_ascii_case("exec") {
        return "code";
    }
    "other"
}

/// Text as one line of at most n characters (UTF-16 units), with "…" in
/// place of what's cut.
pub fn clip(s: &str, n: usize) -> String {
    let t = js_trim(&jsstr::collapse_space(s)).to_string();
    if jsstr::len(&t) > n {
        format!("{}…", jsstr::head(&t, n.saturating_sub(1)))
    } else {
        t
    }
}

/// `clip(v)` for a value, as `String(v ?? "")` makes it a string.
pub fn clip_value(v: Option<&Value>, n: usize) -> String {
    clip(&string_of(v), n)
}

/// The end of a text, at most n units, with "…" in place of what's cut.
pub fn tail(s: &str, n: usize) -> String {
    let t = js_trim(s);
    if jsstr::len(t) > n {
        format!("…{}", jsstr::last(t, n.saturating_sub(1)))
    } else {
        t.to_string()
    }
}

/// The local day a time falls on, as YYYY-MM-DD.
pub fn day_key(ts: f64) -> String {
    jsstr::local_day(ts)
}

/// A file's key for telling two sessions touched the same one: without
/// Windows' `\\?\`, with forward slashes, in lowercase.
pub fn file_key(p: &str) -> String {
    p.strip_prefix(r"\\?\").unwrap_or(p).replace('\\', "/").to_lowercase()
}

static ASKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u)\b(?:want me to|should i|shall i|do you want|would you like|let me know|which (?:one|option|do you)|can you confirm|please confirm|your call)\b").unwrap()
});

/// Whether the agent's last words before handing the turn back read like
/// a question.
pub fn looks_like_question(text: &str) -> bool {
    let t = js_trim(text);
    if t.is_empty() {
        return false;
    }
    let end = jsstr::last(t, 400);
    // /\?["'`*_)\]]*\s*$/
    let bare = end.trim_end_matches(is_js_space).trim_end_matches(|c| "\"'`*_)]".contains(c));
    if bare.ends_with('?') {
        return true;
    }
    // end.split(/\n\s*\n/).pop(): what follows the last run of white space
    // with two line breaks in it.
    let mut from = 0;
    let mut run_newlines = 0;
    for (i, c) in end.char_indices() {
        if is_js_space(c) {
            if c == '\n' {
                run_newlines += 1;
                if run_newlines >= 2 {
                    from = i + 1;
                }
            }
        } else {
            run_newlines = 0;
        }
    }
    ASKS.is_match(&end[from..])
}

/// `tilde(v, home)` as live.js calls it on a tool's input: nothing to do
/// for an empty value or no home, else the value as a string with the
/// home as "~".
fn tilde_value(v: Option<&Value>, home: Option<&str>) -> Option<Value> {
    match home.filter(|h| !h.is_empty()) {
        Some(h) if truthy(v) => Some(Value::String(xenonet_base::paths::tilde(&js_string(v.unwrap()), h))),
        _ => v.cloned(),
    }
}

/// What a tool call is about, in one short line: its command, description,
/// file, pattern, URL, query or prompt.
pub fn tool_detail(input: Option<&Value>, home: Option<&str>) -> String {
    match input {
        Some(Value::Object(m)) => {
            let first = ["command", "description", "file_path", "pattern", "url", "query", "prompt"].iter().find_map(|k| m.get(*k).filter(|v| !v.is_null()));
            let v = first.cloned().unwrap_or_else(|| Value::String(String::new()));
            clip_value(tilde_value(Some(&v), home).as_ref(), 72)
        }
        Some(Value::Array(_)) => clip_value(tilde_value(Some(&Value::String(String::new())), home).as_ref(), 72),
        Some(Value::String(s)) => {
            let first = s.split('\n').next().unwrap_or("").to_string();
            clip_value(tilde_value(Some(&Value::String(first)), home).as_ref(), 72)
        }
        _ => clip_value(tilde_value(Some(&Value::String(String::new())), home).as_ref(), 72),
    }
}

/// Codex records commands as argv, usually [shell, "-Command", script];
/// the script is the interesting part.
pub fn command_text(command: Option<&Value>) -> String {
    let argv: Vec<String> = match command {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a.iter().map(js_string).collect(),
        Some(v) => vec![js_string(v)],
    };
    if argv.len() >= 2 {
        let first = argv[0].to_ascii_lowercase();
        if ["pwsh", "powershell", "bash", "sh", "cmd", "zsh"].iter().any(|s| first.ends_with(s) || first.ends_with(&format!("{s}.exe"))) {
            return argv[argv.len() - 1].clone();
        }
    }
    argv.join(" ")
}

/// The files a tool's input names (file_path, notebook_path, path), those
/// with a slash in them.
pub fn files_of(input: Option<&Value>) -> Vec<String> {
    let Some(Value::Object(m)) = input else { return Vec::new() };
    ["file_path", "notebook_path", "path"].iter().filter_map(|k| m.get(*k)?.as_str()).filter(|p| p.contains(['\\', '/'])).map(String::from).collect()
}

/// The files an apply_patch names (`*** Update File: …`).
pub fn patch_files(text: Option<&Value>) -> Vec<String> {
    let text = string_of(text);
    let mut out = Vec::new();
    for line in text.split(is_line_terminator) {
        for head in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(rest) = line.strip_prefix(head) {
                if !rest.is_empty() {
                    out.push(js_trim(rest).to_string());
                }
            }
        }
    }
    out
}

/// A text's lines, each after `prefix`, at most `max` of them, with how
/// many more there were.
pub fn diff_lines(prefix: &str, text: Option<&Value>, max: usize) -> String {
    let text = string_of(text);
    let lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    let mut out: Vec<String> = lines.iter().take(max).map(|l| format!("{prefix}{l}")).collect();
    if lines.len() > max {
        out.push(format!("{prefix}… ({} more lines)", lines.len() - max));
    }
    out.join("\n")
}

fn edit(path: &Value, kind: &str, diff: String) -> Value {
    json!({ "path": path, "kind": kind, "diff": diff })
}

/// A Claude Code tool call's edits as small diffs: [{ path, kind, diff }].
pub fn claude_edits(name: &str, input: Option<&Value>) -> Result<Vec<Value>, TypeError> {
    let field = |k: &str| opt_prop(input, k);
    if !truthy(field("file_path")) && !truthy(field("notebook_path")) {
        return Ok(Vec::new());
    }
    let file = field("file_path").filter(|v| !v.is_null()).or(field("notebook_path")).cloned().unwrap_or(Value::Null);
    Ok(match name {
        "Edit" => vec![edit(&file, "edit", format!("{}\n{}", diff_lines("- ", field("old_string"), 40), diff_lines("+ ", field("new_string"), 40)))],
        "MultiEdit" => {
            let edits = match field("edits") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.iter().take(4).collect(),
                Some(_) => return Err(TypeError("(input.edits ?? []).slice is not a function".into())),
            };
            let mut parts = Vec::new();
            for e in edits {
                let e = Some(e);
                let get = |k: &str| crate::lines::prop(e, k);
                parts.push(format!("{}\n{}", diff_lines("- ", get("old_string")?, 12), diff_lines("+ ", get("new_string")?, 12)));
            }
            vec![edit(&file, "edit", parts.join("\n···\n"))]
        }
        "Write" => vec![edit(&file, "write", diff_lines("+ ", field("content"), 30))],
        "NotebookEdit" => vec![edit(&file, "edit", diff_lines("+ ", field("new_source"), 20))],
        _ => Vec::new(),
    })
}

/// `Object.entries(v ?? {})`, for the values JSON has.
fn entries(v: Option<&Value>) -> Vec<(String, Value)> {
    match v {
        Some(Value::Object(m)) => jsstr::js_key_order(m.clone()).into_iter().collect(),
        Some(Value::Array(a)) => a.iter().enumerate().map(|(i, x)| (i.to_string(), x.clone())).collect(),
        Some(Value::String(s)) => s.encode_utf16().enumerate().map(|(i, u)| (i.to_string(), Value::String(String::from_utf16_lossy(&[u])))).collect(),
        _ => Vec::new(),
    }
}

/// A Codex FileChange's changes as small diffs: [{ path, kind, diff }].
pub fn codex_edits(changes: Option<&Value>) -> Vec<Value> {
    entries(changes)
        .into_iter()
        .map(|(p, c)| {
            let c = Some(&c);
            let kind = opt_prop(c, "type").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Value::String("update".into()));
            let is_type = |t: &str| opt_prop(c, "type").and_then(Value::as_str) == Some(t);
            let diff = if truthy(opt_prop(c, "unified_diff")) {
                string_of(opt_prop(c, "unified_diff")).split('\n').take(60).collect::<Vec<_>>().join("\n")
            } else if is_type("add") {
                diff_lines("+ ", opt_prop(c, "content"), 30)
            } else if is_type("delete") {
                "(file deleted)".into()
            } else {
                String::new()
            };
            let mut m = Map::new();
            m.insert("path".into(), Value::String(p));
            m.insert("kind".into(), kind);
            m.insert("diff".into(), Value::String(diff));
            Value::Object(m)
        })
        .collect()
}

/// The middle value, the larger of the two middle ones for an even count;
/// 0 for none.
pub fn median(xs: &[f64]) -> f64 {
    let mut s = xs.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if s.is_empty() {
        0.0
    } else {
        s[s.len() / 2]
    }
}

/// The least-squares slope of y over x, in y units per x unit.
pub fn slope(points: &[(f64, f64)]) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    let n = points.len() as f64;
    let mx = points.iter().fold(0.0, |a, p| a + p.0) / n;
    let my = points.iter().fold(0.0, |a, p| a + p.1) / n;
    let (mut num, mut den) = (0.0, 0.0);
    for &(x, y) in points {
        num += (x - mx) * (y - my);
        den += (x - mx).powi(2);
    }
    if den != 0.0 {
        num / den
    } else {
        0.0
    }
}

/// A value with every object's keys sorted (as JS's sort() orders them,
/// then as a JS object lists them), for comparing two inputs whole.
fn ordered(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(ordered).collect()),
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort_by(|a, b| jsstr::cmp_units(a, b));
            Value::Object(jsstr::js_key_order(keys.into_iter().map(|k| (k.clone(), ordered(&m[k]))).collect()))
        }
        v => v.clone(),
    }
}

/// What tells one tool call from another: its name and a digest of its
/// whole input, so no other copy of the input is kept.
pub fn tool_key(name: &str, input: Option<&Value>) -> String {
    let text = xenonet_base::json::stringify(&ordered(input.unwrap_or(&Value::Null)), 0);
    let digest = Sha256::digest(text.as_bytes());
    format!("{name}|{}", jsstr::hex(&digest))
}

/// The pages a write to the shared context names, for the map to beam to.
pub fn wiki_pages_of(server: Option<&str>, input: Option<&Value>) -> Option<Vec<String>> {
    if !server.is_some_and(crate::xenolith::is_context_server) {
        return None;
    }
    let Some(Value::Object(m)) = input else { return None };
    let concat = |v: Option<&Value>| -> Vec<Value> {
        match v {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(v) => vec![v.clone()],
        }
    };
    let pages: Vec<String> = concat(m.get("pages")).into_iter().chain(concat(m.get("slug"))).filter_map(|p| p.as_str().map(String::from)).collect();
    if pages.is_empty() {
        None
    } else {
        Some(pages.into_iter().take(6).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_by_kind() {
        assert_eq!(mcp_server_of("mcp__xenolith__write_page").as_deref(), Some("xenolith"));
        assert_eq!(mcp_server_of("mcp__plugin_tools_github__search").as_deref(), Some("github"));
        assert_eq!(mcp_server_of("mcp__a__b__c").as_deref(), Some("a"));
        assert_eq!(mcp_server_of("mcp__a___b").as_deref(), Some("a_"));
        assert_eq!(mcp_server_of("mcp__x"), None);
        assert_eq!(mcp_server_of("Bash"), None);
        assert_eq!(categorize("mcp__srv__tool"), "mcp");
        assert_eq!(categorize("BASH"), "shell");
        assert_eq!(categorize("apply_patch"), "edit");
        assert_eq!(categorize("TaskCreate"), "agent");
        assert_eq!(categorize("exec"), "code");
        assert_eq!(categorize("nothing"), "other");
    }

    #[test]
    fn text_cut_to_fit() {
        assert_eq!(clip("  a\n b  ", 72), "a b");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(tail("abcdef", 4), "…def");
        assert_eq!(file_key(r"\\?\C:\Users\Me\X.txt"), "c:/users/me/x.txt");
    }

    #[test]
    fn questions() {
        assert!(looks_like_question("Shall we? "));
        assert!(looks_like_question("Done.\n\nWant me to push it."));
        assert!(!looks_like_question("Want me to push it.\n\nDone."));
        assert!(looks_like_question("Done (is that ok?)"));
        assert!(!looks_like_question("Done."));
        assert!(!looks_like_question("I'll let me knowledge"));
    }

    #[test]
    fn commands_and_edits() {
        assert_eq!(command_text(Some(&json!(["bash", "-lc", "npm test"]))), "npm test");
        assert_eq!(command_text(Some(&json!(["git", "status"]))), "git status");
        assert_eq!(command_text(Some(&json!("ls -la"))), "ls -la");
        assert_eq!(command_text(Some(&json!([null, 1]))), "null 1");
        assert_eq!(patch_files(Some(&json!("*** Begin Patch\n*** Update File: a/b.js\n*** Add File:  c.js \n"))), ["a/b.js", "c.js"]);
        assert_eq!(diff_lines("+ ", Some(&json!("a\r\nb\nc")), 2), "+ a\n+ b\n+ … (1 more lines)");
        let e = claude_edits("Edit", Some(&json!({ "file_path": "/x", "old_string": "a", "new_string": "b" }))).unwrap();
        assert_eq!(e, [json!({ "path": "/x", "kind": "edit", "diff": "- a\n+ b" })]);
        assert!(claude_edits("MultiEdit", Some(&json!({ "file_path": "/x", "edits": 3 }))).is_err());
        assert_eq!(codex_edits(Some(&json!({ "/a": { "type": "delete" }, "/b": { "type": "add", "content": "x" } })))[1]["diff"], "+ x");
        assert_eq!(tool_key("Bash", Some(&json!({ "b": 1, "a": 2 }))), tool_key("Bash", Some(&json!({ "a": 2, "b": 1 }))));
        assert_eq!(median(&[3.0, 1.0, 2.0, 4.0]), 3.0);
        assert!((slope(&[(0.0, 0.0), (1.0, 2.0), (2.0, 4.0)]) - 2.0).abs() < 1e-12);
    }
}
