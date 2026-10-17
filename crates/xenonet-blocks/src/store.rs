//! A JSON file with a version (core/blocks/store.js): what XenoNet
//! remembers between runs, moved up from older versions a step at a time,
//! written so a crash can't leave half a file, and never losing data it
//! can't use.
//!
//! - no file: a fresh document;
//! - not JSON, not an object, or a version it doesn't know: moved aside to
//!   <name>.broken-<time>.json, and a fresh document; never replaced by an
//!   empty one and written over;
//! - an older version: kept as it was in <name>.v<n>.json, then migrated
//!   and saved;
//! - a newer version: read as it is and never written back, since this
//!   XenoNet would write it in an older shape;
//! - a file it can't read, back up or move aside: a fresh or migrated
//!   document in memory, and the file left alone (read only).
//!
//! The document itself, its migrations and what fills it in (`prepare`)
//! are the caller's, through `DocHooks`: Rust values in the service, JS
//! objects and functions in the twin.

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use xenonet_base::fs::FsError;
use xenonet_base::json::{self, number_string};
use xenonet_base::time::iso;

pub use crate::Halt;

/// What reading the file found and did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub file: String,
    /// The file's version; None when it had none XenoNet could use.
    pub version: Option<f64>,
    pub migrated: Option<(f64, f64)>,
    pub backup: Option<String>,
    pub broken: Option<String>,
    /// Never write the file.
    pub read_only: bool,
    /// Why, in words.
    pub problem: Option<String>,
    /// The parts `prepare` started afresh, when there are any.
    pub repaired: Vec<String>,
}

/// The caller's side of a document. `Halt` is the caller's own code
/// failing (a JS hook that threw), which stops the read, as it would stop
/// the JS; the other failures are the store's to handle.
pub trait DocHooks<D> {
    /// A fresh document.
    fn fresh(&mut self) -> Result<D, Halt>;
    /// The document a file held, from its JSON.
    fn parsed(&mut self, v: Value) -> Result<D, Halt>;
    /// Its version, as the caller reads it (a number when it's one).
    fn version_of(&mut self, doc: &D) -> Result<Option<f64>, Halt>;
    /// Moved up from `from` to `to`; Err with why when it can't be.
    fn migrate(&mut self, doc: D, from: f64, to: f64) -> Result<Result<D, String>, Halt>;
    /// Fills in what it should have; the parts it started afresh, or Err
    /// when it can't use the document.
    fn prepare(&mut self, doc: &mut D) -> Result<Result<Vec<String>, ()>, Halt>;
    /// Fills in a fresh document, which it can always use.
    fn prepare_fresh(&mut self, doc: &mut D) -> Result<(), Halt>;
}

/// A file name's timestamp: an ISO time without colons.
pub fn stamp(now: f64) -> String {
    iso(now).replace(':', "-")
}

fn join(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// A name beside `file` that isn't taken: <base><ext>, then <base>-<time><ext>.
pub fn free_name(file: &str, base: &str, now: f64, ext: &str) -> String {
    let dir = Path::new(file).parent().unwrap_or(Path::new(""));
    let first = join(dir, &format!("{base}{ext}"));
    if Path::new(&first).exists() {
        join(dir, &format!("{base}-{}{ext}", stamp(now)))
    } else {
        first
    }
}

fn base_name(file: &str) -> String {
    Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Reads a versioned JSON file: the document to use, and what was found.
pub fn read_versioned<D>(file: &str, latest: f64, now: f64, hooks: &mut dyn DocHooks<D>) -> Result<(D, Report), Halt> {
    let name = base_name(file);
    let base = name.strip_suffix(".json").unwrap_or(&name).to_string();
    let report = Report { file: file.to_string(), version: Some(latest), ..Default::default() };

    let freshly = |hooks: &mut dyn DocHooks<D>, r: Report| -> Result<(D, Report), Halt> {
        let mut doc = hooks.fresh()?;
        hooks.prepare_fresh(&mut doc)?;
        Ok((doc, r))
    };
    // The document to use, prepared, or a fresh one in its place, whose
    // report keeps only the file's version of what this read found.
    let plain = report.clone();
    let use_doc = |hooks: &mut dyn DocHooks<D>, mut doc: D, mut r: Report| -> Result<(D, Report), Halt> {
        match hooks.prepare(&mut doc)? {
            Ok(repaired) => {
                r.repaired = repaired;
                Ok((doc, r))
            }
            Err(()) => {
                let problem = format!("{name} has a part XenoNet can't use, so it leaves the file alone");
                freshly(hooks, Report { version: r.version, read_only: true, problem: Some(problem), ..plain.clone() })
            }
        }
    };

    let bytes = match fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            let e = FsError::from(e);
            if e.is("ENOENT") {
                return freshly(hooks, report);
            }
            let problem = format!("XenoNet couldn't read {name} ({})", e.code_or_message());
            return freshly(hooks, Report { read_only: true, problem: Some(problem), ..report });
        }
    };
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let data = json::parse(&text).ok();
    let valid = data.is_some();
    let is_object = matches!(data, Some(Value::Object(_)));
    let field = data.as_ref().and_then(|d| d.get("version")).cloned();
    let doc = data.map(|d| hooks.parsed(d)).transpose()?;
    let version = match (&doc, is_object) {
        (Some(d), true) => hooks.version_of(d)?,
        _ => None,
    };
    let usable = version.filter(|v| v.fract() == 0.0 && *v >= 1.0);
    let (Some(doc), true, Some(version)) = (doc, is_object, usable) else {
        let why = if !is_object {
            if valid {
                "wasn't a JSON object".to_string()
            } else {
                "wasn't valid JSON".to_string()
            }
        } else {
            format!("had a version XenoNet doesn't know ({})", field.as_ref().map_or("undefined".to_string(), |v| json::stringify(v, 0)))
        };
        let aside = join(Path::new(file).parent().unwrap_or(Path::new("")), &format!("{base}.broken-{}.json", stamp(now)));
        return match fs::rename(file, &aside) {
            Ok(()) => {
                let problem = format!("{name} {why}, so XenoNet moved it aside and started afresh");
                freshly(hooks, Report { version: None, broken: Some(aside), problem: Some(problem), ..report })
            }
            Err(e) => {
                let e = FsError::from(e);
                let problem = format!("{name} {why}, and XenoNet couldn't move it aside ({}), so it leaves it alone", e.code_or_message());
                freshly(hooks, Report { version: None, read_only: true, problem: Some(problem), ..report })
            }
        };
    };

    if version > latest {
        let problem = format!("{name} is from a newer XenoNet (version {}), so this one reads it but doesn't save changes to it", number_string(version));
        return use_doc(hooks, doc, Report { version: Some(version), read_only: true, problem: Some(problem), ..report });
    }
    if version == latest {
        return use_doc(hooks, doc, report);
    }

    // Older: moved up, then a copy first, as it was, so the move can be
    // undone. A migration works on a copy, so `doc` is still the file's.
    let moved = match hooks.migrate(doc, version, latest)? {
        Ok(d) => d,
        Err(why) => {
            let doc = hooks.parsed(json::parse(&text).unwrap_or(Value::Null))?;
            let problem = format!("XenoNet couldn't move {name} from version {} to {} ({why}), so it leaves it alone", number_string(version), number_string(latest));
            return use_doc(hooks, doc, Report { version: Some(version), read_only: true, problem: Some(problem), ..report });
        }
    };
    let (doc, r) = use_doc(hooks, moved, Report { version: Some(version), migrated: Some((version, latest)), ..report })?;
    if r.read_only {
        return Ok((doc, r));
    }
    let backup = free_name(file, &format!("{base}.v{}", number_string(version)), now, ".json");
    match fs::OpenOptions::new().write(true).create_new(true).open(&backup).and_then(|mut f| f.write_all(text.as_bytes())) {
        Ok(()) => Ok((doc, Report { backup: Some(backup), ..r })),
        Err(e) => {
            let e = FsError::from(e);
            let problem =
                format!("XenoNet couldn't keep a copy of {name} ({}) before moving it to version {}, so it leaves the file as it is", e.code_or_message(), number_string(latest));
            Ok((doc, Report { read_only: true, problem: Some(problem), ..r }))
        }
    }
}

/// Writes JSON text through a temporary file renamed into place, so a crash
/// can't leave half of one, making its folder first.
pub fn write_atomic(file: &str, text: &str) -> Result<(), FsError> {
    if let Some(dir) = Path::new(file).parent() {
        fs::create_dir_all(dir).map_err(|e| FsError::of(e, "mkdir", &dir.to_string_lossy()))?;
    }
    let tmp = format!("{file}.tmp");
    fs::write(&tmp, text).map_err(|e| FsError::of(e, "open", &tmp))?;
    fs::rename(&tmp, file).map_err(|e| FsError::of(e, "rename", &tmp))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Plain;
    impl DocHooks<Value> for Plain {
        fn fresh(&mut self) -> Result<Value, Halt> {
            Ok(json!({ "version": 2 }))
        }
        fn parsed(&mut self, v: Value) -> Result<Value, Halt> {
            Ok(v)
        }
        fn version_of(&mut self, d: &Value) -> Result<Option<f64>, Halt> {
            Ok(match d.get("version") {
                None => Some(1.0),
                Some(v) => v.as_f64(),
            })
        }
        fn migrate(&mut self, mut d: Value, _: f64, to: f64) -> Result<Result<Value, String>, Halt> {
            d["version"] = json!(to);
            Ok(Ok(d))
        }
        fn prepare(&mut self, _: &mut Value) -> Result<Result<Vec<String>, ()>, Halt> {
            Ok(Ok(Vec::new()))
        }
        fn prepare_fresh(&mut self, _: &mut Value) -> Result<(), Halt> {
            Ok(())
        }
    }

    #[test]
    fn older_newer_and_broken_files() {
        let dir = std::env::temp_dir().join(format!("xenonet-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json").to_string_lossy().into_owned();
        fs::write(&file, r#"{"colour":"teal"}"#).unwrap();
        let (doc, r) = read_versioned(&file, 2.0, 0.0, &mut Plain).unwrap();
        assert_eq!((doc["version"].clone(), r.migrated), (json!(2.0), Some((1.0, 2.0))));
        assert!(r.backup.as_deref().unwrap().ends_with("prefs.v1.json"));
        fs::write(&file, "[1]").unwrap();
        let (_, r) = read_versioned(&file, 2.0, 0.0, &mut Plain).unwrap();
        assert_eq!(r.problem.as_deref(), Some("prefs.json wasn't a JSON object, so XenoNet moved it aside and started afresh"));
        assert!(r.broken.unwrap().ends_with("prefs.broken-1970-01-01T00-00-00.000Z.json"));
        fs::write(&file, r#"{"version":3}"#).unwrap();
        let (_, r) = read_versioned(&file, 2.0, 0.0, &mut Plain).unwrap();
        assert!(r.read_only);
        fs::remove_dir_all(&dir).unwrap();
    }
}
