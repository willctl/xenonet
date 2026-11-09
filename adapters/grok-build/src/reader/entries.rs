//! A Grok Build session's conversation as the console shows it, from its
//! updates.jsonl's events: what you asked, what the agent said and thought,
//! and each tool call with how it went, a page at a time.

use serde_json::{Map, Value};
use xenonet_base::text::{js_string, js_trim};
use xenonet_base::units;
use xenonet_sources::line_events::{self, Kind, LineFile};
use xenonet_sources::lines::GrokLine;

fn nonempty(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// A session's lines as the console shows them, oldest first, from their
/// events (line_events.rs): what you asked, what the agent said and
/// thought (as Claude Code's thinking shows, folded), and each tool call
/// with how it went. A run of chunks is one entry, at its first chunk's
/// time, with the whole text its chunks bring.
pub fn entries(lines: &[GrokLine]) -> Vec<Value> {
    let mut out: Vec<Map<String, Value>> = Vec::new();
    let file = LineFile::default();
    let at = |o: &GrokLine| o.ts().ok().filter(|t| t.is_finite()).map_or(Value::Null, xenonet_base::json::number);
    // Where the entry of the run of chunks still open is, which its later
    // chunks carry on.
    let mut open: Option<usize> = None;
    for (n, o) in lines.iter().enumerate() {
        let ts = at(o);
        let entry = |role: &str, text: &str| -> Map<String, Value> {
            let mut m = Map::new();
            m.insert("id".into(), Value::String(format!("update-{n}")));
            m.insert("ts".into(), ts.clone());
            m.insert("role".into(), role.into());
            m.insert("text".into(), Value::String(units::excerpt(text, 2500)));
            m
        };
        let events = line_events::grok(o, &file);
        let in_run = file.run.borrow().is_some();
        if !in_run || file.began.get() {
            open = None;
        }
        // An entry, which carries on the run when the line's in one.
        let push = |out: &mut Vec<Map<String, Value>>, open: &mut Option<usize>, m: Map<String, Value>| {
            out.push(m);
            if in_run {
                *open = Some(out.len() - 1);
            }
        };
        for e in events {
            match &e.kind {
                Kind::Prompt(p) => {
                    let asked = p.claude.as_ref().is_none_or(|c| c.meta != Some(&Value::Bool(true)));
                    if asked && !js_trim(&p.text).is_empty() {
                        push(&mut out, &mut open, entry("user", &p.text));
                    }
                }
                Kind::Said { parts: Some(Value::Array(blocks)), .. } => {
                    let of = |kind: &str, field: &str| -> String {
                        blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some(kind)).filter_map(|b| b.get(field).and_then(Value::as_str)).collect()
                    };
                    let (said, thought) = (of("text", "text"), of("thinking", "thinking"));
                    if !js_trim(&said).is_empty() {
                        push(&mut out, &mut open, entry("assistant", &said));
                    }
                    if !js_trim(&thought).is_empty() {
                        push(&mut out, &mut open, entry("reasoning", &thought));
                    }
                }
                Kind::More { of, whole, meta, .. } => match open {
                    Some(at) => {
                        out[at].insert("text".into(), Value::String(units::excerpt(whole, 2500)));
                    }
                    None if !meta && !js_trim(whole).is_empty() => {
                        let role = match *of {
                            "prompt" => "user",
                            "thinking" => "reasoning",
                            _ => "assistant",
                        };
                        push(&mut out, &mut open, entry(role, whole));
                    }
                    None => {}
                },
                Kind::Tool { id, name, .. } => {
                    let title = nonempty(o.update().get("title")).map_or_else(|| name.map(js_string).unwrap_or_default(), str::to_string);
                    let mut m = Map::new();
                    m.insert("id".into(), Value::String(id.filter(|v| !v.is_null()).map_or_else(|| format!("call-{n}"), js_string)));
                    m.insert("ts".into(), ts.clone());
                    m.insert("role".into(), "tool".into());
                    m.insert("kind".into(), "tool_use".into());
                    m.insert("title".into(), Value::String(title));
                    m.insert("text".into(), Value::String(String::new()));
                    m.insert("status".into(), Value::Null);
                    out.push(m);
                }
                Kind::Result { id, error, .. } => {
                    let id = id.map_or_else(|| "undefined".into(), js_string);
                    if let Some(m) = out.iter_mut().rev().find(|m| m.get("kind").is_some() && m.get("id").and_then(Value::as_str) == Some(&id)) {
                        m.insert("status".into(), if *error == Some(true) { "failed" } else { "completed" }.into());
                    }
                }
                _ => {}
            }
        }
    }
    out.into_iter().map(Value::Object).collect()
}

/// A page of a session's entries, newest first: `cursor` is the count of
/// entries already shown from the end. { entries, cursor, total }.
pub fn page(all: Vec<Value>, cursor: Option<&str>, limit: usize) -> Value {
    let total = all.len();
    let shown = cursor.and_then(|c| c.trim().parse::<usize>().ok()).unwrap_or(0);
    let end = total.saturating_sub(shown);
    let start = end.saturating_sub(limit);
    let entries: Vec<Value> = all[start..end].to_vec();
    serde_json::json!({ "entries": entries, "cursor": if start > 0 { Value::String((total - start).to_string()) } else { Value::Null }, "total": total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_sessions_entries_and_pages() {
        let up = |t: f64, update: Value| GrokLine::new(json!({ "timestamp": t, "method": "session/update", "params": { "update": update } }));
        let lines = [
            up(1.0, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "Run the tests" } })),
            up(2.0, json!({ "sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Execute `npm test`", "kind": "execute", "status": "in_progress" })),
            up(3.0, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "failed" })),
            up(4.0, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "One failed." } })),
        ];
        let all = entries(&lines);
        assert_eq!(all.iter().map(|e| e["role"].as_str().unwrap()).collect::<Vec<_>>(), ["user", "tool", "assistant"]);
        assert_eq!((all[1]["title"].as_str(), all[1]["status"].as_str(), all[1]["ts"].as_f64()), (Some("Execute `npm test`"), Some("failed"), Some(2000.0)));
        let p = page(all.clone(), None, 2);
        assert_eq!((p["entries"].as_array().map(Vec::len), p["cursor"].as_str(), p["total"].as_u64()), (Some(2), Some("2"), Some(3)));
        let p = page(all, Some("2"), 2);
        assert_eq!((p["entries"][0]["role"].as_str(), p["cursor"].is_null()), (Some("user"), true));
    }
}
