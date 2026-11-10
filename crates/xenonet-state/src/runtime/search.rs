//! Search's runtime: full-text search across the apps'
//! transcripts, in search.sqlite's FTS5 index (`crate::search`, with
//! its own connection to the file). Indexing is incremental: what was
//! added to a file is read from where the last read ended, each line
//! through its app's normalizer on its way in. The reads and the index's
//! writes run off the runtime's threads, one index at a time. Codex's
//! compressed archives (.jsonl.zst) are read whole, decompressed, each
//! time they change.

use crate::runtime::util::{blocking, lock, now_ms, num, wiki_out};
use crate::runtime::vendor::Vendor;
use crate::search::{Begun, Plan, Search};
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};
use xenonet_schema::vendor::Sampling;

// How much of a transcript is read for the index at once.
const INDEX_PIECE: usize = 8 << 20;

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
    piece: std::sync::atomic::AtomicUsize,
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
    Ok(text_of(xenonet_sources::archive::read(file)?))
}

fn message(e: crate::search::Error) -> String {
    match e {
        crate::search::Error::Type(t) => t.0,
        crate::search::Error::Sql(s) => s,
    }
}

impl TranscriptSearch {
    /// `roots`: the folders the apps' transcripts are in, each with its app.
    pub fn new(db_file: &str, roots: &[(String, String)], vendor: Arc<Vendor>) -> Arc<TranscriptSearch> {
        Arc::new(TranscriptSearch {
            core: Mutex::new(Search::new(Some(db_file), roots)),
            status: Mutex::new(Status::default()),
            vendor,
            one: tokio::sync::Mutex::new(()),
            piece: std::sync::atomic::AtomicUsize::new(INDEX_PIECE),
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
        // Nothing of an app XenoNet doesn't tail is indexed.
        let Some(tailed) = xenonet_adapter::apps::tailed_or_text(Some(app), None) else { return Ok(0.0) };
        let core = lock(&self.core);
        let Some(mut plan): Option<Plan> = core.plan(file, app).map_err(message)? else { return Ok(0.0) };
        // What isn't compressed and isn't a Codex rollout is read a piece at
        // a time: a transcript can be hundreds of megabytes, and held whole
        // it was the service's largest allocation, kept by the allocator
        // long after.
        if !crate::search::rollout_lines(app) && !plan.zst {
            return Self::index_pieces(&core, tailed, plan, &self.vendor, self.piece.load(std::sync::atomic::Ordering::Relaxed));
        }
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
        // no completed user message; a run of chunks the last read left open
        // goes on where it ended.
        let mut run = Search::run_for(&plan);
        let mut sampling = Sampling::default();
        for l in &lines {
            let Some(raw) = self.vendor.line(tailed, l, &mut sampling) else { continue };
            if let Err(e) = core.add(&mut run, raw) {
                let _ = core.rollback();
                return Err(message(e));
            }
        }
        core.commit(&plan, Some(&run)).map_err(message)?;
        Ok(run.n)
    }

    // The same, a piece of the file at a time (Search::begin_pieces): each
    // piece's complete lines, the rest carried to the next read.
    fn index_pieces(core: &Search, tailed: &str, mut plan: Plan, vendor: &Vendor, piece: usize) -> Result<f64, String> {
        let from = if plan.whole { 0.0 } else { plan.offset };
        let mut f = std::fs::File::open(&plan.file).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(from as u64)).map_err(|e| e.to_string())?;
        core.begin_pieces(&mut plan).map_err(message)?;
        let fail = |e: String| {
            let _ = core.rollback();
            e
        };
        let mut run = Search::run_for(&plan);
        let mut sampling = Sampling::default();
        let mut left = (plan.size - from).max(0.0) as u64;
        // One buffer for every piece: pieces of a size of their own each
        // time were kept by the allocator, none fit for the next.
        let mut buf = vec![0u8; piece];
        let mut filled = 0;
        while left > 0 {
            let got = (&mut f).take(left.min((buf.len() - filled) as u64)).read(&mut buf[filled..]).map_err(|e| fail(e.to_string()))?;
            if got == 0 {
                break;
            }
            left -= got as u64;
            filled += got;
            let Some(cut) = buf[..filled].iter().rposition(|&b| b == b'\n').map(|i| i + 1) else {
                // A line longer than the buffer: more room.
                if filled == buf.len() {
                    buf.resize(buf.len() * 2, 0);
                }
                continue;
            };
            let piece = String::from_utf8_lossy(&buf[..cut]);
            plan.indexed(cut);
            for l in crate::search::lines_of(&piece) {
                let Some(raw) = vendor.line(tailed, l, &mut sampling) else { continue };
                if let Err(e) = core.add(&mut run, raw) {
                    return Err(fail(message(e)));
                }
            }
            drop(piece);
            buf.copy_within(cut..filled, 0);
            filled -= cut;
        }
        core.commit(&plan, Some(&run)).map_err(message)?;
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
                crate::runtime::util::log("warn", "search.open", &format!("search index failed: {e}"));
                return;
            }
            core.roots.clone()
        };
        lock(&self.status).indexing = true;
        for r in &roots {
            for f in crate::search::list_transcripts(&r.dir) {
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
        xenonet_platforms::install();
        let dir = std::env::temp_dir().join(format!("xenonet-state-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let claude = dir.join("claude").join("p");
        std::fs::create_dir_all(&claude).expect("a folder");
        let id = "0190d3e0-0000-7000-8000-000000000001";
        let line = json!({ "type": "user", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": "Add partial refunds to the orders API" } });
        std::fs::write(claude.join(format!("{id}.jsonl")), format!("{}\n", xenonet_text::json::stringify(&line, 0))).expect("a transcript");
        // An archived Codex rollout, compressed.
        let archived = dir.join("codex").join("2026").join("10").join("05");
        std::fs::create_dir_all(&archived).expect("a folder");
        let thread = "0190d3e0-0000-7000-8000-000000000006";
        let said = json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": "Archive the nightly reports" } });
        let packed = ruzstd::encoding::compress_to_vec(format!("{}\n", xenonet_text::json::stringify(&said, 0)).as_bytes(), ruzstd::encoding::CompressionLevel::Fastest);
        std::fs::write(archived.join(format!("rollout-2026-10-05T13-00-00-{thread}.jsonl.zst")), packed).expect("an archive");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let roots = [(dir.join("claude").to_string_lossy().into_owned(), "claude".to_string()), (dir.join("codex").to_string_lossy().into_owned(), "codex".to_string())];
        let s = TranscriptSearch::new(&dir.join("search.sqlite").to_string_lossy(), &roots, vendor);
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

    #[tokio::test]
    async fn pieces_cut_at_lines_and_miss_none() {
        xenonet_platforms::install();
        let dir = std::env::temp_dir().join(format!("xenonet-state-search-pieces-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let claude = dir.join("claude").join("p");
        std::fs::create_dir_all(&claude).expect("a folder");
        let id = "0190d3e0-0000-7000-8000-000000000002";
        // Prompts of different lengths, one longer than a piece, and a last
        // line nobody has finished.
        let mut text = String::new();
        for i in 0..40 {
            let words = format!("marker{i}x {}", "word ".repeat(1 + i * 3));
            let line = json!({ "type": "user", "timestamp": "2026-10-17T10:00:00.000Z", "message": { "role": "user", "content": words } });
            text.push_str(&xenonet_text::json::stringify(&line, 0));
            text.push('\n');
        }
        text.push_str("{\"type\":\"user\",\"timestamp\"");
        std::fs::write(claude.join(format!("{id}.jsonl")), &text).expect("a transcript");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let s = TranscriptSearch::new(&dir.join("search.sqlite").to_string_lossy(), &[(dir.join("claude").to_string_lossy().into_owned(), "claude".to_string())], vendor);
        s.piece.store(97, std::sync::atomic::Ordering::Relaxed);
        s.index().await;
        assert_eq!(s.status_json().get("errors"), None, "{}", s.status_json());
        assert_eq!(s.status_json()["docs"].as_f64(), Some(40.0), "{}", s.status_json());
        for i in [0, 17, 39] {
            let hits = s.search(&format!("marker{i}x"), 30.0).await.expect("a search");
            assert_eq!(hits.len(), 1, "marker{i}x");
        }
        s.close();
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
