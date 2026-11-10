//! Actions handed back to the desktop apps through their URL schemes: the
//! buttons a node offers and the URL each one opens. URLs are built from
//! nodes the server already knows, never from what a client sent. What
//! opens a session or a project in an app is its adapter's (`Opener`,
//! from `Platform::opener`), for the apps the registry gives `open`.

use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_text::text::{display, truthy};

/// What opens a session or project in an app (its `open` capability).
#[derive(Clone, Copy)]
pub struct Opener {
    /// The label of the action that opens a session in the app, when it
    /// offers one.
    pub action: fn(&Value) -> Option<&'static str>,
    /// The URL that opens a session in the app, if any.
    pub url: fn(&Value) -> Option<String>,
    /// The action that starts a session in a project's folder, if any.
    pub new_session: Option<NewSession>,
    /// A new session, filled in with a handoff brief: the folder, if any,
    /// and the brief.
    pub handoff: fn(Option<&str>, &str) -> String,
}

/// An action that starts a session in a project's folder: its id, its
/// label and its URL for a folder.
#[derive(Clone, Copy)]
pub struct NewSession {
    pub id: &'static str,
    pub label: &'static str,
    pub url: fn(&str) -> String,
}

fn uuid() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?i)^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$").expect("a fixed pattern"))
}

fn desktop_id() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"^local_[A-Za-z0-9-]{1,64}$").expect("a fixed pattern"))
}

/// A node's field as a pattern reads it: a missing one as "undefined".
pub fn as_text(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), display)
}

fn is(node: &Value, key: &str, want: &str) -> bool {
    node.get(key).and_then(Value::as_str) == Some(want)
}

/// Whether a node has a field that's set.
pub fn has(node: &Value, key: &str) -> bool {
    truthy(node.get(key))
}

/// Whether a node's field is a UUID.
pub fn uuid_at(node: &Value, key: &str) -> bool {
    uuid().is_match(&as_text(node.get(key)))
}

/// Whether a node has a desktop app's own session id (`local_…`).
pub fn desktop_at(node: &Value) -> bool {
    has(node, "desktopId") && desktop_id().is_match(&as_text(node.get("desktopId")))
}

/// The apps the registry gives `open`, each with its adapter's opener, in
/// the registry's order.
fn openers() -> Vec<Opener> {
    crate::platforms().iter().filter(|p| p.app().has("open")).filter_map(|p| p.opener()).collect()
}

/// An app's opener, by the id a node or a request names.
fn opener(app: Option<&str>) -> Option<Opener> {
    let p = crate::platforms().get(app?)?;
    if p.app().has("open") {
        p.opener()
    } else {
        None
    }
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
        if let Some(label) = opener(node.get("app").and_then(Value::as_str)).and_then(|o| (o.action)(node)) {
            acts.push(act("open-app", label, false));
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
        acts.push(act("open-folder", "Open folder", false));
    }
    if is(node, "kind", "project") {
        acts.push(act("open-folder", "Open folder", false));
        acts.push(act("open-vscode", "Open in VS Code", false));
        for n in openers().into_iter().filter_map(|o| o.new_session) {
            acts.push(act(n.id, n.label, false));
        }
    }
    if is(node, "kind", "zone") && has(node, "path") {
        acts.push(act("open-folder", "Open folder", false));
    }
    acts
}

/// The URL (or folder, or file) an action opens: None when the action
/// opens nothing, Some(null) when it would but the node hasn't what it
/// needs.
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
    let folder_text = || folder.as_ref().filter(|f| truthy(Some(f))).map(display);
    let null = Some(Value::Null);
    match action {
        "open-app" => Some(opener(node.get("app").and_then(Value::as_str)).and_then(|o| (o.url)(node)).map_or(Value::Null, Value::String)),
        "open-folder" => Some(folder.filter(|f| !f.is_null()).unwrap_or(Value::Null)),
        "open-file" => {
            if is(node, "kind", "page") {
                node.get("path").cloned()
            } else {
                null
            }
        }
        "open-vscode" => Some(folder_text().map_or(Value::Null, |f| Value::String(xenonet_os::open::vscode_url(&f)))),
        // An app's new session in the folder.
        other => match openers().into_iter().filter_map(|o| o.new_session).find(|n| n.id == other) {
            Some(n) => Some(folder_text().map_or(Value::Null, |f| Value::String((n.url)(&f)))),
            None => null,
        },
    }
}

pub const MAX_BRIEF: usize = 6000;

/// A new session in the other (or the same) app, filled in with a handoff
/// brief.
pub fn handoff_url(target: &str, folder: Option<&str>, brief: &str) -> Option<String> {
    let text = xenonet_text::utf16::head(brief, MAX_BRIEF);
    let folder = folder.filter(|f| !f.is_empty());
    opener(Some(target)).map(|o| (o.handoff)(folder, &text))
}
