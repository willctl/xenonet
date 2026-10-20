//! What the Agent SDK reads and writes of Claude Code's own files, which
//! the JS reaches through @anthropic-ai/claude-agent-sdk 0.3.287: its
//! listSessions(), getSessionMessages(id) and renameSession(id, title), as
//! XenoNet calls them (no options). Each session is a transcript,
//! `<projects>/<folder>/<session>.jsonl`. The listing reads only the first
//! and last 64 KB of each, and finds what it shows by scanning their text
//! for keys, as the SDK does; a session's messages are its chain of
//! entries from the last one back to the first.
//!
//! What this leaves out of the SDK: its session stores (unused without
//! options), queued commands' delivery records (a prompt typed while a
//! turn ran shows where the transcript put it), and the skip over what
//! precedes the last compaction in files over 5 MiB, which the chain stops
//! at anyway.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use xenonet_base::text::{js_trim, truthy};

/// The Agent SDK release whose readings these are, as the lenient
/// readers pick their schema by it.
pub const VERSION: &str = "0.3.287";

const CHUNK: u64 = 65536;
const SCAN_MAX: u64 = 16 * 1024 * 1024;

fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { c.is_ascii_hexdigit() })
}

fn sorted_names(dir: &Path) -> Vec<std::fs::DirEntry> {
    xenonet_base::paths::read_dir_sorted(dir).unwrap_or_default()
}

// ---------- the head and tail of a file ----------

struct HeadTail {
    mtime: f64,
    size: u64,
    head: String,
    tail: String,
}

fn open_plain(file: &Path) -> std::io::Result<std::fs::File> {
    // A link to a transcript isn't one (O_NOFOLLOW in the SDK).
    let meta = std::fs::symlink_metadata(file)?;
    if !meta.file_type().is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    std::fs::File::open(file)
}

fn read_at(f: &mut std::fs::File, at: u64, n: u64) -> std::io::Result<Vec<u8>> {
    f.seek(SeekFrom::Start(at))?;
    let mut buf = Vec::with_capacity(n as usize);
    f.take(n).read_to_end(&mut buf)?;
    Ok(buf)
}

fn head_tail(file: &Path) -> Option<HeadTail> {
    let mut f = open_plain(file).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let head = read_at(&mut f, 0, CHUNK).ok()?;
    if head.is_empty() {
        return None;
    }
    let mtime = xenonet_base::fs::mtime_ms_of(&meta).map_or(0.0, f64::floor);
    let head_text = String::from_utf8_lossy(&head).into_owned();
    let tail = if size > CHUNK { String::from_utf8_lossy(&read_at(&mut f, size - CHUNK, CHUNK).ok()?).into_owned() } else { head_text.clone() };
    Some(HeadTail { mtime, size, head: head_text, tail })
}

// ---------- finding values in the text ----------

// The value after a key's opening quote: to the next quote no backslash
// escapes; None when it never closes.
fn value_at(text: &str, from: usize) -> Option<String> {
    let b = text.as_bytes();
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b'"' => {
                let raw = &text[from..i];
                if raw.contains('\\') {
                    return Some(xenonet_base::json::parse(&format!("\"{raw}\"")).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| raw.to_string()));
                }
                return Some(raw.to_string());
            }
            _ => i += 1,
        }
    }
    None
}

fn starts(text: &str, pat: &str) -> Vec<usize> {
    text.match_indices(pat).map(|(i, _)| i + pat.len()).collect()
}

/// The last `"key":"…"` (or `"key": "…"`) in the text that closes.
fn last_value(text: &str, key: &str) -> Option<String> {
    let mut at: Vec<usize> = starts(text, &format!("\"{key}\":\"")).into_iter().chain(starts(text, &format!("\"{key}\": \""))).collect();
    at.sort_unstable();
    at.into_iter().rev().find_map(|i| value_at(text, i))
}

/// The first `"key":"…"`, or, only when there's none of those, the first
/// `"key": "…"`.
fn first_value(text: &str, key: &str) -> Option<String> {
    let tight = starts(text, &format!("\"{key}\":\""));
    let list = if tight.is_empty() { starts(text, &format!("\"{key}\": \"")) } else { tight };
    list.into_iter().next().and_then(|i| value_at(text, i))
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

// The cwd a relocated session moved to: the last relocated line's.
fn relocated(tail: &str) -> Option<String> {
    tail.split('\n').rev().find(|l| l.contains("\"relocatedCwd\":") && l.contains("\"type\":\"relocated\"")).and_then(|l| {
        let v = xenonet_base::json::parse(l).ok()?;
        (v.get("type").and_then(Value::as_str) == Some("relocated")).then(|| v.get("relocatedCwd").and_then(Value::as_str).map(str::to_string)).flatten()
    })
}

// ---------- the first prompt ----------

// <pasted_content id="hhhh">\n BODY \n</pasted_content id="hhhh">, each
// replaced by its body, with up to two newlines before and after the tags.
fn unwrap_pastes(text: &str) -> String {
    let mut out = text.to_string();
    let open = "<pasted_content id=\"";
    let mut from = 0;
    while let Some(i) = out[from..].find(open).map(|i| i + from) {
        let id = &out[i + open.len()..];
        let ok = id.len() >= 6 && id[..4].bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) && &id[4..6] == "\">";
        if !ok {
            from = i + 1;
            continue;
        }
        let hex = id[..4].to_string();
        let start_body = i + open.len() + 6;
        let close = format!("</pasted_content id=\"{hex}\">");
        let Some(end) = out[start_body..].find(&close).map(|e| e + start_body) else {
            from = i + 1;
            continue;
        };
        let body = out[start_body..end].trim_start_matches('\n').trim_end_matches('\n').to_string();
        let mut before = i;
        for _ in 0..2 {
            if before > 0 && out.as_bytes()[before - 1] == b'\n' {
                before -= 1;
            }
        }
        let mut after = end + close.len();
        for _ in 0..2 {
            if out.as_bytes().get(after) == Some(&b'\n') {
                after += 1;
            }
        }
        out.replace_range(before..after, &body);
        from = before + body.len();
    }
    out
}

fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let s = text.find(open)? + open.len();
    let e = text[s..].find(close)? + s;
    Some(&text[s..e])
}

// Text that starts with a tag, or an interrupt marker.
fn is_markup(text: &str) -> bool {
    let t = text.trim_start();
    if t.starts_with("[Request interrupted by user") && t.contains(']') {
        return true;
    }
    let mut c = t.chars();
    if c.next() != Some('<') {
        return false;
    }
    let Some(first) = c.next().filter(char::is_ascii_lowercase) else { return false };
    let _ = first;
    for ch in c {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            continue;
        }
        return ch.is_whitespace() || ch == '>';
    }
    false
}

fn cut_200(text: &str) -> String {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.len() <= 200 {
        return text.to_string();
    }
    let mut keep = units[..200].to_vec();
    if keep.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
        keep.pop();
    }
    format!("{}…", js_trim(&String::from_utf16_lossy(&keep)))
}

// What someone typed in a user line, or the command it ran.
fn prompt_of(obj: &Value, command: &mut Option<String>) -> Option<String> {
    if obj.get("type").and_then(Value::as_str) != Some("user") || truthy(obj.get("isMeta")) || truthy(obj.get("isCompactSummary")) {
        return None;
    }
    let content = obj.get("message")?.get("content")?;
    let texts: Vec<String> = match content {
        Value::String(s) => vec![s.clone()],
        Value::Array(blocks) => {
            if blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result")) {
                return None;
            }
            blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("text")).filter_map(|b| b.get("text").and_then(Value::as_str).map(str::to_string)).collect()
        }
        _ => return None,
    };
    for t in texts {
        let text = js_trim(&unwrap_pastes(&t).replace('\n', " ")).to_string();
        if text.is_empty() {
            continue;
        }
        if let Some(name) = between(&text, "<command-name>", "</command-name>") {
            command.get_or_insert_with(|| name.to_string());
            continue;
        }
        if let Some(input) = between(&text, "<bash-input>", "</bash-input>") {
            return Some(format!("! {}", js_trim(input)));
        }
        if is_markup(&text) {
            continue;
        }
        return Some(cut_200(&text));
    }
    None
}

fn first_prompt(head: &str) -> String {
    let mut command = None;
    for line in head.split('\n') {
        if !(line.contains("\"type\":\"user\"") || line.contains("\"type\": \"user\"")) || line.contains("\"tool_result\"") {
            continue;
        }
        if ["\"isMeta\":true", "\"isMeta\": true", "\"isCompactSummary\":true", "\"isCompactSummary\": true"].iter().any(|m| line.contains(m)) {
            continue;
        }
        let Ok(obj) = xenonet_base::json::parse(line) else { continue };
        if let Some(p) = prompt_of(&obj, &mut command) {
            return p;
        }
    }
    command.unwrap_or_default()
}

// "Image" or "Document", for a session that began with one.
fn attached(head: &str) -> String {
    let line = head.split('\n').find(|l| l.contains("\"type\":\"user\"") && !l.contains("\"tool_result\"") && !l.contains("\"isMeta\":true"));
    match line {
        Some(l) if l.contains("\"type\":\"image\"") => "Image".into(),
        Some(l) if l.contains("\"type\":\"document\"") => "Document".into(),
        _ => String::new(),
    }
}

// ---------- continued in another session ----------

fn continued_in(tail: &str) -> Option<String> {
    if !tail.contains("\"type\":\"continued-in\"") {
        return None;
    }
    for line in tail.split('\n').rev() {
        if !(line.contains("\"type\":\"continued-in\"") || line.contains("\"type\":\"user\"") || line.contains("\"type\":\"assistant\"")) {
            continue;
        }
        let Ok(v) = xenonet_base::json::parse(line) else { continue };
        match v.get("type").and_then(Value::as_str) {
            Some("continued-in") => return v.get("continuedInSessionId").and_then(Value::as_str).filter(|i| is_uuid(i)).map(str::to_string),
            Some("assistant") if !truthy(v.get("isApiErrorMessage")) && v.get("message").and_then(|m| m.get("stop_reason")).is_some_and(Value::is_string) => return None,
            Some("user") if prompt_of(&v, &mut None).is_some() => return None,
            _ => {}
        }
    }
    None
}

// Whether the session it continued in has a conversation of its own.
fn continued_elsewhere(file: &Path, id: &str) -> bool {
    let target = file.with_file_name(format!("{id}.jsonl"));
    let Some(t) = head_tail(&target) else { return false };
    let mark = "\"parentUuid\":";
    if t.head.contains(mark) || t.tail.contains(mark) {
        return true;
    }
    if t.size <= CHUNK {
        return false;
    }
    let Ok(mut f) = open_plain(&target) else { return false };
    let mut at = 0u64;
    let mut carry = Vec::new();
    while at < t.size {
        if at >= SCAN_MAX {
            return true;
        }
        let Ok(chunk) = read_at(&mut f, at, 1 << 20) else { return false };
        if chunk.is_empty() {
            return false;
        }
        at += chunk.len() as u64;
        carry.extend_from_slice(&chunk);
        if carry.windows(mark.len()).any(|w| w == mark.as_bytes()) {
            return true;
        }
        let keep = carry.len().saturating_sub(mark.len());
        carry.drain(..keep);
    }
    false
}

// A name you gave a session in its sidecar, cleaned as the SDK cleans it.
fn sidecar_title(file: &Path, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(file.parent()?.join(id).join("custom-title.json")).ok()?;
    let v = xenonet_base::json::parse(&text).ok()?;
    let raw = v.get("customTitle")?.as_str()?;
    let format = |c: char| matches!(c as u32, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x180E | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0xE0001 | 0xE0020..=0xE007F);
    let mut spaced = String::new();
    let mut run = false;
    for c in js_trim(raw).chars() {
        if c.is_control() || format(c) || c == '\u{2028}' || c == '\u{2029}' {
            if !run {
                spaced.push(' ');
            }
            run = true;
        } else {
            spaced.push(c);
            run = false;
        }
    }
    let clean: String = spaced.chars().filter(|c| !matches!(*c as u32, 0..=0x1f | 0x7f..=0x9f)).take(200).collect();
    nonempty(Some(js_trim(&clean).to_string()))
}

// ---------- listSessions ----------

fn listing(id: &str, file: &Path, t: &HeadTail) -> Option<Value> {
    let first_line = t.head.split('\n').next().unwrap_or("");
    if first_line.contains("\"isSidechain\":true") || first_line.contains("\"isSidechain\": true") {
        return None;
    }
    let tail_title = last_value(&t.tail, "customTitle");
    let sidecar = if tail_title.is_none() { sidecar_title(file, id) } else { None };
    let custom = nonempty(tail_title)
        .or(sidecar)
        .or_else(|| nonempty(last_value(&t.head, "customTitle")))
        .or_else(|| nonempty(last_value(&t.tail, "aiTitle")))
        .or_else(|| nonempty(last_value(&t.head, "aiTitle")));
    let first = nonempty(Some(first_prompt(&t.head)));
    let created = first_value(&t.head, "timestamp").and_then(|s| xenonet_base::time::parse_iso(&s));
    let summary = custom
        .clone()
        .or_else(|| nonempty(last_value(&t.tail, "lastPrompt")))
        .or_else(|| nonempty(last_value(&t.tail, "summary")))
        .or_else(|| first.clone())
        .or_else(|| nonempty(Some(attached(&t.head))))?;
    let branch = nonempty(last_value(&t.tail, "gitBranch")).or_else(|| nonempty(first_value(&t.head, "gitBranch")));
    let cwd = nonempty(relocated(&t.tail)).or_else(|| nonempty(first_value(&t.head, "cwd")));
    let tag = t.tail.split('\n').rfind(|l| l.contains("\"type\":\"tag\"") && l.contains("\"tag\":\"")).and_then(|l| nonempty(last_value(l, "tag")));
    let mut m = Map::new();
    m.insert("sessionId".into(), json!(id));
    m.insert("summary".into(), json!(summary));
    m.insert("lastModified".into(), xenonet_base::json::number(t.mtime));
    m.insert("fileSize".into(), json!(t.size));
    let mut put = |k: &str, v: Option<Value>| {
        if let Some(v) = v {
            m.insert(k.into(), v);
        }
    };
    put("customTitle", custom.map(Value::String));
    put("firstPrompt", first.map(Value::String));
    put("gitBranch", branch.map(Value::String));
    put("cwd", cwd.map(Value::String));
    put("tag", tag.map(Value::String));
    put("createdAt", created.map(xenonet_base::json::number));
    Some(Value::Object(m))
}

/// listSessions(): every session's listing, newest first.
pub fn list_sessions(projects_dir: &str) -> Vec<Value> {
    let mut found: Vec<(String, f64, Value)> = Vec::new();
    for folder in sorted_names(Path::new(projects_dir)) {
        if !folder.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for f in sorted_names(&folder.path()) {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".jsonl").filter(|s| is_uuid(s)) else { continue };
            let file = f.path();
            let Some(t) = head_tail(&file) else { continue };
            if let Some(other) = continued_in(&t.tail) {
                if continued_elsewhere(&file, &other) {
                    continue;
                }
            }
            let Some(l) = listing(id, &file, &t) else { continue };
            match found.iter_mut().find(|(i, _, _)| i == id) {
                Some(slot) if t.mtime > slot.1 => *slot = (id.to_string(), t.mtime, l),
                Some(_) => {}
                None => found.push((id.to_string(), t.mtime, l)),
            }
        }
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    found.into_iter().map(|(_, _, l)| l).collect()
}

// ---------- getSessionMessages ----------

fn find_file(projects_dir: &str, id: &str) -> Option<std::path::PathBuf> {
    sorted_names(Path::new(projects_dir)).into_iter().map(|d| d.path().join(format!("{id}.jsonl"))).find(|f| std::fs::metadata(f).is_ok_and(|m| m.len() > 0))
}

fn uuid(e: &Value) -> Option<&str> {
    e.get("uuid").and_then(Value::as_str)
}

fn parent(e: &Value) -> Option<&str> {
    e.get("parentUuid").and_then(Value::as_str)
}

fn kind(e: &Value) -> &str {
    e.get("type").and_then(Value::as_str).unwrap_or("")
}

fn is_message(e: &Value) -> bool {
    matches!(kind(e), "user" | "assistant")
}

fn on_main(e: &Value) -> bool {
    let briefing = kind(e) == "attachment" && e.get("attachment").and_then(|a| a.get("type")).and_then(Value::as_str) == Some("fork_briefing");
    !(truthy(e.get("isSidechain")) || truthy(e.get("teamName")) || kind(e) == "progress" || briefing)
}

fn tool_results(e: &Value) -> Vec<String> {
    match e.get("message").and_then(|m| m.get("content")) {
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            .filter_map(|b| b.get("tool_use_id").and_then(Value::as_str).map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn is_tool_result(e: &Value) -> bool {
    kind(e) == "user"
        && parent(e).is_some()
        && matches!(e.get("message").and_then(|m| m.get("content")), Some(Value::Array(b)) if b.iter().any(|x| x.get("type").and_then(Value::as_str) == Some("tool_result")))
}

fn msg_id(e: &Value) -> Option<&str> {
    (kind(e) == "assistant").then(|| e.get("message").and_then(|m| m.get("id")).and_then(Value::as_str)).flatten()
}

fn tool_uses(e: &Value) -> Vec<String> {
    match e.get("message").and_then(|m| m.get("content")) {
        Some(Value::Array(blocks)) => {
            blocks.iter().filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use")).filter_map(|b| b.get("id").and_then(Value::as_str).map(str::to_string)).collect()
        }
        _ => Vec::new(),
    }
}

// The chain from the last message back to the first, root first, with
// what ran beside it.
fn chain(entries: &mut [Value]) -> Vec<usize> {
    let mut by: HashMap<String, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(u) = uuid(e) {
            by.insert(u.to_string(), i);
        }
    }
    // Compactions that kept messages: chained back on to what they follow.
    let relinks: Vec<(String, Value)> = entries
        .iter()
        .filter(|e| kind(e) == "system" && e.get("subtype").and_then(Value::as_str) == Some("compact_boundary"))
        .filter_map(|e| e.get("compactMetadata").cloned().map(|m| (String::new(), m)))
        .collect();
    for (_, meta) in relinks {
        if let Some(pm) = meta.get("preservedMessages") {
            let anchor = pm.get("anchorUuid").and_then(Value::as_str).map(str::to_string);
            let list: Vec<String> = pm.get("uuids").and_then(Value::as_array).map(|a| a.iter().filter_map(|u| u.as_str().map(str::to_string)).collect()).unwrap_or_default();
            if let (Some(anchor), false) = (anchor, list.is_empty()) {
                if list.iter().all(|u| by.contains_key(u)) {
                    let mut prev = anchor.clone();
                    for u in &list {
                        entries[by[u]]["parentUuid"] = json!(prev);
                        prev.clone_from(u);
                    }
                    let first = list[0].clone();
                    for e in entries.iter_mut() {
                        if parent(e) == Some(anchor.as_str()) && uuid(e) != Some(first.as_str()) {
                            e["parentUuid"] = json!(prev);
                        }
                    }
                    continue;
                }
            }
        }
        if let Some(seg) = meta.get("preservedSegment") {
            let get = |k: &str| seg.get(k).and_then(Value::as_str).map(str::to_string);
            if let (Some(head), Some(anchor), Some(tail)) = (get("headUuid"), get("anchorUuid"), get("tailUuid")) {
                if let Some(&h) = by.get(&head) {
                    entries[h]["parentUuid"] = json!(anchor);
                }
                for e in entries.iter_mut() {
                    if parent(e) == Some(anchor.as_str()) && uuid(e) != Some(head.as_str()) {
                        e["parentUuid"] = json!(tail);
                    }
                }
            }
        }
    }
    let up_to_message = |from: usize, seen: &mut HashSet<usize>| -> Option<usize> {
        let mut cur = Some(from);
        while let Some(i) = cur {
            if !seen.insert(i) {
                return None;
            }
            if is_message(&entries[i]) {
                return Some(i);
            }
            cur = parent(&entries[i]).and_then(|p| by.get(p).copied());
        }
        None
    };
    let named: HashSet<&str> = entries.iter().filter(|e| on_main(e)).filter_map(parent).collect();
    let mut leaves: Vec<usize> = (0..entries.len()).filter(|&i| on_main(&entries[i]) && uuid(&entries[i]).is_some_and(|u| !named.contains(u))).collect();
    leaves.sort_unstable_by(|a, b| b.cmp(a));
    let mut seen = HashSet::new();
    let mut leaf = leaves.into_iter().find_map(|l| up_to_message(l, &mut seen));
    if leaf.is_none() {
        let parents: HashSet<&str> = entries.iter().filter_map(parent).collect();
        let mut best: Option<(bool, usize)> = None;
        for i in (0..entries.len()).filter(|&i| uuid(&entries[i]).is_some_and(|u| !parents.contains(u))) {
            if let Some(m) = up_to_message(i, &mut HashSet::new()) {
                let e = &entries[m];
                let plain = !truthy(e.get("isSidechain")) && !truthy(e.get("teamName")) && !truthy(e.get("isMeta"));
                if best.is_none_or(|(p, j)| (plain, m) > (p, j)) {
                    best = Some((plain, m));
                }
            }
        }
        leaf = best.map(|(_, m)| m);
    }
    let Some(leaf) = leaf else { return Vec::new() };
    let mut out = Vec::new();
    let mut walked = HashSet::new();
    let mut cur = Some(leaf);
    while let Some(i) = cur {
        if !walked.insert(i) {
            break;
        }
        out.push(i);
        cur = parent(&entries[i]).and_then(|p| by.get(p).copied());
    }
    out.reverse();
    // What ran beside the chain: a reply's other parts, and the results of
    // its tool calls, after the last of its parts.
    let in_chain: HashSet<usize> = out.iter().copied().collect();
    let mut owner: HashMap<String, Option<String>> = HashMap::new();
    for e in entries.iter() {
        if let Some(m) = msg_id(e) {
            for t in tool_uses(e) {
                let slot = owner.entry(t).or_insert_with(|| Some(m.to_string()));
                if slot.as_deref() != Some(m) {
                    *slot = None;
                }
            }
        }
    }
    let mut ids: Vec<String> = Vec::new();
    for &i in &out {
        if let Some(m) = msg_id(&entries[i]) {
            if !ids.iter().any(|x| x == m) {
                ids.push(m.to_string());
            }
        }
    }
    let mut added: HashSet<usize> = HashSet::new();
    for m in ids {
        let group: Vec<usize> = (0..entries.len()).filter(|&i| msg_id(&entries[i]) == Some(m.as_str())).collect();
        let group_uuids: HashSet<&str> = group.iter().filter_map(|&i| uuid(&entries[i])).collect();
        let mut missing: Vec<usize> = group.iter().copied().filter(|i| !in_chain.contains(i) && !added.contains(i)).collect();
        let mut direct: Vec<usize> = (0..entries.len())
            .filter(|&i| is_tool_result(&entries[i]) && !in_chain.contains(&i) && !added.contains(&i))
            .filter(|&i| {
                parent(&entries[i]).is_some_and(|p| group_uuids.contains(p))
                    || tool_results(&entries[i]).iter().any(|t| owner.get(t).is_some_and(|o| o.as_deref() == Some(m.as_str())))
            })
            .collect();
        let ts = |i: &usize| entries[*i].get("timestamp").and_then(Value::as_str).unwrap_or("").to_string();
        missing.sort_by_key(ts);
        direct.sort_by_key(ts);
        if missing.is_empty() && direct.is_empty() {
            continue;
        }
        let Some(last) = out.iter().rposition(|&i| msg_id(&entries[i]) == Some(m.as_str())) else { continue };
        let insert: Vec<usize> = missing.into_iter().chain(direct).collect();
        added.extend(insert.iter().copied());
        out.splice((last + 1)..=last, insert);
    }
    out
}

// A queued command: the prompt typed while a turn ran, as the message it
// became.
fn queued_message(e: &Value) -> Option<Value> {
    let a = e.get("attachment")?;
    if a.get("type").and_then(Value::as_str) != Some("queued_command") {
        return None;
    }
    let prompt = a.get("prompt").filter(|p| p.is_string() || p.is_array())?;
    let special = |o: Option<&Value>| {
        o.and_then(|o| o.get("kind")).and_then(Value::as_str).is_some_and(|k| matches!(k, "channel" | "observer" | "observer-activity" | "slack-ping" | "peer"))
    };
    if truthy(a.get("isMeta")) && !special(a.get("origin")) {
        return None;
    }
    let origin = if a.get("commandMode").and_then(Value::as_str) == Some("task-notification") { Some(json!({ "kind": "task-notification" })) } else { a.get("origin").cloned() };
    let mut m = Map::new();
    m.insert("type".into(), json!("user"));
    m.insert("uuid".into(), a.get("source_uuid").cloned().filter(Value::is_string).or_else(|| e.get("uuid").cloned()).unwrap_or(Value::Null));
    m.insert("parentUuid".into(), e.get("parentUuid").cloned().unwrap_or(Value::Null));
    m.insert("sessionId".into(), e.get("sessionId").cloned().unwrap_or(Value::Null));
    m.insert("timestamp".into(), e.get("timestamp").cloned().unwrap_or(Value::Null));
    m.insert("message".into(), json!({ "role": "user", "content": prompt }));
    m.insert("isMeta".into(), a.get("isMeta").cloned().unwrap_or(Value::Null));
    if let Some(o) = origin {
        m.insert("origin".into(), o);
    }
    m.insert("isQueuedCommand".into(), json!(true));
    m.insert("isSidechain".into(), e.get("isSidechain").cloned().unwrap_or(Value::Null));
    m.insert("teamName".into(), e.get("teamName").cloned().unwrap_or(Value::Null));
    Some(Value::Object(m))
}

// The text a user line ends with, for what local commands printed.
fn last_text(e: &Value) -> String {
    match e.get("message").and_then(|m| m.get("content")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(b)) => {
            b.iter().rev().find(|x| x.get("type").and_then(Value::as_str) == Some("text")).and_then(|x| x.get("text")).and_then(Value::as_str).unwrap_or("").to_string()
        }
        _ => String::new(),
    }
}

/// getSessionMessages(id): a session's messages, root first, as the SDK
/// gives them; none for an id that isn't one or a session not found.
pub fn session_messages(projects_dir: &str, id: &str) -> Vec<Value> {
    if !is_uuid(id) {
        return Vec::new();
    }
    let Some(file) = find_file(projects_dir, id) else { return Vec::new() };
    let Ok(data) = std::fs::read(&file) else { return Vec::new() };
    let mut entries: Vec<Value> = data
        .split(|b| *b == b'\n')
        .filter_map(|l| {
            let start = l.iter().position(|b| *b > 0x20)?;
            xenonet_base::json::parse(&String::from_utf8_lossy(&l[start..])).ok()
        })
        .filter(|e| matches!(kind(e), "user" | "assistant" | "progress" | "system" | "attachment") && uuid(e).is_some())
        .collect();
    let order = chain(&mut entries);
    let mut list: Vec<Value> = order.iter().map(|&i| entries[i].clone()).collect();
    // A local command's records, after its caveat, until the next reply.
    let mut marking = false;
    for e in &mut list {
        if kind(e) == "assistant" {
            marking = false;
            continue;
        }
        if kind(e) != "user" {
            continue;
        }
        if truthy(e.get("isMeta")) {
            if last_text(e).starts_with("<local-command-caveat>") {
                marking = true;
            }
            continue;
        }
        let t = last_text(e);
        if marking && e.get("promptSource").is_none() && (t.contains("<command-name>") || t.contains("<local-command-stdout>") || t.contains("<local-command-stderr>")) {
            e["isCompletedLocalCommand"] = json!(true);
        }
    }
    // Queued commands a reply took up become the messages they were.
    let n = list.len();
    for i in 0..n {
        if kind(&list[i]) != "attachment" {
            continue;
        }
        let consumed =
            list[i + 1..].iter().find(|x| is_message(x)).is_some_and(|x| kind(x) == "assistant" || is_tool_result(x) || last_text(x).starts_with("[Request interrupted by user"));
        if consumed || i + 1 == n {
            if let Some(q) = queued_message(&list[i]) {
                list[i] = q;
            }
        }
    }
    let special = |o: Option<&Value>| {
        o.and_then(|o| o.get("kind")).and_then(Value::as_str).is_some_and(|k| matches!(k, "channel" | "observer" | "observer-activity" | "slack-ping" | "peer"))
    };
    list.into_iter()
        .filter(|e| is_message(e) && (!truthy(e.get("isMeta")) || special(e.get("origin"))) && !truthy(e.get("isSidechain")) && !truthy(e.get("teamName")))
        .map(|e| {
            let mut m = Map::new();
            m.insert("type".into(), e.get("type").cloned().unwrap_or(Value::Null));
            m.insert("uuid".into(), e.get("uuid").cloned().unwrap_or(Value::Null));
            m.insert("session_id".into(), e.get("sessionId").cloned().unwrap_or(Value::Null));
            m.insert("message".into(), e.get("message").cloned().unwrap_or(Value::Null));
            m.insert("parent_tool_use_id".into(), Value::Null);
            m.insert("parent_agent_id".into(), Value::Null);
            let on = |k: &str| e.get(k) == Some(&Value::Bool(true));
            if on("interruptedByShutdown") {
                m.insert("interruptedByShutdown".into(), json!(true));
            }
            if e.get("toolDenialUnanswered").and_then(Value::as_str) == Some("stream-closed") {
                m.insert("toolDenialUnanswered".into(), json!("stream-closed"));
            }
            if on("isCompactSummary") {
                m.insert("isCompactSummary".into(), json!(true));
            }
            if on("isMeta") || on("isCompactSummary") || on("isVisibleInTranscriptOnly") {
                m.insert("is_meta".into(), json!(true));
            }
            if on("isQueuedCommand") {
                m.insert("isQueuedCommand".into(), json!(true));
            }
            if on("isCompletedLocalCommand") {
                m.insert("isCompletedLocalCommand".into(), json!(true));
            }
            if let Some(t) = e.get("timestamp").filter(|t| !t.is_null()) {
                m.insert("timestamp".into(), t.clone());
            }
            if let Some(o) = e.get("origin").filter(|o| truthy(Some(o))) {
                let o = if o.get("kind").and_then(Value::as_str) == Some("task-notification") {
                    Value::Object(["kind", "subkind", "fireReason", "producer"].iter().filter_map(|k| o.get(*k).map(|v| (k.to_string(), v.clone()))).collect())
                } else {
                    o.clone()
                };
                m.insert("origin".into(), o);
            }
            Value::Object(m)
        })
        .collect()
}

// ---------- renameSession ----------

/// renameSession(id, title): a custom-title line appended to the session's
/// transcript, the first one found.
pub fn rename_session(projects_dir: &str, id: &str, title: &str) -> Result<(), String> {
    use std::io::Write;
    if !is_uuid(id) {
        return Err(format!("Invalid sessionId: {id}"));
    }
    let title = js_trim(title);
    if title.is_empty() {
        return Err("title must be non-empty".into());
    }
    let line = format!("{}\n", xenonet_base::json::stringify(&json!({ "type": "custom-title", "customTitle": title, "sessionId": id }), 0));
    let folders = xenonet_base::paths::read_dir_sorted(Path::new(projects_dir)).map_err(|_| format!("Session {id} not found (no projects directory)"))?;
    for d in folders {
        let file = d.path().join(format!("{id}.jsonl"));
        let mut f = match std::fs::OpenOptions::new().append(true).open(&file) {
            Ok(f) => f,
            Err(e) if matches!(xenonet_base::fs::code_of(&e), Some("ENOENT" | "ENOTDIR")) => continue,
            Err(e) => return Err(e.to_string()),
        };
        if f.metadata().map_err(|e| e.to_string())?.len() == 0 {
            continue;
        }
        return f.write_all(line.as_bytes()).map_err(|e| e.to_string());
    }
    Err(format!("Session {id} not found in any project directory"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "0190d3e0-0000-7000-8000-000000000001";
    const B: &str = "0190d3e0-0000-7000-8000-000000000002";

    fn write(dir: &Path, id: &str, lines: &[Value]) {
        std::fs::create_dir_all(dir).expect("a folder");
        let text: String = lines.iter().map(|l| xenonet_base::json::stringify(l, 0) + "\n").collect();
        std::fs::write(dir.join(format!("{id}.jsonl")), text).expect("a transcript");
    }

    #[test]
    fn lists_sessions_as_the_sdk_does() {
        let root = std::env::temp_dir().join(format!("xenonet-wire-sdk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = root.join("-Users-nova-Projects-atlas");
        write(
            &p,
            A,
            &[
                json!({ "type": "user", "uuid": "u1", "parentUuid": null, "sessionId": A, "cwd": "/Users/nova/Projects/atlas", "gitBranch": "main", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": "<command-name>/init</command-name>" } }),
                json!({ "type": "user", "uuid": "u2", "parentUuid": "u1", "sessionId": A, "timestamp": "2026-10-17T10:00:01.000Z", "message": { "role": "user", "content": "Add partial\nrefunds" } }),
                json!({ "type": "assistant", "uuid": "a1", "parentUuid": "u2", "sessionId": A, "timestamp": "2026-10-17T10:00:02.000Z", "message": { "id": "m1", "role": "assistant", "content": [{ "type": "tool_use", "id": "t1", "name": "Bash", "input": {} }] } }),
                json!({ "type": "user", "uuid": "u3", "parentUuid": "a1", "sessionId": A, "timestamp": "2026-10-17T10:00:03.000Z", "message": { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "t1", "content": "ok" }] } }),
                json!({ "type": "assistant", "uuid": "a2", "parentUuid": "u3", "sessionId": A, "timestamp": "2026-10-17T10:00:04.000Z", "message": { "id": "m2", "role": "assistant", "content": [{ "type": "text", "text": "Done" }] } }),
            ],
        );
        write(&p, B, &[json!({ "type": "user", "isSidechain": true, "uuid": "x", "message": { "content": "side" } })]);
        let list = list_sessions(&root.to_string_lossy());
        assert_eq!(list.len(), 1, "a sidechain's session isn't listed");
        let s = &list[0];
        assert_eq!((s["sessionId"].as_str(), s["summary"].as_str(), s["firstPrompt"].as_str()), (Some(A), Some("Add partial refunds"), Some("Add partial refunds")));
        assert_eq!((s["cwd"].as_str(), s["gitBranch"].as_str()), (Some("/Users/nova/Projects/atlas"), Some("main")));
        assert!(s.get("customTitle").is_none());
        assert!(s["createdAt"].as_f64().is_some());
        rename_session(&root.to_string_lossy(), A, "  Refunds  ").expect("renamed");
        let list = list_sessions(&root.to_string_lossy());
        assert_eq!((list[0]["customTitle"].as_str(), list[0]["summary"].as_str()), (Some("Refunds"), Some("Refunds")));
        assert!(rename_session(&root.to_string_lossy(), "nope", "x").is_err());
        let ms = session_messages(&root.to_string_lossy(), A);
        let uuids: Vec<&str> = ms.iter().filter_map(|m| m["uuid"].as_str()).collect();
        assert_eq!(uuids, ["u1", "u2", "a1", "u3", "a2"]);
        assert_eq!(ms[0]["session_id"], A);
        assert!(session_messages(&root.to_string_lossy(), B).is_empty(), "a sidechain's messages aren't the session's");
        std::fs::remove_dir_all(&root).expect("cleaned up");
    }

    #[test]
    fn values_found_in_the_text() {
        let t = r#"{"customTitle":"a"} {"customTitle": "b\"c"} {"customTitle":"open"#;
        assert_eq!(last_value(t, "customTitle").as_deref(), Some("b\"c"));
        assert_eq!(first_value(t, "customTitle").as_deref(), Some("a"));
        assert_eq!(cut_200(&"x".repeat(250)).chars().count(), 201);
        assert!(is_markup("<system-reminder>hi"));
        assert!(!is_markup("Fix <b> tags"));
        assert_eq!(unwrap_pastes("see\n<pasted_content id=\"a1b2\">\nbody\n</pasted_content id=\"a1b2\">\nend"), "seebodyend");
    }
}
