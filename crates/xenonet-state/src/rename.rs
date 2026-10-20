//! core/rename.js: a session's name as you give it in XenoNet, before it's
//! saved in the session's own app.

use serde_json::Value;
use xenonet_base::jsstr;
use xenonet_base::text::{js_trim, truthy};

/// Long enough for any title the apps make, short enough to show whole.
pub const MAX_TITLE: usize = 120;

/// A name on one line, without control characters, at most MAX_TITLE
/// units long; "" when there's nothing left of it.
pub fn clean_title(title: &str) -> String {
    let mut no_controls = String::with_capacity(title.len());
    let mut in_run = false;
    for c in title.chars() {
        if matches!(c as u32, 0..=0x1f | 0x7f..=0x9f) {
            if !in_run {
                no_controls.push(' ');
                in_run = true;
            }
        } else {
            no_controls.push(c);
            in_run = false;
        }
    }
    let one_line = jsstr::collapse_space(&no_controls);
    js_trim(&jsstr::head(js_trim(&one_line), MAX_TITLE)).to_string()
}

/// Whether a node is a session XenoNet can rename: one of this machine's,
/// of Claude Code or Codex.
pub fn renamable(n: Option<&Value>) -> bool {
    let Some(Value::Object(m)) = n else { return false };
    m.get("kind").and_then(Value::as_str) == Some("session") && !truthy(m.get("host")) && matches!(m.get("app").and_then(Value::as_str), Some("claude" | "codex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_on_one_line() {
        assert_eq!(clean_title("  Fix\n\tthe\u{0}\u{1}tests \u{2028} now "), "Fix the tests now");
        assert_eq!(clean_title(&"x".repeat(200)).len(), 120);
        assert_eq!(clean_title(&format!("{} y", "x".repeat(119))), "x".repeat(119));
        assert!(renamable(Some(&serde_json::json!({ "kind": "session", "app": "codex" }))));
        assert!(!renamable(Some(&serde_json::json!({ "kind": "session", "app": "codex", "host": "@h" }))));
    }
}
