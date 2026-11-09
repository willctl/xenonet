//! core/native/: the readers of other harnesses' own records, which the
//! adapters bring (`Platform::native`), and numbat's. Each harness's run is
//! a session on the map with its edits, read from the harness's own files
//! (or, for numbat and the gateway, from what reports them). What the
//! readers share is here: the contract (`NativeReader`, from
//! xenonet-adapter's reader module), the files they read (`files`), the
//! vendor checks of their records (`Vendor`), a run's edits, what each made
//! of a file (`FileCache`), and the answers as JS values (`Out`). Paths keep
//! the syntax of the machine that wrote them, through xenonet_base::nodepath.

pub mod opencode_events;

pub use xenonet_adapter::reader::files;
pub use xenonet_adapter::reader::native::{edits_out, no_check, Check, Checked, Edit, Edits, NativeReader, Out, Threw, Vendor};

use serde_json::Value;
use std::collections::{HashMap, HashSet};
use xenonet_base::nodepath::Style;
use xenonet_base::paths::path_style;

// A run stays on the map for a day after it was last at work, its edits
// count for half an hour, and a reader keeps its newest runs up to
// SESSIONS_MAX.
pub const DAY_MS: f64 = 24.0 * 3600e3;
pub const EDIT_KEEP_MS: f64 = 30.0 * 60e3;
pub const SESSIONS_MAX: usize = 500;
// The edits a file keeps, the newest last.
const EDITS_PER_FILE: usize = 8;

/// The problem a reader's block reports when its vendor reader couldn't
/// be got, or a check it doesn't pass over threw.
pub const VENDOR_FAILED: &str = "the vendor check failed";

/// What a reader made of each file it read, kept until the file changes:
/// by the mtime and size it was read at.
pub struct FileCache<T>(HashMap<String, (f64, f64, T)>);

impl<T> Default for FileCache<T> {
    fn default() -> Self {
        FileCache(HashMap::new())
    }
}

impl<T> FileCache<T> {
    /// What `read` gives for the file, read again only when its mtime or
    /// size changed since.
    pub fn get(&mut self, file: &str, mtime_ms: f64, size: f64, read: impl FnOnce() -> T) -> &T {
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
    pub fn keep(&mut self, files: &HashSet<String>) {
        self.0.retain(|k, _| files.contains(k));
    }
}

/// Math.min of two: NaN when either is, and -0 below 0.
pub fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a < b || (a == b && a.is_sign_negative()) {
        a
    } else {
        b
    }
}

/// Math.max of two: NaN when either is, and 0 above -0.
pub fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a > b || (a == b && b.is_sign_negative()) {
        a
    } else {
        b
    }
}

pub fn push_edit(edits: &mut Edits, file: &str, ts: f64) {
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
pub fn older(now: f64, at: f64, keep: f64) -> bool {
    now - at > keep
}

/// Bytes as lowercase hex.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Drops the files last edited more than the keep ago.
pub fn prune_edits(edits: &mut Edits, now: f64) {
    edits.retain(|_, list| !list.last().is_some_and(|e| older(now, e.ts, EDIT_KEEP_MS)));
}

/// A harness's run id as XenoNet's session ids take it:
/// `/^[0-9A-Za-z][0-9A-Za-z-]{6,max-1}$/`.
pub fn fits_id(id: &str, max: usize) -> bool {
    let b = id.as_bytes();
    (7..=max).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

/// Where an edit's file is: its path when that's absolute, else under
/// `base`, either way in the style of `base` (of the path, with no base);
/// None for a relative path with no base. As the readers did with
/// pathStyle(base ?? path).
pub fn edit_file(base: Option<&str>, path: &str) -> Option<String> {
    let style = Style::from(path_style(base.unwrap_or(path)));
    if style.is_absolute(path) {
        Some(style.resolve(&[path]))
    } else {
        base.filter(|b| !b.is_empty()).map(|b| style.resolve(&[b, path]))
    }
}

/// `"<name> in <the folder's name>"`, or the name alone without a folder.
pub fn titled(name: &str, cwd: Option<&str>) -> String {
    match cwd.filter(|c| !c.is_empty()) {
        Some(c) => format!("{name} in {}", Style::from(path_style(c)).basename(c, None)),
        None => name.to_string(),
    }
}

/// `typeof v === "string" ? v : null`.
pub fn str_of(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// `Date.parse(v)` of a string, finite, else None (the readers' `ms`).
pub fn ms_of(v: Option<&Value>) -> Option<f64> {
    str_of(v).and_then(xenonet_base::time::parse_iso)
}

/// A JS sort by a number, ascending, stable, as `(a, b) => a.n - b.n`
/// sorts (NaN as equal).
pub fn by_num<T>(items: &mut [T], key: impl Fn(&T) -> f64) {
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
}
