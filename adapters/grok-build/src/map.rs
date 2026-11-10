//! Where the map finds a Grok Build session's transcript: its updates, in
//! a folder named for it under the one for its working directory.

use std::path::Path;
use xenonet_adapter::platform::SessionFiles;
use xenonet_os::paths;

/// A session's updates.jsonl, in its own folder.
pub struct Files;

impl SessionFiles for Files {
    fn files(&mut self, dirs: &[String], raw: &str, _now: f64) -> Vec<(String, Option<String>)> {
        let root = dirs.first().map_or("", String::as_str);
        let Ok(ents) = paths::read_dir_sorted(Path::new(root)) else { return Vec::new() };
        for e in ents {
            let name = e.file_name().to_string_lossy().into_owned();
            let file = paths::join_all(&[root, &name, raw, "updates.jsonl"]);
            if e.file_type().is_ok_and(|t| t.is_dir()) && Path::new(&file).exists() {
                return vec![(file, None)];
            }
        }
        Vec::new()
    }
}
