//! core/search.js's runtime: full-text search across both apps'
//! transcripts, in search.sqlite's FTS5 index (xenonet-state's search.rs,
//! its own connection to the file). Indexing is incremental: what was
//! added to a file is read from where the last read ended, each line
//! through its app's normalizer on its way in. The reads and the index's
//! writes run off the runtime's threads, one index at a time. Codex's
//! compressed archives (.jsonl.zst) are read whole, decompressed, each
//! time they change.

use crate::util::{blocking, lock, now_ms, num, wiki_out};
use crate::vendor::Vendor;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use xenonet_schema::vendor::Sampling;
use xenonet_state::search::{Begun, Plan, Search};

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub files: f64,
    pub docs: f64,
    pub indexing: bool,
    pub last_index_at: f64,
    pub ms: f64,
    pub errors: Option<f64>,
}

pub struct TranscriptSearch {
    core: Mutex<Search>,
    pub status: Mutex<Status>,
    vendor: Arc<Vendor>,
    one: tokio::sync::Mutex<()>,
}

// The bytes as text, bad UTF-8 replaced, without a copy when there's none:
// a transcript can be hundreds of megabytes.
fn text_of(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
}

fn read_from(file: &str, offset: f64, size: f64) -> std::io::Result<String> {
    let mut f = std::fs::File::open(file)?;
    f.seek(SeekFrom::Start(offset as u64))?;
    let mut buf = Vec::with_capacity((size - offset).max(0.0) as usize);
    f.take((size - offset).max(0.0) as u64).read_to_end(&mut buf)?;
    Ok(text_of(buf))
}

// A transcript's text: a compressed archive's decompressed.
fn read_whole(file: &str) -> std::io::Result<String> {
    Ok(text_of(crate::archive::read(file)?))
}

fn message(e: xenonet_state::search::Error) -> String {
    match e {
        xenonet_state::search::Error::Type(t) => t.0,
        xenonet_state::search::Error::Sql(s) => s,
    }
}

impl TranscriptSearch {
    pub fn new(db_file: &str, claude_root: &str, codex_roots: &[String], vendor: Arc<Vendor>) -> Arc<TranscriptSearch> {
        let codex: Vec<Option<String>> = codex_roots.iter().map(|d| Some(d.clone())).collect();
        Arc::new(TranscriptSearch {
            core: Mutex::new(Search::new(Some(db_file), Some(claude_root).filter(|r| !r.is_empty()), &codex)),
            status: Mutex::new(Status::default()),
            vendor,
            one: tokio::sync::Mutex::new(()),
        })
    }

    fn open(core: &mut Search) -> Result<(), String> {
        if core.is_open() {
            return Ok(());
        }
        core.connect().map_err(message)?;
        core.prepare().map_err(message)
    }

    // Indexes a file's new lines; how many documents went in.
    fn index_file(&self, file: &str, app: &str) -> Result<f64, String> {
        // Any app but Codex is read as Claude Code, as linesOf reads it.
        let codex = xenonet_base::apps::tailed_app(app, "claude") == "codex";
        let core = lock(&self.core);
        let Some(mut plan): Option<Plan> = core.plan(file, app).map_err(message)? else { return Ok(0.0) };
        let text = if plan.whole { read_whole(file) } else { read_from(file, plan.offset, plan.size) }.map_err(|e| e.to_string())?;
        // A Codex rollout found to have a completed user message is read
        // whole.
        let whole;
        let lines = match core.begin(&mut plan, &text).map_err(message)? {
            Begun::Lines(l) => l,
            Begun::ReadWhole => {
                whole = read_whole(file).map_err(|e| e.to_string())?;
                match core.begin(&mut plan, &whole).map_err(message)? {
                    Begun::Lines(l) => l,
                    Begun::ReadWhole => Vec::new(),
                }
            }
        };
        // A Codex rollout's prompts are its user_message events while it has
        // no completed user message.
        let user_events = plan.app == "codex" && !plan.items;
        let mut run = Search::run(file, app, &plan.session_id, user_events);
        let mut sampling = Sampling::default();
        for l in &lines {
            let Some(raw) = self.vendor.line(codex, l, &mut sampling) else { continue };
            if let Err(e) = core.add(&mut run, raw) {
                let _ = core.rollback();
                return Err(message(e));
            }
        }
        core.commit(&plan).map_err(message)?;
        Ok(run.n)
    }

    fn index_now(&self) {
        let t0 = now_ms();
        let roots = {
            let mut core = lock(&self.core);
            if let Err(e) = Self::open(&mut core) {
                let mut st = lock(&self.status);
                st.errors = Some(st.errors.unwrap_or(0.0) + 1.0);
                st.last_index_at = now_ms();
                crate::util::log("warn", "search.open", &format!("search index failed: {e}"));
                return;
            }
            core.roots.clone()
        };
        lock(&self.status).indexing = true;
        for r in &roots {
            for f in xenonet_state::search::list_transcripts(&r.dir) {
                match self.index_file(&f, &r.app) {
                    Ok(n) => lock(&self.status).docs += n,
                    Err(_) => {
                        let mut st = lock(&self.status);
                        st.errors = Some(st.errors.unwrap_or(0.0) + 1.0);
                    }
                }
            }
        }
        let counts = lock(&self.core).counts();
        let mut st = lock(&self.status);
        if let Ok((files, docs)) = counts {
            st.files = files;
            st.docs = docs;
        }
        st.indexing = false;
        st.last_index_at = now_ms();
        st.ms = now_ms() - t0;
    }

    /// Indexes what's new; one index at a time.
    pub async fn index(self: &Arc<Self>) {
        let Ok(_one) = self.one.try_lock() else { return };
        let me = self.clone();
        blocking(move || me.index_now()).await;
    }

    /// The sessions whose transcripts say every word of `q`, best first.
    pub async fn search(self: &Arc<Self>, q: &str, limit: f64) -> Result<Vec<Value>, String> {
        let me = self.clone();
        let q = q.to_string();
        blocking(move || {
            let mut core = lock(&me.core);
            Self::open(&mut core)?;
            let hits = core.search(&q, limit).map_err(message)?;
            Ok(hits.iter().filter_map(wiki_out).collect())
        })
        .await
    }

    /// `state.search.status`, as /api/sessions sends it.
    pub fn status_json(&self) -> Value {
        let st = lock(&self.status);
        let mut v = json!({ "files": num(st.files), "docs": num(st.docs), "indexing": st.indexing, "lastIndexAt": num(st.last_index_at), "ms": num(st.ms) });
        if let Some(e) = st.errors {
            v["errors"] = num(e);
        }
        v
    }

    pub fn close(&self) {
        lock(&self.core).close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_schema::drift::DriftLog;

    #[tokio::test]
    async fn indexes_and_finds_a_prompt() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let claude = dir.join("claude").join("p");
        std::fs::create_dir_all(&claude).expect("a folder");
        let id = "0190d3e0-0000-7000-8000-000000000001";
        let line = json!({ "type": "user", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": "Add partial refunds to the orders API" } });
        std::fs::write(claude.join(format!("{id}.jsonl")), format!("{}\n", xenonet_base::json::stringify(&line, 0))).expect("a transcript");
        // An archived Codex rollout, compressed.
        let archived = dir.join("codex").join("2026").join("10").join("05");
        std::fs::create_dir_all(&archived).expect("a folder");
        let thread = "0190d3e0-0000-7000-8000-000000000006";
        let said = json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": "Archive the nightly reports" } });
        let packed = ruzstd::encoding::compress_to_vec(format!("{}\n", xenonet_base::json::stringify(&said, 0)).as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
        std::fs::write(archived.join(format!("rollout-2026-10-05T13-00-00-{thread}.jsonl.zst")), packed).expect("an archive");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let codex = [dir.join("codex").to_string_lossy().into_owned()];
        let s = TranscriptSearch::new(&dir.join("search.sqlite").to_string_lossy(), &dir.join("claude").to_string_lossy(), &codex, vendor);
        s.index().await;
        assert!(s.status_json()["docs"].as_f64() >= Some(2.0), "{}", s.status_json());
        assert_eq!(s.status_json().get("errors"), None, "{}", s.status_json());
        let hits = s.search("partial refunds", 30.0).await.expect("a search");
        assert_eq!(hits[0]["sessionId"], format!("claude:{id}"));
        let hits = s.search("nightly reports", 30.0).await.expect("a search");
        assert_eq!(hits[0]["sessionId"], format!("codex:{thread}"), "the archive is indexed");
        s.close();
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
