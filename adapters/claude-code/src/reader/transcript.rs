//! A Claude Code transcript's messages as the console shows them, a page
//! at a time: each message's text, thinking, tool calls and results, the
//! long ones cut.

use serde_json::Value;
use xenonet_base::text::{js_number, js_string, truthy};
use xenonet_base::{json, time};
use xenonet_sources::lines::{opt_prop, prop, Js, TypeError};
use xenonet_sources::record::{excerpt, excerpt_json, js_max0, nn, slice_index, tpl, Obj, F};

/// The page of `len` messages to show, newest first: (start, end, cursor
/// further back). `cursor` is the count of messages already shown from
/// the end.
pub fn page(len: usize, cursor: Js<'_>, limit: f64) -> (usize, usize, Option<String>) {
    let shown = js_number(Some(nn(cursor).unwrap_or(&Value::from(0))));
    let end = len as f64 - shown;
    let start = js_max0(end - limit);
    let (a, b) = (slice_index(start, len), slice_index(end, len));
    let next = (start > 0.0).then(|| json::number_string(len as f64 - start));
    (a, b.max(a), next)
}

// What a tool result's content says: its text, an image as "[image]".
fn block_text(content: Js<'_>) -> Result<String, TypeError> {
    match content {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(parts)) => {
            let mut out = Vec::with_capacity(parts.len());
            for c in parts {
                let ty = prop(Some(c), "type")?;
                out.push(match ty.and_then(Value::as_str) {
                    Some("text") => nn(prop(Some(c), "text")?).map_or_else(String::new, js_string),
                    Some("image") => "[image]".into(),
                    _ => String::new(),
                });
            }
            Ok(out.join("\n"))
        }
        _ => Ok(String::new()),
    }
}

// A message's entries, as the console shows them.
fn message_entries(m: &Value, out: &mut Vec<Obj>) -> Result<(), TypeError> {
    let m = Some(m);
    let stamp = prop(m, "timestamp")?;
    let ts = if let Some(stamp) = stamp.filter(|s| truthy(Some(s))) {
        match time::parse_iso(&js_string(stamp)) {
            Some(t) => F::V(json::number(t)),
            None => F::Num(f64::NAN),
        }
    } else {
        F::null()
    };
    let kind = prop(m, "type")?.and_then(Value::as_str).map(str::to_string);
    let uuid = prop(m, "uuid")?;
    let content = opt_prop(prop(m, "message")?, "content");
    if let Some(Value::String(text)) = content {
        let role = match kind.as_deref() {
            Some("assistant") => "assistant",
            Some("user") => "user",
            _ => "system",
        };
        out.push(vec![("id", F::of(uuid)), ("ts", ts), ("role", F::s(role)), ("text", F::s(text.clone()))]);
        return Ok(());
    }
    let blocks: &[Value] = match nn(content) {
        None => &[],
        Some(Value::Array(a)) => a,
        Some(_) => return Err(TypeError("(content ?? []).entries is not a function".into())),
    };
    for (i, b) in blocks.iter().enumerate() {
        let b = Some(b);
        let id = F::s(format!("{}:{i}", tpl(uuid)));
        match prop(b, "type")?.and_then(Value::as_str) {
            Some("text") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s(if kind.as_deref() == Some("assistant") { "assistant" } else { "user" })),
                ("text", F::of(prop(b, "text")?)),
            ]),
            Some("thinking") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s("reasoning")),
                ("text", F::V(nn(prop(b, "thinking")?).cloned().unwrap_or(Value::String(String::new())))),
            ]),
            Some("tool_use") => out.push(vec![
                ("id", id),
                ("ts", ts.clone()),
                ("role", F::s("tool")),
                ("kind", F::s("tool_use")),
                ("title", F::of(prop(b, "name")?)),
                ("text", excerpt_json(prop(b, "input")?, 2500)),
            ]),
            Some("tool_result") => {
                let status = if truthy(prop(b, "is_error")?) { "failed" } else { "completed" };
                let text = block_text(prop(b, "content")?)?;
                out.push(vec![
                    ("id", id),
                    ("ts", ts.clone()),
                    ("role", F::s("tool")),
                    ("kind", F::s("tool_result")),
                    ("title", F::s("result")),
                    ("status", F::s(status)),
                    ("text", excerpt(Some(&Value::String(text)), 4000)),
                ]);
            }
            _ => {}
        }
    }
    Ok(())
}

fn shows(e: &Obj) -> bool {
    let field = |k: &str| e.iter().find(|(n, _)| *n == k).map(|(_, f)| f);
    let truthy_f = |f: Option<&F>| match f {
        Some(F::V(v)) => truthy(Some(v)),
        Some(F::Num(n)) => *n != 0.0 && !n.is_nan(),
        Some(F::Arr(_)) => true,
        _ => false,
    };
    truthy_f(field("text")) || truthy_f(field("title"))
}

/// A page of a transcript's messages (after the lenient reader) as the
/// console shows them: each message's text, thinking, tool calls and
/// results, the long ones cut, and only those with something to show.
pub fn entries(messages: &[Value]) -> Result<Vec<Obj>, TypeError> {
    let mut out = Vec::new();
    for m in messages {
        message_entries(m, &mut out)?;
    }
    out.retain(shows);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn get<'a>(o: &'a Obj, k: &str) -> Option<&'a F> {
        o.iter().find(|(n, _)| *n == k).map(|(_, f)| f)
    }

    #[test]
    fn pages_and_entries() {
        assert_eq!(page(4, None, 120.0), (0, 4, None));
        assert_eq!(page(4, None, 2.0), (2, 4, Some("2".into())));
        assert_eq!(page(4, Some(&json!("2")), 2.0), (0, 2, None));
        assert_eq!(page(5, Some(&json!(7)), 120.0), (0, 3, None));
        let ms = [
            json!({ "type": "user", "uuid": "u2", "timestamp": "2026-10-03T12:02:00Z", "message": { "content": [{ "type": "tool_result", "is_error": true, "content": [{ "type": "text", "text": "1 failed" }, { "type": "image" }] }, { "type": "text" }] } }),
        ];
        let es = entries(&ms).unwrap();
        assert_eq!(es.len(), 1);
        assert_eq!(get(&es[0], "text"), Some(&F::s("1 failed\n[image]")));
        assert_eq!(get(&es[0], "id"), Some(&F::s("u2:0")));
        assert!(entries(&[json!({ "message": { "content": {} } })]).is_err());
    }
}
