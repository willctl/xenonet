//! core/integrations/pi.js: Pi's sessions and pi-harness exports, read as
//! metadata only: their model, provider, tokens, cost, tool calls and
//! checkpoints, never what was said. Sources are only read, at most a
//! bound of each, and never changed.

use crate::b64::sha256_hex;
use crate::common::{clean, fail, Fail};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::Read;
use xenonet_app::nodepath::NATIVE;
use xenonet_base::json::number as num;
use xenonet_base::text::js_number;

fn number(v: Option<&Value>) -> f64 {
    let n = js_number(v);
    if n.is_finite() && n >= 0.0 {
        n
    } else {
        0.0
    }
}

fn time(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::String(s)) => {
            let t = xenonet_base::time::parse_iso(s).unwrap_or(f64::NAN);
            if t.is_finite() && t >= 0.0 {
                t
            } else {
                0.0
            }
        }
        other => number(other),
    }
}

fn id_of(v: &str) -> String {
    sha256_hex(v.as_bytes())[..24].to_string()
}

fn count(v: Option<&Value>) -> usize {
    v.and_then(Value::as_array).map_or(0, Vec::len)
}

pub fn export_metadata(value: &Value) -> Result<Value, Fail> {
    let run = value.get("run").filter(|r| xenonet_app::jsv::truthy(Some(r)));
    let ok = run.is_some_and(|r| r.get("id").and_then(Value::as_str).is_some_and(|i| !i.is_empty()) && r.get("status").is_some_and(Value::is_string));
    let Some(run) = run.filter(|_| ok) else { return fail("Choose a pi-harness JSON run export.") };
    let finished = run.get("finishedAt").filter(|v| !v.is_null()).or_else(|| run.get("startedAt"));
    let usage = run.get("usage");
    Ok(json!({
        "id": format!("pi-export:{}", id_of(run["id"].as_str().unwrap_or(""))),
        "kind": "pi-export",
        "externalId": clean(run.get("id"), 160),
        "status": clean(run.get("status"), 40),
        "at": num(time(finished)),
        "startedAt": num(time(run.get("startedAt"))),
        "model": clean(run.get("model"), 160),
        "provider": clean(run.get("provider"), 160),
        "tokens": num(number(usage.and_then(|u| u.get("totalTokens")))),
        "cost": num(number(usage.and_then(|u| u.get("estimatedCost")))),
        "artifacts": count(value.get("artifacts")),
        "diagnostics": count(value.get("diagnostics")),
        "checkpoints": count(value.get("trace").and_then(|t| t.get("checkpoints"))),
    }))
}

// A key for a Set or Map of JS values: its JSON.
fn key(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), |v| xenonet_base::json::stringify(v, 0))
}

fn truthy_or<'a>(a: Option<&'a Value>, b: String) -> Value {
    match a.filter(|v| xenonet_app::jsv::truthy(Some(v))) {
        Some(v) => v.clone(),
        None => json!(b),
    }
}

pub fn session_metadata(content: &str, file: &str) -> Result<Value, Fail> {
    let mut skipped = 0;
    let mut entries: Vec<Value> = Vec::new();
    for line in content.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)) {
        if xenonet_base::text::js_trim(line).is_empty() {
            continue;
        }
        match xenonet_base::json::parse(line) {
            Ok(e) if e.is_object() || e.is_array() => entries.push(e),
            Ok(_) => {}
            Err(_) => skipped += 1,
        }
    }
    let header = entries.first().cloned().unwrap_or(Value::Null);
    if header.get("type").and_then(Value::as_str) != Some("session") || !header.get("id").is_some_and(Value::is_string) {
        return fail("Not a Pi session.");
    }
    if number(header.get("version")) > 3.0 {
        return fail("This Pi session version is newer than XenoNet supports.");
    }
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        if xenonet_app::jsv::truthy(e.get("id")) {
            by_id.insert(key(e.get("id")), i);
        }
    }
    let mut branch: Vec<&Value> = Vec::new();
    let last = entries.last();
    if last.is_some_and(|l| l.get("parentId").is_some()) {
        let mut seen: HashSet<String> = HashSet::new();
        let mut leaf = last;
        while let Some(l) = leaf {
            if seen.contains(&key(l.get("id"))) || l.get("type").and_then(Value::as_str) == Some("session") {
                break;
            }
            seen.insert(key(l.get("id")));
            branch.insert(0, l);
            leaf = by_id.get(&key(l.get("parentId"))).map(|i| &entries[*i]);
        }
    } else {
        branch.extend(entries.iter().skip(1));
    }
    let (mut model, mut provider) = (String::new(), String::new());
    let (mut tokens, mut cost, mut tools, mut failures, mut checkpoints) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let mut pending: Vec<String> = Vec::new();
    for entry in &branch {
        let t = entry.get("type").and_then(Value::as_str);
        if t == Some("model_change") {
            model = clean(entry.get("modelId"), 160);
            provider = clean(entry.get("provider"), 160);
        }
        if matches!(t, Some("compaction" | "branch_summary")) {
            checkpoints += 1.0;
        }
        let m = entry.get("message").filter(|m| !m.is_null());
        let role = m.and_then(|m| m.get("role")).and_then(Value::as_str);
        if role == Some("assistant") {
            let m = m.unwrap();
            model = clean(Some(&truthy_or(m.get("model"), model.clone())), 160);
            provider = clean(Some(&truthy_or(m.get("provider"), provider.clone())), 160);
            tokens += number(m.get("usage").and_then(|u| u.get("totalTokens")));
            cost += number(m.get("usage").and_then(|u| u.get("cost")).and_then(|c| c.get("total")));
            for c in m.get("content").and_then(Value::as_array).into_iter().flatten() {
                if c.get("type").and_then(Value::as_str) == Some("toolCall") && xenonet_app::jsv::truthy(c.get("id")) {
                    tools += 1.0;
                    let k = key(c.get("id"));
                    if !pending.contains(&k) {
                        pending.push(k);
                    }
                }
            }
        }
        if role == Some("toolResult") {
            let m = m.unwrap();
            let k = key(m.get("toolCallId"));
            pending.retain(|p| *p != k);
            if m.get("isError") == Some(&json!(true)) {
                failures += 1.0;
            }
        }
    }
    let at = branch.last().and_then(|e| e.get("timestamp")).filter(|v| !v.is_null()).or_else(|| header.get("timestamp"));
    let mut out = Map::new();
    out.insert("id".into(), json!(format!("pi-session:{}", id_of(file))));
    out.insert("kind".into(), json!("pi-session"));
    out.insert("externalId".into(), json!(clean(header.get("id"), 160)));
    out.insert("status".into(), json!(if pending.is_empty() { "recorded" } else { "awaiting-result" }));
    out.insert("at".into(), num(time(at)));
    out.insert("startedAt".into(), num(time(header.get("timestamp"))));
    out.insert("cwd".into(), json!(clean(header.get("cwd"), 1024)));
    out.insert("model".into(), json!(model));
    out.insert("provider".into(), json!(provider));
    for (k, v) in [("tokens", tokens), ("cost", cost), ("tools", tools), ("failures", failures), ("checkpoints", checkpoints)] {
        out.insert(k.into(), num(v));
    }
    out.insert("skipped".into(), json!(skipped));
    out.insert("pending".into(), json!(pending.len()));
    out.insert("note".into(), json!("Read-only recorded metadata for the latest branch. A pending result does not establish a live or stuck process."));
    Ok(Value::Object(out))
}

/// Pi's sessions under `root`: { records, warnings }.
pub fn scan(root: &str, max_files: usize, max_bytes: u64) -> Value {
    struct Scan {
        records: Vec<Value>,
        warnings: Vec<String>,
        visited: usize,
        max_files: usize,
        max_bytes: u64,
    }
    impl Scan {
        fn warn(&mut self, w: &str) {
            if !self.warnings.iter().any(|x| x == w) {
                self.warnings.push(w.into());
            }
        }
        // false once a limit stops the walk here.
        fn walk(&mut self, dir: &str, depth: u32) {
            let Ok(rd) = std::fs::read_dir(dir) else {
                if depth == 0 {
                    self.warn("Pi sessions directory is unavailable.");
                }
                return;
            };
            let mut entries: Vec<(String, std::fs::FileType)> = rd.filter_map(|e| e.ok()).filter_map(|e| Some((e.file_name().to_string_lossy().into_owned(), e.file_type().ok()?))).collect();
            entries.sort_by(|a, b| xenonet_app::jstext::locale_compare(&a.0, &b.0));
            for (name, kind) in entries {
                self.visited += 1;
                if self.visited > self.max_files * 4 {
                    self.warn("Directory scan limit reached.");
                    return;
                }
                if self.records.len() >= self.max_files {
                    self.warn("Session limit reached.");
                    return;
                }
                let file = NATIVE.join(&[dir, &name]);
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() && depth < 3 {
                    self.walk(&file, depth + 1);
                } else if kind.is_file() && name.ends_with(".jsonl") {
                    self.read(&file);
                }
            }
        }
        // Opened once and read at most the bound, even if the session grows
        // during the read.
        fn read(&mut self, file: &str) {
            let Ok(mut f) = std::fs::File::open(file) else { return self.warn("A session could not be read or has an unsupported format.") };
            let Ok(st) = f.metadata() else { return self.warn("A session could not be read or has an unsupported format.") };
            if !st.is_file() || st.len() > self.max_bytes {
                return self.warn("A session exceeded the file size limit.");
            }
            let mut buf = Vec::new();
            if (&mut f).take(self.max_bytes + 1).read_to_end(&mut buf).is_err() {
                return self.warn("A session could not be read or has an unsupported format.");
            }
            if buf.len() as u64 > self.max_bytes {
                return self.warn("A session grew beyond the file size limit.");
            }
            match session_metadata(&String::from_utf8_lossy(&buf), file) {
                Ok(r) => self.records.push(r),
                Err(_) => self.warn("A session could not be read or has an unsupported format."),
            }
        }
    }
    let mut s = Scan { records: vec![], warnings: vec![], visited: 0, max_files, max_bytes };
    s.walk(root, 0);
    json!({ "records": s.records, "warnings": s.warnings })
}
