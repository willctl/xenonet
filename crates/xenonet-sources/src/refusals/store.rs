//! core/refusals/store.js: every refusal XenoNet sees, kept on this
//! computer to build a dataset from: a folder each under <state>/refusals,
//! with record.json (refusal/record/v1) and, when you've turned on keeping
//! context, what the model had been given up to it (context.jsonl.gz, the
//! transcript's own lines, and request.json, the same as a request would
//! carry it). Keeping context is off unless you turn it on.

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use xenonet_base::json::{number, parse, stringify};
use xenonet_base::jsstr;
use xenonet_base::text::{is_blank, js_string, truthy};
use xenonet_base::{log, paths, time};

pub const SCHEMA: &str = "refusal/record/v1";
const CONTEXT_MAX: usize = 16 << 20;
const LONG: usize = 20000;

/// Why a call failed where the JS would have thrown: the message, and
/// Node's code for a file's error.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreError {
    pub message: String,
    pub code: Option<String>,
}

impl StoreError {
    fn io(e: std::io::Error, syscall: &str, path: &Path) -> Self {
        let f = xenonet_base::fs::FsError::of(e, syscall, &path.to_string_lossy());
        StoreError { message: f.message.clone(), code: f.code.clone() }
    }
}

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn nn(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

// detect.js's keyOf, with the session's id as a template writes it.
fn key_of(sig: &Value, session_id: Option<&Value>) -> String {
    super::detect::key_of(sig, &session_id.map_or_else(|| "undefined".to_string(), js_string))
}

/// A transcript's lines as JSON, what doesn't read as JSON left out.
fn read_jsonl(file: &Path) -> std::io::Result<Vec<Value>> {
    let text = String::from_utf8_lossy(&fs::read(file)?).into_owned();
    Ok(text.split('\n').filter(|l| !is_blank(l)).filter_map(|l| parse(l).ok()).collect())
}

/// Long strings (tool output, file contents) cut to their head and tail,
/// so a context fits; says whether anything was cut.
fn trimmed(lines: Vec<Value>) -> (Vec<Value>, bool) {
    fn trim(v: Value, cut: &mut bool) -> Value {
        match v {
            Value::String(s) if jsstr::len(&s) > LONG => {
                *cut = true;
                let n = jsstr::len(&s);
                Value::String(format!("{}\n[… {} characters cut by XenoNet …]\n{}", jsstr::head(&s, LONG / 2), n - LONG, jsstr::last(&s, LONG / 2)))
            }
            Value::Array(a) => Value::Array(a.into_iter().map(|x| trim(x, cut)).collect()),
            Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, trim(x, cut))).collect()),
            v => v,
        }
    }
    let mut cut = false;
    let lines = lines.into_iter().map(|l| trim(l, &mut cut)).collect();
    (lines, cut)
}

fn is(v: &Value, k: &str, want: &str) -> bool {
    get(v, k).and_then(Value::as_str) == Some(want)
}

/// What a refusal had been given: the lines, the request and what a
/// transcript doesn't hold.
pub struct Context {
    pub lines: Vec<Value>,
    pub request: Value,
    pub missing: Vec<&'static str>,
}

impl Context {
    pub fn to_json(&self) -> Value {
        json!({ "lines": self.lines, "request": self.request, "missing": self.missing })
    }
}

/// Claude Code: the chain from the refusal back through parentUuid to the
/// last compaction, in order, with the system lines on it.
pub fn claude_context(all: &[Value], sig: &Value) -> Option<Context> {
    let mut by_uuid: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, l) in all.iter().enumerate() {
        if truthy(get(l, "uuid")) {
            by_uuid.insert(js_string(&l["uuid"]), i);
        }
    }
    let lookup = |v: Option<&Value>| v.and_then(|u| by_uuid.get(&js_string(u)).copied());
    let sig_uuid = get(sig, "uuid").filter(|v| truthy(Some(v)));
    let request_id = get(sig, "requestId").filter(|v| truthy(Some(v)));
    let message_id = get(sig, "messageId").filter(|v| truthy(Some(v)));
    let end = sig_uuid.and_then(|u| lookup(Some(u))).or_else(|| {
        all.iter().rposition(|l| request_id.is_some_and(|r| get(l, "requestId") == Some(r)) || message_id.is_some_and(|m| get(l, "message").and_then(|x| get(x, "id")) == Some(m)))
    })?;
    let mut chain: Vec<&Value> = Vec::new();
    let mut seen: HashSet<Option<String>> = HashSet::new();
    let mut at = Some(end);
    while let Some(i) = at {
        let l = &all[i];
        let uuid = get(l, "uuid").map(js_string);
        if !seen.insert(uuid) {
            break;
        }
        chain.push(l);
        if is(l, "type", "system") && is(l, "subtype", "compact_boundary") {
            break;
        }
        at = if truthy(get(l, "parentUuid")) { lookup(get(l, "parentUuid")) } else { None };
    }
    chain.reverse();
    let messages: Vec<Value> = chain
        .iter()
        .filter(|l| (is(l, "type", "user") || is(l, "type", "assistant")) && truthy(get(l, "message")) && !(is(l, "type", "assistant") && truthy(get(l, "isApiErrorMessage"))))
        .map(|l| {
            let m = &l["message"];
            let mut out = Map::new();
            out.insert("role".into(), nn(get(m, "role")).or(get(l, "type")).cloned().unwrap_or(Value::Null));
            if let Some(c) = get(m, "content") {
                out.insert("content".into(), c.clone());
            }
            Value::Object(out)
        })
        .collect();
    fn model_of(l: &Value) -> Option<&Value> {
        get(l, "message").and_then(|m| get(m, "model")).filter(|m| truthy(Some(m)) && m.as_str() != Some("<synthetic>"))
    }
    let model = nn(get(sig, "model")).cloned().or_else(|| chain.iter().rev().filter(|l| is(l, "type", "assistant")).find_map(|l| model_of(l).cloned())).unwrap_or(Value::Null);
    Some(Context {
        lines: chain.into_iter().cloned().collect(),
        request: json!({ "provider": "anthropic", "model": model, "system": null, "messages": messages }),
        missing: vec!["system prompt (Claude Code doesn't write it to the transcript)", "tool definitions", "context Claude Code adds at request time (CLAUDE.md, reminders)"],
    })
}

/// Codex: the session's settings, its turn contexts, and what came since
/// its last compaction, up to the refusal.
pub fn codex_context(all: &[Value], sig: &Value) -> Result<Context, String> {
    let want = if is(sig, "kind", "reroute") { "model_reroute" } else { "error" };
    let sig_at = get(sig, "at");
    let end = all
        .iter()
        .position(|l| {
            is(l, "type", "event_msg") && get(l, "payload").is_some_and(|p| is(p, "type", want)) && {
                let t = get(l, "timestamp").and_then(Value::as_str).and_then(time::parse_iso);
                matches!((t, sig_at.and_then(Value::as_f64)), (Some(a), Some(b)) if a == b)
            }
        })
        .map_or(all.len() as i64 - 1, |i| i as i64);
    let up_to = &all[..((end + 1).max(0) as usize).min(all.len())];
    let last_compact = up_to.iter().rposition(|l| is(l, "type", "compacted")).map_or(-1, |i| i as i64);
    let meta = up_to.iter().position(|l| is(l, "type", "session_meta"));
    let mut lines: Vec<Value> = Vec::new();
    if let Some(m) = meta.filter(|&m| (m as i64) < last_compact) {
        lines.push(up_to[m].clone());
    }
    lines.extend(up_to[last_compact.max(0) as usize..].iter().cloned());
    let mut input: Vec<Value> = Vec::new();
    if last_compact >= 0 {
        match get(&up_to[last_compact as usize], "payload").and_then(|p| nn(get(p, "replacement_history"))) {
            None => {}
            Some(Value::Array(h)) => input.extend(h.iter().cloned()),
            Some(Value::String(s)) => input.extend(s.encode_utf16().map(|u| Value::String(String::from_utf16_lossy(&[u])))),
            Some(_) => return Err("replacement_history is not iterable".into()),
        }
    }
    input.extend(lines.iter().filter(|l| is(l, "type", "response_item")).map(|l| get(l, "payload").cloned().unwrap_or(Value::Null)));
    let empty = Value::Object(Map::new());
    let ctx = up_to.iter().rev().find(|l| is(l, "type", "turn_context")).and_then(|l| nn(get(l, "payload"))).unwrap_or(&empty);
    let meta_payload = meta.and_then(|m| get(&up_to[m], "payload"));
    let instructions = meta_payload
        .and_then(|p| get(p, "base_instructions"))
        .and_then(|b| nn(get(b, "text")))
        .or_else(|| meta_payload.and_then(|p| nn(get(p, "instructions"))))
        .cloned()
        .unwrap_or(Value::Null);
    let model = nn(get(ctx, "model")).or(nn(get(sig, "model"))).cloned().unwrap_or(Value::Null);
    let effort = nn(get(ctx, "effort")).cloned().unwrap_or(Value::Null);
    Ok(Context {
        lines,
        request: json!({ "provider": "openai", "model": model, "effort": effort, "instructions": instructions, "input": input }),
        missing: vec!["tool definitions", "what OpenAI adds to the prompt on its side", "encrypted reasoning's content"],
    })
}

/// Whether to keep context: XENONET_REFUSAL_CONTEXT (on or off) when set,
/// else your choice in <state>/refusals/settings.json (off until you turn
/// it on).
pub fn context_on(dir: &Path, env_value: Option<&str>) -> bool {
    if let Some(v @ ("on" | "off")) = env_value {
        return v == "on";
    }
    fs::read(dir.join("settings.json")).ok().and_then(|b| parse(&String::from_utf8_lossy(&b)).ok()).is_some_and(|v| get(&v, "context") == Some(&Value::Bool(true)))
}

fn create_private_dir(dir: &Path, recursive: bool) -> std::io::Result<()> {
    let mut b = fs::DirBuilder::new();
    b.recursive(recursive);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir)
}

fn write_private(file: &Path, data: &[u8], append: bool) -> std::io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.create(true);
    if append {
        o.append(true);
    } else {
        o.write(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(file)?.write_all(data)
}

pub fn set_context(dir: &Path, on: bool) -> Result<bool, StoreError> {
    create_private_dir(dir, true).map_err(|e| StoreError::io(e, "mkdir", dir))?;
    let file = dir.join("settings.json");
    write_private(&file, format!("{}\n", stringify(&json!({ "context": on }), 0)).as_bytes(), false).map_err(|e| StoreError::io(e, "open", &file))?;
    Ok(on)
}

/// Where a refusal was seen: { sessionId, app, agentId, file, cwd, model }.
pub type At = Value;

/// The refusals kept under `dir`.
pub struct Refusals {
    pub dir: PathBuf,
    home: String,
    keys: Option<HashSet<String>>,
}

fn is_record_name(n: &str) -> bool {
    let b = n.as_bytes();
    b.len() == 17 && b[..8].iter().all(u8::is_ascii_digit) && b[8] == b'-' && b[9..].iter().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
}

fn random_hex4() -> String {
    let mut bytes = [0u8; 4];
    // A record's id needs to be unlike the others, not secret; the time
    // keeps it apart if the system has no randomness to give.
    if getrandom::fill(&mut bytes).is_err() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        bytes = (t as u32).to_le_bytes();
    }
    jsstr::hex(&bytes)
}

impl Refusals {
    pub fn new(dir: &str, home: &str) -> Self {
        Refusals { dir: PathBuf::from(dir), home: home.to_string(), keys: None }
    }

    // paths.tilde: nothing to do without a home.
    fn tilde(&self, text: &str) -> String {
        if self.home.is_empty() || text.is_empty() {
            text.to_string()
        } else {
            paths::tilde(text, &self.home)
        }
    }

    fn index_file(&self) -> PathBuf {
        self.dir.join("index.jsonl")
    }

    fn load_keys(&mut self) -> &mut HashSet<String> {
        let index = self.index_file();
        self.keys.get_or_insert_with(|| {
            let mut keys = HashSet::new();
            if let Ok(b) = fs::read(index) {
                for line in String::from_utf8_lossy(&b).split('\n') {
                    if let Ok(v) = parse(line) {
                        // JSON.parse(line).key, as a Set keeps it: undefined
                        // for a line without one.
                        keys.insert(get(&v, "key").map_or_else(|| "\u{0}undefined".to_string(), js_string));
                    }
                }
            }
            keys
        })
    }

    /// A record's folder: its id with anything but letters, digits and "-"
    /// taken out.
    pub fn folder_of(&self, id: &str) -> PathBuf {
        self.dir.join(id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>())
    }

    pub fn context_file(&self, id: &str) -> PathBuf {
        self.folder_of(id).join("context.jsonl.gz")
    }

    fn add_context(&self, rec: &mut Map<String, Value>) -> Result<Value, StoreError> {
        let no = |why: &str| Ok(json!({ "ok": false, "why": why }));
        let file = rec.get("source").and_then(|s| get(s, "path")).filter(|p| truthy(Some(p))).map(js_string);
        let Some(file) = file.filter(|f| Path::new(f).exists()) else { return no("its transcript isn't there any more") };
        let all = read_jsonl(Path::new(&file)).map_err(|e| StoreError::io(e, "open", Path::new(&file)))?;
        let sig = rec.get("signal").cloned().unwrap_or(Value::Null);
        let got = if rec.get("provider").and_then(Value::as_str) == Some("openai") {
            Some(codex_context(&all, &sig).map_err(|m| StoreError { message: m, code: None })?)
        } else {
            claude_context(&all, &sig)
        };
        let Some(got) = got else { return no("its line isn't in the transcript") };
        let (mut lines, mut cut) = trimmed(got.lines);
        let join = |lines: &[Value]| lines.iter().map(|l| stringify(l, 0)).collect::<Vec<_>>().join("\n");
        let mut text = join(&lines);
        if text.len() > CONTEXT_MAX {
            let keep = 1.max(lines.len() / 2);
            lines = lines.split_off(lines.len() - keep);
            text = join(&lines);
            cut = true;
        }
        let folder = self.folder_of(rec.get("id").and_then(Value::as_str).unwrap_or(""));
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(format!("{text}\n").as_bytes()).and_then(|()| gz.flush()).map_err(|e| StoreError { message: e.to_string(), code: None })?;
        let packed = gz.finish().map_err(|e| StoreError { message: e.to_string(), code: None })?;
        let ctx_file = folder.join("context.jsonl.gz");
        write_private(&ctx_file, &packed, false).map_err(|e| StoreError::io(e, "open", &ctx_file))?;
        let mut request = got.request.as_object().cloned().unwrap_or_default();
        if cut {
            request.insert("trimmed".into(), Value::Bool(true));
        }
        let req_file = folder.join("request.json");
        write_private(&req_file, format!("{}\n", stringify(&Value::Object(request), 1)).as_bytes(), false).map_err(|e| StoreError::io(e, "open", &req_file))?;
        let digest = jsstr::hex(&Sha256::digest(text.as_bytes()));
        rec.insert(
            "context".into(),
            json!({ "file": "context.jsonl.gz", "request": "request.json", "lines": lines.len(), "bytes": text.len(), "sha256": digest, "truncated": cut }),
        );
        rec.insert("missing".into(), json!(got.missing));
        Ok(json!({ "ok": true }))
    }

    /// Whether a refusal is one already kept.
    pub fn known(&mut self, sig: &Value, at: &At) -> bool {
        let key = key_of(sig, get(at, "sessionId"));
        self.load_keys().contains(&key)
    }

    /// A refusal seen in a session's transcript: a new record, or None for
    /// one already kept. `now` is the time to record it at, `context`
    /// whether to keep its context.
    pub fn record(&mut self, sig: &Value, at: &At, now: f64, context: bool) -> Result<Option<Value>, StoreError> {
        let key = key_of(sig, get(at, "sessionId"));
        if self.load_keys().contains(&key) {
            return Ok(None);
        }
        create_private_dir(&self.dir, true).map_err(|e| StoreError::io(e, "mkdir", &self.dir))?;
        let when = nn(get(sig, "at")).cloned().unwrap_or_else(|| number(now));
        let when_ms = when.as_f64().unwrap_or(f64::NAN);
        if !when_ms.is_finite() || when_ms.abs() > 8.64e15 {
            return Err(StoreError { message: "Invalid time value".into(), code: Some("RangeError".into()) });
        }
        let id = format!("{}-{}", time::iso(when_ms)[..10].replace('-', ""), random_hex4());
        let folder = self.folder_of(&id);
        create_private_dir(&folder, false).map_err(|e| StoreError::io(e, "mkdir", &folder))?;
        let field = |v: &Value, k: &str| nn(get(v, k)).cloned().unwrap_or(Value::Null);
        let cwd = nn(get(at, "cwd")).filter(|v| truthy(Some(v))).map_or(Value::Null, |c| Value::String(self.tilde(&js_string(c))));
        let model = nn(get(sig, "model")).or(nn(get(at, "model"))).cloned().unwrap_or(Value::Null);
        let source = nn(get(at, "file")).filter(|v| truthy(Some(v))).map_or(Value::Null, |f| json!({ "file": self.tilde(&js_string(f)), "path": f }));
        let mut rec = Map::new();
        rec.insert("schema".into(), Value::String(SCHEMA.into()));
        rec.insert("id".into(), Value::String(id.clone()));
        rec.insert("key".into(), Value::String(key.clone()));
        rec.insert("at".into(), when);
        rec.insert("recordedAt".into(), number(now));
        rec.insert("provider".into(), get(sig, "provider").cloned().unwrap_or(Value::Null));
        rec.insert("app".into(), field(at, "app"));
        rec.insert("kind".into(), get(sig, "kind").cloned().unwrap_or(Value::Null));
        rec.insert("session".into(), json!({ "id": field(at, "sessionId"), "agentId": field(at, "agentId"), "cwd": cwd, "model": model }));
        rec.insert("signal".into(), sig.clone());
        rec.insert("source".into(), source);
        rec.insert("context".into(), Value::Null);
        rec.insert("missing".into(), Value::Array(Vec::new()));
        if context {
            // A context that can't be read leaves the record without one.
            let _ = self.add_context(&mut rec);
        }
        let record_file = folder.join("record.json");
        write_private(&record_file, format!("{}\n", stringify(&Value::Object(rec.clone()), 1)).as_bytes(), false).map_err(|e| StoreError::io(e, "open", &record_file))?;
        let line = json!({
            "id": id,
            "key": key,
            "at": rec["at"],
            "provider": rec["provider"],
            "app": rec["app"],
            "kind": rec["kind"],
            "category": get(sig, "category").cloned().unwrap_or(Value::Null),
            "model": rec["session"]["model"],
            "session": rec["session"]["id"],
            "context": truthy(rec.get("context")),
        });
        let mut line = line.as_object().cloned().expect("an object literal");
        // JSON.stringify leaves out a category that's undefined.
        if get(sig, "category").is_none() {
            line.remove("category");
        }
        if get(sig, "kind").is_none() {
            line.remove("kind");
        }
        if get(sig, "provider").is_none() {
            line.remove("provider");
        }
        let index = self.index_file();
        write_private(&index, format!("{}\n", stringify(&Value::Object(line), 0)).as_bytes(), true).map_err(|e| StoreError::io(e, "open", &index))?;
        self.load_keys().insert(key);
        // A key the signal didn't have stays out of the record, as
        // JSON.stringify leaves undefined out.
        if get(sig, "provider").is_none() {
            rec.remove("provider");
        }
        if get(sig, "kind").is_none() {
            rec.remove("kind");
        }
        Ok(Some(Value::Object(rec)))
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        let b = fs::read(self.folder_of(id).join("record.json")).ok()?;
        parse(&String::from_utf8_lossy(&b)).ok()
    }

    /// The records, newest first: since `since` (ms), of a provider (or
    /// app) or kind.
    pub fn list(&self, since: f64, provider: Option<&str>, kind: Option<&str>) -> Vec<Value> {
        let Ok(entries) = paths::read_dir_sorted(&self.dir) else { return Vec::new() };
        let mut out: Vec<Value> = entries
            .iter()
            .filter_map(|e| e.file_name().to_str().map(String::from))
            .filter(|n| is_record_name(n))
            .filter_map(|n| self.get(&n))
            .filter(|r| {
                truthy(Some(r))
                    && get(r, "at").and_then(Value::as_f64).is_some_and(|t| t >= since)
                    && provider.is_none_or(|p| is(r, "provider", p) || is(r, "app", p))
                    && kind.is_none_or(|k| is(r, "kind", k))
            })
            .collect();
        out.sort_by(|a, b| {
            let t = |r: &Value| get(r, "at").and_then(Value::as_f64).unwrap_or(f64::NAN);
            t(b).partial_cmp(&t(a)).unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// Adds context to a record kept without it, from its transcript if
    /// it's still there.
    pub fn capture(&self, id: &str) -> Result<Value, StoreError> {
        let Some(Value::Object(mut rec)) = self.get(id) else { return Ok(json!({ "ok": false, "why": "no such record" })) };
        if truthy(rec.get("context")) {
            return Ok(json!({ "ok": true, "already": true }));
        }
        let r = self.add_context(&mut rec)?;
        if r["ok"] == true {
            let file = self.folder_of(id).join("record.json");
            write_private(&file, format!("{}\n", stringify(&Value::Object(rec), 1)).as_bytes(), false).map_err(|e| StoreError::io(e, "open", &file))?;
        }
        Ok(r)
    }

    /// The records as JSON lines for a dataset, each with its context's
    /// request when asked; scrubbed of home folders and keys when asked.
    pub fn export_lines(&self, since: f64, provider: Option<&str>, kind: Option<&str>, with_context: bool, scrubbed: bool) -> Vec<Value> {
        fn deep(v: Value, home: &str) -> Value {
            match v {
                Value::String(s) => Value::String(log::scrub(&s, home)),
                Value::Array(a) => Value::Array(a.into_iter().map(|x| deep(x, home)).collect()),
                Value::Object(m) => Value::Object(m.into_iter().map(|(k, x)| (k, deep(x, home))).collect()),
                v => v,
            }
        }
        self.list(since, provider, kind)
            .into_iter()
            .map(|r| {
                let mut request = Value::Null;
                if with_context && truthy(get(&r, "context")) {
                    let id = get(&r, "id").map(js_string).unwrap_or_default();
                    if let Ok(b) = fs::read(self.folder_of(&id).join("request.json")) {
                        request = parse(&String::from_utf8_lossy(&b)).unwrap_or(Value::Null);
                    }
                }
                let mut out = super::query::public_record(&r).as_object().cloned().unwrap_or_default();
                if with_context {
                    out.insert("request".into(), request);
                }
                let out = Value::Object(out);
                if scrubbed {
                    deep(out, &self.home)
                } else {
                    out
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-refusals-rs-{}", random_hex4()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn kept_once_with_context_when_asked() {
        let dir = tmp();
        let transcript = dir.join("t.jsonl");
        let long = "x".repeat(LONG + 5);
        let lines = [
            json!({ "type": "system", "subtype": "compact_boundary", "uuid": "c1" }),
            json!({ "type": "user", "uuid": "u1", "parentUuid": "c1", "message": { "role": "user", "content": long } }),
            json!({ "type": "assistant", "uuid": "a1", "parentUuid": "u1", "requestId": "req_1", "isApiErrorMessage": true, "message": { "model": "<synthetic>", "stop_reason": "refusal" } }),
        ];
        fs::write(&transcript, lines.iter().map(|l| stringify(l, 0)).collect::<Vec<_>>().join("\n")).unwrap();
        let store = dir.join("refusals");
        let mut r = Refusals::new(store.to_str().unwrap(), "/home/me");
        let sig = json!({ "provider": "anthropic", "kind": "refusal", "requestId": "req_1", "uuid": "a1", "at": 1.8e12, "model": null });
        let at = json!({ "sessionId": "claude:x", "app": "claude", "file": transcript.to_str().unwrap(), "cwd": "/home/me/p" });
        let rec = r.record(&sig, &at, 1.8e12, true).unwrap().unwrap();
        assert_eq!(rec["session"]["cwd"], "~/p");
        assert_eq!(rec["context"]["lines"], 3);
        assert_eq!(rec["context"]["truncated"], true);
        assert!(r.record(&sig, &at, 1.8e12, false).unwrap().is_none());
        assert!(Refusals::new(store.to_str().unwrap(), "/home/me").record(&sig, &at, 1.8e12, false).unwrap().is_none(), "once across starts");
        assert_eq!(r.list(0.0, None, None).len(), 1);
        let out = r.export_lines(0.0, None, None, true, true);
        assert!(out[0]["request"]["messages"].is_array());
        assert!(!stringify(&Value::Array(out), 0).contains("\"key\""));
        assert!(!context_on(&store, None));
        set_context(&store, true).unwrap();
        assert!(context_on(&store, None) && !context_on(&store, Some("off")));
        fs::remove_dir_all(&dir).unwrap();
    }
}
