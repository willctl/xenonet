//! core/hosts/cache.js: each host's last snapshot, in the cache folder, so
//! an offline host is still on the map after a restart. Kept by the host's
//! key; only what came with the snapshot and the events after it, never a
//! transcript or stats.

use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;
use xenonet_app::fsx;
use xenonet_app::nodepath::NATIVE;

pub const CACHE_FOLDER: &str = "hosts";

/// A host's key, as schema/host.js has it.
pub fn host_key() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new("^[0-9a-f]{32}$").unwrap())
}

fn file_of(dir: &str, key: &str) -> String {
    NATIVE.join(&[dir, CACHE_FOLDER, &format!("{key}.json")])
}

/// { key, id, savedAt, hello, graph, live, engine }, or None when there's
/// none that can be read.
pub fn read_cache(dir: Option<&str>, key: &str) -> Option<Value> {
    let dir = dir.filter(|d| !d.is_empty())?;
    if !host_key().is_match(key) {
        return None;
    }
    let data = xenonet_base::json::parse(&fsx::read_text(&file_of(dir, key)).ok()?).ok()?;
    let ok = data.get("key").and_then(Value::as_str) == Some(key)
        && data.get("savedAt").and_then(Value::as_f64).is_some()
        && data.get("graph").and_then(|g| g.get("nodes")).is_some_and(Value::is_array)
        && data.get("live").is_some_and(|l| l.is_object() || l.is_array());
    ok.then_some(data)
}

/// Keeps a host's snapshot; whether it could.
pub fn write_cache(dir: Option<&str>, key: &str, id: &Value, saved_at: &Value, hello: &Value, graph: Option<&Value>, live: Option<&Value>, engine: &Value) -> bool {
    let (Some(dir), Some(graph), Some(live)) =
        (dir.filter(|d| !d.is_empty()), graph.filter(|g| xenonet_app::jsv::truthy(Some(g))), live.filter(|l| xenonet_app::jsv::truthy(Some(l))))
    else {
        return false;
    };
    if !host_key().is_match(key) {
        return false;
    }
    let file = file_of(dir, key);
    let tmp = format!("{file}.tmp");
    let doc = json!({ "key": key, "id": id, "savedAt": saved_at, "hello": hello, "graph": graph, "live": live, "engine": engine });
    fsx::mkdir_p(&NATIVE.dirname(&file), Some(0o700))
        .and_then(|()| fsx::write_file(&tmp, xenonet_base::json::stringify(&doc, 0).as_bytes(), Some(0o600)))
        .and_then(|()| fsx::rename(&tmp, &file))
        .is_ok()
}

pub fn remove_cache(dir: Option<&str>, key: &str) -> Result<(), fsx::FsError> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else { return Ok(()) };
    if !host_key().is_match(key) {
        return Ok(());
    }
    fsx::rm_rf(&file_of(dir, key))
}
