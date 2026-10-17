//! The JSON Schema files in schemas/, against the documents the
//! definitions make (core/schema/files.js): which are missing, out of date
//! or left over, and writing them, with folders left empty removed.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use xenonet_base::{jsstr, paths};

/// What schemaFilesState() says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub current: bool,
    pub missing: Vec<String>,
    pub changed: Vec<String>,
    pub extra: Vec<String>,
}

// The .json files under `dir`, as paths inside it with forward slashes; a
// folder that can't be read has none.
fn list_json(dir: &str, at: &str, out: &mut Vec<String>) {
    let Ok(ents) = paths::read_dir_sorted(Path::new(&paths::join(dir, at))) else { return };
    for e in ents {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if at.is_empty() { name.clone() } else { format!("{at}/{name}") };
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            list_json(dir, &rel, out);
        } else if t.is_file() && name.ends_with(".json") {
            out.push(rel);
        }
    }
}

fn read(file: &str) -> Option<String> {
    fs::read(file).ok().map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// The files in `dir` against `want` (a path under it, and its text).
pub fn state(dir: &str, want: &[(String, String)]) -> State {
    let mut listed = Vec::new();
    list_json(dir, "", &mut listed);
    let have: BTreeSet<String> = listed.into_iter().collect();
    let missing: Vec<String> = want.iter().filter(|(f, _)| !have.contains(f)).map(|(f, _)| f.clone()).collect();
    let changed: Vec<String> = want.iter().filter(|(f, text)| have.contains(f) && read(&paths::join(dir, f)).as_deref() != Some(text.as_str())).map(|(f, _)| f.clone()).collect();
    let mut extra: Vec<String> = have.iter().filter(|f| !want.iter().any(|(w, _)| w == *f)).cloned().collect();
    extra.sort_by(|a, b| jsstr::cmp_units(a, b));
    State { current: missing.is_empty() && changed.is_empty() && extra.is_empty(), missing, changed, extra }
}

// path.dirname.
fn dirname(p: &str) -> String {
    if cfg!(windows) {
        return Path::new(p).parent().map_or_else(|| p.to_string(), |d| d.to_string_lossy().into_owned());
    }
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        return if p.starts_with('/') { "/".into() } else { ".".into() };
    }
    match trimmed.rfind('/') {
        Some(0) => "/".into(),
        Some(i) => trimmed[..i].to_string(),
        None => ".".into(),
    }
}

/// Writes what's missing or out of date in `dir`, and removes files for
/// schemas no longer defined, with folders left empty: what was written,
/// sorted, and what was removed.
pub fn write(dir: &str, want: &[(String, String)]) -> io::Result<(Vec<String>, Vec<String>)> {
    let State { missing, changed, extra, .. } = state(dir, want);
    let todo: BTreeSet<String> = missing.into_iter().chain(changed).collect();
    for (file, text) in want {
        if !todo.contains(file) {
            continue;
        }
        let full = paths::join(dir, file);
        fs::create_dir_all(dirname(&full))?;
        fs::write(&full, text)?;
    }
    let sep = std::path::MAIN_SEPARATOR;
    for file in &extra {
        let full = paths::join(dir, file);
        fs::remove_file(&full)?;
        let mut d = dirname(&full);
        while d.starts_with(&format!("{dir}{sep}")) && fs::read_dir(&d)?.next().is_none() {
            fs::remove_dir(&d)?;
            d = dirname(&d);
        }
    }
    let mut written: Vec<String> = todo.into_iter().collect();
    written.sort_by(|a, b| jsstr::cmp_units(a, b));
    Ok((written, extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirnames_as_node_has_them() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(dirname("/a/b/c.json"), "/a/b");
        assert_eq!(dirname("/a"), "/");
        assert_eq!(dirname("a"), ".");
        assert_eq!(dirname("a/b/"), "a");
    }

    #[test]
    fn writes_what_is_missing_and_removes_what_is_left_over() {
        let dir = std::env::temp_dir().join(format!("xenonet-schema-files-{}", std::process::id()));
        let d = dir.to_string_lossy().into_owned();
        let want = vec![("a/v1.json".to_string(), "{}\n".to_string()), ("b/v1.json".to_string(), "[]\n".to_string())];
        assert_eq!(write(&d, &want).unwrap(), (vec!["a/v1.json".into(), "b/v1.json".into()], vec![]));
        assert!(state(&d, &want).current);
        fs::write(dir.join("a/v1.json"), "{ }\n").unwrap();
        fs::create_dir_all(dir.join("gone/deep")).unwrap();
        fs::write(dir.join("gone/deep/v1.json"), "{}").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();
        let s = state(&d, &want);
        assert_eq!((s.changed.clone(), s.extra.clone()), (vec!["a/v1.json".to_string()], vec!["gone/deep/v1.json".to_string()]));
        assert_eq!(write(&d, &want).unwrap(), (vec!["a/v1.json".into()], vec!["gone/deep/v1.json".into()]));
        assert!(!dir.join("gone").exists() && dir.join("notes.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
