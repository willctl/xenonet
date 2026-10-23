//! core/native/: the experimental readers of other harnesses' own records.
//! Each harness's run is a session on the map with its edits, read from the
//! harness's own files (or, for numbat and the gateway, from what reports
//! them). What the readers share is here (core/native/reader.js): what the
//! map asks of each (`NativeReader`), the files they read (`files`), the
//! vendor checks of their records (`Vendor`), a run's edits, what each made
//! of a file (`FileCache`), and the answers as JS values (`Out`). Paths keep
//! the syntax of the machine that wrote them, through xenonet_base::nodepath.

pub mod copilot_cli;
pub mod cursor;
pub mod files;
pub mod gemini_cli;
pub mod opencode;

use files::Files;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use xenonet_base::nodepath::Style;
use xenonet_base::ordered::OrderedMap;
use xenonet_base::paths::path_style;

// A run stays on the map for a day after it was last at work, its edits
// count for half an hour, and a reader keeps its newest runs up to
// SESSIONS_MAX.
pub const DAY_MS: f64 = 24.0 * 3600e3;
pub const EDIT_KEEP_MS: f64 = 30.0 * 60e3;
pub(crate) const SESSIONS_MAX: usize = 500;
// The edits a file keeps, the newest last.
const EDITS_PER_FILE: usize = 8;

/// A check of a record against its vendor's schema threw (vendor.js's
/// reader, which the twin calls back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Threw;

/// The problem a reader's block reports when its vendor reader couldn't
/// be got, or a check it doesn't pass over threw.
pub(crate) const VENDOR_FAILED: &str = "the vendor check failed";

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

/// The vendor reader a native reader's records are checked by: vendor.js's
/// reader for the reader's source (the twin's, called back), or the map's.
pub trait Vendor {
    /// The reader is got: Threw when getting it threw. Each reader gets it
    /// where its JS does, and reports Threw: Gemini CLI's and Copilot CLI's
    /// before anything is read, Cursor's when its file changed, OpenCode's
    /// once the database has opened.
    fn ready(&mut self) -> Result<(), Threw> {
        Ok(())
    }
    /// A table or column the reader needs is missing.
    fn missing(&mut self, path: &str, version: Option<&str>) -> Result<(), Threw>;
    /// A record checked against its vendor's schema, with the vendor's
    /// version when known.
    fn check(&mut self, record: &Value, version: Option<&str>) -> Result<Checked, Threw>;
}

/// What the map asks of a native reader, and of numbat's
/// (core/native/reader.js's contract): read what's new, say how what it
/// reads was found, and give its runs, by id, as session/v1, and as the
/// block's `runs` holds them. wire-map's observed.rs makes the engine's
/// view of a run and the reader's line in the map's sources from these.
pub trait NativeReader: Send {
    /// Reads what's new in the file or folder at `at`, as of `now`, through
    /// `files`, the records it samples checked by `vendor`. Err with the
    /// problem its block reports; each reader keeps its own rule for which
    /// failures it reports.
    fn read(&mut self, at: &str, now: f64, files: &mut dyn Files, vendor: &mut dyn Vendor) -> Result<(), String>;
    /// How its file or folder read: ok, missing, unreadable (or denied), or
    /// unread before its first read.
    fn source_state(&self) -> &'static str;
    /// Its runs' ids, in order.
    fn ids(&self) -> Vec<String>;
    /// Its runs as session/v1.
    fn sessions(&self) -> Vec<Out>;
    /// Its runs as the block's `runs` Map holds them, edits as pairs.
    fn runs_out(&self) -> Vec<Out>;
}

/// What a reader made of each file it read, kept until the file changes:
/// by the mtime and size it was read at.
pub(crate) struct FileCache<T>(HashMap<String, (f64, f64, T)>);

impl<T> Default for FileCache<T> {
    fn default() -> Self {
        FileCache(HashMap::new())
    }
}

impl<T> FileCache<T> {
    /// What `read` gives for the file, read again only when its mtime or
    /// size changed since.
    pub(crate) fn get(&mut self, file: &str, mtime_ms: f64, size: f64, read: impl FnOnce() -> T) -> &T {
        use std::collections::hash_map::Entry;
        match self.0.entry(file.to_string()) {
            Entry::Occupied(mut e) => {
                if !(e.get().0 == mtime_ms && e.get().1 == size) {
                    *e.get_mut() = (mtime_ms, size, read());
                }
                &e.into_mut().2
            }
            Entry::Vacant(e) => &e.insert((mtime_ms, size, read())).2,
        }
    }

    /// Forgets every file but these.
    pub(crate) fn keep(&mut self, files: &HashSet<String>) {
        self.0.retain(|k, _| files.contains(k));
    }
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
    let style = Style::from(path_style(base.unwrap_or(path)));
    if style.is_absolute(path) {
        Some(style.resolve(&[path]))
    } else {
        base.filter(|b| !b.is_empty()).map(|b| style.resolve(&[b, path]))
    }
}

/// "<name> in <the folder's name>", or the name alone without a folder.
pub(crate) fn titled(name: &str, cwd: Option<&str>) -> String {
    match cwd.filter(|c| !c.is_empty()) {
        Some(c) => format!("{name} in {}", Style::from(path_style(c)).basename(c, None)),
        None => name.to_string(),
    }
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
    xenonet_base::sort::sort_by(items, |a, b| key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal));
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A vendor reader that can't be got (`got` false), or whose every
    /// check throws.
    struct Broken {
        got: bool,
    }

    impl Vendor for Broken {
        fn ready(&mut self) -> Result<(), Threw> {
            if self.got {
                Ok(())
            } else {
                Err(Threw)
            }
        }
        fn missing(&mut self, _: &str, _: Option<&str>) -> Result<(), Threw> {
            Ok(())
        }
        fn check(&mut self, _: &Value, _: Option<&str>) -> Result<Checked, Threw> {
            Err(Threw)
        }
    }

    #[test]
    fn each_reader_reports_a_failed_vendor_check_as_its_js_does() {
        use files::StdFiles;
        let dir = std::env::temp_dir().join(format!("xenonet-native-vendor-rs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
        let now = xenonet_base::time::parse_iso("2026-10-16T12:00:00.000Z").unwrap();
        let read = |r: &mut dyn NativeReader, path: &str, got: bool| r.read(path, now, &mut StdFiles, &mut Broken { got });
        let failed = Err(VENDOR_FAILED.to_string());

        // Gemini CLI and Copilot CLI get the reader before anything else,
        // and pass over a file whose check threw.
        let readers: [fn() -> Box<dyn NativeReader>; 2] = [|| Box::new(gemini_cli::GeminiCli::new()), || Box::new(copilot_cli::CopilotCli::new())];
        for make in readers {
            let mut r = make();
            assert_eq!(read(r.as_mut(), &at("none"), false), failed);
            assert_eq!(r.source_state(), "unread", "nothing was read");
            assert_eq!(read(r.as_mut(), &at("none"), true), Ok(()));
        }

        // Cursor gets it when its file changed, and a check that threw is
        // reported too.
        let events = at("cursor-hooks.ndjson");
        std::fs::write(&events, "{\"hook_event_name\":\"afterFileEdit\",\"conversation_id\":\"6b0e5f1c-1111\",\"at\":\"2026-10-16T12:00:00.000Z\"}\n").unwrap();
        let mut c = cursor::Cursor::new();
        assert_eq!(read(&mut c, &at("none.ndjson"), false), Ok(()), "a missing file is nothing to parse");
        assert_eq!(read(&mut c, &events, false), failed);
        assert_eq!(c.source_state(), "ok");
        let mut c = cursor::Cursor::new();
        assert_eq!(read(&mut c, &events, true), failed, "its check threw");

        // OpenCode gets it once its database has opened; a check that threw
        // leaves the database unreadable.
        let db = at("opencode.db");
        rusqlite::Connection::open(&db)
            .unwrap()
            .execute_batch(
                "CREATE TABLE session (id text, directory text, time_created integer, time_updated integer); CREATE TABLE part (session_id text, time_created integer, data text);",
            )
            .unwrap();
        let mut o = opencode::OpenCode::new();
        assert_eq!(read(&mut o, &at("none.db"), false), Ok(()), "a missing database is nothing to open");
        assert_eq!(read(&mut o, &db, false), failed);
        assert_eq!(read(&mut o, &db, true), Ok(()));
        assert_eq!(o.source_state(), "unreadable");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
