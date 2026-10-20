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
use std::path::{Path, PathBuf};
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
    stamp: Stamp,
    head: String,
    tail: String,
}

/// A file as it stood: its size, when it was last written and which file
/// it is. A transcript that still has the same one hasn't changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    ino: u64,
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Stamp {
        #[cfg(unix)]
        let ino = std::os::unix::fs::MetadataExt::ino(meta);
        #[cfg(not(unix))]
        let ino = 0;
        Stamp { len: meta.len(), modified: meta.modified().ok(), ino }
    }
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
    Some(HeadTail { mtime, size, stamp: Stamp::of(&meta), head: head_text, tail })
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

// How much of the text, back from the end, last_value looks at first.
const STRETCH: usize = 8192;

/// The last `"key":"…"` (or `"key": "…"`) in the text that closes. It's
/// looked for a stretch of whole lines at a time from the end, where it
/// usually is: a key never spans a line break, so the matches in a
/// stretch that starts a line are those a search of the whole text finds
/// there.
fn last_value(text: &str, key: &str) -> Option<String> {
    last_value_by(text, key, STRETCH)
}

fn last_value_by(text: &str, key: &str, stretch: usize) -> Option<String> {
    let (tight, spaced) = (format!("\"{key}\":\""), format!("\"{key}\": \""));
    let mut end = text.len();
    while end > 0 {
        let from = text.as_bytes()[..end.saturating_sub(stretch)].iter().rposition(|&b| b == b'\n').map_or(0, |n| n + 1);
        let part = &text[from..end];
        let mut at: Vec<usize> = starts(part, &tight).into_iter().chain(starts(part, &spaced)).map(|i| from + i).collect();
        at.sort_unstable();
        if let Some(v) = at.into_iter().rev().find_map(|i| value_at(text, i)) {
            return Some(v);
        }
        end = from;
    }
    None
}

/// The first `"key":"…"`, or, only when there's none of those, the first
/// `"key": "…"`.
fn first_value(text: &str, key: &str) -> Option<String> {
    let first = |pat: String| text.find(&pat).map(|i| i + pat.len());
    first(format!("\"{key}\":\"")).or_else(|| first(format!("\"{key}\": \""))).and_then(|i| value_at(text, i))
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

/// What a listing takes from a transcript's head and tail: all of it but
/// the sidecar's name and whether a session it continued in has begun,
/// which are other files. Kept from one listing to the next while the
/// transcript stays as it was, so an unchanged one isn't read again.
#[derive(Clone, Debug)]
struct Seen {
    stamp: Stamp,
    mtime: f64,
    size: u64,
    /// The session this one continued in, by its tail.
    continued: Option<String>,
    /// A subagent's transcript, which isn't a session of its own.
    sidechain: bool,
    /// The name in the tail, even an empty one: only when there's none at
    /// all is the sidecar's looked at.
    tail_title: Option<String>,
    /// The name when neither the tail nor the sidecar has one.
    later_title: Option<String>,
    first: Option<String>,
    created: Option<f64>,
    /// What it's called without a name.
    later_summary: Option<String>,
    branch: Option<String>,
    cwd: Option<String>,
    tag: Option<String>,
}

impl Seen {
    fn of(t: &HeadTail) -> Seen {
        let first_line = t.head.split('\n').next().unwrap_or("");
        let sidechain = first_line.contains("\"isSidechain\":true") || first_line.contains("\"isSidechain\": true");
        let first = nonempty(Some(first_prompt(&t.head)));
        Seen {
            stamp: t.stamp,
            mtime: t.mtime,
            size: t.size,
            continued: continued_in(&t.tail),
            sidechain,
            tail_title: last_value(&t.tail, "customTitle"),
            later_title: nonempty(last_value(&t.head, "customTitle")).or_else(|| nonempty(last_value(&t.tail, "aiTitle"))).or_else(|| nonempty(last_value(&t.head, "aiTitle"))),
            created: first_value(&t.head, "timestamp").and_then(|s| xenonet_base::time::parse_iso(&s)),
            later_summary: nonempty(last_value(&t.tail, "lastPrompt"))
                .or_else(|| nonempty(last_value(&t.tail, "summary")))
                .or_else(|| first.clone())
                .or_else(|| nonempty(Some(attached(&t.head)))),
            first,
            branch: nonempty(last_value(&t.tail, "gitBranch")).or_else(|| nonempty(first_value(&t.head, "gitBranch"))),
            cwd: nonempty(relocated(&t.tail)).or_else(|| nonempty(first_value(&t.head, "cwd"))),
            tag: t.tail.split('\n').rfind(|l| l.contains("\"type\":\"tag\"") && l.contains("\"tag\":\"")).and_then(|l| nonempty(last_value(l, "tag"))),
        }
    }
}

fn listing(id: &str, file: &Path, t: &Seen) -> Option<Value> {
    if t.sidechain {
        return None;
    }
    let sidecar = if t.tail_title.is_none() { sidecar_title(file, id) } else { None };
    let custom = nonempty(t.tail_title.clone()).or(sidecar).or_else(|| t.later_title.clone());
    let summary = custom.clone().or_else(|| t.later_summary.clone())?;
    let (first, created, branch, cwd, tag) = (t.first.clone(), t.created, t.branch.clone(), t.cwd.clone(), t.tag.clone());
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

/// What listings read of each transcript, by its path, kept from one
/// listing to the next.
#[derive(Default)]
pub struct Listings {
    seen: HashMap<PathBuf, Seen>,
}

// How many transcripts are read at once when many are new or changed, as
// they all are at the start; fewer than READ_TOGETHER are read in turn.
const READERS: usize = 4;
const READ_TOGETHER: usize = 64;

fn read_one(file: &Path) -> Option<(PathBuf, Seen)> {
    head_tail(file).map(|t| (file.to_path_buf(), Seen::of(&t)))
}

fn read_all(files: &[PathBuf]) -> Vec<(PathBuf, Seen)> {
    let readers = std::thread::available_parallelism().map_or(1, std::num::NonZero::get).min(READERS);
    if files.len() < READ_TOGETHER || readers < 2 {
        return files.iter().filter_map(|f| read_one(f)).collect();
    }
    std::thread::scope(|s| {
        let parts: Vec<_> = files.chunks(files.len().div_ceil(readers)).map(|part| s.spawn(move || part.iter().filter_map(|f| read_one(f)).collect::<Vec<_>>())).collect();
        parts.into_iter().flat_map(|p| p.join().unwrap_or_default()).collect()
    })
}

/// listSessions(): every session's listing, newest first.
pub fn list_sessions(projects_dir: &str) -> Vec<Value> {
    list_sessions_with(projects_dir, &mut Listings::default())
}

/// listSessions(), with what earlier listings read: a transcript that
/// hasn't changed since isn't read again. Only its sidecar's name, and
/// whether a session it continued in has begun, are looked at each time.
pub fn list_sessions_with(projects_dir: &str, listings: &mut Listings) -> Vec<Value> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for folder in sorted_names(Path::new(projects_dir)) {
        if !folder.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for f in sorted_names(&folder.path()) {
            let name = f.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".jsonl").filter(|s| is_uuid(s)) else { continue };
            files.push((id.to_string(), f.path()));
        }
    }
    let mut seen: HashMap<PathBuf, Seen> = HashMap::with_capacity(files.len());
    let mut stale = Vec::new();
    for (_, file) in &files {
        let now = std::fs::symlink_metadata(file).ok().filter(|m| m.file_type().is_file()).map(|m| Stamp::of(&m));
        match listings.seen.remove(file) {
            Some(s) if Some(s.stamp) == now => {
                seen.insert(file.clone(), s);
            }
            _ => stale.push(file.clone()),
        }
    }
    seen.extend(read_all(&stale));
    let mut found: Vec<(String, f64, Value)> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for (id, file) in &files {
        let Some(t) = seen.get(file) else { continue };
        if let Some(other) = &t.continued {
            if continued_elsewhere(file, other) {
                continue;
            }
        }
        let Some(l) = listing(id, file, t) else { continue };
        match at.get(id) {
            Some(&k) => {
                if t.mtime > found[k].1 {
                    found[k] = (id.clone(), t.mtime, l);
                }
            }
            None => {
                at.insert(id.clone(), found.len());
                found.push((id.clone(), t.mtime, l));
            }
        }
    }
    listings.seen = seen;
    found.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    found.into_iter().map(|(_, _, l)| l).collect()
}

// ---------- getSessionMessages ----------

fn find_file(projects_dir: &str, id: &str) -> Option<PathBuf> {
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
    beside(entries, out)
}

// What ran beside the chain: a reply's other parts, and the results of its
// tool calls, after the last of its parts. Each reply's parts and results
// are found once, by index, so a long transcript costs no more than a
// short one does per line.
fn beside(entries: &[Value], out: Vec<usize>) -> Vec<usize> {
    let in_chain: HashSet<usize> = out.iter().copied().collect();
    // Whose each tool call is: the reply that made it, unless more than one
    // reply says it made it.
    let mut owner: HashMap<String, Option<&str>> = HashMap::new();
    // Each reply's parts, in the file's order, and the replies each uuid is
    // a part of.
    let mut parts: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut replies_of: HashMap<&str, Vec<&str>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if let Some(m) = msg_id(e) {
            for t in tool_uses(e) {
                let slot = owner.entry(t).or_insert(Some(m));
                if *slot != Some(m) {
                    *slot = None;
                }
            }
            parts.entry(m).or_default().push(i);
            if let Some(u) = uuid(e) {
                let of = replies_of.entry(u).or_default();
                if !of.contains(&m) {
                    of.push(m);
                }
            }
        }
    }
    // Each reply's tool results off the chain, in the file's order: those
    // that follow one of its parts, or answer a call only it made.
    let mut results: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if in_chain.contains(&i) || !is_tool_result(e) {
            continue;
        }
        let mut of: Vec<&str> = parent(e).and_then(|p| replies_of.get(p)).cloned().unwrap_or_default();
        for t in tool_results(e) {
            if let Some(Some(m)) = owner.get(&t) {
                if !of.contains(m) {
                    of.push(m);
                }
            }
        }
        for m in of {
            results.entry(m).or_default().push(i);
        }
    }
    // The replies on the chain, each once, in its order, and where each
    // last appears on it.
    let mut ids: Vec<&str> = Vec::new();
    let mut last_at: HashMap<&str, usize> = HashMap::new();
    for (at, &i) in out.iter().enumerate() {
        if let Some(m) = msg_id(&entries[i]) {
            if last_at.insert(m, at).is_none() {
                ids.push(m);
            }
        }
    }
    let ts = |i: &usize| entries[*i].get("timestamp").and_then(Value::as_str).unwrap_or("");
    let mut added: HashSet<usize> = HashSet::new();
    // What goes after each place on the chain. What's put in is a reply's
    // own parts and tool results, never another reply's part, so where a
    // reply last appears is where it did on the chain.
    let mut after: HashMap<usize, Vec<usize>> = HashMap::new();
    for m in ids {
        let mut missing: Vec<usize> = parts.get(m).map_or(&[][..], Vec::as_slice).iter().copied().filter(|i| !in_chain.contains(i) && !added.contains(i)).collect();
        let mut direct: Vec<usize> = results.get(m).map_or(&[][..], Vec::as_slice).iter().copied().filter(|i| !added.contains(i)).collect();
        missing.sort_by(|a, b| ts(a).cmp(ts(b)));
        direct.sort_by(|a, b| ts(a).cmp(ts(b)));
        if missing.is_empty() && direct.is_empty() {
            continue;
        }
        let Some(&last) = last_at.get(m) else { continue };
        let insert: Vec<usize> = missing.into_iter().chain(direct).collect();
        added.extend(insert.iter().copied());
        after.insert(last, insert);
    }
    if after.is_empty() {
        return out;
    }
    let mut full = Vec::with_capacity(out.len() + added.len());
    for (at, i) in out.into_iter().enumerate() {
        full.push(i);
        if let Some(insert) = after.remove(&at) {
            full.extend(insert);
        }
    }
    full
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

// A line's entry, if it's JSON.
fn parse_line(l: &[u8]) -> Option<Value> {
    let start = l.iter().position(|b| *b > 0x20)?;
    xenonet_base::json::parse(&String::from_utf8_lossy(&l[start..])).ok()
}

// The fields of an entry that finding its session's messages reads, with
// its message cut to what's read of that: its id, and of each block its
// type, a call's id, the call a result answers and, in what you said, the
// text. The rest (tool results, file contents, thinking) stays in the
// line until the message is shown. A field read below that isn't kept
// here reads as missing, so a field newly read is added here too.
fn slim(e: Value) -> Value {
    let Value::Object(mut all) = e else { return e };
    let user = all.get("type").and_then(Value::as_str) == Some("user");
    let mut m = Map::new();
    for k in [
        "type",
        "uuid",
        "parentUuid",
        "isSidechain",
        "teamName",
        "subtype",
        "compactMetadata",
        "isMeta",
        "promptSource",
        "sessionId",
        "timestamp",
        "interruptedByShutdown",
        "toolDenialUnanswered",
        "isCompactSummary",
        "isVisibleInTranscriptOnly",
        "isQueuedCommand",
        "isCompletedLocalCommand",
        "origin",
    ] {
        if let Some(v) = all.shift_remove(k) {
            m.insert(k.into(), v);
        }
    }
    // A queued command's attachment becomes the message, so it's kept
    // whole; of any other, only its type is read.
    if let Some(a) = all.shift_remove("attachment") {
        let queued = a.get("type").and_then(Value::as_str) == Some("queued_command");
        let a = match a {
            Value::Object(o) if !queued => Value::Object(o.into_iter().filter(|(k, _)| k == "type").collect()),
            a => a,
        };
        m.insert("attachment".into(), a);
    }
    if let Some(message) = all.shift_remove("message") {
        let message = match message {
            Value::Object(mut msg) => {
                let mut cut = Map::new();
                if let Some(id) = msg.shift_remove("id") {
                    cut.insert("id".into(), id);
                }
                if let Some(content) = msg.shift_remove("content") {
                    let content = match content {
                        Value::Array(blocks) => Value::Array(
                            blocks
                                .into_iter()
                                .map(|b| match b {
                                    Value::Object(mut b) => {
                                        let text = user && b.get("type").and_then(Value::as_str) == Some("text");
                                        let mut kept = Map::new();
                                        for k in ["type", "id", "tool_use_id"] {
                                            if let Some(v) = b.shift_remove(k) {
                                                kept.insert(k.into(), v);
                                            }
                                        }
                                        if let Some(t) = b.shift_remove("text").filter(|_| text) {
                                            kept.insert("text".into(), t);
                                        }
                                        Value::Object(kept)
                                    }
                                    b => b,
                                })
                                .collect(),
                        ),
                        c => c,
                    };
                    cut.insert("content".into(), content);
                }
                Value::Object(cut)
            }
            message => message,
        };
        m.insert("message".into(), message);
    }
    Value::Object(m)
}

/// A session's messages, root first, as getSessionMessages gives them,
/// each one's message left in its line in the transcript until it's
/// taken: a long session's messages, all held at once, would take several
/// times their size in the file.
pub struct Messages {
    data: Vec<u8>,
    /// Each message, with where its line is, or none when it was made here
    /// (a queued command), whole.
    list: Vec<(Value, Line)>,
}

impl Messages {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The messages from `start` to `end`, whole.
    pub fn page(&self, start: usize, end: usize) -> Vec<Value> {
        self.list[start..end]
            .iter()
            .map(|(m, line)| {
                let mut m = m.clone();
                if let Some(line) = line {
                    m["message"] = parse_line(&self.data[line.clone()]).and_then(|mut e| e.get_mut("message").map(Value::take)).unwrap_or(Value::Null);
                }
                m
            })
            .collect()
    }

    pub fn all(&self) -> Vec<Value> {
        self.page(0, self.len())
    }
}

/// getSessionMessages(id): a session's messages, root first, as the SDK
/// gives them; none for an id that isn't one or a session not found.
pub fn session_messages(projects_dir: &str, id: &str) -> Vec<Value> {
    messages(projects_dir, id).all()
}

/// The same, each message read whole only when it's taken.
pub fn messages(projects_dir: &str, id: &str) -> Messages {
    let none = || Messages { data: Vec::new(), list: Vec::new() };
    if !is_uuid(id) {
        return none();
    }
    let Some(file) = find_file(projects_dir, id) else { return none() };
    let Ok(data) = std::fs::read(&file) else { return none() };
    let mut entries: Vec<Value> = Vec::new();
    let mut lines: Vec<std::ops::Range<usize>> = Vec::new();
    let mut at = 0;
    for l in data.split(|b| *b == b'\n') {
        let range = at..at + l.len();
        at = range.end + 1;
        let Some(e) = parse_line(l) else { continue };
        if matches!(kind(&e), "user" | "assistant" | "progress" | "system" | "attachment") && uuid(&e).is_some() {
            entries.push(slim(e));
            lines.push(range);
        }
    }
    let order = chain(&mut entries);
    // The chain names each entry once, so each is moved out, not copied.
    let list: Vec<(Value, Line)> = order.iter().map(|&i| (std::mem::take(&mut entries[i]), Some(lines[i].clone()))).collect();
    drop(entries);
    Messages { list: shown(list), data }
}

type Line = Option<std::ops::Range<usize>>;

// What of the chain's entries getSessionMessages gives, and how; each
// one's message is put in when it's taken, from its line.
fn shown(mut list: Vec<(Value, Line)>) -> Vec<(Value, Line)> {
    // A local command's records, after its caveat, until the next reply.
    let mut marking = false;
    for (e, _) in &mut list {
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
        if kind(&list[i].0) != "attachment" {
            continue;
        }
        let consumed = list[i + 1..]
            .iter()
            .map(|(x, _)| x)
            .find(|x| is_message(x))
            .is_some_and(|x| kind(x) == "assistant" || is_tool_result(x) || last_text(x).starts_with("[Request interrupted by user"));
        if consumed || i + 1 == n {
            if let Some(q) = queued_message(&list[i].0) {
                list[i] = (q, None);
            }
        }
    }
    let special = |o: Option<&Value>| {
        o.and_then(|o| o.get("kind")).and_then(Value::as_str).is_some_and(|k| matches!(k, "channel" | "observer" | "observer-activity" | "slack-ping" | "peer"))
    };
    list.into_iter()
        .filter(|(e, _)| is_message(e) && (!truthy(e.get("isMeta")) || special(e.get("origin"))) && !truthy(e.get("isSidechain")) && !truthy(e.get("teamName")))
        .map(|(e, line)| {
            let mut m = Map::new();
            m.insert("type".into(), e.get("type").cloned().unwrap_or(Value::Null));
            m.insert("uuid".into(), e.get("uuid").cloned().unwrap_or(Value::Null));
            m.insert("session_id".into(), e.get("sessionId").cloned().unwrap_or(Value::Null));
            // A line's message is read from it when it's taken.
            m.insert("message".into(), if line.is_some() { Value::Null } else { e.get("message").cloned().unwrap_or(Value::Null) });
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
            (Value::Object(m), line)
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

    // A fixed run of numbers, for the texts and transcripts the tests below
    // make up.
    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    // The searches as they were, over the whole text.
    fn last_value_whole(text: &str, key: &str) -> Option<String> {
        let mut at: Vec<usize> = starts(text, &format!("\"{key}\":\"")).into_iter().chain(starts(text, &format!("\"{key}\": \""))).collect();
        at.sort_unstable();
        at.into_iter().rev().find_map(|i| value_at(text, i))
    }

    fn first_value_whole(text: &str, key: &str) -> Option<String> {
        let tight = starts(text, &format!("\"{key}\":\""));
        let list = if tight.is_empty() { starts(text, &format!("\"{key}\": \"")) } else { tight };
        list.into_iter().next().and_then(|i| value_at(text, i))
    }

    #[test]
    fn values_found_from_the_end_are_those_the_whole_text_has() {
        let bits = ["\"k\":\"", "\"k\": \"", "\n", "x", "\\\"", "\"", "\\\\", "k", ":", " ", "é"];
        let mut d = Dice(0x9e37_79b9_7f4a_7c15);
        for _ in 0..5000 {
            let n = d.roll(48);
            let text: String = (0..n).map(|_| bits[d.roll(bits.len())]).collect();
            for stretch in [1, 2, 5, 16, STRETCH] {
                assert_eq!(last_value_by(&text, "k", stretch), last_value_whole(&text, "k"), "{text:?}, {stretch} at a time");
            }
            assert_eq!(first_value(&text, "k"), first_value_whole(&text, "k"), "{text:?}");
        }
    }

    // What ran beside the chain, found as it was: each reply's parts and
    // results looked for through every entry.
    fn beside_as_it_was(entries: &[Value], mut out: Vec<usize>) -> Vec<usize> {
        let in_chain: HashSet<usize> = out.iter().copied().collect();
        let mut owner: HashMap<String, Option<String>> = HashMap::new();
        for e in entries {
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

    #[test]
    fn what_ran_beside_the_chain_is_found_as_it_was() {
        let mut d = Dice(0x2545_f491_4f6c_dd1d);
        let mut grew = 0;
        for round in 0..400 {
            let n = 1 + d.roll(48);
            let mut entries = Vec::new();
            for _ in 0..n {
                let mut e = json!({ "uuid": format!("u{}", d.roll(n + 4)) });
                if d.roll(6) > 0 {
                    e["parentUuid"] = json!(format!("u{}", d.roll(n + 4)));
                }
                if d.roll(8) > 0 {
                    e["timestamp"] = json!(format!("2026-10-17T10:00:{:02}.000Z", d.roll(12)));
                }
                let calls = |d: &mut Dice, kind: &str, key: &str| -> Vec<Value> { (0..d.roll(3)).map(|_| json!({ "type": kind, key: format!("t{}", d.roll(10)) })).collect() };
                match d.roll(3) {
                    0 => {
                        e["type"] = json!("assistant");
                        e["message"] = json!({ "id": format!("m{}", d.roll(8)), "content": calls(&mut d, "tool_use", "id") });
                    }
                    1 => {
                        e["type"] = json!("user");
                        e["message"] = json!({ "content": calls(&mut d, "tool_result", "tool_use_id") });
                    }
                    _ => {
                        e["type"] = json!("user");
                        e["message"] = json!({ "content": "a prompt" });
                    }
                }
                entries.push(e);
            }
            // A chain: some of the entries, each once, in any order.
            let mut out: Vec<usize> = (0..n).filter(|_| d.roll(3) > 0).collect();
            for i in (1..out.len()).rev() {
                out.swap(i, d.roll(i + 1));
            }
            let found = beside(&entries, out.clone());
            grew += usize::from(found.len() > out.len());
            assert_eq!(found, beside_as_it_was(&entries, out), "round {round}: {entries:?}");
        }
        assert!(grew > 100, "most rounds find something beside the chain: {grew}");
    }

    #[test]
    fn a_listing_reads_again_only_what_changed() {
        let root = std::env::temp_dir().join(format!("xenonet-wire-sdk-listings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = root.join("-Users-nova-Projects-atlas");
        let dir = root.to_string_lossy().into_owned();
        let prompt = |id: &str, text: &str| json!({ "type": "user", "uuid": "u1", "sessionId": id, "cwd": "/Users/nova/Projects/atlas", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": text } });
        write(&p, A, &[prompt(A, "Add partial refunds")]);
        write(&p, B, &[prompt(B, "Review the refunds change")]);
        let mut l = Listings::default();
        assert_eq!(list_sessions_with(&dir, &mut l), list_sessions(&dir));
        assert_eq!(l.seen.len(), 2);
        // A name in the sidecar shows though the transcript is as it was.
        std::fs::create_dir_all(p.join(A)).expect("a folder");
        std::fs::write(p.join(A).join("custom-title.json"), r#"{"customTitle":"Refunds"}"#).expect("a sidecar");
        let list = list_sessions_with(&dir, &mut l);
        assert_eq!(list, list_sessions(&dir));
        assert!(list.iter().any(|s| s["customTitle"] == "Refunds"));
        // A transcript written to is read again.
        rename_session(&dir, B, "Review").expect("renamed");
        let list = list_sessions_with(&dir, &mut l);
        assert_eq!(list, list_sessions(&dir));
        assert!(list.iter().any(|s| s["customTitle"] == "Review"));
        // One that's gone is forgotten.
        std::fs::remove_file(p.join(format!("{A}.jsonl"))).expect("removed");
        assert_eq!(list_sessions_with(&dir, &mut l).len(), 1);
        assert_eq!(l.seen.len(), 1);
        std::fs::remove_dir_all(&root).expect("cleaned up");
    }

    // getSessionMessages as it was: every entry held whole throughout.
    fn session_messages_as_it_was(projects_dir: &str, id: &str) -> Vec<Value> {
        let Some(file) = find_file(projects_dir, id) else { return Vec::new() };
        let data = std::fs::read(file).expect("a transcript");
        let mut entries: Vec<Value> = data
            .split(|b| *b == b'\n')
            .filter_map(parse_line)
            .filter(|e| matches!(kind(e), "user" | "assistant" | "progress" | "system" | "attachment") && uuid(e).is_some())
            .collect();
        let order = chain(&mut entries);
        let mut list: Vec<Value> = order.iter().map(|&i| entries[i].clone()).collect();
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
        let n = list.len();
        for i in 0..n {
            if kind(&list[i]) != "attachment" {
                continue;
            }
            let consumed = list[i + 1..]
                .iter()
                .find(|x| is_message(x))
                .is_some_and(|x| kind(x) == "assistant" || is_tool_result(x) || last_text(x).starts_with("[Request interrupted by user"));
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

    // A made-up transcript's entry, of every kind getSessionMessages reads.
    fn entry(d: &mut Dice, n: usize, prev: &str) -> Value {
        let pick = |d: &mut Dice, list: &[Value]| list[d.roll(list.len())].clone();
        let mut e = json!({ "uuid": format!("u{}", d.roll(n + 4)), "sessionId": A });
        e["parentUuid"] = match d.roll(8) {
            0 => Value::Null,
            1 => json!(format!("u{}", d.roll(n + 4))),
            _ => json!(prev),
        };
        if d.roll(8) > 0 {
            e["timestamp"] = json!(format!("2026-10-17T10:{:02}:00.000Z", d.roll(30)));
        }
        let texts = [
            json!("Add partial refunds"),
            json!("<local-command-caveat>Caveat</local-command-caveat>"),
            json!("<command-name>/cost</command-name>"),
            json!("<local-command-stdout>ok</local-command-stdout>"),
            json!("[Request interrupted by user]"),
        ];
        let block = |d: &mut Dice| match d.roll(6) {
            0 => json!({ "type": "text", "text": pick(d, &texts) }),
            1 => json!({ "type": "tool_result", "tool_use_id": format!("t{}", d.roll(8)), "content": [{ "type": "text", "text": "x".repeat(300) }], "is_error": false }),
            2 => json!({ "type": "image", "source": { "data": "aGk=".repeat(50) } }),
            3 => json!({ "type": "tool_use", "id": format!("t{}", d.roll(8)), "name": "Bash", "input": { "command": "npm test" } }),
            4 => json!({ "type": "thinking", "thinking": "Where are refunds?", "signature": "sig" }),
            _ => json!("a bare string"),
        };
        match d.roll(7) {
            0 | 1 => {
                e["type"] = json!("user");
                let content = if d.roll(2) == 0 { pick(d, &texts) } else { Value::Array((0..=d.roll(3)).map(|_| block(d)).collect()) };
                e["message"] = json!({ "role": "user", "content": content });
                e["toolUseResult"] = json!({ "stdout": "y".repeat(200) });
            }
            2 | 3 => {
                e["type"] = json!("assistant");
                e["message"] =
                    json!({ "id": format!("m{}", d.roll(6)), "role": "assistant", "model": "claude-x", "content": (0..=d.roll(3)).map(|_| block(d)).collect::<Vec<_>>() });
            }
            4 => {
                e["type"] = json!("attachment");
                e["attachment"] = match d.roll(3) {
                    0 => json!({ "type": "queued_command", "prompt": pick(d, &texts), "source_uuid": format!("q{}", d.roll(5)) }),
                    1 => {
                        json!({ "type": "queued_command", "prompt": [{ "type": "text", "text": "later" }], "isMeta": d.roll(2) == 0, "origin": pick(d, &[json!({ "kind": "peer" }), json!({ "kind": "x" })]), "commandMode": pick(d, &[json!("task-notification"), json!("prompt")]) })
                    }
                    _ => json!({ "type": pick(d, &[json!("file"), json!("fork_briefing")]), "content": "z".repeat(300) }),
                };
            }
            5 => {
                e["type"] = json!("system");
                e["subtype"] = json!("compact_boundary");
                let u: Vec<String> = (0..3).map(|_| format!("u{}", d.roll(n))).collect();
                e["compactMetadata"] = if d.roll(2) == 0 {
                    json!({ "preservedSegment": { "headUuid": u[0], "anchorUuid": u[1], "tailUuid": u[2] } })
                } else {
                    json!({ "preservedMessages": { "anchorUuid": u[0], "uuids": [u[1], u[2]] } })
                };
            }
            _ => e["type"] = pick(d, &[json!("progress"), json!("summary")]),
        }
        for (k, v) in [
            ("isMeta", json!(true)),
            ("isSidechain", json!(true)),
            ("teamName", json!("crew")),
            ("promptSource", json!("typed")),
            ("interruptedByShutdown", json!(true)),
            ("toolDenialUnanswered", json!("stream-closed")),
            ("isCompactSummary", json!(true)),
            ("isVisibleInTranscriptOnly", json!(true)),
            ("isQueuedCommand", json!(true)),
            ("origin", json!({ "kind": "task-notification", "subkind": "done", "extra": 1 })),
        ] {
            if d.roll(12) == 0 {
                e[k] = v;
            }
        }
        if d.roll(30) == 0 {
            e.as_object_mut().expect("an object").shift_remove("uuid");
        }
        e
    }

    #[test]
    fn messages_read_whole_only_when_taken_are_those_held_whole() {
        let root = std::env::temp_dir().join(format!("xenonet-wire-sdk-messages-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let p = root.join("-Users-nova-Projects-atlas");
        std::fs::create_dir_all(&p).expect("a folder");
        let dir = root.to_string_lossy().into_owned();
        let mut d = Dice(0x1234_5678_9abc_def1);
        let mut shown = 0;
        for round in 0..300 {
            let n = 1 + d.roll(60);
            let mut text = String::new();
            let mut prev = String::from("u0");
            for _ in 0..n {
                let e = entry(&mut d, n, &prev);
                prev = e["uuid"].as_str().unwrap_or("u0").to_string();
                match d.roll(20) {
                    0 => text.push_str("not json\n"),
                    1 => text.push_str("\n  "),
                    _ => {}
                }
                text.push_str(&xenonet_base::json::stringify(&e, 0));
                text.push('\n');
            }
            std::fs::write(p.join(format!("{A}.jsonl")), &text).expect("a transcript");
            let want = session_messages_as_it_was(&dir, A);
            let got = messages(&dir, A);
            assert_eq!(got.all(), want, "round {round}: {text}");
            let (a, b) = (d.roll(want.len() + 1), d.roll(want.len() + 1));
            let (a, b) = (a.min(b), a.max(b));
            assert_eq!(got.page(a, b), want[a..b], "round {round}, {a}..{b}");
            shown += want.len();
        }
        assert!(shown > 500, "the transcripts have messages to show: {shown}");
        std::fs::remove_dir_all(&root).expect("cleaned up");
    }
}
