//! core/open.js: actions handed back to the desktop apps through their URL
//! schemes. URLs are built from nodes the server already knows, never from
//! what a client sent.

use crate::jstext::{encode_uri_component, search_params, slice16};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_base::text::{js_string, truthy};

fn uuid() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").unwrap())
}

fn desktop_id() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^local_[A-Za-z0-9-]{1,64}$").unwrap())
}

// RegExp.test reads its argument as a string: a missing one as "undefined".
fn as_text(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

fn is(node: &Value, key: &str, want: &str) -> bool {
    node.get(key).and_then(Value::as_str) == Some(want)
}

fn has(node: &Value, key: &str) -> bool {
    truthy(node.get(key))
}

fn uuid_at(node: &Value, key: &str) -> bool {
    uuid().is_match(&as_text(node.get(key)))
}

fn desktop_at(node: &Value) -> bool {
    has(node, "desktopId") && desktop_id().is_match(&as_text(node.get("desktopId")))
}

fn act(id: &str, label: &str, client: bool) -> Value {
    if client {
        json!({ "id": id, "label": label, "client": true })
    } else {
        json!({ "id": id, "label": label })
    }
}

/// The actions a node offers, which the page shows as buttons.
pub fn actions_for(node: Option<&Value>) -> Vec<Value> {
    let Some(node) = node.filter(|n| truthy(Some(n))) else { return Vec::new() };
    let mut acts = Vec::new();
    let session = is(node, "kind", "session");
    // A run XenoNet only observes, or one an experimental reader reads, has
    // no transcript here to preview, and nothing to open it in.
    if session && (has(node, "observed") || has(node, "reader")) {
        if has(node, "cwd") {
            acts.push(act("open-folder", "Open folder", false));
        }
        return acts;
    }
    if session {
        if is(node, "app", "codex") && uuid_at(node, "rawId") {
            acts.push(act("open-app", "Open in ChatGPT · Codex", false));
        }
        if is(node, "app", "claude") && desktop_at(node) {
            acts.push(act("open-app", "Open in Claude desktop", false));
        }
        if is(node, "app", "claude") && !has(node, "desktopId") && uuid_at(node, "rawId") {
            acts.push(act("open-app", "Import into Claude desktop", false));
        }
        acts.push(act("preview", "Preview transcript", true));
        acts.push(act("stats", "Stats", true));
        acts.push(act("handoff", "Hand off…", true));
        if has(node, "cwd") {
            acts.push(act("open-folder", "Open folder", false));
        }
    }
    if is(node, "kind", "page") && has(node, "path") {
        acts.push(act("open-file", "Open page", false));
    }
    if is(node, "kind", "wiki") && has(node, "path") {
        acts.push(act("open-folder", "Open wiki folder", false));
    }
    if is(node, "kind", "project") {
        acts.push(act("open-folder", "Open folder", false));
        acts.push(act("open-vscode", "Open in VS Code", false));
        acts.push(act("new-claude", "New Claude Code session here", false));
    }
    if is(node, "kind", "zone") && has(node, "path") {
        acts.push(act("open-folder", "Open folder", false));
    }
    acts
}

/// The URL (or folder, or file) an action opens; None for JS's undefined,
/// Some(null) for its null.
pub fn url_for(node: &Value, action: &str) -> Option<Value> {
    let folder = if is(node, "kind", "session") {
        if has(node, "worktree") {
            node.get("worktree").cloned()
        } else {
            node.get("cwd").cloned()
        }
    } else {
        node.get("path").cloned()
    };
    let folder_text = || folder.as_ref().filter(|f| truthy(Some(f))).map(js_string);
    let null = Some(Value::Null);
    match action {
        "open-app" => {
            if is(node, "app", "codex") && uuid_at(node, "rawId") {
                return Some(Value::String(format!("codex://threads/{}", as_text(node.get("rawId")))));
            }
            if is(node, "app", "claude") && desktop_at(node) {
                return Some(Value::String(format!("claude://code/continue?session={}", encode_uri_component(&as_text(node.get("desktopId"))))));
            }
            if is(node, "app", "claude") && uuid_at(node, "rawId") {
                return Some(Value::String(format!("claude://resume?session={}", as_text(node.get("rawId")))));
            }
            null
        }
        "open-folder" => Some(folder.filter(|f| !f.is_null()).unwrap_or(Value::Null)),
        "open-file" => {
            if is(node, "kind", "page") {
                node.get("path").cloned()
            } else {
                null
            }
        }
        "open-vscode" => Some(folder_text().map_or(Value::Null, |f| Value::String(vscode_url(&f)))),
        "new-claude" => Some(folder_text().map_or(Value::Null, |f| Value::String(format!("claude://code/new?folder={}", encode_uri_component(&f))))),
        _ => null,
    }
}

fn drive() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^[A-Za-z]:/").unwrap())
}

/// vscode://file/C:/x on Windows, vscode://file/Users/x on macOS and
/// Linux. POSIX paths are encoded a segment at a time, because macOS's
/// `open` refuses a URL with a space in it; Explorer takes Windows paths
/// as they are.
pub fn vscode_url(folder: &str) -> String {
    let p = folder.replace('\\', "/");
    if drive().is_match(&p) {
        return format!("vscode://file/{p}");
    }
    format!("vscode://file/{}", p.trim_start_matches('/').split('/').map(encode_uri_component).collect::<Vec<_>>().join("/"))
}

pub const MAX_BRIEF: usize = 6000;

/// A new session in the other (or the same) app, filled in with a handoff
/// brief.
pub fn handoff_url(target: &str, folder: Option<&str>, brief: &str) -> Option<String> {
    let text = slice16(brief, MAX_BRIEF);
    let folder = folder.filter(|f| !f.is_empty());
    match target {
        "claude" => {
            let mut pairs = Vec::new();
            if let Some(f) = folder {
                pairs.push(("folder", f));
            }
            pairs.push(("q", text.as_str()));
            Some(format!("claude://code/new?{}", search_params(&pairs)))
        }
        "codex" => {
            let mut pairs = vec![("prompt", text.as_str())];
            if let Some(f) = folder {
                pairs.push(("path", f));
            }
            Some(format!("codex://threads/new?{}", search_params(&pairs)))
        }
        _ => None,
    }
}

/// Claude desktop's list of Code sessions waiting for you.
pub const NEEDS_INPUT_URL: &str = "claude://code/needs-input";

/// The program that opens a URL or a file on `platform`, as [program, args].
pub fn launch_command(target: &str, platform: &str) -> (String, Vec<String>) {
    let cmd = match platform {
        "win32" => "explorer.exe",
        "darwin" => "open",
        _ => "xdg-open",
    };
    (cmd.into(), vec![target.into()])
}

/// A file shown in the file manager: selected in Finder or Explorer, its
/// folder opened elsewhere.
pub fn reveal_command(file: &str, platform: &str) -> (String, Vec<String>) {
    match platform {
        "darwin" => ("open".into(), vec!["-R".into(), file.into()]),
        "win32" => ("explorer.exe".into(), vec![format!("/select,{file}")]),
        _ => {
            // The folder: everything before the last separator, or "/".
            let cut = file.rfind(['/', '\\']).map_or(file, |i| &file[..i]);
            ("xdg-open".into(), vec![if cut.is_empty() { "/".into() } else { cut.into() }])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_links_only_for_ids_that_look_like_ids() {
        let codex = json!({ "kind": "session", "app": "codex", "rawId": "0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b", "cwd": "C:\\p" });
        assert_eq!(url_for(&codex, "open-app"), Some(json!("codex://threads/0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b")));
        let bad = json!({ "kind": "session", "app": "codex", "rawId": "x&calc.exe", "cwd": "C:\\p" });
        assert_eq!(url_for(&bad, "open-app"), Some(Value::Null));
        let ids: Vec<Value> = actions_for(Some(&bad)).iter().map(|a| a["id"].clone()).collect();
        assert_eq!(ids, [json!("preview"), json!("stats"), json!("handoff"), json!("open-folder")]);
        assert_eq!(url_for(&json!({ "kind": "page" }), "open-file"), None);
    }

    #[test]
    fn folders_become_editor_and_session_links() {
        let p = json!({ "kind": "project", "path": "/Users/me/Projects/a b#2" });
        assert_eq!(url_for(&p, "open-vscode"), Some(json!("vscode://file/Users/me/Projects/a%20b%232")));
        let w = json!({ "kind": "project", "path": "C:\\Users\\me\\Projects\\a b" });
        assert_eq!(url_for(&w, "new-claude"), Some(json!("claude://code/new?folder=C%3A%5CUsers%5Cme%5CProjects%5Ca%20b")));
        assert_eq!(handoff_url("codex", Some("C:\\p"), "go").as_deref(), Some("codex://threads/new?prompt=go&path=C%3A%5Cp"));
        assert_eq!(reveal_command("/home/w/x.json", "linux").1, ["/home/w"]);
        assert_eq!(reveal_command("x.json", "linux").1, ["x.json"]);
        assert_eq!(reveal_command("/x.json", "linux").1, ["/"]);
    }
}
