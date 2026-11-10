//! The actions a node offers and the URLs they open, with every adapter's
//! opener in place.

use serde_json::{json, Value};
use xenonet_adapter::open::{actions_for, handoff_url, url_for};

#[test]
fn deep_links_only_for_ids_that_look_like_ids() {
    xenonet_platforms::install();
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
    xenonet_platforms::install();
    let p = json!({ "kind": "project", "path": "/Users/me/Projects/a b#2" });
    assert_eq!(url_for(&p, "open-vscode"), Some(json!("vscode://file/Users/me/Projects/a%20b%232")));
    let ids: Vec<Value> = actions_for(Some(&p)).iter().map(|a| a["id"].clone()).collect();
    assert_eq!(ids, [json!("open-folder"), json!("open-vscode"), json!("new-claude")]);
    let w = json!({ "kind": "project", "path": "C:\\Users\\me\\Projects\\a b" });
    assert_eq!(url_for(&w, "new-claude"), Some(json!("claude://code/new?folder=C%3A%5CUsers%5Cme%5CProjects%5Ca%20b")));
    assert_eq!(handoff_url("codex", Some("C:\\p"), "go").as_deref(), Some("codex://threads/new?prompt=go&path=C%3A%5Cp"));
    assert_eq!(handoff_url("grok", None, "go"), None);
}
