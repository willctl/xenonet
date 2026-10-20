//! core/integrations/common.js: what every integration shares. Text
//! cleaned of anything that reads as a credential, previews' tickets that
//! expire and work once, and the records of decisions and sandbox jobs,
//! on disk or in memory, with their retention and queries.

use crate::b64::uuid;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_app::fsx;
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::NATIVE;
use xenonet_base::text::js_string;

/// An error with its HTTP status and code (IntegrationError).
#[derive(Clone, Debug, PartialEq)]
pub struct Fail {
    pub status: u16,
    pub code: String,
    pub message: String,
}

pub fn fail<T>(message: impl Into<String>) -> Result<T, Fail> {
    failed(message, "bad-request", 400)
}

pub fn failed<T>(message: impl Into<String>, code: &str, status: u16) -> Result<T, Fail> {
    Err(Fail { status, code: code.into(), message: message.into() })
}

impl From<fsx::FsError> for Fail {
    fn from(e: fsx::FsError) -> Self {
        // A system error the JS let through: its message, with its code.
        Fail { status: 0, code: e.code.unwrap_or_default(), message: e.message }
    }
}

// JS's \s, as a class's contents.
pub const SPACE: &str = r"\t\n\x0B\x0C\r \u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}";

fn secrets() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(r"(?i-u:sk-)[A-Za-z0-9_-]{{16,}}|(?i-u:bearer)[{SPACE}]+[^{SPACE}]+|(?i-u:password|api[_-]?key|token|secret)[{SPACE}]*[=:][{SPACE}]*[^{SPACE},;]+"))
            .expect("a fixed pattern")
    })
}

/// Whether a task carries something that reads as a credential.
pub fn has_credential(text: &str) -> bool {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(&format!(r"(?i-u:sk-)[A-Za-z0-9_-]{{16,}}|(?i-u:bearer)[{SPACE}]+[^{SPACE}]+|(?i-u:password|api[_-]?key|token|secret)[{SPACE}]*[=:]")).expect("a fixed pattern")
    })
    .is_match(text)
}

/// clean(value, max): a value as text, credentials as `[redacted]`, control
/// characters as spaces, at most `max` UTF-16 units.
pub fn clean(value: Option<&Value>, max: usize) -> String {
    let s = value.filter(|v| !v.is_null()).map_or_else(String::new, js_string);
    let s = secrets().replace_all(&s, "[redacted]");
    let s: String = s.chars().map(|c| if (c as u32) < 0x20 || c == '\u{7f}' { ' ' } else { c }).collect();
    xenonet_app::jstext::slice16(&s, max)
}

/// text(value, name, max): text, trimmed, of at most `max` units, or why not.
pub fn text(value: Option<&Value>, name: &str, max: usize) -> Result<String, Fail> {
    match value {
        Some(Value::String(s)) if !xenonet_base::text::js_trim(s).is_empty() && xenonet_app::jstext::utf16_len(s) <= max && !s.contains('\0') => {
            Ok(xenonet_base::text::js_trim(s).to_string())
        }
        _ => fail(format!("{name} must be text, at most {max} characters.")),
    }
}

/// A job with no sandbox behind it: deleted, or never made.
pub fn sandbox_gone(r: &Value) -> bool {
    matches!(r.get("status").and_then(Value::as_str), Some("deleted" | "not-created"))
}

// ---------- tickets ----------

const TICKET_MS: f64 = 10.0 * 60e3;
const TICKETS: usize = 32;

/// Previews' tickets: reviewed inputs kept in memory only, that expire and
/// can be used once.
#[derive(Default)]
pub struct Tickets {
    held: Vec<(String, Value, f64)>,
}

impl Tickets {
    pub fn check(&mut self, now: f64) -> Result<(), Fail> {
        self.held.retain(|(_, _, until)| *until > now);
        if self.held.len() >= TICKETS {
            return failed("Close an old preview before making another.", "busy", 429);
        }
        Ok(())
    }

    /// { ticket, expires }.
    pub fn issue(&mut self, value: Value, now: f64) -> Result<Value, Fail> {
        self.check(now)?;
        let (ticket, expires) = (uuid(), now + TICKET_MS);
        self.held.push((ticket.clone(), value, expires));
        Ok(json!({ "ticket": ticket, "expires": xenonet_base::json::number(expires) }))
    }

    pub fn take(&mut self, id: &str, now: f64) -> Result<Value, Fail> {
        let at = self.held.iter().position(|(t, _, _)| t == id);
        let held = at.map(|i| self.held.remove(i));
        match held {
            Some((_, v, until)) if until > now => Ok(v),
            _ => failed("This preview expired or was already used. Preview again.", "expired", 409),
        }
    }
}

// ---------- records ----------

#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's pattern takes .json in lower case only")]
fn record_name(n: &str) -> bool {
    n.len() == 41 && n.ends_with(".json") && n[..36].bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || c == b'-')
}

fn newest_first(a: &Value, b: &Value) -> std::cmp::Ordering {
    let at = |r: &Value| r.get("at").and_then(Value::as_f64).unwrap_or(f64::NAN);
    let d = at(b) - at(a);
    if let Some(o) = d.partial_cmp(&0.0).filter(|o| o.is_ne()) {
        return o;
    }
    xenonet_app::jstext::locale_compare(&js_string(&a["id"]), &js_string(&b["id"]))
}

/// The records of decisions and sandbox jobs: a folder of one JSON file
/// each, readable by you alone, or memory.
pub struct Records {
    dir: Option<String>,
    limit: usize,
    memory: Vec<Value>,
}

impl Records {
    pub fn new(dir: Option<String>, limit: usize) -> Result<Self, Fail> {
        if let Some(d) = &dir {
            fsx::mkdir_p(d, Some(0o700))?;
        }
        Ok(Records { dir, limit, memory: Vec::new() })
    }

    pub fn list(&self) -> Result<Vec<Value>, Fail> {
        let Some(dir) = &self.dir else {
            let mut all = self.memory.clone();
            xenonet_base::sort::sort_by(&mut all, newest_first);
            return Ok(all);
        };
        let mut out = Vec::new();
        for name in fsx::read_dir_names(dir)?.into_iter().filter(|n| record_name(n)) {
            let file = NATIVE.join(&[dir, &name]);
            let Ok(st) = std::fs::symlink_metadata(&file) else { continue };
            if !st.is_file() || st.len() > 64 * 1024 {
                continue;
            }
            let Ok(text) = fsx::read_text(&file) else { continue };
            let Ok(r) = xenonet_base::json::parse(&text) else { continue };
            if r.get("id").and_then(Value::as_str) != Some(&name[..36]) || !r.get("at").and_then(Value::as_f64).is_some_and(f64::is_finite) {
                continue;
            }
            out.push(r);
        }
        xenonet_base::sort::sort_by(&mut out, newest_first);
        Ok(out)
    }

    fn save(&mut self, record: Value) -> Result<Value, Fail> {
        let id = js_string(&record["id"]);
        match &self.dir {
            Some(dir) => {
                let tmp = NATIVE.join(&[dir, &format!("{id}.{}.tmp", uuid())]);
                let r = write_new(&tmp, xenonet_base::json::stringify(&record, 0).as_bytes(), 0o600).and_then(|()| fsx::rename(&tmp, &NATIVE.join(&[dir, &format!("{id}.json")])));
                let _ = std::fs::remove_file(&tmp);
                r?;
            }
            None => match self.memory.iter_mut().find(|r| r["id"] == record["id"]) {
                Some(slot) => *slot = record.clone(),
                None => self.memory.push(record.clone()),
            },
        }
        Ok(record)
    }

    pub fn add(&mut self, value: &Value, now: f64) -> Result<Value, Fail> {
        let mut m: Map<String, Value> = value.as_object().cloned().unwrap_or_default();
        m.insert("id".into(), json!(uuid()));
        m.insert("at".into(), xenonet_base::json::number(now));
        let record = self.save(Value::Object(m))?;
        // A saved live job is the owner's way to inspect or delete its
        // remote sandbox: retention never orphans it.
        let completed: Vec<Value> =
            self.list()?.into_iter().filter(|r| !truthy(r.get("reviewId")) && (r.get("kind").and_then(Value::as_str) != Some("sandbox") || sandbox_gone(r))).collect();
        for old in completed.iter().skip(self.limit) {
            let id = js_string(&old["id"]);
            match &self.dir {
                Some(dir) => fsx::unlink(&NATIVE.join(&[dir, &format!("{id}.json")]))?,
                None => self.memory.retain(|r| r["id"] != old["id"]),
            }
        }
        Ok(record)
    }

    pub fn update(&mut self, id: &Value, fields: &Value) -> Result<Value, Fail> {
        let Some(record) = self.list()?.into_iter().find(|r| xenonet_app::jsv::strict_eq(r.get("id"), Some(id))) else { return failed("No such record.", "missing", 404) };
        let mut m: Map<String, Value> = record.as_object().cloned().unwrap_or_default();
        for (k, v) in fields.as_object().into_iter().flatten() {
            m.insert(k.clone(), v.clone());
        }
        m.insert("id".into(), record["id"].clone());
        m.insert("at".into(), record["at"].clone());
        self.save(Value::Object(m))
    }
}

// fs.writeFileSync(p, data, { flag: "wx", mode }).
pub fn write_new(p: &str, data: &[u8], mode: u32) -> Result<(), fsx::FsError> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = o.open(p).map_err(|e| fsx::FsError::of(e, "open", p))?;
    f.write_all(data).map_err(|e| fsx::FsError::of(e, "write", p))
}

/// queryRecords: a page of fifty, by words, kind and status.
pub fn query(records: &[Value], q: &Value) -> Value {
    let words: Vec<String> = q
        .get("q")
        .filter(|v| !v.is_null())
        .map_or_else(String::new, js_string)
        .to_lowercase()
        .split(xenonet_base::text::is_js_space)
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect();
    let kind = q.get("kind").filter(|v| truthy(Some(v)));
    let status = q.get("status").filter(|v| truthy(Some(v)));
    let kept: Vec<&Value> = records.iter().filter(|r| kind.is_none_or(|k| xenonet_app::jsv::strict_eq(r.get("kind"), Some(k)))).collect();
    let mut found: Vec<Value> = kept
        .iter()
        .filter(|r| status.is_none_or(|s| xenonet_app::jsv::strict_eq(r.get("status"), Some(s))))
        .filter(|r| {
            let text = xenonet_base::json::stringify(r, 0).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        })
        .map(|r| (*r).clone())
        .collect();
    if q.get("sort").and_then(Value::as_str) == Some("oldest") {
        xenonet_base::sort::sort_by(&mut found, |a, b| {
            let at = |r: &Value| r.get("at").and_then(Value::as_f64).unwrap_or(f64::NAN);
            let d = at(a) - at(b);
            if let Some(o) = d.partial_cmp(&0.0).filter(|o| o.is_ne()) {
                return o;
            }
            xenonet_app::jstext::locale_compare(&js_string(&a["id"]), &js_string(&b["id"]))
        });
    } else {
        xenonet_base::sort::sort_by(&mut found, newest_first);
    }
    let pages = (found.len() as f64 / 50.0).ceil();
    let given = q.get("page").map_or(1.0, |p| xenonet_base::text::js_number(Some(p)));
    let given = if given.is_nan() || given == 0.0 { 1.0 } else { given };
    let page = (if pages == 0.0 { 1.0 } else { pages }).min(given.floor()).max(1.0);
    let start = ((page - 1.0) * 50.0) as usize;
    let shown: Vec<Value> = found.iter().skip(start).take(50).cloned().collect();
    json!({ "total": kept.len(), "matched": found.len(), "page": xenonet_base::json::number(page), "pages": xenonet_base::json::number(pages), "records": shown })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_go_as_redacted() {
        assert_eq!(clean(Some(&json!("use sk-abcdefghijklmnop1 and Bearer x.y and TOKEN = s3cr3t,ok")), 160), "use [redacted] and [redacted] and [redacted],ok");
        assert_eq!(clean(Some(&json!("a\u{1}b")), 2), "a ");
        assert_eq!(clean(None, 9), "");
        assert!(has_credential("password: x"));
        assert!(text(Some(&json!("  ")), "Task", 10).is_err());
    }

    #[test]
    fn tickets_expire_and_work_once() {
        let mut t = Tickets::default();
        let a = t.issue(json!("private"), 0.0).unwrap();
        let id = a["ticket"].as_str().unwrap().to_string();
        assert_eq!(t.take(&id, 1.0).unwrap(), json!("private"));
        assert!(t.take(&id, 1.0).is_err());
    }
}
