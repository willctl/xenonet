//! core/desktop-connectors.js: the connectors Claude desktop has set up, by
//! uuid. Each of its session records lists the account's remote MCP servers
//! as {uuid, name, url, tools}, and the uuid is what the tools' names carry
//! (`mcp__<uuid>__search`). So this is the name the person knows the connector
//! by, declared, which beats any guess from transcripts (live/names.rs). It
//! reads the uuid and the name and nothing else: the url and the tools stay
//! in the record.

use crate::live::names::is_uuid;
use serde_json::Value;
use std::fs;
use std::path::Path;
use xenonet_base::jsstr;
use xenonet_base::text::js_trim;

// The newest records say what the account has now; older ones only repeat
// them, and there are hundreds.
const RECENT: usize = 8;
const MAX_NAME: usize = 48;
const TTL_MS: f64 = 30000.0;

/// A name as one line to show: no control characters, and never a uuid.
pub fn clean_name(value: &str) -> Option<String> {
    let spaced: String = value.chars().map(|c| if c <= '\u{1f}' || c == '\u{7f}' { ' ' } else { c }).collect();
    let name = js_trim(&jsstr::head(js_trim(&jsstr::collapse_space(&spaced)), MAX_NAME)).to_string();
    (!name.is_empty() && !is_uuid(&name)).then_some(name)
}

// /^local_.+\.json$/: a desktop session record's file name.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the JS's pattern, which doesn't take .JSON")]
fn is_record(name: &str) -> bool {
    name.len() > "local_.json".len() && name.starts_with("local_") && name.ends_with(".json")
}

// Every record's file under claude-code-sessions/<account>/<organization>/,
// newest first.
fn records(dir: &str) -> Vec<String> {
    fn walk(folder: &Path, depth: usize, found: &mut Vec<(String, f64)>) {
        let Ok(entries) = fs::read_dir(folder) else { return };
        for ent in entries.flatten() {
            let (Ok(kind), Some(name)) = (ent.file_type(), ent.file_name().to_str().map(String::from)) else { continue };
            let path = ent.path();
            if kind.is_dir() {
                if depth < 3 && !name.starts_with("deleted_") {
                    walk(&path, depth + 1, found);
                }
            } else if kind.is_file() && is_record(&name) {
                if let (Some(full), Ok(meta)) = (path.to_str(), ent.metadata()) {
                    found.push((full.to_string(), xenonet_base::fs::mtime_ms_of(&meta).unwrap_or(0.0)));
                }
            }
        }
    }
    let mut found = Vec::new();
    walk(&Path::new(dir).join("claude-code-sessions"), 0, &mut found);
    found.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| jsstr::cmp_units(&a.0, &b.0)));
    found.into_iter().map(|(path, _)| path).collect()
}

/// The connectors in the newest `limit` records, uuid to name. Where records
/// differ, the newest says.
pub fn read_connectors(dir: &str, limit: usize) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for file in records(dir).iter().take(limit) {
        let Some(list) = fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
        for c in list.get("remoteMcpServersConfig").and_then(Value::as_array).into_iter().flatten() {
            let name = c.get("name").and_then(Value::as_str).and_then(clean_name);
            let uuid = c.get("uuid").and_then(Value::as_str).filter(|u| is_uuid(u));
            if let (Some(name), Some(uuid)) = (name, uuid) {
                if !out.iter().any(|(u, _)| u == uuid) {
                    out.push((uuid.to_string(), name));
                }
            }
        }
    }
    out
}

/// The connectors, read again at most every 30 seconds: the map asks on every
/// refresh, and the records change when a connector is added, not often.
#[derive(Debug, Default)]
pub struct Connectors {
    dir: Option<String>,
    at: Option<f64>,
    hit: Vec<(String, String)>,
}

impl Connectors {
    pub fn new(dir: Option<&str>) -> Self {
        Connectors { dir: dir.map(String::from), at: None, hit: Vec::new() }
    }

    pub fn get(&mut self, now: f64) -> &[(String, String)] {
        if let Some(dir) = &self.dir {
            if self.at.is_none_or(|at| now - at >= TTL_MS) {
                self.at = Some(now);
                self.hit = read_connectors(dir, RECENT);
            }
        }
        &self.hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    const A: &str = "5c1e4a90-27b3-4d6f-9a08-3b7e61d2f4c8";
    const B: &str = "a9d3f0b2-6e41-4c57-8b12-77c0e5a49d36";
    const C: &str = "0f8e2d41-93ab-4c60-b7d5-1e6a0c9f3b72";

    fn server(uuid: &str, name: &str) -> Value {
        serde_json::json!({ "uuid": uuid, "name": name, "url": "https://mcp.example.test/mcp", "tools": [] })
    }

    // A Claude desktop folder with one record per entry, oldest first, each
    // a minute newer than the one before.
    fn desktop(label: &str, records: &[String]) -> String {
        let dir = std::env::temp_dir().join(format!("xenonet-connectors-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let folder = dir.join("claude-code-sessions").join("acct").join("org");
        fs::create_dir_all(&folder).unwrap();
        for (i, r) in records.iter().enumerate() {
            let file = folder.join(format!("local_{i}.json"));
            fs::write(&file, r).unwrap();
            let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_790_000_000 + 60 * i as u64);
            fs::File::options().write(true).open(&file).unwrap().set_modified(t).unwrap();
        }
        dir.to_str().unwrap().to_string()
    }
    fn record(servers: &[Value]) -> String {
        serde_json::json!({ "remoteMcpServersConfig": servers }).to_string()
    }

    #[test]
    fn a_name_is_one_line_to_show_and_never_a_uuid() {
        assert_eq!(clean_name("  Tunebox  ").as_deref(), Some("Tunebox"));
        assert_eq!(clean_name("Tune\nbox\t Pro\u{7}").as_deref(), Some("Tune box Pro"));
        assert_eq!(clean_name(&"x".repeat(80)), Some("x".repeat(48)));
        for bad in ["", "   ", "\n", A] {
            assert_eq!(clean_name(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_connectors_are_read_from_the_records_remote_servers_by_uuid() {
        let dir = desktop("read", &[record(&[server(A, "Tunebox"), server(B, "Brightdesk")])]);
        assert_eq!(read_connectors(&dir, RECENT), [(A.to_string(), "Tunebox".to_string()), (B.to_string(), "Brightdesk".to_string())]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn where_records_differ_the_newest_says_and_only_the_newest_few_are_read() {
        let mut records: Vec<String> = (0..12).map(|i| record(&[server(A, if i == 11 { "Tunebox 2" } else { "Tunebox" })])).collect();
        records[0] = record(&[server(C, "Only in the oldest")]);
        let dir = desktop("newest", &records);
        let found = read_connectors(&dir, RECENT);
        assert_eq!(found.iter().find(|(u, _)| u == A).map(|f| f.1.as_str()), Some("Tunebox 2"));
        assert!(found.iter().all(|(u, _)| u != C), "a record past the newest eight isn't read");
        assert!(read_connectors(&dir, 12).iter().any(|(u, n)| u == C && n == "Only in the oldest"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_that_isnt_json_or_lists_nothing_usable_is_skipped() {
        let dir = desktop(
            "skip",
            &[
                record(&[server(A, "Tunebox")]),
                "{ not json".to_string(),
                r#"{"remoteMcpServersConfig":"nope"}"#.to_string(),
                record(&[
                    Value::Null,
                    serde_json::json!(3),
                    serde_json::json!({ "uuid": "not-a-uuid", "name": "Bad id" }),
                    serde_json::json!({ "uuid": B }),
                    serde_json::json!({ "uuid": C, "name": B }),
                    server(B, "Brightdesk"),
                ]),
                r#"{"title":"a session with no connectors"}"#.to_string(),
            ],
        );
        assert_eq!(read_connectors(&dir, RECENT), [(B.to_string(), "Brightdesk".to_string()), (A.to_string(), "Tunebox".to_string())]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn no_folder_or_no_records_is_no_connectors() {
        assert_eq!(read_connectors("/no/such/xenonet/desktop", RECENT).len(), 0);
        assert_eq!(Connectors::new(None).get(1.0).len(), 0);
    }

    #[test]
    fn the_connectors_are_read_again_only_after_the_ttl() {
        let dir = desktop("ttl", &[record(&[server(A, "Tunebox")])]);
        let mut c = Connectors::new(Some(&dir));
        assert_eq!(c.get(1000.0).len(), 1);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(c.get(30999.0).len(), 1, "inside the ttl it isn't read again");
        assert!(c.get(31000.0).is_empty(), "after it, it is");
    }
}
