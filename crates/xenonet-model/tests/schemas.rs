//! The files in schemas/ against the documents the types write: every one
//! there, none stale, none left over. XENONET_SCHEMAS_WRITE=1 writes the
//! missing and stale ones.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn schemas_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas")
}

// Every .json file under `dir` but the vendors', as paths inside it.
fn listed(dir: &Path, at: &str, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let rel = if at.is_empty() { name.clone() } else { format!("{at}/{name}") };
        if e.path().is_dir() {
            if rel != "vendor" {
                listed(&e.path(), &rel, out);
            }
        } else if e.path().extension().is_some_and(|x| x == "json") {
            out.insert(rel);
        }
    }
}

#[test]
fn the_files_are_what_the_types_write() {
    let dir = schemas_dir();
    let want = xenonet_model::schema::documents();
    let write = std::env::var_os("XENONET_SCHEMAS_WRITE").is_some_and(|v| v == "1");
    let mut have = BTreeSet::new();
    listed(&dir, "", &mut have);
    let mut problems = Vec::new();
    for (file, text) in &want {
        let path = dir.join(file);
        let current = std::fs::read_to_string(&path).ok();
        if current.as_deref() == Some(text.as_str()) {
            continue;
        }
        if write {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        } else {
            problems.push(format!("{file}: {}", if current.is_none() { "missing" } else { "stale" }));
        }
    }
    let wanted: BTreeSet<&str> = want.iter().map(|(f, _)| f.as_str()).collect();
    // A file no type writes is left for a person to remove: it may be
    // another crate's, or a type that's still being written.
    for extra in have.iter().filter(|f| !wanted.contains(f.as_str())) {
        problems.push(format!("{extra}: no type writes it"));
    }
    assert!(problems.is_empty(), "schemas/ is out of date with the types (XENONET_SCHEMAS_WRITE=1 cargo test -p xenonet-model writes it):\n  {}", problems.join("\n  "));
}

#[test]
fn ids_are_well_formed_and_unique() {
    let docs = xenonet_model::schema::documents();
    let mut seen = BTreeSet::new();
    for (file, text) in &docs {
        assert!(seen.insert(file.clone()), "{file} is written twice");
        let id = file.strip_suffix(".json").unwrap();
        let (path, v) = id.rsplit_once("/v").unwrap_or_else(|| panic!("{id} has no version"));
        assert!(v.parse::<u32>().is_ok_and(|n| n >= 1), "{id}: not a major version");
        assert!(path.split('/').all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')), "{id}: not a schema path");
        let doc: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(doc["$id"], format!("https://xenonet.dev/schemas/{id}.json"));
        assert!(doc["title"].as_str().is_some_and(|t| !t.is_empty()) && doc["description"].as_str().is_some_and(|d| !d.is_empty()), "{id} needs a title and a description");
    }
}
