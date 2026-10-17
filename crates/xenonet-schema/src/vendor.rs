//! Vendor data, read leniently (core/vendor.js; contracts, section 1). Each
//! source XenoNet reads has a reader, which checks its records against the
//! source's schema, picked by the version the data carries, and keeps what
//! differs as drift:
//!
//! - a record that doesn't fit is used for the fields that do: a field of
//!   another type is taken out, so the reader sees it as missing;
//! - a record is dropped only when nothing can be read from it (it isn't an
//!   object);
//! - every difference is a drift record, and so is a version outside every
//!   range the source's schemas were made for (new-version).
//!
//! Records come by the hundred thousand, so a whole check is made of a
//! sample; the rest are only guarded: a type change found once in a kind of
//! record is looked for again in every record of that kind, along the path
//! it was found at.

use crate::drift::{clean_version, shape_of, variant_shape, DriftLog, Piece};
use crate::lenient::{self, json_type, Finding, Key, Major, SpecRef};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use xenonet_base::ordered::OrderedMap;

// ---------- versions ----------

/// A version: major, minor, patch and prerelease.
pub type Version = (f64, f64, f64, Option<String>);

/// "2.1.288" or "0.155.0-alpha.9.2" → (2, 1, 288, pre); a missing part is 0.
pub fn parse_version(v: &str) -> Option<Version> {
    // /^v?(\d+)(?:\.(\d+))?(?:\.(\d+))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/
    let s = xenonet_base::text::js_trim(v);
    let s = s.strip_prefix('v').unwrap_or(s);
    let ident = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-');
    let (main, build) = match s.split_once('+') {
        Some((m, b)) => (m, Some(b)),
        None => (s, None),
    };
    if build.is_some_and(|b| !ident(b)) {
        return None;
    }
    let (nums, pre) = match main.split_once('-') {
        Some((n, p)) => (n, Some(p)),
        None => (main, None),
    };
    if pre.is_some_and(|p| !ident(p)) {
        return None;
    }
    let parts: Vec<&str> = nums.split('.').collect();
    if parts.is_empty() || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let n = |i: usize| parts.get(i).map_or(0.0, |p| p.parse::<f64>().unwrap_or(0.0));
    Some((n(0), n(1), n(2), pre.map(str::to_string)))
}

// JS's localeCompare for the ASCII identifiers a prerelease has.
fn locale_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| b.cmp(a))
}

fn compare_pre(a: &Option<String>, b: &Option<String>) -> Ordering {
    match (a, b) {
        (x, y) if x == y => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let (x, y): (Vec<&str>, Vec<&str>) = (a.split('.').collect(), b.split('.').collect());
            for i in 0..x.len().max(y.len()) {
                let (Some(p), Some(q)) = (x.get(i), y.get(i)) else { return if x.get(i).is_none() { Ordering::Less } else { Ordering::Greater } };
                let digits = |t: &str| !t.is_empty() && t.bytes().all(|c| c.is_ascii_digit());
                let c = match (digits(p), digits(q)) {
                    (true, true) => p.parse::<f64>().unwrap_or(0.0).total_cmp(&q.parse::<f64>().unwrap_or(0.0)),
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => locale_cmp(p, q),
                };
                if c != Ordering::Equal {
                    return c;
                }
            }
            Ordering::Equal
        }
    }
}

pub fn compare_versions(a: &Version, b: &Version) -> Ordering {
    a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.total_cmp(&b.2)).then_with(|| compare_pre(&a.3, &b.3))
}

type Comparator = (&'static str, Version);

// A range as comparators to meet, one set per || alternative: "*",
// ">=1.0.0 <3.0.0", "<1.0.0", "2" (any 2.x.y), "2.1", "^0.3.0", "~2.1.0".
fn parse_range(range: &str) -> Option<Vec<Vec<Comparator>>> {
    let mut sets = Vec::new();
    for alt in range.split("||") {
        let mut set = Vec::new();
        for part in alt.split_whitespace() {
            if part == "*" || part.eq_ignore_ascii_case("x") {
                continue;
            }
            let (op, rest) = [">=", "<=", ">", "<", "=", "^", "~"].iter().find_map(|o| part.strip_prefix(o).map(|r| (*o, r))).unwrap_or(("", part));
            let rest = rest.strip_prefix('v').unwrap_or(rest);
            let (nums, pre) = match rest.split_once('-') {
                Some((n, p)) if !p.is_empty() && p.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') => (n, Some(p.to_string())),
                Some(_) => return None,
                None => (rest, None),
            };
            let parts: Vec<&str> = nums.split('.').collect();
            if parts.is_empty() || parts.len() > 3 {
                return None;
            }
            let mut n: [Option<f64>; 3] = [None; 3];
            for (i, p) in parts.iter().enumerate() {
                if p.eq_ignore_ascii_case("x") || *p == "*" {
                    n[i] = None;
                } else if !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) {
                    n[i] = Some(p.parse().ok()?);
                } else {
                    return None;
                }
            }
            let given = n.iter().position(Option::is_none);
            let exact = given.is_none();
            let v: Version = (n[0].unwrap_or(0.0), n[1].unwrap_or(0.0), n[2].unwrap_or(0.0), pre);
            // What a partial version means: 2 is 2.0.0 up to 3.0.0, 2.1 up to 2.2.0.
            let bump = |i: usize| -> Option<Version> {
                match i {
                    0 => Some((v.0 + 1.0, 0.0, 0.0, None)),
                    1 => Some((v.0, v.1 + 1.0, 0.0, None)),
                    _ => None,
                }
            };
            let g = given.unwrap_or(usize::MAX);
            match op {
                "^" => {
                    let i = if v.0 > 0.0 || (!exact && g <= 1) {
                        0
                    } else if v.1 > 0.0 || (!exact && g == 2) {
                        1
                    } else {
                        2
                    };
                    let upper = if i == 2 { Some((v.0, v.1, v.2 + 1.0, None)) } else { bump(i) };
                    set.push((">=", v.clone()));
                    set.push(("<", upper?));
                }
                "~" => {
                    set.push((">=", v.clone()));
                    set.push(("<", bump(if exact || g > 1 { 1 } else { 0 })?));
                }
                "" | "=" if !exact => {
                    if g > 0 {
                        set.push((">=", v.clone()));
                        set.push(("<", bump(g - 1)?));
                    }
                }
                ">" if !exact => set.push((">=", bump(g.checked_sub(1)?)?)),
                "<=" if !exact => set.push(("<", bump(g.checked_sub(1)?)?)),
                o => set.push((if o.is_empty() { "=" } else { o }, v)),
            }
        }
        sets.push(set);
    }
    Some(sets)
}

pub fn valid_range(range: &str) -> bool {
    parse_range(range).is_some()
}

/// Whether `version` is in `range`. A prerelease counts as the version it
/// comes before (0.155.0-alpha is in "<1.0.0"), unlike npm's rule, since
/// vendors ship prereleases to the people XenoNet runs for.
pub fn satisfies(version: &str, range: &str) -> bool {
    let (Some(v), Some(sets)) = (parse_version(version), parse_range(range)) else { return false };
    sets.iter().any(|set| {
        set.iter().all(|(op, w)| {
            let c = compare_versions(&v, w);
            match *op {
                ">=" => c != Ordering::Less,
                ">" => c == Ordering::Greater,
                "<=" => c != Ordering::Greater,
                "<" => c == Ordering::Less,
                _ => c == Ordering::Equal,
            }
        })
    })
}

/// Codex's version, from the app-server's user agent: <client>/<version> (…).
pub fn codex_version_of(user_agent: &str) -> Option<String> {
    // /\/(\d+\.\d+\.\d+[\w.+-]*)/
    let b = user_agent.as_bytes();
    for (i, _) in user_agent.match_indices('/') {
        let mut j = i + 1;
        let mut dots = 0;
        let start = j;
        let mut ok = true;
        for part in 0..3 {
            let from = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            if j == from {
                ok = false;
                break;
            }
            if part < 2 {
                if b.get(j) != Some(&b'.') {
                    ok = false;
                    break;
                }
                j += 1;
                dots += 1;
            }
        }
        if !ok || dots != 2 {
            continue;
        }
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b"_.+-".contains(&b[j])) {
            j += 1;
        }
        return clean_version(&user_agent[start..j]).map(str::to_string);
    }
    None
}

/// Where JSON text first goes wrong, as (line, column), both from 1, the
/// column in UTF-16 units as JS counts it; None when it doesn't. This finds
/// the place itself, a value at a time, rather than quote the text (V8's
/// messages quote it, and a sign-in file's text can hold a token).
pub fn json_error_at(text: &str) -> Option<(usize, usize)> {
    let s: Vec<u16> = text.encode_utf16().collect();
    let n = s.len();
    let mut open: Vec<u16> = Vec::new();
    let mut i = 0usize;
    #[derive(PartialEq, Clone, Copy)]
    enum Want {
        Value,
        ValueOrEnd,
        Key,
        KeyOrEnd,
        After,
        End,
    }
    let mut want = Want::Value;
    let at = |i: usize| {
        let (mut line, mut start) = (1, 0);
        for (k, c) in s.iter().enumerate().take(i) {
            if *c == u16::from(b'\n') {
                line += 1;
                start = k + 1;
            }
        }
        Some((line, i - start + 1))
    };
    let space = |i: &mut usize| {
        while *i < n && [b' ', b'\t', b'\n', b'\r'].iter().any(|c| s[*i] == u16::from(*c)) {
            *i += 1;
        }
    };
    // A string, from its opening quote; false where it goes wrong.
    let string = |i: &mut usize| -> bool {
        *i += 1;
        while *i < n {
            let c = s[*i];
            if c == 0x22 {
                *i += 1;
                return true;
            }
            if c < 0x20 {
                return false;
            }
            if c == 0x5c {
                *i += 1;
                match s.get(*i).copied() {
                    Some(0x75) => {
                        let hex = s.get(*i + 1..*i + 5).is_some_and(|h| h.iter().all(|c| char::from_u32(u32::from(*c)).is_some_and(|c| c.is_ascii_hexdigit())));
                        if !hex {
                            return false;
                        }
                        *i += 4;
                    }
                    Some(c) if b"\"\\/bfnrt".iter().any(|x| u16::from(*x) == c) => {}
                    _ => return false,
                }
            }
            *i += 1;
        }
        false
    };
    let starts = |i: usize, w: &str| w.encode_utf16().enumerate().all(|(k, c)| s.get(i + k) == Some(&c));
    loop {
        space(&mut i);
        if i >= n {
            return if want == Want::End { None } else { at(i) };
        }
        let c = s[i];
        let done = |open: &Vec<u16>| if open.is_empty() { Want::End } else { Want::After };
        match want {
            Want::End => return at(i),
            Want::After => {
                let close = if open.last() == Some(&u16::from(b'{')) { u16::from(b'}') } else { u16::from(b']') };
                if c == u16::from(b',') {
                    want = if close == u16::from(b'}') { Want::Key } else { Want::Value };
                } else if c == close {
                    open.pop();
                    want = done(&open);
                } else {
                    return at(i);
                }
                i += 1;
            }
            Want::Key | Want::KeyOrEnd => {
                if want == Want::KeyOrEnd && c == u16::from(b'}') {
                    i += 1;
                    open.pop();
                    want = done(&open);
                    continue;
                }
                if c != 0x22 || !string(&mut i) {
                    return at(i);
                }
                space(&mut i);
                if s.get(i) != Some(&u16::from(b':')) {
                    return at(i);
                }
                i += 1;
                want = Want::Value;
            }
            Want::ValueOrEnd if c == u16::from(b']') => {
                i += 1;
                open.pop();
                want = done(&open);
            }
            _ => {
                if c == u16::from(b'{') || c == u16::from(b'[') {
                    i += 1;
                    open.push(c);
                    want = if c == u16::from(b'{') { Want::KeyOrEnd } else { Want::ValueOrEnd };
                } else if c == 0x22 {
                    if !string(&mut i) {
                        return at(i);
                    }
                    want = done(&open);
                } else {
                    // /-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?/y, or a literal.
                    let digit = |k: usize| s.get(k).is_some_and(|c| (0x30..=0x39).contains(c));
                    let mut j = i;
                    if s.get(j) == Some(&u16::from(b'-')) {
                        j += 1;
                    }
                    let num_start = j;
                    if s.get(j) == Some(&0x30) {
                        j += 1;
                    } else if digit(j) {
                        while digit(j) {
                            j += 1;
                        }
                    }
                    if j > num_start {
                        if s.get(j) == Some(&u16::from(b'.')) && digit(j + 1) {
                            j += 1;
                            while digit(j) {
                                j += 1;
                            }
                        }
                        if matches!(s.get(j), Some(&0x65) | Some(&0x45)) {
                            let mut k = j + 1;
                            if matches!(s.get(k), Some(&0x2b) | Some(&0x2d)) {
                                k += 1;
                            }
                            if digit(k) {
                                while digit(k) {
                                    k += 1;
                                }
                                j = k;
                            }
                        }
                        i = j;
                    } else if let Some(w) = ["true", "false", "null"].iter().find(|w| starts(i, w)) {
                        i += w.len();
                    } else {
                        return at(i);
                    }
                    want = done(&open);
                }
            }
        }
    }
}

// ---------- readers ----------

/// A type change found in a kind of record, looked for again.
#[derive(Clone, Debug)]
pub struct Careful {
    pub path: Option<String>,
    pub variant: Option<Vec<Value>>,
    pub segments: Vec<String>,
    pub spec: SpecRef,
    pub major: usize,
}

/// What a reader has read: records offered, checked whole, dropped, and
/// with fields taken out; and the versions seen (version → records checked).
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub records: u64,
    pub checked: u64,
    pub dropped: u64,
    pub stripped: u64,
    /// version → records checked, in the order first seen; a record with
    /// no version counts under NO_VERSION.
    pub versions: OrderedMap<u64>,
}

/// The key `Stats::versions` counts records without a version under.
pub const NO_VERSION: &str = "\u{0}";

/// What a check made of a record: the record as it was, one with fields
/// taken out, or none to read.
#[derive(Clone, Debug, PartialEq)]
pub enum Checked {
    Same,
    Stripped(Value),
    Dropped,
}

/// A source's reader: its schemas (newest first) and what it has read.
pub struct VendorReader {
    pub source: String,
    majors: Option<Vec<Major>>,
    dir: PathBuf,
    picks: HashMap<Option<String>, Option<usize>>,
    careful: HashMap<Option<String>, Vec<Careful>>,
    pub calls: u64,
    pub stats: Stats,
}

impl VendorReader {
    pub fn new(source: &str, dir: PathBuf, majors: Option<Vec<Major>>) -> Self {
        VendorReader { source: source.into(), majors, dir, picks: HashMap::new(), careful: HashMap::new(), calls: 0, stats: Stats::default() }
    }

    /// The schemas, read from schemas/ the first time they're needed.
    pub fn majors(&mut self) -> &[Major] {
        if self.majors.is_none() {
            self.majors = Some(lenient::load_vendor(&self.source, &self.dir));
        }
        self.majors.as_deref().unwrap_or_default()
    }

    /// The schema for data of `version`: the newest whose range has it, or,
    /// for a version none has, the newest (and the version is drift).
    pub fn pick(&mut self, version: Option<&str>, log: &mut DriftLog, now: f64) -> Option<usize> {
        let v = version.and_then(clean_version).map(str::to_string);
        if let Some(p) = self.picks.get(&v) {
            return *p;
        }
        let source = self.source.clone();
        let majors = self.majors();
        let mut schema = if majors.is_empty() { None } else { Some(0) };
        if let (Some(v), false) = (&v, majors.is_empty()) {
            let found = majors.iter().position(|m| satisfies(v, m.range.as_str().unwrap_or("*")));
            schema = Some(found.unwrap_or(0));
            if found.is_none() {
                log.record(&Piece { source, kind: "new-version".into(), vendor_version: Value::String(v.clone()), ..Default::default() }, now);
            }
        }
        self.picks.insert(v, schema);
        schema
    }

    pub fn careful_about(&self, kind: &Option<String>) -> bool {
        self.careful.contains_key(kind)
    }

    /// Whether it has found a type change in any kind of record.
    pub fn any_careful(&self) -> bool {
        !self.careful.is_empty()
    }

    /// The whole check, for a record already counted.
    pub fn inspect(&mut self, record: &Value, version: Option<&str>, kind: Option<String>, log: &mut DriftLog, now: f64) -> Checked {
        self.stats.checked += 1;
        let v = version.and_then(clean_version).map(str::to_string);
        let k = v.as_deref().unwrap_or(NO_VERSION);
        let n = self.stats.versions.get(k).copied().unwrap_or(0);
        self.stats.versions.insert(k, n + 1);
        let Some(i) = self.pick(v.as_deref(), log, now) else { return Checked::Same };
        let schema = self.majors.as_ref().unwrap()[i].schema.clone();
        let found = schema.inspect(record);
        for f in &found.findings {
            self.keep(f, v.as_deref(), log, now);
        }
        if found.drop {
            self.stats.dropped += 1;
            return Checked::Dropped;
        }
        for f in &found.findings {
            if let Finding::TypeChange { path, variant, segments, spec, .. } = f {
                if segments.is_empty() {
                    continue;
                }
                let list = self.careful.entry(kind.clone()).or_default();
                let same = |c: &Careful| c.path == *path && variant_string(&c.variant) == variant_string(variant);
                if !list.iter().any(same) {
                    list.push(Careful { path: path.clone(), variant: variant.clone(), segments: segments.clone(), spec: spec.clone(), major: i });
                }
            }
        }
        self.fix(record, found.strip, &kind, v.as_deref(), log, now)
    }

    /// A record not checked whole: the type changes found before in its
    /// kind are looked for, and taken out.
    pub fn guard(&mut self, record: &Value, version: Option<&str>, kind: &Option<String>, log: &mut DriftLog, now: f64) -> Checked {
        if !self.careful.contains_key(kind) {
            return Checked::Same;
        }
        let v = version.and_then(clean_version).map(str::to_string);
        self.fix(record, Vec::new(), kind, v.as_deref(), log, now)
    }

    fn fix(&mut self, record: &Value, paths: Vec<Vec<Key>>, kind: &Option<String>, v: Option<&str>, log: &mut DriftLog, now: f64) -> Checked {
        let mut all = paths;
        if let Some(careful) = self.careful.get(kind).cloned() {
            let mut seen: Vec<Vec<Key>> = all.clone();
            for c in careful {
                let schema = self.majors.as_ref().unwrap()[c.major].schema.clone();
                for keys in schema.recheck(record, &c.segments, &c.spec) {
                    if seen.contains(&keys) {
                        continue;
                    }
                    seen.push(keys.clone());
                    all.push(keys.clone());
                    let value = value_at(record, &keys).unwrap_or(&Value::Null);
                    let names = !c.segments.iter().any(|s| s == "*");
                    let p = Piece {
                        source: self.source.clone(),
                        kind: "type-change".into(),
                        path: c.path.clone().map_or(Value::Null, Value::String),
                        variant: c.variant.clone().map_or(Value::Null, Value::Array),
                        type_: Value::String(json_type(value).into()),
                        vendor_version: v.map_or(Value::Null, |v| Value::String(v.into())),
                        shape: Value::Null,
                    };
                    let shape = if log.knows(&p) { Value::Null } else { Value::String(shape_of(value, 4, 400, names)) };
                    log.record(&Piece { shape, ..p }, now);
                }
            }
        }
        if all.is_empty() {
            return Checked::Same;
        }
        self.stats.stripped += 1;
        Checked::Stripped(lenient::strip(record, &all))
    }

    // What a finding inside an object keyed by data found keeps no key's name.
    fn keep(&self, f: &Finding, v: Option<&str>, log: &mut DriftLog, now: f64) {
        let base = |kind: &str, path: &Option<String>, variant: &Option<Vec<Value>>| Piece {
            source: self.source.clone(),
            kind: kind.into(),
            path: path.clone().map_or(Value::Null, Value::String),
            variant: variant.clone().map_or(Value::Null, Value::Array),
            vendor_version: v.map_or(Value::Null, |v| Value::String(v.into())),
            ..Default::default()
        };
        let lazy = |log: &mut DriftLog, p: Piece, value: &Value, names: bool| {
            let shape = if log.knows(&p) { Value::Null } else { Value::String(shape_of(value, 4, 400, names)) };
            log.record(&Piece { shape, ..p }, now);
        };
        match f {
            Finding::NewField { path, variant, value, data } => lazy(log, base("new-field", path, variant), value, !data),
            Finding::TypeChange { path, variant, value, data, .. } => {
                lazy(log, Piece { type_: Value::String(json_type(value).into()), ..base("type-change", path, variant) }, value, !data)
            }
            Finding::NewVariant { path, variant, value } => {
                log.record(&Piece { shape: Value::String(variant_shape(value)), ..base("new-variant", path, variant) }, now);
            }
            Finding::MissingField { path, variant } => {
                log.record(&base("missing-field", path, variant), now);
            }
        }
    }

    /// Every `every`th record checked whole, in turn, the first included;
    /// the rest guarded.
    pub fn sample(&mut self, record: &Value, version: Option<&str>, kind: Option<String>, every: u64, log: &mut DriftLog, now: f64) -> Checked {
        self.stats.records += 1;
        let whole = self.calls % every.max(1) == 0;
        self.calls += 1;
        if whole {
            self.inspect(record, version, kind, log, now)
        } else {
            self.guard(record, version, &kind, log, now)
        }
    }

    /// The versions its records said, oldest first.
    pub fn versions(&self) -> Vec<String> {
        let mut v: Vec<String> = self.stats.versions.keys().filter(|k| *k != NO_VERSION).map(str::to_string).collect();
        let zero: Version = (0.0, 0.0, 0.0, None);
        v.sort_by(|a, b| compare_versions(&parse_version(a).unwrap_or(zero.clone()), &parse_version(b).unwrap_or(zero.clone())));
        v
    }
}

fn variant_string(v: &Option<Vec<Value>>) -> String {
    // String(variant), as the JS compares them: a list joined with commas.
    v.as_ref().map_or("null".into(), |v| v.iter().map(|x| xenonet_base::text::join_item(Some(x))).collect::<Vec<_>>().join(","))
}

fn value_at<'a>(record: &'a Value, keys: &[Key]) -> Option<&'a Value> {
    keys.iter().try_fold(record, |v, k| match (v, k) {
        (Value::Object(m), Key::Name(n)) => m.get(n),
        (Value::Array(a), Key::Index(i)) => a.get(*i),
        _ => None,
    })
}

/// How a transcript's lines are sampled, per file: the version its lines
/// say, and how many of each kind it has seen. The 1st, 2nd, 4th, 8th… of a
/// kind are checked whole.
#[derive(Clone, Debug, Default)]
pub struct Sampling {
    pub version: Option<String>,
    counts: HashMap<String, u64>,
}

impl Sampling {
    /// A line of `kind` came: whether to check it whole.
    pub fn whole(&mut self, kind: &str) -> bool {
        let n = self.counts.get(kind).copied().unwrap_or(0) + 1;
        self.counts.insert(kind.to_string(), n);
        n & (n - 1) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        for (v, r, ok) in [
            ("2.1.288", ">=2.0.0 <3.0.0", true),
            ("0.155.0-alpha", "<1.0.0", true),
            ("1.0.0-alpha", "<1.0.0", true),
            ("2.5.0", "2", true),
            ("3.0.0", "2", false),
            ("2.1.9", "2.1", true),
            ("0.3.9", "^0.3.0", true),
            ("0.4.0", "^0.3.0", false),
            ("2.1.5", "~2.1.0", true),
            ("2.2.0", "~2.1.0", false),
            ("1.0.0", "*", true),
            ("5.0.0", "<1.0.0 || >=4", true),
            ("x", "*", false),
        ] {
            assert_eq!(satisfies(v, r), ok, "{v} in {r}");
        }
        assert!(!valid_range(">=1.0.0 junk"));
        assert_eq!(compare_versions(&parse_version("1.0.0-alpha.2").unwrap(), &parse_version("1.0.0-alpha.10").unwrap()), Ordering::Less);
    }

    #[test]
    fn where_json_goes_wrong() {
        assert_eq!(json_error_at(r#"{"a": 1}"#), None);
        assert_eq!(json_error_at("{\n  \"a\": tru }"), Some((2, 8)));
        assert_eq!(json_error_at("[1, 2"), Some((1, 6)));
        assert_eq!(codex_version_of("codex_cli_rs/0.130.0 (Mac OS 26.1; arm64)").as_deref(), Some("0.130.0"));
        assert_eq!(codex_version_of("xenonet/0.155.0-alpha.9.2 (Mac OS 26.0.0; arm64) unknown").as_deref(), Some("0.155.0-alpha.9.2"));
    }

    #[test]
    fn samples_by_powers_of_two() {
        let mut s = Sampling::default();
        let whole: Vec<bool> = (0..9).map(|_| s.whole("user")).collect();
        assert_eq!(whole, [true, true, false, true, false, false, false, true, false]);
    }
}
