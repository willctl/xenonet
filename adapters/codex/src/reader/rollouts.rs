//! A Codex thread's items from its rollout file, for a thread whose
//! app-server can't list them: the finished items a rollout writes, or
//! the older lines of one without (`RolloutEntries`), read a page at a
//! time from a file that grows, or whole from a compressed archive.

use super::threads::{base, is_object, mcp_text, reasoning, spread, text_of, with};
use serde_json::{json, Value};
use std::sync::Mutex;
use xenonet_adapter::reader::VendorCheck;
use xenonet_base::text::{js_string, truthy};
use xenonet_base::{json as js, units};
use xenonet_sources::archive;
use xenonet_sources::lines::{opt_prop, prop, CodexLine, Js, TypeError};
use xenonet_sources::live::command_text;
use xenonet_sources::record::{excerpt, excerpt_json, js_max0, nn, obj_json, slice_index, tpl, Obj, F};

fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// A finished item as a rollout writes it (event_msg's item_completed: the
// app-server's items, in Codex's own names), as the console shows it.
fn rollout_item(it: &Value, ts: F, n: usize) -> Result<Obj, TypeError> {
    let it = Some(it);
    let get = |k: &str| prop(it, k);
    let ty = get("type")?;
    let id = match nn(get("id")?) {
        Some(v) => F::V(v.clone()),
        None => F::s(format!("item-{n}")),
    };
    let b = base(id, ts, ty);
    Ok(match ty.and_then(Value::as_str) {
        Some("UserMessage") => with(b, vec![("role", F::s("user")), ("text", F::s(text_of(get("content")?)))]),
        Some("AgentMessage") => with(b, vec![("role", F::s("assistant")), ("text", F::s(text_of(get("content")?)))]),
        Some("Reasoning") => {
            let mut parts = spread(get("summary_text")?, "it.summary_text")?;
            parts.extend(spread(get("raw_content")?, "it.raw_content")?);
            with(b, vec![("role", F::s("reasoning")), ("text", F::s(reasoning(parts, true)?))])
        }
        Some("CommandExecution") => {
            let code = get("exit_code")?;
            let meta = nn(code).map_or(F::null(), |c| F::s(format!("exit {}", js_string(c))));
            with(
                b,
                vec![
                    ("role", F::s("tool")),
                    ("title", F::s(command_text(get("command")?))),
                    ("status", F::of(get("status")?)),
                    ("meta", meta),
                    ("text", excerpt(get("aggregated_output")?, 4000)),
                ],
            )
        }
        Some("McpToolCall") => {
            let title = format!("{}.{}", tpl(get("server")?), tpl(get("tool")?));
            let text = mcp_text(it)?;
            with(b, vec![("role", F::s("tool")), ("title", F::s(title)), ("status", F::of(get("status")?)), ("text", text)])
        }
        Some("FileChange") => {
            // Object.entries(it.changes ?? {})
            let entries: Vec<(String, Value)> = match nn(get("changes")?) {
                Some(Value::Object(m)) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Some(Value::Array(a)) => a.iter().enumerate().map(|(i, v)| (i.to_string(), v.clone())).collect(),
                Some(Value::String(s)) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::String(c.to_string()))).collect(),
                _ => Vec::new(),
            };
            let text: Vec<String> = entries.iter().map(|(p, c)| format!("{} {p}", nn(opt_prop(Some(c), "type")).map_or_else(String::new, js_string))).collect();
            with(b, vec![("role", F::s("tool")), ("title", F::s(format!("edit {} file(s)", entries.len()))), ("status", F::of(get("status")?)), ("text", F::s(text.join("\n")))])
        }
        // An extension's web search is a web search; its other kinds (an
        // image made, a pause) show as any other item.
        Some("WebSearch") => web_search(b, it)?,
        Some("Extension") if get("kind")?.and_then(Value::as_str) == Some("web.search") => web_search(b, it)?,
        Some("ContextCompaction") => with(b, vec![("role", F::s("system")), ("text", F::s("context compacted"))]),
        // The plan, as the app-server's plan item shows it.
        Some("Plan") => with(b, vec![("role", F::s("assistant")), ("title", F::s("plan")), ("text", F::V(nn(get("text")?).cloned().unwrap_or(Value::String(String::new()))))]),
        _ => with(b, vec![("role", F::s("system")), ("title", F::of(ty)), ("text", excerpt_json(it, 1500))]),
    })
}

/// A web search, as a thread shown from its rollout reads it: an empty
/// query is no query, and the action's says what was searched.
fn web_search(b: Obj, it: Js<'_>) -> Result<Obj, TypeError> {
    let q = prop(it, "query")?.filter(|q| truthy(Some(q))).or_else(|| opt_prop(prop(it, "action").ok().flatten(), "query").filter(|q| truthy(Some(q))));
    let q = q.map_or_else(String::new, js_string);
    Ok(with(b, vec![("role", F::s("tool")), ("title", F::s(format!("search: {q}"))), ("text", F::s(""))]))
}

fn tool_call(t: &str, output: bool) -> bool {
    let t = if output {
        match t.strip_suffix("_output") {
            Some(t) => t,
            None => return false,
        }
    } else {
        t
    };
    matches!(t, "function_call" | "custom_tool_call" | "local_shell_call")
}

/// A thread's items from its rollout, oldest first, as its lines are read:
/// its finished items, each at the time it started (or finished, or was
/// written). A rollout with none, as `codex exec` writes them, has the
/// older lines: what you said and the agent said as events, and each call
/// and its output, kept only until a finished item turns up.
#[derive(Default, Clone, Debug)]
pub struct RolloutEntries {
    items: Vec<Obj>,
    older: Vec<Obj>,
}

impl RolloutEntries {
    pub fn list(&self) -> &[Obj] {
        if self.items.is_empty() {
            &self.older
        } else {
            &self.items
        }
    }

    /// A rollout's line (its record, as lines.js read it).
    pub fn add(&mut self, raw: Value) -> Result<(), TypeError> {
        let o = CodexLine { raw };
        let p = o.payload()?.clone();
        let p = Some(&p);
        let ts = o.ts()?;
        let at = if ts.is_finite() { F::V(js::number(ts)) } else { F::null() };
        let kind = o.kind()?.cloned();
        let is = |k: &str| kind.as_ref().and_then(Value::as_str) == Some(k);
        let ptype = prop(p, "type")?;
        let item = prop(p, "item")?;
        if is("event_msg") && ptype.and_then(Value::as_str) == Some("item_completed") && truthy(item) && is_object(item) {
            let when = match nn(prop(p, "started_at_ms")?).or(nn(prop(p, "completed_at_ms")?)) {
                Some(v) => F::V(v.clone()),
                None => at,
            };
            let entry = rollout_item(item.expect("checked truthy above"), when, self.items.len())?;
            self.items.push(entry);
            self.older.clear();
            return Ok(());
        }
        if !self.items.is_empty() {
            return Ok(());
        }
        let id = match nn(prop(p, "id")?).or(nn(prop(p, "call_id")?)) {
            Some(v) => F::V(v.clone()),
            None => F::s(format!("item-{}", self.older.len())),
        };
        let b = base(id, at, ptype);
        let pt = tpl(ptype);
        if is("event_msg") && (pt == "user_message" || pt == "agent_message") && ptype.is_some_and(Value::is_string) {
            let text = nn(prop(p, "message")?).map_or_else(String::new, js_string);
            self.older.push(with(b, vec![("role", F::s(if pt == "user_message" { "user" } else { "assistant" })), ("text", F::s(text))]));
        } else if is("response_item") && tool_call(&pt, false) {
            let input = nn(prop(p, "input")?).or(nn(prop(p, "arguments")?)).or(nn(prop(p, "action")?));
            let text = match input {
                Some(Value::String(s)) => F::s(units::excerpt(s, 2500)),
                Some(v) => F::s(units::excerpt(&js::stringify(v, 2), 2500)),
                None => F::s("null"),
            };
            let title = nn(prop(p, "name")?).or(ptype).cloned().map_or(F::Undef, F::V);
            self.older.push(with(b, vec![("role", F::s("tool")), ("title", title), ("text", text)]));
        } else if is("response_item") && tool_call(&pt, true) {
            let text = match prop(p, "output")? {
                Some(Value::String(s)) => F::s(units::excerpt(s, 4000)),
                out => F::s(units::excerpt(&js::stringify(nn(out).unwrap_or(&Value::Null), 2), 4000)),
            };
            self.older.push(with(b, vec![("role", F::s("tool")), ("title", F::s("result")), ("text", text)]));
        }
        Ok(())
    }
}

/// A page of `len` entries, newest first: the range to show, and the
/// cursor further back. `cursor` ("`rollout:<n>`") counts the items already
/// shown from the end.
pub fn rollout_page(len: usize, cursor: Js<'_>, limit: f64) -> (usize, usize, Option<String>) {
    let c = match nn(cursor) {
        Some(v) => js_string(v),
        None => String::new(),
    };
    let shown = c.strip_prefix("rollout:").filter(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())).map_or(0.0, |d| d.parse::<f64>().unwrap_or(0.0));
    let end = (len as f64 - shown).max(0.0);
    let start = js_max0(end - limit);
    let (a, b) = (slice_index(start, len), slice_index(end, len));
    let cursor = (start > 0.0).then(|| format!("rollout:{}", js::number_string(len as f64 - start)));
    (a, b.max(a), cursor)
}

// The rollout read last, kept while it's paged through, and read on from
// where it ended as it grows; a compressed archive, by its size and time
// when it was read whole (`stamp`).
struct Rollout {
    file: String,
    offset: u64,
    rest: Vec<u8>,
    stamp: Option<String>,
    entries: RolloutEntries,
}

impl Rollout {
    fn new(file: &str, offset: u64) -> Rollout {
        Rollout { file: file.into(), offset, rest: Vec::new(), stamp: None, entries: RolloutEntries::default() }
    }
}

/// The rollouts a thread is read from, one at a time: the last read,
/// kept while it's paged through.
#[derive(Default)]
pub struct Rollouts {
    last: Mutex<Option<Rollout>>,
}

impl Rollouts {
    /// A page of a thread from its rollout file: newest first, `cursor`
    /// (`"rollout:<n>"`) continuing further back.
    pub fn page(&self, vendor: &dyn VendorCheck, file: &str, cursor: Option<&str>, limit: f64) -> Result<Value, String> {
        let mut slot = lock(&self.last);
        if slot.as_ref().is_none_or(|r| r.file != file) {
            *slot = Some(Rollout::new(file, 0));
        }
        let Some(r) = slot.as_mut() else { return Err("no rollout".into()) };
        if archive::is_archive(file) {
            // A compressed archive is read whole, and again only when it
            // changes.
            let meta = std::fs::metadata(file).map_err(|e| e.to_string())?;
            let stamp = format!("{}:{}", meta.len(), xenonet_base::fs::mtime_ms_of(&meta).unwrap_or(f64::NAN));
            if r.stamp.as_deref() != Some(stamp.as_str()) {
                let data = archive::read(file).map_err(|e| e.to_string())?;
                *r = Rollout::new(file, 0);
                let _ = add_lines(vendor, &mut r.entries, &data);
                r.stamp = Some(stamp);
            }
        } else {
            let data = std::fs::read(file).map_err(|e| e.to_string())?;
            let (restarted, start) = xenonet_sources::lines::read_from(data.len() as u64, r.offset);
            if restarted {
                *r = Rollout::new(file, start);
            }
            if (data.len() as u64) > r.offset {
                let mut chunk = std::mem::take(&mut r.rest);
                chunk.extend_from_slice(&data[r.offset as usize..]);
                r.offset = data.len() as u64;
                r.rest = add_lines(vendor, &mut r.entries, &chunk);
            }
        }
        let cursor = cursor.map(|c| Value::String(c.into()));
        let all = r.entries.list();
        let (a, b, next) = rollout_page(all.len(), cursor.as_ref(), limit);
        Ok(json!({ "entries": all[a..b].iter().map(obj_json).collect::<Vec<_>>(), "cursor": next, "total": all.len() }))
    }
}

// Adds a rollout's lines to its entries, through the Codex normalizer, a
// last line without a newline too when it reads whole; what's left of a
// half-written one comes back, for the next read.
fn add_lines(vendor: &dyn VendorCheck, entries: &mut RolloutEntries, chunk: &[u8]) -> Vec<u8> {
    let t = xenonet_sources::lines::take(chunk);
    let mut sampling = xenonet_schema::vendor::Sampling::default();
    for l in &t.lines {
        if let Some(raw) = vendor.line("codex", l, &mut sampling) {
            let _ = entries.add(raw);
        }
    }
    match t.last {
        Some(last) => {
            if let Some(raw) = vendor.record("codex", last, &mut sampling) {
                let _ = entries.add(raw);
            }
            Vec::new()
        }
        None => t.rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::reader::Unchecked;

    fn get<'a>(o: &'a Obj, k: &str) -> &'a F {
        &o.iter().find(|(n, _)| *n == k).unwrap().1
    }

    #[test]
    fn reads_a_rollout() {
        let mut r = RolloutEntries::default();
        r.add(json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": "Hi" } })).unwrap();
        r.add(json!({ "type": "response_item", "timestamp": "2026-10-05T13:00:01Z", "payload": { "type": "custom_tool_call", "call_id": "c", "name": "exec", "input": "{}" } }))
            .unwrap();
        assert_eq!(r.list().len(), 2);
        r.add(
            json!({ "type": "event_msg", "payload": { "type": "item_completed", "item": { "type": "CommandExecution", "command": ["/bin/zsh", "-lc", "make"], "exit_code": 2 } } }),
        )
        .unwrap();
        assert_eq!(r.list().len(), 1);
        assert_eq!(get(&r.list()[0], "title"), &F::s("make"));
        assert_eq!(get(&r.list()[0], "id"), &F::s("item-0"));
        assert_eq!(rollout_page(10, Some(&json!("rollout:4")), 4.0), (2, 6, Some("rollout:8".into())));
        assert_eq!(rollout_page(10, None, 120.0), (0, 10, None));
    }

    #[test]
    fn a_rollouts_plan_and_an_extensions_web_search() {
        let item = |it: Value| rollout_item(&it, F::null(), 0).unwrap();
        let plan = item(json!({ "type": "Plan", "id": "p1", "text": "1. Read the Makefile" }));
        assert_eq!([get(&plan, "role"), get(&plan, "title"), get(&plan, "text")], [&F::s("assistant"), &F::s("plan"), &F::s("1. Read the Makefile")]);
        let search = item(json!({ "type": "Extension", "id": "x1", "kind": "web.search", "query": "", "action": { "type": "search", "query": "gnu make jobserver" } }));
        assert_eq!([get(&search, "role"), get(&search, "title")], [&F::s("tool"), &F::s("search: gnu make jobserver")]);
        let image = item(json!({ "type": "Extension", "id": "x2", "kind": "image_gen.generation", "status": "completed" }));
        assert_eq!([get(&image, "role"), get(&image, "title")], [&F::s("system"), &F::s("Extension")]);
    }

    #[test]
    fn a_rollout_read_a_page_at_a_time() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-rollout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("rollout-x.jsonl");
        let line =
            |m: &str| format!("{}\n", js::stringify(&json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": m } }), 0));
        std::fs::write(&file, format!("{}{}", line("one"), line("two"))).expect("a rollout");
        let c = Rollouts::default();
        let f = file.to_string_lossy().into_owned();
        let page = c.page(&Unchecked, &f, None, 1.0).expect("a page");
        assert_eq!((page["entries"][0]["text"].as_str(), page["cursor"].as_str()), (Some("two"), Some("rollout:1")));
        let older = c.page(&Unchecked, &f, Some("rollout:1"), 1.0).expect("a page");
        assert_eq!(older["entries"][0]["text"], "one");

        // An archived rollout is read whole, decompressed, and again once it
        // changes: here, archived again with a third line, which has no
        // newline.
        let packed = dir.join("rollout-x.jsonl.zst");
        let pack = |text: &str| ruzstd::encoding::compress_to_vec(text.as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
        std::fs::write(&packed, pack(&format!("{}{}", line("one"), line("two")))).expect("an archive");
        let p = packed.to_string_lossy().into_owned();
        let page = c.page(&Unchecked, &p, None, 5.0).expect("a page of the archive");
        assert_eq!((page["total"].as_u64(), page["entries"][1]["text"].as_str()), (Some(2), Some("two")));
        assert_eq!(c.page(&Unchecked, &p, None, 5.0).expect("the same page")["total"], 2, "read once while it's as it was");
        std::fs::write(&packed, pack(&format!("{}{}{}", line("one"), line("two"), line("three").trim_end()))).expect("archived again");
        let page = c.page(&Unchecked, &p, None, 5.0).expect("a page of the new archive");
        assert_eq!((page["total"].as_u64(), page["entries"][2]["text"].as_str()), (Some(3), Some("three")));
        std::fs::write(&packed, b"not zstd").expect("a broken archive");
        assert!(c.page(&Unchecked, &p, None, 5.0).is_err(), "a broken archive is an error, not a panic");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
