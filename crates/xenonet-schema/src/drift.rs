//! Drift (core/drift.js): where the vendor data XenoNet reads differs from
//! XenoNet's schemas for it (contracts, section 1). The vendor readers find
//! it; this keeps it, as drift/record/v1 records, in drift.json in the cache
//! folder (a drift/log/v1).
//!
//! A record holds shapes, never values: where in the record (`path`), and
//! the types found there (`shape`), at most four levels deep. The only
//! values ever kept are a declared discriminator's and vendor versions. Keys
//! that read as data make their object a record<…>, so they're never kept.
//!
//! The same drift seen again is the same record, counted: a new or missing
//! field by its path and variant, a type change by its path, variant and
//! the type found, a new variant by its value, a new version by the
//! version, lines that aren't JSON by source. Several processes can share
//! the file: each adds the counts it saw since it last wrote, and
//! `looked_at` only moves forward.

use crate::fields::is_field;
use crate::lenient::json_type;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::Path;
use xenonet_base::json;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::text::js_string;

pub const DRIFT_MAX: usize = 500;
pub const DRIFT_FILE: &str = "drift.json";
const FILE_VERSION: u32 = 1;

pub const KINDS: [&str; 6] = ["new-field", "missing-field", "type-change", "new-variant", "new-version", "unparsable"];
/// Kinds that can't break what XenoNet reads: something was added.
pub const ADDITIONS: [&str; 2] = ["new-field", "new-variant"];

/// What each source is, in words.
pub const SOURCES: &[(&str, &str)] = &[
    ("anthropic/claude-code/transcript-line", "Claude Code's transcripts"),
    ("anthropic/claude-code/config", "Claude Code's ~/.claude.json"),
    ("anthropic/claude-desktop/session", "Claude desktop's session files"),
    ("anthropic/claude-desktop/plan-usage", "Claude desktop's plan usage"),
    ("anthropic/agent-sdk/session-listing", "the Agent SDK's session list"),
    ("anthropic/agent-sdk/session-message", "the Agent SDK's session messages"),
    ("anthropic/oauth/usage", "Anthropic's plan usage answer"),
    ("openai/codex/rollout-line", "Codex's rollouts"),
    ("openai/codex/app-server", "Codex's app-server"),
    ("openai/codex/state-db", "Codex's state database"),
    ("openai/codex/auth-claims", "Codex's sign-in"),
    ("github/gh/hosts", "gh's hosts.yml"),
    ("nous/hermes-agent/state-db", "Hermes Agent's session database"),
    ("nous/hermes-agent/spawn-ledger", "Hermes Agent's spawn ledger"),
    ("nous/hermes-agent/gateway-state", "Hermes Agent's gateway state"),
    ("nous/hermes-agent/processes", "Hermes Agent's background processes"),
    ("nous/hermes-agent/cron-jobs", "Hermes Agent's scheduled jobs"),
    ("nous/hermes-agent/cron-ticker", "Hermes Agent's scheduler ticker"),
    ("nous/hermes-agent/install-stamp", "Hermes Agent's build stamp"),
    ("anomaly/opencode/db", "OpenCode's database"),
    ("google/gemini-cli/chat-record", "Gemini CLI's chat records"),
    ("github/copilot-cli/session-event", "Copilot CLI's session events"),
    ("anysphere/cursor/hook-input", "Cursor's hook input"),
];

pub fn source_name(source: &str) -> &str {
    SOURCES.iter().find(|(s, _)| *s == source).map_or(source, |(_, n)| n)
}

// ---------- shapes ----------

const MAX_KEYS: usize = 32;
const MAX_ITEMS: usize = 8;
const MAX_ALTERNATIVES: usize = 4;

// JS's .length: UTF-16 code units.
fn units(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

fn union(shapes: Vec<String>) -> String {
    let mut distinct: Vec<String> = Vec::new();
    for s in shapes {
        if !distinct.contains(&s) {
            distinct.push(s);
        }
    }
    // JS's default sort: by UTF-16 code units.
    distinct.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    if distinct.len() > MAX_ALTERNATIVES {
        let mut first: Vec<String> = distinct.into_iter().take(MAX_ALTERNATIVES).collect();
        first.push("…".into());
        first.join("|")
    } else {
        distinct.join("|")
    }
}

/// The types in `value`, at most `depth` levels of objects and arrays deep,
/// and at most about `budget` characters long, key names included. Without
/// `names`, no key's name: every object is a record<…> of its values' types.
pub fn shape_of(value: &Value, depth: i64, budget: i64, names: bool) -> String {
    let mut left = budget;
    fn walk(v: &Value, d: i64, names: bool, left: &mut i64) -> String {
        if *left <= 0 {
            return "…".into();
        }
        let s = match v {
            Value::Array(a) => {
                if d <= 0 {
                    "array".into()
                } else {
                    let inner: Vec<String> = a.iter().take(MAX_ITEMS).map(|x| walk(x, d - 1, names, left)).collect();
                    format!("array<{}>", union(inner))
                }
            }
            Value::Object(m) => {
                let keys: Vec<&String> = m.keys().collect();
                if d <= 0 {
                    "object".into()
                } else if !names || keys.len() > MAX_KEYS || keys.iter().any(|k| !is_field(k)) {
                    let inner: Vec<String> = keys.iter().take(MAX_ITEMS).map(|k| walk(&m[k.as_str()], d - 1, names, left)).collect();
                    format!("record<{}>", union(inner))
                } else {
                    let mut sorted = keys.clone();
                    sorted.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
                    let mut parts = Vec::new();
                    for k in sorted {
                        if *left <= 0 {
                            parts.push("…".to_string());
                            break;
                        }
                        // The name, its colon and the comma after it.
                        *left -= units(k) + 2;
                        let inner = walk(&m[k.as_str()], d - 1, names, left);
                        parts.push(format!("{k}:{inner}"));
                    }
                    format!("object{{{}}}", parts.join(","))
                }
            }
            other => json_type(other).to_string(),
        };
        *left -= units(&s);
        s
    }
    walk(value, depth, names, &mut left)
}

/// A discriminator's value as a shape: the value itself when it's a short
/// name, else only its type.
pub fn variant_shape(value: &Value) -> String {
    match value {
        Value::String(s) => {
            let short = (1..=64).contains(&s.encode_utf16().count()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c));
            if short {
                json::stringify(value, 0)
            } else {
                "string".into()
            }
        }
        Value::Number(_) | Value::Bool(_) => json::stringify(value, 0),
        other => json_type(other).into(),
    }
}

// /^(?:\d+|(?:alpha|beta|rc|pre|preview|dev|post|canary|nightly|next|snapshot|stable|build|sha|a|b|c)\d*|g?[0-9a-f]{7,40})$/i
fn tag(part: &str) -> bool {
    if !part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let lower = part.to_ascii_lowercase();
    const WORDS: [&str; 16] = ["alpha", "beta", "rc", "pre", "preview", "dev", "post", "canary", "nightly", "next", "snapshot", "stable", "build", "sha", "a", "b"];
    for w in WORDS.iter().chain(std::iter::once(&"c")) {
        if let Some(rest) = lower.strip_prefix(w) {
            if rest.bytes().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }
    let hex = lower.strip_prefix('g').filter(|h| (7..=40).contains(&h.len()) && h.bytes().all(|c| c.is_ascii_hexdigit()));
    hex.is_some() || ((7..=40).contains(&lower.len()) && lower.bytes().all(|c| c.is_ascii_hexdigit()))
}

/// A vendor version as it's kept: one that reads as a version (2.1.288,
/// 0.155.0-alpha.9.2, 0.22.0rc1, 0.22.0.dev0, 31), or None. What follows
/// its numbers is numbers, the usual tags and commit hashes only, since
/// anything else there (a branch's name, a person's) is data.
pub fn clean_version(v: &str) -> Option<&str> {
    if v.encode_utf16().count() > 64 {
        return None;
    }
    // /^v?\d+(?:\.\d+){0,3}/
    let b = v.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'v'));
    let digits = |i: &mut usize| {
        let from = *i;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
        *i > from
    };
    if !digits(&mut i) {
        return None;
    }
    for _ in 0..3 {
        if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits(&mut i);
        } else {
            break;
        }
    }
    let rest = &v[i..];
    let ok = rest.split(['.', '+', '-']).enumerate().all(|(n, part)| if part.is_empty() { n == 0 } else { tag(part) });
    ok.then_some(v)
}

// ---------- records ----------

fn root_type(shape: Option<&str>) -> String {
    let s = shape.unwrap_or("");
    let t: String = s.chars().take_while(char::is_ascii_lowercase).collect();
    if t == "record" {
        "object".into()
    } else {
        t
    }
}

fn is_value(v: &Value) -> bool {
    matches!(v, Value::String(_) | Value::Number(_) | Value::Bool(_))
}

/// A variant as a record keeps it: a list of the discriminators' values, or
/// None for none (an empty list, or anything else).
pub fn clean_variant(variant: Option<&Value>) -> Option<&Vec<Value>> {
    match variant {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= 8 && a.iter().all(is_value) => Some(a),
        _ => None,
    }
}

fn variant_key(variant: Option<&Value>) -> String {
    clean_variant(variant).map_or(String::new(), |v| format!("|{}", json::stringify(&Value::Array(v.clone()), 0)))
}

// A value as a template literal writes it.
fn tpl(v: Option<&Value>) -> String {
    v.map_or("undefined".into(), js_string)
}

/// A piece of drift: what `record` takes, and what `key_of` reads.
#[derive(Clone, Debug, Default)]
pub struct Piece {
    pub source: String,
    pub kind: String,
    pub path: Value,
    pub variant: Value,
    pub shape: Value,
    pub type_: Value,
    pub vendor_version: Value,
}

impl Piece {
    fn of(r: &Value) -> Piece {
        let f = |k: &str| r.get(k).cloned().unwrap_or(Value::Null);
        Piece {
            source: tpl(r.get("source")),
            kind: tpl(r.get("kind")),
            path: f("path"),
            variant: f("variant"),
            shape: f("shape"),
            type_: f("type"),
            vendor_version: f("vendorVersion"),
        }
    }
}

/// Which record a piece of drift adds to. A type change is told apart by
/// the type found (`type`, or the shape's). One in a variant adds to that
/// variant's.
pub fn key_of(p: &Piece) -> String {
    let (src, kind) = (&p.source, &p.kind);
    let at = js_string(&p.path);
    let vk = variant_key(Some(&p.variant));
    match kind.as_str() {
        "new-version" => format!("{src}|{kind}|{}", js_string(&p.vendor_version)),
        "type-change" => {
            let t = if p.type_.is_null() { root_type(p.shape.as_str()) } else { js_string(&p.type_) };
            format!("{src}|{kind}|{at}|{t}{vk}")
        }
        "new-variant" => format!("{src}|{kind}|{at}|{}{vk}", js_string(&p.shape)),
        "unparsable" => format!("{src}|{kind}"),
        _ => format!("{src}|{kind}|{at}{vk}"),
    }
}

/// What a record says, in a few words.
pub fn describe(r: &Value) -> String {
    let s = |k: &str| r.get(k).map_or("undefined".to_string(), js_string);
    let wh = clean_variant(r.get("variant")).map_or(String::new(), |v| format!(", in {}", v.iter().map(js_string).collect::<Vec<_>>().join(" › ")));
    match r.get("kind").and_then(Value::as_str) {
        Some("new-field") => format!("{} is new ({}){wh}", s("path"), s("shape")),
        Some("missing-field") => format!("{} is missing{wh}", s("path")),
        Some("type-change") => format!("{} is {} now{wh}", r.get("path").filter(|p| !p.is_null()).map_or("a record".to_string(), js_string), s("shape")),
        Some("new-variant") => format!("{} {} isn't one the schema lists{wh}", s("path"), s("shape")),
        Some("new-version") => format!("version {} is outside the versions the schema was made for", s("vendorVersion")),
        Some("unparsable") => "lines that aren't JSON".into(),
        _ => s("kind"),
    }
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64).filter(|n| n.is_finite())
}

fn is_record(r: &Value) -> bool {
    r.is_object()
        && r.get("source").is_some_and(Value::is_string)
        && r.get("kind").and_then(Value::as_str).is_some_and(|k| KINDS.contains(&k))
        && finite(r.get("firstSeen")).is_some()
        && finite(r.get("lastSeen")).is_some()
        && r.get("count").and_then(Value::as_f64).is_some_and(|c| c.fract() == 0.0 && c > 0.0)
}

// The key a record in a variant had in a drift.json from before records
// said their variant: the same, without it.
fn legacy_key(r: &Value) -> Option<String> {
    clean_variant(r.get("variant"))?;
    let mut p = Piece::of(r);
    p.variant = Value::Null;
    Some(key_of(&p))
}

/// drift.json as it is: its records (what doesn't read as one left out)
/// and when you last looked; None when there's no file or it isn't one.
pub fn read_drift_file(file: &Path) -> Option<(Option<f64>, Vec<Value>)> {
    let text = std::fs::read_to_string(file).ok()?;
    let data = json::parse(&text).ok()?;
    let records = data.get("records")?.as_array()?;
    let records = records
        .iter()
        .filter(|r| is_record(r))
        .map(|r| {
            let mut r = r.clone();
            // A variant that isn't a list of values is no variant.
            if clean_variant(r.get("variant")).is_none() {
                if let Value::Object(m) = &mut r {
                    m.shift_remove("variant");
                }
            }
            r
        })
        .collect();
    Some((finite(data.get("lookedAt")), records))
}

struct Entry {
    record: Value,
    base: f64,
    delta: f64,
}

fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// What `DriftLog::view` gives: the log's file, when its drift was last
/// looked at, and its records.
pub type DriftView = (Option<String>, Option<f64>, Vec<Value>);

/// The drift XenoNet has seen, merged, at most `max` records (the ones seen
/// least lately go first), and kept in a file once `open` names one.
pub struct DriftLog {
    pub max: usize,
    records: OrderedMap<Entry>,
    retired: HashSet<String>,
    pub looked_at: Option<f64>,
    pub file: Option<String>,
    mtime: Option<f64>,
    pub dirty: bool,
}

impl Default for DriftLog {
    fn default() -> Self {
        DriftLog::new(DRIFT_MAX)
    }
}

impl DriftLog {
    pub fn new(max: usize) -> Self {
        DriftLog { max, records: OrderedMap::new(), retired: HashSet::new(), looked_at: None, file: None, mtime: None, dirty: false }
    }

    /// Whether this piece of drift adds to a record kept already (so a lazy
    /// shape needn't be worked out).
    pub fn knows(&self, p: &Piece) -> bool {
        let mut p = p.clone();
        p.variant = clean_variant(Some(&p.variant)).map_or(Value::Null, |v| Value::Array(v.clone()));
        if p.kind == "type-change" && !p.type_.is_null() {
            p.shape = Value::Null;
        }
        self.records.contains(&key_of(&p))
    }

    /// Adds a piece of drift at `now`, and gives back the record.
    pub fn record(&mut self, p: &Piece, now: f64) -> Value {
        let where_ = clean_variant(Some(&p.variant)).cloned();
        let key_piece = Piece { variant: where_.clone().map_or(Value::Null, Value::Array), ..p.clone() };
        let key = key_of(&key_piece);
        let out = if let Some(e) = self.records.get_mut(&key) {
            let last = num(&e.record, "lastSeen").max(now);
            e.record["lastSeen"] = json::number(last);
            e.delta += 1.0;
            e.record["count"] = json::number(e.base + e.delta);
            e.record.clone()
        } else {
            let mut r = Map::new();
            r.insert("schema".into(), json!("drift/record/v1"));
            r.insert("source".into(), json!(p.source));
            r.insert("vendorVersion".into(), p.vendor_version.as_str().and_then(clean_version).map_or(Value::Null, |v| Value::String(v.into())));
            r.insert("kind".into(), json!(p.kind));
            r.insert("path".into(), p.path.clone());
            if let Some(v) = &where_ {
                r.insert("variant".into(), Value::Array(v.clone()));
            }
            r.insert("shape".into(), p.shape.clone());
            r.insert("firstSeen".into(), json::number(now));
            r.insert("lastSeen".into(), json::number(now));
            r.insert("count".into(), json!(1));
            let record = Value::Object(r);
            let e = self.adopt(&record, &p.type_).unwrap_or(Entry { record, base: 0.0, delta: 1.0 });
            let out = e.record.clone();
            self.records.insert(&key, e);
            self.prune();
            out
        };
        self.dirty = true;
        out
    }

    // A record that says its variant takes the place of the same drift kept
    // before records said theirs, with its history.
    fn adopt(&mut self, record: &Value, type_: &Value) -> Option<Entry> {
        let mut with_type = record.clone();
        with_type["type"] = type_.clone();
        let old = legacy_key(&with_type)?;
        let e = self.records.get(&old)?;
        if e.record.get("variant").is_some() {
            return None;
        }
        let e = self.records.remove(&old)?;
        self.retired.insert(old);
        let r = &e.record;
        let mut m = Map::new();
        for k in ["schema", "source", "vendorVersion", "kind", "path"] {
            m.insert(k.into(), r.get(k).cloned().unwrap_or(Value::Null));
        }
        m.insert("variant".into(), record["variant"].clone());
        for k in ["shape", "firstSeen"] {
            m.insert(k.into(), r.get(k).cloned().unwrap_or(Value::Null));
        }
        m.insert("lastSeen".into(), json::number(num(r, "lastSeen").max(num(record, "lastSeen"))));
        m.insert("count".into(), json::number(e.base + e.delta + 1.0));
        Some(Entry { record: Value::Object(m), base: e.base, delta: e.delta + 1.0 })
    }

    /// Every record, oldest first.
    pub fn list(&self) -> Vec<Value> {
        let mut all: Vec<&Value> = self.records.iter().map(|(_, e)| &e.record).collect();
        all.sort_by(|a, b| num(a, "firstSeen").total_cmp(&num(b, "firstSeen")).then(num(a, "lastSeen").total_cmp(&num(b, "lastSeen"))));
        all.into_iter().cloned().collect()
    }

    /// You've looked at the drift there is: what comes after is new.
    pub fn looked(&mut self, at: f64) {
        self.looked_at = Some(self.looked_at.unwrap_or(0.0).max(at));
        self.dirty = true;
    }

    fn prune(&mut self) {
        if self.records.len() <= self.max {
            return;
        }
        let mut oldest: Vec<(String, f64, f64)> = self.records.iter().map(|(k, e)| (k.to_string(), num(&e.record, "lastSeen"), num(&e.record, "firstSeen"))).collect();
        oldest.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.2.total_cmp(&b.2)));
        let n = self.records.len() - self.max;
        for (k, _, _) in oldest.into_iter().take(n) {
            self.records.remove(&k);
        }
    }

    /// Keeps the drift in `file` from now on, with what's there already.
    pub fn open(&mut self, file: &str) {
        self.file = Some(file.to_string());
        self.mtime = None;
        self.sync(false);
    }

    /// What the file holds now, merged in: its counts are the base this
    /// process's own add to.
    pub fn sync(&mut self, force: bool) {
        let Some(file) = self.file.clone() else { return };
        let Ok(meta) = std::fs::metadata(&file) else { return };
        let mtime = xenonet_base::fs::mtime_ms_of(&meta).ok();
        if !force && self.mtime.is_some() && self.mtime == mtime {
            return;
        }
        let data = read_drift_file(Path::new(&file));
        self.mtime = mtime;
        let Some((looked_at, records)) = data else { return };
        if let Some(l) = looked_at {
            self.looked_at = Some(self.looked_at.unwrap_or(0.0).max(l));
        }
        for r in records {
            let key = key_of(&Piece::of(&r));
            if self.retired.contains(&key) {
                continue;
            }
            if let Some(old) = legacy_key(&r) {
                self.retired.insert(old.clone());
                self.records.remove(&old);
            }
            let count = num(&r, "count");
            match self.records.get_mut(&key) {
                None => self.records.insert(&key, Entry { record: r, base: count, delta: 0.0 }),
                Some(e) => {
                    let first = num(&r, "firstSeen") < num(&e.record, "firstSeen");
                    e.record["firstSeen"] = json::number(num(&e.record, "firstSeen").min(num(&r, "firstSeen")));
                    e.record["lastSeen"] = json::number(num(&e.record, "lastSeen").max(num(&r, "lastSeen")));
                    if first {
                        e.record["vendorVersion"] = r.get("vendorVersion").cloned().unwrap_or(Value::Null);
                        e.record["shape"] = r.get("shape").cloned().unwrap_or(Value::Null);
                    }
                    e.base = count;
                    e.record["count"] = json::number(e.base + e.delta);
                }
            }
        }
        self.prune();
    }

    /// Writes what's changed, merged with what other processes wrote since:
    /// None, or why it couldn't.
    pub fn flush(&mut self) -> Option<String> {
        let file = self.file.clone()?;
        if !self.dirty {
            return None;
        }
        self.sync(true);
        let doc = json!({ "version": FILE_VERSION, "lookedAt": self.looked_at.map(json::number), "records": self.list() });
        let tmp = format!("{file}.{}.tmp", std::process::id());
        let wrote = (|| -> std::io::Result<()> {
            if let Some(dir) = Path::new(&file).parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&tmp, json::stringify(&doc, 1) + "\n")?;
            std::fs::rename(&tmp, &file)?;
            Ok(())
        })();
        if let Err(e) = wrote {
            return Some(xenonet_base::fs::FsError::of(e, "open", &tmp).message);
        }
        self.mtime = std::fs::metadata(&file).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok());
        for (_, e) in self.records.iter_mut() {
            e.base = num(&e.record, "count");
            e.delta = 0.0;
        }
        self.dirty = false;
        None
    }

    /// The drift as the doctor and the report see it, with what other
    /// processes wrote since.
    pub fn view(&mut self) -> DriftView {
        self.sync(false);
        (self.file.clone(), self.looked_at, self.list())
    }

    /// Forgets everything (tests).
    pub fn clear(&mut self) {
        self.records.clear();
        self.retired.clear();
        self.looked_at = None;
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes() {
        assert_eq!(shape_of(&json!({ "text": "x", "n": 1, "list": [1, "a", null] }), 4, 400, true), "object{list:array<null|number|string>,n:number,text:string}");
        assert_eq!(shape_of(&json!({ "/Users/x": 1 }), 4, 400, true), "record<number>");
        assert_eq!(shape_of(&json!({ "a": { "b": { "c": { "d": { "e": 1 } } } } }), 4, 400, true), "object{a:object{b:object{c:object{d:object}}}}");
        assert_eq!(variant_shape(&json!("assistant")), "\"assistant\"");
        assert_eq!(variant_shape(&json!("has space")), "string");
    }

    #[test]
    fn versions() {
        for v in ["2.1.288", "0.155.0-alpha.9.2", "0.22.0rc1", "0.22.0.dev0", "31", "v1.2", "1.0.0+g1a2b3c4"] {
            assert_eq!(clean_version(v), Some(v), "{v}");
        }
        for v in ["main", "1.0.0-my-branch", "1.0.0-", "1..2", "x1"] {
            assert_eq!(clean_version(v), None, "{v}");
        }
    }

    #[test]
    fn records_count_and_merge() {
        let mut log = DriftLog::new(2);
        let p = Piece { source: "s".into(), kind: "new-field".into(), path: json!("a.b"), shape: json!("number"), vendor_version: json!("1.0.0"), ..Default::default() };
        log.record(&p, 10.0);
        let r = log.record(&p, 20.0);
        assert_eq!((r["count"].clone(), r["firstSeen"].clone(), r["lastSeen"].clone()), (json!(2), json!(10), json!(20)));
        for (i, path) in ["c", "d"].iter().enumerate() {
            log.record(&Piece { path: json!(path), ..p.clone() }, 30.0 + i as f64);
        }
        assert_eq!(log.list().len(), 2, "the one seen least lately went");
        assert_eq!(describe(&log.list()[0]), "c is new (number)");
    }
}
