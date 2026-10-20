//! xenonet messages (cli/commands/messages.js): what agents said to each
//! other in the last day, from the engine (core/engine), with who has read
//! each one.

use crate::args::Flags;
use crate::clock;
use crate::js::{have, list, num, on, str, t};
use crate::out::{c, json_at, print, table, Done, Layout};
use crate::reach::need;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const HELP: &str = include_str!("help/messages.txt");

/// An id cut short, as id.replace(/^(\w+):(.{8}).*$/, "$1:$2") does:
/// claude:0190d3e0-… becomes claude:0190d3e0.
pub fn short_id(id: &str) -> String {
    let word = id.chars().take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_').count();
    let rest: Vec<char> = id.chars().skip(word).collect();
    let line_break = |ch: &char| matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    if word == 0 || rest.first() != Some(&':') || rest.len() < 9 || rest.iter().any(line_break) {
        return id.to_string();
    }
    let head: String = id.chars().take(word).collect();
    format!("{head}:{}", rest[1..9].iter().collect::<String>())
}

pub fn run(_args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let state = s.get("/api/engine/state")?;
    let agents = s.get("/api/engine/agents").unwrap_or_else(|_| json!([]));
    if flags.on("json") {
        json_at(state.get("messages"));
        return Ok(0);
    }
    let messages = state.get("messages").cloned().unwrap_or(Value::Null);
    let mut messages = messages.as_array().cloned().unwrap_or_default();
    if messages.is_empty() {
        print(c::dim("No messages between sessions or actors in the last 24 hours."));
        return Ok(0);
    }
    let titles: HashMap<String, Value> = agents.as_array().into_iter().flatten().map(|a| (t(a, "id"), a.get("title").cloned().unwrap_or(Value::Null))).collect();
    let name = |id: &str| titles.get(id).filter(|v| !v.is_null()).map_or_else(|| short_id(id), |v| crate::js::text(Some(v)));
    messages.sort_by(|a, b| num(a, "at").partial_cmp(&num(b, "at")).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows = Vec::new();
    for m in &messages {
        let target = t(m, "target");
        let to = if target.starts_with("repo:") || target == "all" { target.strip_prefix("repo:").map_or(target.clone(), |r| format!("everyone in {r}")) } else { name(&target) };
        let recipients = list(m, "to");
        let seen = recipients.iter().filter(|d| on(d, "seen")).count();
        let from = have(m, "from").map_or_else(|| t(m, "from"), |f| name(&crate::js::text(Some(f))));
        rows.push(vec![
            c::dim(clock::hour_minute(num(m, "at"))),
            format!("{} {} {to} {}", c::bold(from), c::dim("to"), c::dim(format!("#{}, read by {seen} of {}", t(m, "id"), recipients.len()))),
        ]);
        rows.push(vec![String::new(), str(m, "text").map_or_else(|| t(m, "text"), str::to_string)]);
    }
    print(table(&rows, Layout::wrapped(0)));
    Ok(0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn ids_cut_short() {
        assert_eq!(super::short_id("claude:0190d3e0-0000-7000"), "claude:0190d3e0");
        assert_eq!(super::short_id("actor:wren"), "actor:wren");
        assert_eq!(super::short_id("repo"), "repo");
    }
}
