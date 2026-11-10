//! The contract of a native reader: a reader of another harness's own
//! records (OpenCode's database, Gemini CLI's folder, Cursor's hook file,
//! numbat's reports). Each harness's run is a session on the map with its
//! edits. What the map asks of a reader (`NativeReader`), the vendor checks
//! of its records (`Vendor`), a run's edits, and the answers as JS values
//! (`Out`).

use super::files::Files;
use serde_json::Value;
use xenonet_text::ordered::OrderedMap;

/// A check of a record against its vendor's schema threw.
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

/// The vendor reader a native reader's records are checked by: the map's
/// reader for the reader's source.
pub trait Vendor {
    /// The reader is got: Threw when getting it threw. Each reader gets it
    /// where its reading begins: Gemini CLI's and Copilot CLI's before
    /// anything is read, Cursor's when its file changed, OpenCode's once
    /// the database has opened.
    fn ready(&mut self) -> Result<(), Threw> {
        Ok(())
    }
    /// A table or column the reader needs is missing.
    fn missing(&mut self, path: &str, version: Option<&str>) -> Result<(), Threw>;
    /// A record checked against its vendor's schema, with the vendor's
    /// version when known.
    fn check(&mut self, record: &Value, version: Option<&str>) -> Result<Checked, Threw>;
}

/// What the map asks of a native reader, and of numbat's: read what's new,
/// say how what it reads was found, and give its runs, by id, as
/// session/v1, and as the block's `runs` holds them. The map makes the
/// engine's view of a run and the reader's line in its sources from these.
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

/// An edit as the engine reads one: when, and the file.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub ts: f64,
    pub path: String,
}

/// A run's edits: each file's last few, in a JS Map's order.
pub type Edits = OrderedMap<Vec<Edit>>;

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

/// A run's edits as a Map's entries: [[file, [{ ts, path }]]].
pub fn edits_out(edits: &Edits) -> Out {
    Out::Arr(
        edits
            .iter()
            .map(|(file, list)| Out::Arr(vec![Out::str(file), Out::Arr(list.iter().map(|e| Out::Obj(vec![("ts", Out::Num(e.ts)), ("path", Out::str(&e.path))])).collect())]))
            .collect(),
    )
}
