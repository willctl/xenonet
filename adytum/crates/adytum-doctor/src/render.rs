//! The doctor's answer as lines for a person and as JSON for a script.

use std::fmt::Write as _;

use serde_json::json;

use crate::{Check, Level};

fn mark(level: Level) -> &'static str {
    match level {
        Level::Info => "● ok  ",
        Level::Warn => "▲ warn",
        Level::Fail => "✕ fail",
    }
}

/// One line per check, and a `fix:` line under each that is not ok.
pub fn render_human(checks: &[Check]) -> String {
    let mut out = String::new();
    for c in checks {
        let _ = writeln!(out, "{} {}: {}", mark(c.level), c.id, c.message);
        if let (false, Some(fix)) = (c.ok, &c.fix) {
            let _ = writeln!(out, "       fix: {fix}");
        }
    }
    out
}

/// True when any check failed: the doctor's exit status.
pub fn has_failure(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.level == Level::Fail)
}

/// `{"ok": bool, "checks": [...]}`, where `ok` means nothing failed.
pub fn render_json(checks: &[Check]) -> String {
    let value = json!({ "ok": !has_failure(checks), "checks": checks });
    let mut text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned());
    text.push('\n');
    text
}
