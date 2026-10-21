//! core/wiki.js: the shared context's folder (Xenolith's, or Agent Wiki's)
//! read for the map: its pages, their `[[links]]` and the projects they
//! mention, the last two days' activity log, and the notes still waiting in
//! the inbox for the curator.
//!
//! Where JS would throw on an odd value (a tag that can't be made a string,
//! a project without a name), this gives the `TypeError` lines.rs gives, and
//! the caller treats it as the JS caller treated the throw.

use crate::lines::{prop, TypeError};
use crate::live::day_key;
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use xenonet_base::text::{is_js_space, js_number, js_trim};
use xenonet_base::{json, jsstr, paths, time};

/// A JS value as an answer is built: what JSON can't hold too (undefined,
/// NaN, bytes), and objects with keys made as it goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Undef,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Bytes(Vec<u8>),
    Json(Value),
    Arr(Vec<Out>),
    Obj(Vec<(String, Out)>),
    /// An object made by setting each key in turn, as `o[k] = v` does: a
    /// "__proto__" sets its prototype.
    Assigned(Vec<(String, Out)>),
}

impl Out {
    pub fn obj(fields: Vec<(&str, Out)>) -> Out {
        Out::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    /// A value read from a record: undefined when it wasn't there.
    pub fn of(v: Option<&Value>) -> Out {
        v.map_or(Out::Undef, |v| Out::Json(v.clone()))
    }

    pub fn strs(xs: &[String]) -> Out {
        Out::Arr(xs.iter().map(|s| Out::Str(s.clone())).collect())
    }
}

/// `String(v)` for a value JSON.parse made: what an object without a
/// toString of its own writes, and the TypeError JS throws for one whose
/// own "toString" isn't a function (it can't be, coming from JSON).
pub fn to_str(v: Option<&Value>) -> Result<String, TypeError> {
    Ok(match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => json::number_string(n.as_f64().unwrap_or(f64::NAN)),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => {
            let mut parts = Vec::with_capacity(a.len());
            for x in a {
                parts.push(if x.is_null() { String::new() } else { to_str(Some(x))? });
            }
            parts.join(",")
        }
        Some(Value::Object(m)) => {
            if m.contains_key("toString") {
                return Err(TypeError("Cannot convert object to primitive value".into()));
            }
            "[object Object]".into()
        }
    })
}

/// `String(v ?? "")`.
pub fn to_str_or_empty(v: Option<&Value>) -> Result<String, TypeError> {
    match v {
        None | Some(Value::Null) => Ok(String::new()),
        v => to_str(v),
    }
}

fn truthy(v: Option<&Value>) -> bool {
    xenonet_base::text::truthy(v)
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `text.split(/\r?\n/)`.
fn split_lines(text: &str) -> Vec<&str> {
    let mut out: Vec<&str> = text.split('\n').collect();
    let n = out.len();
    for (i, part) in out.iter_mut().enumerate() {
        if i + 1 < n {
            *part = part.strip_suffix('\r').unwrap_or(part);
        }
    }
    out
}

// ---------- frontmatter ----------

// /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/: where the block's lines start and
// end, and where the match ends.
fn frontmatter(text: &str) -> Option<(usize, usize, usize)> {
    let b = text.as_bytes();
    let start = if text.starts_with("---\r\n") {
        5
    } else if text.starts_with("---\n") {
        4
    } else {
        return None;
    };
    for g in start..=b.len() {
        let close = if b[g..].starts_with(b"\r\n---") {
            g + 5
        } else if b[g..].starts_with(b"\n---") {
            g + 4
        } else {
            continue;
        };
        let mut end = close;
        if b.get(end) == Some(&b'\r') {
            end += 1;
        }
        if b.get(end) == Some(&b'\n') {
            end += 1;
        }
        return Some((start, g, end));
    }
    None
}

// /^([A-Za-z_][\w-]*):\s*(.*)$/: a key and what follows its colon.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let b = line.as_bytes();
    if !b.first().is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') {
        return None;
    }
    let mut i = 1;
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'-') {
        i += 1;
    }
    if b.get(i) != Some(&b':') {
        return None;
    }
    let rest = line[i + 1..].trim_start_matches(is_js_space);
    if rest.chars().any(is_line_terminator) {
        return None;
    }
    Some((&line[..i], rest))
}

/// A page's frontmatter (`key: value` lines between `---` lines, a value
/// that starts with "[" read as JSON when it is, one in quotes without
/// them), and where the text after it starts (None without frontmatter).
///
/// The keys are set as JS sets them, so "__proto__" is never a key of its
/// own: a list given for it is the object's prototype (meta keeps the last
/// one under that name, for whoever builds the JS object to set it), and
/// a string is passed over.
pub fn frontmatter_at(text: &str) -> (Map<String, Value>, Option<usize>) {
    let mut meta = Map::new();
    let Some((start, end, after)) = frontmatter(text) else { return (meta, None) };
    for line in split_lines(&text[start..end]) {
        let Some((key, raw)) = key_value(line) else { continue };
        let v = js_trim(raw);
        let value = if v.starts_with('[') {
            json::parse(v).unwrap_or_else(|_| Value::String(v.to_string()))
        } else if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            Value::String(v[1..v.len() - 1].to_string())
        } else {
            Value::String(v.to_string())
        };
        if key != "__proto__" || !value.is_string() {
            meta.insert(key.to_string(), value);
        }
    }
    (meta, Some(after))
}

/// A page's frontmatter (see frontmatter_at), and the text after it.
pub fn parse_frontmatter(text: &str) -> (Map<String, Value>, String) {
    let (meta, after) = frontmatter_at(text);
    (meta, text[after.unwrap_or(0)..].to_string())
}

/// When a page says it was updated, as `Date.parse` reads it: an ISO time,
/// in local time when it has a time of day and no zone.
fn date_parse(s: &str) -> Option<f64> {
    let t = time::parse_iso(s)?;
    let Some(sep) = s.find(['T', 't', ' ']) else { return Some(t) };
    let time_of_day = &s[sep + 1..];
    if time_of_day.ends_with(['Z', 'z']) || time_of_day.contains(['+', '-']) {
        return Some(t);
    }
    let first = t - jsstr::local_offset_ms(t);
    let utc = t - jsstr::local_offset_ms(first);
    (utc.abs() <= 8.64e15).then_some(utc)
}

/// The pages a text links to, as `[[slug]]` or `[[slug|text]]` (Xenolith's
/// links_in, xenolith/rust/crates/core/src/activity.rs), each once, in
/// lowercase.
pub fn wiki_links(text: &str) -> Vec<String> {
    let b = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] == b'[' && b[i + 1] == b'[' && b.get(i + 2).is_some_and(u8::is_ascii_alphanumeric) {
            let mut j = i + 3;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            let slug_end = j;
            // (?:\|[^\]]*)?: the text after a bar, up to the first "]".
            if b.get(j) == Some(&b'|') {
                j += 1;
                while j < b.len() && b[j] != b']' {
                    j += 1;
                }
            }
            if b[j..].starts_with(b"]]") {
                let slug = text[i + 2..slug_end].to_ascii_lowercase();
                if !out.contains(&slug) {
                    out.push(slug);
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_line_start(text: &str, i: usize) -> bool {
    i == 0 || text[..i].chars().next_back().is_some_and(is_line_terminator)
}

/// A page's first "# " heading, its title when the frontmatter gives none
/// (page_from_text in xenolith/rust/crates/core/src/wiki.rs), as
/// `/^#\s+(.+)$/m` finds it: a "#" that starts a line, white space (line
/// breaks too), then the rest of that line. Empty when there's none.
fn first_heading(body: &str) -> String {
    for (i, _) in body.match_indices('#').filter(|(i, _)| is_line_start(body, *i)) {
        let rest = &body[i + 1..];
        let after = rest.trim_start_matches(is_js_space);
        if after.len() == rest.len() {
            continue;
        }
        // White space to the very end leaves a match of white space alone,
        // which comes to nothing once trimmed.
        let line = after.split(is_line_terminator).next().unwrap_or("");
        return js_trim(line).to_string();
    }
    String::new()
}

/// Each run of white space with a line break in it made one space, as
/// Xenolith's one_line does (`s.replace(/\s*[\r\n]\s*/g, " ")`).
fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.contains(['\r', '\n']) {
            out.push(' ');
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in s.chars() {
        if is_js_space(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Other names a page goes by, as Xenolith reads its aliases (norm_tags in
/// xenolith/rust/crates/core/src/wiki.rs): a list, or words split at
/// commas, each on one line, lowercase, without a leading #, once.
fn aliases_of(v: Option<&Value>) -> Result<Vec<String>, TypeError> {
    let list: Vec<String> = match v {
        Some(Value::Array(a)) => a.iter().map(|x| to_str(Some(x))).collect::<Result<_, _>>()?,
        v if truthy(v) => to_str(v)?.split(',').map(String::from).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for a in list {
        let one = one_line(&a);
        let name = js_trim(js_trim(&one).trim_start_matches('#')).to_lowercase();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

// ---------- projects ----------

fn wordish(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

// /(?<![\w.-])<needle>(?![\w-]|\.[\w-])/ on the text.
fn named_in(hay: &str, needle: &str) -> bool {
    let b = hay.as_bytes();
    let mut from = 0;
    while let Some(k) = hay[from..].find(needle) {
        let i = from + k;
        let before = i == 0 || !(wordish(b[i - 1]) || b[i - 1] == b'.');
        let j = i + needle.len();
        let after = match b.get(j) {
            None => true,
            Some(&c) if wordish(c) => false,
            Some(&b'.') => !b.get(j + 1).copied().is_some_and(wordish),
            _ => true,
        };
        if before && after {
            return true;
        }
        from = i + 1;
        while !hay.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

// `v.toLowerCase()` on a value: a string's, or the TypeError JS throws.
fn lower(v: Option<&Value>, what: &str) -> Result<String, TypeError> {
    match v {
        Some(Value::String(s)) => Ok(s.to_lowercase()),
        None => Err(TypeError("Cannot read properties of undefined (reading 'toLowerCase')".into())),
        Some(Value::Null) => Err(TypeError("Cannot read properties of null (reading 'toLowerCase')".into())),
        Some(_) => Err(TypeError(format!("{what}.toLowerCase is not a function"))),
    }
}

fn items<'a>(v: Option<&'a Value>, method: &str) -> Result<&'a [Value], TypeError> {
    match v {
        Some(Value::Array(a)) => Ok(a),
        None => Err(TypeError(format!("Cannot read properties of undefined (reading '{method}')"))),
        Some(Value::Null) => Err(TypeError(format!("Cannot read properties of null (reading '{method}')"))),
        Some(_) => Err(TypeError(format!("projects.{method} is not a function"))),
    }
}

/// The projects a page talks about, by key: explicit paths into the
/// projects folder, `<folder>/<path>` where `<folder>` is the folder's own name
/// (Projects for ~/Projects, code for ~/code), or tags. A path into a
/// project inside a group names that project, not the group too. Without
/// `root`, the folder is taken to be called Projects.
pub fn mentioned_projects(text: &str, tags: Option<&Value>, projects: Option<&Value>, root: Option<&Value>) -> Result<Vec<Option<Value>>, TypeError> {
    let hay = text.replace('\\', "/").to_lowercase();
    let mut tagset: Vec<String> = Vec::new();
    if let Some(Value::Array(ts)) = tags {
        for t in ts {
            tagset.push(to_str(Some(t))?.to_lowercase());
        }
    }
    let root = to_str_or_empty(root)?;
    let folder = root.split(['\\', '/']).rfind(|s| !s.is_empty()).unwrap_or("projects").to_lowercase();
    let list = items(projects, "filter")?;
    let mut hits: Vec<usize> = Vec::new();
    for (i, p) in list.iter().enumerate() {
        let (rel, name) = (prop(Some(p), "rel")?, prop(Some(p), "name")?);
        let mut named = false;
        for r in [rel, name].into_iter().filter(|r| truthy(*r)) {
            if named_in(&hay, &format!("{folder}/{}", lower(r, "r")?)) {
                named = true;
                break;
            }
        }
        if named || tagset.contains(&lower(name, "p.name")?) {
            hits.push(i);
        }
    }
    let mut out = Vec::new();
    for &i in &hits {
        let p = &list[i];
        let key = prop(Some(p), "key")?;
        let mut inside = false;
        for &j in &hits {
            if j == i {
                continue;
            }
            let prefix = format!("{}/", to_str(key)?);
            let starts = match prop(Some(&list[j]), "key")? {
                Some(Value::String(k)) => k.starts_with(&prefix),
                None => return Err(TypeError("Cannot read properties of undefined (reading 'startsWith')".into())),
                Some(Value::Null) => return Err(TypeError("Cannot read properties of null (reading 'startsWith')".into())),
                Some(_) => return Err(TypeError("q.key.startsWith is not a function".into())),
            };
            if starts {
                inside = true;
                break;
            }
        }
        if !inside {
            out.push(key.cloned());
        }
    }
    Ok(out)
}

// ---------- the log ----------

/// An entry of the activity log.
#[derive(Clone, Debug, PartialEq)]
pub struct LogEntry {
    pub ts: f64,
    pub app: String,
    pub title: String,
    pub pages: Vec<String>,
}

impl LogEntry {
    pub fn out(&self) -> Out {
        Out::obj(vec![("ts", Out::Num(self.ts)), ("app", Out::Str(self.app.clone())), ("title", Out::Str(self.title.clone())), ("pages", Out::strs(&self.pages))])
    }
}

// JS's \s, and . (anything but a line terminator), as classes.
const S: &str = r"[\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}]";
const DOT: &str = r"[^\n\r\x{2028}\x{2029}]";

fn entry(lead: &str) -> Regex {
    Regex::new(&format!(r"\A{lead}{S}+([0-9]{{1,2}}):([0-9]{{2}}){S}+·{S}+([^·]+?){S}+·{S}+({DOT}+)\z")).expect("a pattern of fixed parts")
}

static HEADING: LazyLock<Regex> = LazyLock::new(|| entry("##"));
static BULLET: LazyLock<Regex> = LazyLock::new(|| entry("-"));
static LINK_AT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(&format!(r"{S}*\[\[[^\]]+\]\]{S}*\z")).expect("a pattern of fixed parts"));

// new Date(y, m, d, h, min).getTime(): a local time, as JS works it out
// from its parts.
fn local_time(y: f64, m: f64, d: f64, h: f64, min: f64) -> f64 {
    if ![y, m, d, h, min].iter().all(|x| x.is_finite()) {
        return f64::NAN;
    }
    let (y, m, d, h, min) = (y.trunc(), m.trunc(), d.trunc(), h.trunc(), min.trunc());
    let y = if (0.0..=99.0).contains(&y) { 1900.0 + y } else { y };
    let ym = y + (m / 12.0).floor();
    let mn = m.rem_euclid(12.0);
    if ym.abs() > 400_000.0 {
        return f64::NAN;
    }
    let day = time::days_from_civil(ym as i64, mn as u32 + 1, 1) as f64 + d - 1.0;
    let local = day * 86_400_000.0 + h * 3_600_000.0 + min * 60_000.0;
    let first = local - jsstr::local_offset_ms(local);
    let utc = local - jsstr::local_offset_ms(first);
    if utc.abs() > 8.64e15 {
        f64::NAN
    } else {
        utc
    }
}

// `date.split("-").map(Number)`, the year, month and day of it.
fn date_parts(date: Option<&Value>) -> Result<[f64; 3], TypeError> {
    let date = match date {
        Some(Value::String(s)) => s,
        None => return Err(TypeError("Cannot read properties of undefined (reading 'split')".into())),
        Some(Value::Null) => return Err(TypeError("Cannot read properties of null (reading 'split')".into())),
        Some(_) => return Err(TypeError("date.split is not a function".into())),
    };
    let mut parts = date.split('-').map(|p| js_number(Some(&Value::String(p.to_string()))));
    Ok([parts.next().unwrap_or(f64::NAN), parts.next().unwrap_or(f64::NAN), parts.next().unwrap_or(f64::NAN)])
}

/// A day's log (`date` is YYYY-MM-DD): entries like "## 13:27 · codex ·
/// Title" with a "pages: `[[a]]`" line, and curator one-liners like "- 13:53
/// · curator · Page updated: Title `[[slug]]`".
pub fn parse_log(text: &str, date: Option<&Value>) -> Result<Vec<LogEntry>, TypeError> {
    let mut out: Vec<LogEntry> = Vec::new();
    let mut cur: Option<usize> = None;
    for line in split_lines(text) {
        let h = HEADING.captures(line);
        let b = BULLET.captures(line);
        let bullet = b.is_some();
        if let Some(hit) = h.or(b) {
            let [y, mo, d] = date_parts(date)?;
            let n = |i: usize| hit[i].parse::<f64>().unwrap_or(f64::NAN);
            let said = &hit[4];
            let title = LINK_AT_END.find(said).map_or(said.to_string(), |m| format!("{}{}", &said[..m.start()], &said[m.end()..]));
            out.push(LogEntry { ts: local_time(y, mo - 1.0, d, n(1), n(2)), app: js_trim(&hit[3]).to_string(), title: js_trim(&title).to_string(), pages: wiki_links(said) });
            cur = if bullet { None } else { Some(out.len() - 1) };
            continue;
        }
        if let Some(i) = cur {
            if line.len() >= 6 && line.is_char_boundary(6) && line[..6].eq_ignore_ascii_case("pages:") {
                for slug in wiki_links(line) {
                    if !out[i].pages.contains(&slug) {
                        out[i].pages.push(slug);
                    }
                }
            }
        }
    }
    Ok(out)
}

// ---------- the folder ----------

/// Whether a path is a link. Xenolith refuses a link anywhere below the
/// wiki's folder (confined_path in xenolith/rust/crates/core/src/wiki.rs),
/// so a page or log linked in from elsewhere isn't the wiki's, and XenoNet
/// passes it over too. On Windows that's any reparse point, as Xenolith
/// has it. The folder itself may be a link.
fn linked(p: &str) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| {
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            m.file_attributes() & 0x400 != 0
        }
        #[cfg(not(windows))]
        {
            m.file_type().is_symlink()
        }
    })
}

// A folder's names in Node's order, or none when it's a link or can't be
// read.
fn names(dir: &str) -> Vec<String> {
    if linked(dir) {
        return Vec::new();
    }
    paths::read_dir_sorted(std::path::Path::new(dir)).map(|es| es.into_iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default()
}

// A file's text, as fs.readFile(f, "utf8") reads it; None when it can't be.
fn read_text(file: &str) -> Option<String> {
    std::fs::read(file).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

// `v || fallback`.
fn or(v: Option<&Value>, fallback: Out) -> Out {
    if truthy(v) {
        Out::of(v)
    } else {
        fallback
    }
}

/// The shared context's folder, read: its pages (with the projects they
/// mention, paths under `root`, the projects folder), the last two days'
/// log, as of `now`, and the inbox. `server` is the MCP server that owns
/// it.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "the JS's endsWith is case-sensitive, and so is this")]
pub fn read_wiki(dir: &str, projects: Option<&Value>, root: Option<&Value>, now: f64, server: Out) -> Result<Out, TypeError> {
    struct Page {
        slug: String,
        fields: Vec<(String, Out)>,
        writes: Vec<Out>,
    }
    let pages_dir = paths::join(dir, "pages");
    let mut pages: Vec<Page> = Vec::new();
    for f in names(&pages_dir).into_iter().filter(|f| f.ends_with(".md")) {
        let file = paths::join(&pages_dir, &f);
        if linked(&file) {
            continue;
        }
        let text = read_text(&file).unwrap_or_default();
        let (meta, body) = parse_frontmatter(&text);
        let slug = f[..f.len() - 3].to_lowercase();
        let tags = meta.get("tags");
        // The title, then the first heading, then the slug; the type in
        // lowercase: as Xenolith reads a page. (In the order JS reads them,
        // so a value that can't be made text fails as it did there.)
        let heading = first_heading(&body);
        let title = or(meta.get("title"), Out::Str(if heading.is_empty() { slug.clone() } else { heading }));
        let kind = to_str_or_empty(meta.get("type"))?;
        let kind = if kind.is_empty() { "topic".to_string() } else { kind.to_lowercase() };
        let aliases = aliases_of(meta.get("aliases"))?;
        let updated = date_parse(&to_str(meta.get("updated"))?).filter(|t| *t != 0.0);
        let links: Vec<String> = wiki_links(&body).into_iter().filter(|l| *l != slug).collect();
        let projects = mentioned_projects(&body, tags, projects, root)?;
        let fields = vec![
            ("slug", Out::Str(slug.clone())),
            ("title", title),
            ("type", Out::Str(kind)),
            ("summary", or(meta.get("summary"), Out::Str(String::new()))),
            ("tags", if matches!(tags, Some(Value::Array(_))) { Out::of(tags) } else { Out::Arr(vec![]) }),
            ("aliases", Out::strs(&aliases)),
            ("updated", updated.map_or(Out::Null, Out::Num)),
            ("updatedBy", or(meta.get("updated_by"), Out::Null)),
            ("links", Out::strs(&links)),
            ("projects", Out::Arr(projects.into_iter().map(|k| Out::of(k.as_ref())).collect())),
            ("path", Out::Str(file)),
        ];
        pages.push(Page { slug, fields: fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), writes: Vec::new() });
    }
    let mut log: Vec<LogEntry> = Vec::new();
    for offset in [1.0, 0.0] {
        let date = day_key(now - offset * 86_400e3);
        let year = paths::join_all(&[dir, "log", &jsstr::slice(&date, 0, Some(4))]);
        let file = paths::join(&year, &format!("{date}.md"));
        if linked(&paths::join(dir, "log")) || linked(&year) || linked(&file) {
            continue;
        }
        if let Some(text) = read_text(&file).filter(|t| !t.is_empty()) {
            log.extend(parse_log(&text, Some(&Value::String(date.clone())))?);
        }
    }
    let inbox_dir = paths::join(dir, "inbox");
    let mut inbox = Vec::new();
    for f in names(&inbox_dir).into_iter().filter(|f| f.ends_with(".md")) {
        if linked(&paths::join(&inbox_dir, &f)) {
            continue;
        }
        let text = read_text(&paths::join(&inbox_dir, &f)).unwrap_or_default();
        let (meta, body) = parse_frontmatter(&text);
        let first = split_lines(&body).into_iter().find(|l| !js_trim(l).is_empty()).map(|l| {
            let rest = l.trim_start_matches('#');
            if rest.len() < l.len() {
                rest.trim_start_matches(is_js_space).to_string()
            } else {
                l.to_string()
            }
        });
        let title = if truthy(meta.get("title")) { Out::of(meta.get("title")) } else { Out::Str(first.filter(|t| !t.is_empty()).unwrap_or_else(|| f.clone())) };
        inbox.push(Out::obj(vec![("id", Out::Str(f[..f.len() - 3].to_string())), ("title", title), ("app", or(meta.get("app"), Out::Null))]));
    }
    // A slug two files share is the later file's.
    for e in &log {
        for slug in &e.pages {
            if let Some(p) = pages.iter_mut().rev().find(|p| p.slug == *slug) {
                p.writes.push(e.out());
            }
        }
    }
    xenonet_base::sort::sort_by(&mut log, |a, b| (a.ts - b.ts).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
    let pages = pages
        .into_iter()
        .map(|p| {
            let mut fields = p.fields;
            fields.push(("writes".into(), Out::Arr(p.writes)));
            Out::Obj(fields)
        })
        .collect();
    Ok(Out::obj(vec![
        ("dir", Out::Str(dir.to_string())),
        ("server", server),
        ("pages", Out::Arr(pages)),
        ("inbox", Out::Arr(inbox)),
        ("log", Out::Arr(log.iter().map(LogEntry::out).collect())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frontmatter_as_js_reads_it() {
        let (meta, body) = parse_frontmatter("---\ntitle: \"Ledger env\"\ntags: [\"ledger\",\"x\"]\n---\nSee [[home-vpn]].");
        assert_eq!(Value::Object(meta), json!({ "title": "Ledger env", "tags": ["ledger", "x"] }));
        assert_eq!(body, "See [[home-vpn]].");
        let (meta, body) = parse_frontmatter("---\r\na: [1,\r\nb:\u{2028} x \r\nc: d\re\n__proto__: [1]\n---more\nrest");
        assert_eq!(Value::Object(meta), json!({ "a": "[1,", "b": "x", "__proto__": [1] }), "the list is its prototype");
        assert_eq!(frontmatter_at("---\n__proto__: x\n---\n").0.len(), 0, "a string is passed over");
        assert_eq!(date_parse("2026-10-02T10:00:00-05:00"), Some(1_790_953_200_000.0));
        assert_eq!(date_parse("2026-10-02"), Some(1_790_899_200_000.0), "a day alone is UTC");
        let local = 1_790_935_200_000.0;
        assert_eq!(date_parse("2026-10-02 10:00"), Some(local - jsstr::local_offset_ms(local - jsstr::local_offset_ms(local))));
        assert_eq!(body, "more\nrest");
        assert_eq!(parse_frontmatter("---\n---\n").1, "---\n---\n", "no lines between");
        assert_eq!(parse_frontmatter("---\n\n---").1, "");
        assert_eq!(parse_frontmatter("no front").0.len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn links_below_the_folder_are_passed_over() {
        let base = std::env::temp_dir().join(format!("xenonet-wiki-links-rs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (wiki, outside) = (base.join("wiki"), base.join("outside"));
        std::fs::create_dir_all(wiki.join("pages")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(wiki.join("pages/atlas.md"), "# Atlas\n").unwrap();
        std::fs::write(outside.join("secret.md"), "# Secret\n").unwrap();
        std::os::unix::fs::symlink(outside.join("secret.md"), wiki.join("pages/linked.md")).unwrap();
        std::os::unix::fs::symlink(&outside, wiki.join("inbox")).unwrap();
        let read = read_wiki(wiki.to_str().unwrap(), Some(&json!([])), None, 0.0, Out::Null).unwrap();
        let Out::Obj(fields) = read else { panic!("an object") };
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap();
        let Out::Arr(pages) = get("pages") else { panic!("a list") };
        assert_eq!(pages.len(), 1, "the linked page isn't the wiki's");
        assert_eq!(get("inbox"), Out::Arr(vec![]), "nor a linked inbox");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn links_once_in_lowercase() {
        assert_eq!(wiki_links("[[A-b]] [[[c]] [[a-B]] [[-x]] [[d e]] [[Ü]] [[f]"), ["a-b", "c"]);
        assert_eq!(wiki_links("[[atlas|The Atlas, ü]] [[x|a]b]] [[y|z] [[db|]]"), ["atlas", "db"], "with the text after a bar");
    }

    #[test]
    fn pages_as_xenolith_reads_them() {
        assert_eq!(first_heading("text\n#tag\n#  Title  \n# Later"), "Title");
        assert_eq!(first_heading("#\n\nNext line"), "Next line", "white space includes line breaks, as in JS");
        assert_eq!(first_heading("x\r#\tCR\rmore"), "CR", "any line terminator starts a line");
        assert_eq!(first_heading("#   "), "");
        assert_eq!(first_heading("no # heading"), "");
        assert_eq!(aliases_of(Some(&json!(["Atlas API", "#Orders", "atlas api", null]))).unwrap(), ["atlas api", "orders", "null"]);
        assert_eq!(aliases_of(Some(&json!("Postgres, #pg ,\u{2028}"))).unwrap(), ["postgres", "pg"]);
        assert_eq!(aliases_of(Some(&json!(["a \n\t b"]))).unwrap(), ["a b"], "on one line");
        assert!(aliases_of(Some(&json!([{ "toString": 1 }]))).is_err());
        assert_eq!(aliases_of(None).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn projects_by_path_or_tag() {
        let ps = json!([{ "name": "studio", "key": "/p/studio", "rel": "studio" }, { "name": "app", "key": "/p/studio/app", "rel": "studio/app" }, { "name": "ledger", "key": "/p/ledger", "rel": "ledger" }]);
        let m = |text: &str, tags: Value, root: Option<Value>| {
            mentioned_projects(text, Some(&tags), Some(&ps), root.as_ref()).unwrap().into_iter().map(|k| k.unwrap()).collect::<Vec<_>>()
        };
        assert_eq!(m("Lives in ~/Projects/studio/app/src", json!([]), None), [json!("/p/studio/app")]);
        assert_eq!(m("See Projects/studio for the lot, and Projects/ledger-old", json!([]), None), [json!("/p/studio")]);
        let code = Some(json!("/Users/ada/code"));
        assert_eq!(m("unicode/ledger and barcode/studio", json!([]), code.clone()), Vec::<Value>::new());
        assert_eq!(m("The guide is in ~/code/ledger.", json!([]), code.clone()), [json!("/p/ledger")]);
        assert_eq!(m("Moved to ~/code/ledger.old and ~/code/ledger.js", json!([]), code.clone()), Vec::<Value>::new());
        assert_eq!(m("Tagged only", json!(["LEDGER"]), code), [json!("/p/ledger")]);
        assert_eq!(m(r"C:\Users\ada\Code\ledger\README.md", json!([]), Some(json!(r"C:\Users\ada\Code\"))), [json!("/p/ledger")]);
        assert!(mentioned_projects("x", Some(&json!([{ "toString": 1 }])), Some(&ps), None).is_err(), "String() of it throws");
        assert!(mentioned_projects("x", None, Some(&json!([{ "key": "k" }])), None).is_err(), "no name to lowercase");
    }

    #[test]
    fn log_entries() {
        let date = json!("2026-10-02");
        let log = parse_log(
            "## 13:27 · codex · Recovered Ledger\n\npages: [[ledger]], [[windows]]\n\n- 13:53 · curator · Page updated: Agent Wiki [[agent-wiki]]\nPAGES: [[later]]\n",
            Some(&date),
        )
        .unwrap();
        assert_eq!(
            log.iter().map(|e| (e.app.as_str(), e.title.as_str(), e.pages.clone())).collect::<Vec<_>>(),
            [("codex", "Recovered Ledger", vec!["ledger".to_string(), "windows".to_string()]), ("curator", "Page updated: Agent Wiki", vec!["agent-wiki".to_string()])]
        );
        assert_eq!(log[0].ts, local_time(2026.0, 9.0, 2.0, 13.0, 27.0));
        assert!(parse_log("## 13:27 · x · y", Some(&json!(5))).is_err());
        assert_eq!(parse_log("## 133:27 · x · y\n##13:27 · x · y\n## 1:05 ·  · y", Some(&date)).unwrap().len(), 0);
        let odd = parse_log("## 1:05 ·   · tail · more", Some(&date)).unwrap();
        assert_eq!((odd[0].app.as_str(), odd[0].title.as_str()), ("", "tail · more"));
    }
}
