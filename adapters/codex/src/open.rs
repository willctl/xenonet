//! What opens Codex's threads in the ChatGPT app.

use serde_json::Value;
use xenonet_adapter::open::{as_text, uuid_at, Opener};

fn action(node: &Value) -> Option<&'static str> {
    uuid_at(node, "rawId").then_some("Open in ChatGPT · Codex")
}

fn url(node: &Value) -> Option<String> {
    uuid_at(node, "rawId").then(|| format!("codex://threads/{}", as_text(node.get("rawId"))))
}

fn handoff(folder: Option<&str>, text: &str) -> String {
    let mut pairs = vec![("prompt", text)];
    if let Some(f) = folder {
        pairs.push(("path", f));
    }
    format!("codex://threads/new?{}", xenonet_text::url::form(&pairs))
}

pub fn opener() -> Opener {
    Opener { action, url, new_session: None, handoff }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deep_links_only_for_ids_that_look_like_ids() {
        let o = opener();
        let codex = json!({ "rawId": "0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b" });
        assert_eq!((o.url)(&codex).as_deref(), Some("codex://threads/0190a1b2-c3d4-7e5f-8a6b-7c8d9e0f1a2b"));
        assert_eq!((o.url)(&json!({ "rawId": "x&calc.exe" })), None);
        assert_eq!((o.handoff)(Some("C:\\p"), "go"), "codex://threads/new?prompt=go&path=C%3A%5Cp");
    }
}
