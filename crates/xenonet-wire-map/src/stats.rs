//! core/stats.js's runtime: a session's stats from its transcript (the
//! models it used and when, its tokens and what they'd cost, its turns,
//! tools, subagents and context), which xenonet-state's stats.rs finds
//! and adds up; the files are read here, each line through its app's
//! normalizer, and the last 40 sessions' stats are kept while their files
//! stay as they were.

use crate::util::{blocking, lock, now_ms, wiki_out};
use crate::vendor::Vendor;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use xenonet_schema::vendor::Sampling;
use xenonet_state::stats::{self, File, Kept, Transcripts};

/// A whole transcript's records, through its app's normalizer (lines.js's
/// readLines): a last line without a newline too.
#[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the JS's endsWith, which tells .zst from .ZST")]
pub fn read_lines(vendor: &Vendor, file: &str, codex: bool) -> std::io::Result<Vec<Value>> {
    if file.ends_with(".zst") {
        return Err(std::io::Error::other("XenoNet can't read a compressed archive here yet"));
    }
    let data = std::fs::read(file)?;
    let t = xenonet_sources::lines::take(&data);
    let mut sampling = Sampling::default();
    let mut out: Vec<Value> = t.lines.iter().filter_map(|l| vendor.line(codex, l, &mut sampling)).collect();
    if let Some(last) = t.last {
        if let Some(r) = vendor.record(codex, last, &mut sampling) {
            out.push(r);
        }
    }
    Ok(out)
}

pub struct Stats {
    claude_root: String,
    codex_roots: Vec<String>,
    vendor: Arc<Vendor>,
    transcripts: Mutex<Transcripts>,
    kept: Mutex<(Kept, HashMap<String, Value>)>,
}

impl Stats {
    pub fn new(claude_root: &str, codex_roots: &[String], vendor: Arc<Vendor>) -> Arc<Stats> {
        Arc::new(Stats { claude_root: claude_root.into(), codex_roots: codex_roots.to_vec(), vendor, transcripts: Mutex::default(), kept: Mutex::default() })
    }

    /// A session's files (its own, then its subagents'), each with the
    /// subagent it's from.
    pub fn files(&self, session_id: &str) -> Vec<(String, Option<String>)> {
        let root = Some(self.claude_root.as_str()).filter(|r| !r.is_empty());
        lock(&self.transcripts).files(session_id, root, &self.codex_roots, now_ms())
    }

    fn stats_now(&self, session_id: &str) -> Result<Option<Value>, String> {
        let files = self.files(session_id);
        if files.is_empty() {
            return Ok(None);
        }
        let key = Kept::stamp(&files.iter().map(|(f, _)| f.clone()).collect::<Vec<_>>());
        {
            let k = lock(&self.kept);
            if k.0.hit(session_id, &key) {
                return Ok(k.1.get(session_id).cloned());
            }
        }
        // Any app but Codex is read as Claude Code.
        let codex = xenonet_base::apps::tailed_app(session_id.split(':').next().unwrap_or_default(), "claude") == "codex";
        let read: Vec<File> = files
            .iter()
            .map(|(f, agent)| File { agent: Some(agent.clone().map_or(Value::Null, Value::String)), lines: read_lines(&self.vendor, f, codex).unwrap_or_default() })
            .collect();
        let out = stats::session_stats(session_id, read).map_err(|e| e.0)?;
        let value = wiki_out(&out).unwrap_or(Value::Null);
        let mut k = lock(&self.kept);
        let gone = k.0.keep(session_id, &key);
        k.1.insert(session_id.to_string(), value.clone());
        if let Some(g) = gone {
            k.1.remove(&g);
        }
        Ok(Some(value))
    }

    /// A session's stats, None when its transcript can't be found.
    pub async fn stats(self: &Arc<Self>, session_id: &str) -> Result<Option<Value>, String> {
        let me = self.clone();
        let id = session_id.to_string();
        blocking(move || me.stats_now(&id)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_schema::drift::DriftLog;

    #[tokio::test]
    async fn a_sessions_calls_and_turns() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-stats-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("p");
        std::fs::create_dir_all(&p).expect("a folder");
        let id = "0190d3e0-0000-7000-8000-000000000001";
        let lines = [
            json!({ "type": "user", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": "Go" } }),
            json!({ "type": "assistant", "timestamp": "2026-10-17T10:00:05.000Z", "message": { "id": "m1", "model": "claude-sonnet-4-5", "stop_reason": "end_turn", "usage": { "input_tokens": 10, "output_tokens": 20 }, "content": [{ "type": "text", "text": "Done" }] } }),
        ];
        let text: String = lines.iter().map(|l| xenonet_base::json::stringify(l, 0) + "\n").collect();
        std::fs::write(p.join(format!("{id}.jsonl")), text).expect("a transcript");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let s = Stats::new(&dir.to_string_lossy(), &[], vendor);
        let st = s.stats(&format!("claude:{id}")).await.expect("stats").expect("its transcript");
        assert_eq!(st["id"], format!("claude:{id}"));
        assert!(st["totals"]["calls"].as_f64() > Some(0.0), "{st}");
        assert_eq!(s.stats("claude:0190d3e0-0000-7000-8000-00000000000f").await.expect("stats"), None);
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
