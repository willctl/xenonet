//! What opens Claude Code's sessions and projects in Claude desktop: its
//! own Code sessions by their id, and Claude Code's by theirs, which it
//! imports.

use serde_json::Value;
use xenonet_adapter::open::{as_text, desktop_at, has, uuid_at, NewSession, Opener};

fn action(node: &Value) -> Option<&'static str> {
    if desktop_at(node) {
        Some("Open in Claude desktop")
    } else if !has(node, "desktopId") && uuid_at(node, "rawId") {
        Some("Import into Claude desktop")
    } else {
        None
    }
}

fn url(node: &Value) -> Option<String> {
    if desktop_at(node) {
        Some(format!("claude://code/continue?session={}", xenonet_text::url::component(&as_text(node.get("desktopId")))))
    } else if uuid_at(node, "rawId") {
        Some(format!("claude://resume?session={}", as_text(node.get("rawId"))))
    } else {
        None
    }
}

fn new_session(folder: &str) -> String {
    format!("claude://code/new?folder={}", xenonet_text::url::component(folder))
}

fn handoff(folder: Option<&str>, text: &str) -> String {
    let mut pairs = Vec::new();
    if let Some(f) = folder {
        pairs.push(("folder", f));
    }
    pairs.push(("q", text));
    format!("claude://code/new?{}", xenonet_text::url::form(&pairs))
}

pub fn opener() -> Opener {
    Opener { action, url, new_session: Some(NewSession { id: "new-claude", label: "New Claude Code session here", url: new_session }), handoff }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn desktop_sessions_and_new_ones_in_a_folder() {
        let o = opener();
        let desktop = json!({ "desktopId": "local_ab-1", "rawId": "x" });
        assert_eq!(((o.action)(&desktop), (o.url)(&desktop).as_deref()), (Some("Open in Claude desktop"), Some("claude://code/continue?session=local_ab-1")));
        let raw = json!({ "rawId": "0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b" });
        assert_eq!((o.action)(&raw), Some("Import into Claude desktop"));
        assert_eq!((o.url)(&json!({ "rawId": "x&calc.exe" })), None);
        assert_eq!((o.new_session.expect("a new session").url)("C:\\Users\\me\\Projects\\a b"), "claude://code/new?folder=C%3A%5CUsers%5Cme%5CProjects%5Ca%20b");
        assert_eq!((o.handoff)(Some("/p"), "go on"), "claude://code/new?folder=%2Fp&q=go+on");
    }
}
