//! core/native/: the experimental readers of other harnesses' own records.
//! Each harness's run is a session on the map with its edits, read from the
//! harness's own files (or, for numbat and the gateway, from what reports
//! them). What the readers share is here: the files they read (`files`),
//! Node's path rules (`node_path`), a run's edits, and the answers as JS
//! values (`Out`).

pub mod copilot_cli;
pub mod cursor;
pub mod files;
pub mod gemini_cli;
pub(crate) mod node_path;
pub mod opencode;

use serde_json::Value;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::path_style;
use xenonet_base::text::js_trim;

pub(crate) const DAY_MS: f64 = 24.0 * 3600e3;
pub(crate) const EDIT_KEEP_MS: f64 = 30.0 * 60e3;
pub(crate) const SESSIONS_MAX: usize = 500;
// The edits a file keeps, the newest last.
const EDITS_PER_FILE: usize = 8;

/// A check of a record against its vendor's schema threw (vendor.js's
/// reader, which the twin calls back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threw;

/// What a check gave back: the record it was given, null (or undefined),
/// or another record.
#[derive(Clone, Debug, PartialEq)]
pub enum Checked {
    Same,
    Null,
    Other(Value),
}

/// A record checked against its vendor's schema, with the vendor's version
/// when known, or Threw.
pub type Check<'a> = dyn FnMut(&Value, Option<&str>) -> Result<Checked, Threw> + 'a;

/// A check that takes every record as it is.
pub fn no_check(_: &Value, _: Option<&str>) -> Result<Checked, Threw> {
    Ok(Checked::Same)
}

/// Math.min of two: NaN when either is, and -0 below 0.
pub(crate) fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// Math.max of two: NaN when either is, and 0 above -0.
pub(crate) fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && b.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// An edit as the engine reads one: when, and the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub ts: f64,
    pub path: String,
}

/// A run's edits: each file's last few, in a JS Map's order.
pub type Edits = OrderedMap<Vec<Edit>>;

pub(crate) fn push_edit(edits: &mut Edits, file: &str, ts: f64) {
    let edit = Edit { ts, path: file.to_string() };
    match edits.get_mut(file) {
        Some(list) => {
            list.push(edit);
            if list.len() > EDITS_PER_FILE {
                list.remove(0);
            }
        }
        None => edits.insert(file, vec![edit]),
    }
}

/// Whether `at` is more than `keep` before `now` (never, for NaN).
pub(crate) fn older(now: f64, at: f64, keep: f64) -> bool {
    now - at > keep
}

/// Bytes as lowercase hex.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Drops the files last edited more than the keep ago.
pub(crate) fn prune_edits(edits: &mut Edits, now: f64) {
    edits.retain(|_, list| !list.last().is_some_and(|e| older(now, e.ts, EDIT_KEEP_MS)));
}

/// A JS value as an answer holds it: JSON, or what JSON can't (undefined,
/// a number that isn't finite), in objects whose keys keep their order.
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Undef,
    Num(f64),
    Json(Value),
    Obj(Vec<(&'static str, Out)>),
    Arr(Vec<Out>),
}

impl Out {
    pub fn str(s: &str) -> Out {
        Out::Json(Value::String(s.to_string()))
    }
    pub fn opt_str(s: Option<&str>) -> Out {
        s.map_or(Out::Json(Value::Null), Out::str)
    }
    pub fn opt_num(n: Option<f64>) -> Out {
        n.map_or(Out::Json(Value::Null), Out::Num)
    }
    pub fn bool(b: bool) -> Out {
        Out::Json(Value::Bool(b))
    }
    pub fn null() -> Out {
        Out::Json(Value::Null)
    }
}

/// A run's edits as the twin makes them a Map: [[file, [{ ts, path }]]].
pub fn edits_out(edits: &Edits) -> Out {
    Out::Arr(
        edits
            .iter()
            .map(|(file, list)| Out::Arr(vec![Out::str(file), Out::Arr(list.iter().map(|e| Out::Obj(vec![("ts", Out::Num(e.ts)), ("path", Out::str(&e.path))])).collect())]))
            .collect(),
    )
}

/// A harness's run id as XenoNet's session ids take it:
/// /^[0-9A-Za-z][0-9A-Za-z-]{6,max-1}$/.
pub(crate) fn fits_id(id: &str, max: usize) -> bool {
    let b = id.as_bytes();
    (7..=max).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

/// Where an edit's file is: its path when that's absolute, else under
/// `base`, either way in the style of `base` (of the path, with no base);
/// None for a relative path with no base. As the readers did with
/// pathStyle(base ?? path).
pub(crate) fn edit_file(base: Option<&str>, path: &str) -> Option<String> {
    let style = path_style(base.unwrap_or(path));
    if node_path::is_absolute(style, path) {
        Some(node_path::resolve(style, &[path]))
    } else {
        base.filter(|b| !b.is_empty()).map(|b| node_path::resolve(style, &[b, path]))
    }
}

/// "<name> in <the folder's name>", or the name alone without a folder.
pub(crate) fn titled(name: &str, cwd: Option<&str>) -> String {
    match cwd.filter(|c| !c.is_empty()) {
        Some(c) => format!("{name} in {}", node_path::basename(path_style(c), c)),
        None => name.to_string(),
    }
}

/// The files an apply_patch text names, from its headers, as
/// /^\*\*\* (?:Add|Update|Delete) File: (.+)$|^\*\*\* Move to: (.+)$/gm
/// reads them (JS's line terminators), each trimmed.
pub(crate) fn patch_headers(text: &str) -> Vec<String> {
    const HEADS: [&str; 4] = ["*** Add File: ", "*** Update File: ", "*** Delete File: ", "*** Move to: "];
    text.split(['\n', '\r', '\u{2028}', '\u{2029}'])
        .filter_map(|line| HEADS.iter().find_map(|h| line.strip_prefix(h).filter(|rest| !rest.is_empty())).map(|rest| js_trim(rest).to_string()))
        .collect()
}

/// `typeof v === "string" ? v : null`.
pub(crate) fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `Date.parse(v)` of a string, finite, else None (the readers' `ms`).
pub(crate) fn ms_of(v: Option<&Value>) -> Option<f64> {
    str_of(v).and_then(xenonet_base::time::parse_iso)
}

/// A JS sort by a number, ascending, stable, as `(a, b) => a.n - b.n`
/// sorts (NaN as equal).
pub(crate) fn by_num<T>(items: &mut [T], key: impl Fn(&T) -> f64) {
    items.sort_by(|a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_headers_are_the_files() {
        let text = "*** Begin Patch\n*** Add File: a.txt\n+x\n*** Update File: b.txt\r\n*** Move to:  c.txt \n*** Delete File: d.txt\n*** Add File: \n*** End Patch";
        assert_eq!(patch_headers(text), ["a.txt", "b.txt", "c.txt", "d.txt"]);
        assert_eq!(patch_headers("*** Add File:  "), [""]);
    }

    #[test]
    fn ids_and_edits() {
        assert!(fits_id("0f3c2b1a-1111", 128) && !fits_id("-abcdefg", 128) && !fits_id("abc", 128) && !fits_id("has space", 128));
        assert!(!fits_id(&"a".repeat(129), 128) && fits_id(&"a".repeat(128), 128));
        let mut e = Edits::new();
        for i in 0..10 {
            push_edit(&mut e, "/a", f64::from(i));
        }
        push_edit(&mut e, "/b", 0.0);
        assert_eq!(e.get("/a").unwrap().iter().map(|x| x.ts).collect::<Vec<_>>(), [2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        prune_edits(&mut e, EDIT_KEEP_MS + 5.0);
        assert_eq!(e.keys().collect::<Vec<_>>(), ["/a"]);
        assert_eq!(edit_file(Some("/w/atlas"), "src/a.js").as_deref(), Some("/w/atlas/src/a.js"));
        assert_eq!(edit_file(None, "src/a.js"), None);
        assert_eq!(edit_file(Some(""), "src/a.js"), None);
        assert_eq!(edit_file(Some("C:\\w"), "src\\a.js").as_deref(), Some("C:\\w\\src\\a.js"));
        assert_eq!(titled("Cursor", Some("/w/atlas/")), "Cursor in atlas");
        assert_eq!(titled("Cursor", Some("")), "Cursor");
    }
}
