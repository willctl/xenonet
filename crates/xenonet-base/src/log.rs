//! XenoNet's error log (core/log.js): what went wrong in each part of
//! XenoNet, kept where you can find it. Each part writes its own file of JSON
//! lines, log/entry/v1, under <state>/logs (service.ndjson, desktop.ndjson,
//! cli.ndjson).
//!
//! An entry has a code that says what it is, XenoNet's words, the fix when
//! there's one, and a stack for what XenoNet didn't expect. Everything is
//! scrubbed as it's written: the home folder becomes ~ and anything that
//! looks like a key is taken out, so the log is safe to read out to an
//! agent or to copy into an issue. A file over 1 MB moves aside, and two are
//! kept; the same code with the same words is written at most every ten
//! seconds, with how many times it happened meanwhile.

use crate::json;
use crate::ordered::{Cursor, OrderedMap};
use crate::paths::{read_dir_sorted, replace_unless, tilde};
use regex::Regex;
use serde_json::{Map, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const LEVELS: [&str; 3] = ["error", "warn", "info"];
pub const COMPONENTS: [&str; 3] = ["service", "desktop", "cli"];
pub const MAX_BYTES: u64 = 1 << 20;
const KEEP: usize = 2;
const AGAIN_MS: f64 = 10e3;
const MESSAGE_MAX: usize = 2000;
const STACK_MAX: usize = 4000;

// Words that start the way keys and tokens do, bearer tokens and JWTs:
// taken out wherever they turn up. (\b is ASCII's, as JS's is.)
fn secrets() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)[A-Za-z0-9_-]{6,}",
            r"|(?-u:\b)Bearer\s+[A-Za-z0-9._~+/=-]{8,}",
            r"|(?-u:\b)eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}",
            r"|-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
        ))
        .expect("the key patterns")
    })
}

/// The first `n` UTF-16 units of a text, as JS's slice(0, n) takes them
/// (a character that would be cut in half is left out).
pub fn js_slice(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

fn trimmed_home(home: &str) -> &str {
    home.trim_end_matches(['\\', '/'])
}

/// Text as the log keeps it: the home folder as ~ (in its path forms, and
/// as Claude Code writes it into folder names, C--Users-nova), keys out,
/// control characters but line breaks out.
pub fn scrub(text: &str, home: &str) -> String {
    let mut out = tilde(text, home);
    let h = trimmed_home(home);
    if h.encode_utf16().count() > 2 {
        let form: Vec<char> = h.chars().map(|c| if matches!(c, '\\' | '/' | '.' | ':') { '-' } else { c }).collect();
        out = replace_unless(&out, &form, |c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
    }
    let out = secrets().replace_all(&out, "[key]");
    out.chars().filter(|c| !matches!(*c as u32, 0..=8 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f)).collect()
}

/// Whether text still holds the home folder or a key, after scrubbing:
/// what leaves XenoNet is checked with this last.
pub fn unsafe_text(text: &str, home: &str) -> Option<&'static str> {
    let h = trimmed_home(home);
    let forms: Vec<String> = if h.encode_utf16().count() > 2 { vec![h.to_string(), h.replace('\\', "/"), h.replace('/', "\\")] } else { Vec::new() };
    let lower = text.to_lowercase();
    if forms.iter().any(|f| lower.contains(&f.to_lowercase())) {
        return Some("the home folder");
    }
    if secrets().is_match(text) {
        return Some("something that looks like a key");
    }
    None
}

fn scrub_all(v: &Value, home: &str, depth: usize) -> Value {
    match v {
        Value::String(s) => Value::String(js_slice(&scrub(s, home), 500).to_string()),
        Value::Array(a) => Value::Array(if depth > 3 { Vec::new() } else { a.iter().take(20).map(|x| scrub_all(x, home, depth + 1)).collect() }),
        Value::Object(m) => Value::Object(if depth > 3 { Map::new() } else { m.iter().take(30).map(|(k, x)| (k.clone(), scrub_all(x, home, depth + 1))).collect() }),
        other => other.clone(),
    }
}

pub fn log_dir(state_dir: &str) -> String {
    crate::paths::join(state_dir, "logs")
}

/// What an error the caller had says of itself.
#[derive(Clone, Debug, Default)]
pub struct ErrInfo {
    pub message: Option<String>,
    pub stack: Option<String>,
    pub code: Option<String>,
}

/// A log for one part of XenoNet. With no folder it logs nothing (as in
/// the demo), but still makes each entry. It never fails: a log that can't
/// be written is let go.
pub struct Log {
    pub dir: Option<PathBuf>,
    pub component: String,
    pub instance: Option<String>,
    pub version: Option<String>,
    pub home: String,
    pub max_bytes: u64,
    pub pid: u32,
    seen: OrderedMap<(f64, u64)>,
    ready: bool,
}

impl Log {
    pub fn new(dir: Option<&str>, component: &str, instance: Option<&str>, version: Option<&str>, home: &str, max_bytes: u64) -> Self {
        Log {
            dir: dir.map(PathBuf::from),
            component: component.into(),
            instance: instance.map(Into::into),
            version: version.map(Into::into),
            home: home.into(),
            max_bytes,
            pid: std::process::id(),
            seen: OrderedMap::new(),
            ready: false,
        }
    }

    pub fn file(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{}.ndjson", self.component)))
    }

    /// Writes an entry at `now`, and gives it back; None when the same code
    /// with the same words came too soon after the last.
    #[allow(clippy::too_many_arguments)]
    pub fn write(&mut self, level: &str, code: &str, message: Option<&str>, err: Option<&ErrInfo>, fix: Option<&str>, ctx: Option<&Value>, now: f64) -> Option<Value> {
        let raw = message.map(str::to_string).or_else(|| err.and_then(|e| e.message.clone())).unwrap_or_else(|| code.to_string());
        let text = js_slice(&scrub(&raw, &self.home), MESSAGE_MAX).to_string();
        let key = format!("{code}|{text}");
        let had = self.seen.get(&key).copied();
        if let Some((at, repeats)) = had {
            if now - at < AGAIN_MS {
                self.seen.insert(&key, (at, repeats + 1));
                return None;
            }
        }
        self.seen.insert(&key, (now, 0));
        if self.seen.len() > 500 {
            if let Some((_, first)) = self.seen.next_after(Cursor::default()).map(|(c, k)| (c, k.to_string())) {
                self.seen.remove(&first);
            }
        }
        let mut e = Map::new();
        e.insert("schema".into(), Value::String("log/entry/v1".into()));
        e.insert("at".into(), json::number(now));
        e.insert("component".into(), Value::String(self.component.clone()));
        if let Some(i) = self.instance.as_ref().filter(|i| !i.is_empty()) {
            e.insert("instance".into(), Value::String(i.clone()));
        }
        if let Some(v) = self.version.as_ref().filter(|v| !v.is_empty()) {
            e.insert("version".into(), Value::String(v.clone()));
        }
        e.insert("pid".into(), Value::from(self.pid));
        e.insert("level".into(), Value::String(level.into()));
        e.insert("code".into(), Value::String(code.into()));
        e.insert("message".into(), Value::String(text));
        if let Some((_, repeats)) = had.filter(|(_, r)| *r > 0) {
            e.insert("repeats".into(), Value::from(repeats));
        }
        if let Some(f) = fix.filter(|f| !f.is_empty()) {
            e.insert("fix".into(), Value::String(js_slice(&scrub(f, &self.home), 500).to_string()));
        }
        if let Some(stack) = err.and_then(|e| e.stack.as_ref()).filter(|s| !s.is_empty() && level == "error") {
            e.insert("stack".into(), Value::String(js_slice(&scrub(stack, &self.home), STACK_MAX).to_string()));
        }
        if let Some(c) = err.and_then(|e| e.code.as_ref()).filter(|c| !c.is_empty()) {
            e.insert("errno".into(), Value::String(c.clone()));
        }
        if let Some(c) = ctx {
            e.insert("ctx".into(), scrub_all(c, &self.home, 0));
        }
        let entry = Value::Object(e);
        self.append(&entry);
        Some(entry)
    }

    fn append(&mut self, entry: &Value) {
        let (Some(dir), Some(file)) = (self.dir.clone(), self.file()) else { return };
        let _ = (|| -> std::io::Result<()> {
            if !self.ready {
                create_private_dir(&dir)?;
                self.ready = true;
            }
            let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            if size > self.max_bytes {
                rotate(&file);
            }
            let mut f = open_private(&file)?;
            f.write_all(format!("{}\n", json::stringify(entry, 0)).as_bytes())
        })();
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(dir)
    }
}

fn open_private(file: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(file)
}

fn rotate(file: &Path) {
    let s = file.to_string_lossy();
    let at = |i: usize| PathBuf::from(s.strip_suffix(".ndjson").map_or(s.to_string(), |b| format!("{b}.{i}.ndjson")));
    for i in (1..=KEEP).rev() {
        let from = if i == 1 { file.to_path_buf() } else { at(i - 1) };
        let _ = std::fs::rename(from, at(i));
    }
}

fn level_rank(level: &str) -> i64 {
    LEVELS.iter().position(|l| *l == level).map_or(-1, |i| i as i64)
}

// /^[a-z]+(\.\d+)?\.ndjson$/
fn log_file_name(n: &str) -> bool {
    let Some(stem) = n.strip_suffix(".ndjson") else { return false };
    let (name, num) = stem.split_once('.').unwrap_or((stem, ""));
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_lowercase()) && (stem.find('.').is_none() || (!num.is_empty() && num.bytes().all(|c| c.is_ascii_digit())))
}

/// The entries in `dir`, newest first: since `since` (ms), at `level` or
/// worse, of a component or a code (a prefix: doctor. is every check's).
pub fn read_logs(dir: &Path, since: f64, level: &str, component: Option<&str>, code: Option<&str>, limit: usize) -> Vec<Value> {
    let worst = level_rank(level);
    let Ok(names) = read_dir_sorted(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for n in names.iter().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| log_file_name(n)) {
        if component.is_some_and(|c| !n.starts_with(&format!("{c}."))) {
            continue;
        }
        let Ok(text) = std::fs::read(dir.join(&n)) else { continue };
        let text = String::from_utf8_lossy(&text);
        for line in text.split('\n') {
            if line.is_empty() {
                continue;
            }
            let Ok(e) = json::parse(line) else { continue };
            if e.get("schema").and_then(Value::as_str) != Some("log/entry/v1") || e.get("at").and_then(Value::as_f64).is_some_and(|at| at < since) {
                continue;
            }
            if level_rank(e.get("level").and_then(Value::as_str).unwrap_or("")) > worst {
                continue;
            }
            if code.is_some_and(|c| !e.get("code").map_or("undefined".to_string(), crate::text::js_string).starts_with(c)) {
                continue;
            }
            out.push(e);
        }
    }
    let at = |e: &Value| e.get("at").and_then(Value::as_f64).unwrap_or(f64::NAN);
    out.sort_by(|a, b| at(b).partial_cmp(&at(a)).unwrap_or(std::cmp::Ordering::Equal));
    out.truncate(limit);
    out
}

/// Entries by what they are: each code once, with how often, when first
/// and last, and its latest words and fix; the worst and most recent first.
pub fn summarize(entries: &[Value]) -> Vec<Value> {
    let mut by: OrderedMap<Value> = OrderedMap::new();
    let s = |e: &Value, k: &str| e.get(k).cloned().unwrap_or(Value::Null);
    let n = |e: &Value, k: &str| e.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    for e in entries {
        let key = format!("{}|{}", crate::text::js_string(&s(e, "component")), crate::text::js_string(&s(e, "code")));
        let count = 1.0 + n(e, "repeats");
        match by.get_mut(&key) {
            None => {
                let mut m = Map::new();
                for k in ["code", "component", "level"] {
                    m.insert(k.into(), s(e, k));
                }
                m.insert("count".into(), json::number(count));
                m.insert("first".into(), s(e, "at"));
                m.insert("last".into(), s(e, "at"));
                m.insert("message".into(), s(e, "message"));
                m.insert("fix".into(), s(e, "fix"));
                m.insert("version".into(), s(e, "version"));
                by.insert(&key, Value::Object(m));
            }
            Some(had) => {
                had["count"] = json::number(n(had, "count") + count);
                had["first"] = json::number(n(had, "first").min(n(e, "at")));
                if n(e, "at") > n(had, "last") {
                    had["last"] = s(e, "at");
                    had["message"] = s(e, "message");
                    if let Some(f) = e.get("fix").filter(|f| !f.is_null()) {
                        had["fix"] = f.clone();
                    }
                    if let Some(v) = e.get("version").filter(|v| !v.is_null()) {
                        had["version"] = v.clone();
                    }
                }
                if level_rank(had["level"].as_str().unwrap_or("")) > level_rank(e.get("level").and_then(Value::as_str).unwrap_or("")) {
                    had["level"] = s(e, "level");
                }
            }
        }
    }
    let mut rows: Vec<Value> = by.iter().map(|(_, v)| v.clone()).collect();
    rows.sort_by(|a, b| level_rank(a["level"].as_str().unwrap_or("")).cmp(&level_rank(b["level"].as_str().unwrap_or(""))).then(n(b, "last").total_cmp(&n(a, "last"))));
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "C:\\Users\\nova";

    #[test]
    fn scrubs_homes_and_keys() {
        assert_eq!(
            scrub("C:\\Users\\nova\\Projects\\atlas and C:/Users/nova/.codex and C--Users-nova-Projects-atlas", HOME),
            "~\\Projects\\atlas and ~/.codex and ~-Projects-atlas"
        );
        assert_eq!(scrub("token sk-proj-CANARY0123456789abcdefABCDEF and Bearer abcdefgh12345678 and ghp_ABCDEF1234567890", HOME), "token [key] and [key] and [key]");
        assert_eq!(scrub("bell\u{7} and tab\tkept\nand lines", HOME), "bell and tab\tkept\nand lines");
        assert_eq!(unsafe_text("C:\\Users\\nova\\x", HOME), Some("the home folder"));
        assert_eq!(unsafe_text("~\\x [key]", HOME), None);
        assert_eq!(js_slice("a😀b", 2), "a", "half a pair is left out");
    }
}
