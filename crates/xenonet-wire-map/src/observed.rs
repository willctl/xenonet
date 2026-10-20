//! The runs XenoNet observes without a reader of its own (state.js's
//! `observed`): the experimental native readers of other harnesses' own
//! records (core/native/: OpenCode, Gemini CLI, Copilot CLI and Cursor,
//! each on when XENONET_READERS names it), and numbat's records
//! (core/numbat.js, when XENONET_NUMBAT_FILE names them). Each reads with
//! xenonet-sources' reader, on every refresh, and is a block of its own;
//! the runs actors report come with them in the state.

use crate::blocks::{health_of, Block};
use crate::util::{lock, native_out, now_ms, obj};
use crate::vendor::Vendor;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_blocks::lifecycle::Lifecycle;
use xenonet_sources::native::files::StdFiles;
use xenonet_sources::native::{copilot_cli, cursor, gemini_cli, opencode, Checked, Threw};
use xenonet_sources::numbat::Numbat;

enum Reader {
    OpenCode(opencode::OpenCode),
    Gemini(gemini_cli::GeminiCli),
    Copilot(copilot_cli::CopilotCli),
    Cursor(cursor::Cursor),
    Numbat(Numbat),
}

/// What a reader is: its name as a source of the map, its block's, what
/// it reads in words (for its status), its vendor schema, and whether its
/// runs are observed (numbat's) rather than read from the harness's own
/// records.
#[derive(Clone, Copy)]
struct Kind {
    source: &'static str,
    block: &'static str,
    what: &'static str,
    schema: &'static str,
    observed: bool,
}

const OPENCODE: Kind = Kind { source: "opencode", block: "opencode-sessions", what: "OpenCode's database", schema: "anomaly/opencode/db", observed: false };
const GEMINI: Kind = Kind { source: "gemini-cli", block: "gemini-cli-sessions", what: "Gemini CLI's folder", schema: "google/gemini-cli/chat-record", observed: false };
const COPILOT: Kind = Kind { source: "copilot", block: "copilot-sessions", what: "Copilot CLI's folder", schema: "github/copilot-cli/session-event", observed: false };
const CURSOR: Kind = Kind { source: "cursor", block: "cursor-sessions", what: "the Cursor hook's file", schema: "anysphere/cursor/hook-input", observed: false };
const NUMBAT: Kind = Kind { source: "numbat", block: "numbat-sessions", what: "numbat's file", schema: "", observed: true };

pub struct Observed {
    kind: Kind,
    /// Its name as a source of the map.
    pub source: &'static str,
    /// The file or folder it reads.
    path: String,
    vendor: Arc<Vendor>,
    inner: Mutex<(Reader, Lifecycle)>,
}

// A record checked against its vendor's schema, as the native readers ask.
fn checked(vendor: &Vendor, source: &str, rec: &Value, version: Option<&str>) -> Checked {
    match vendor.check(source, rec.clone(), version, None) {
        None => Checked::Null,
        Some(v) if &v == rec => Checked::Same,
        Some(v) => Checked::Other(v),
    }
}

struct OpenCodeVendor<'a> {
    vendor: &'a Vendor,
}

impl opencode::Vendor for OpenCodeVendor<'_> {
    fn ready(&mut self) -> Result<(), Threw> {
        Ok(())
    }
    fn missing(&mut self, path: &str, version: Option<&str>) -> Result<(), Threw> {
        self.vendor.missing(opencode_source(), path, version);
        Ok(())
    }
    fn check(&mut self, record: &Value, version: Option<&str>) -> Result<Checked, Threw> {
        Ok(checked(self.vendor, opencode_source(), record, version))
    }
}

fn opencode_source() -> &'static str {
    "anomaly/opencode/db"
}

impl Observed {
    fn make(kind: Kind, path: String, reader: Reader, vendor: Arc<Vendor>) -> Arc<Observed> {
        Arc::new(Observed { kind, source: kind.source, path, vendor, inner: Mutex::new((reader, Lifecycle::new(now_ms()))) })
    }

    /// The readers XENONET_READERS names, and numbat's when its file is
    /// named, in the order the state lists them: the native readers first,
    /// so a run they read wins over numbat's.
    pub fn all(cfg: &crate::config::MapConfig, vendor: &Arc<Vendor>) -> Vec<Arc<Observed>> {
        let var = |k: &str| cfg.env.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
        let on = |name: &str| cfg.readers.contains(name);
        let mut out = Vec::new();
        if on("opencode") {
            let named = var("OPENCODE_DB").filter(|n| n != ":memory:");
            let file = opencode::opencode_db_path(&cfg.home_dir, var("XDG_DATA_HOME").as_deref(), named.as_deref());
            out.push(Observed::make(OPENCODE, file, Reader::OpenCode(opencode::OpenCode::new()), vendor.clone()));
        }
        if on("gemini-cli") {
            let root = gemini_cli::gemini_root(var("GEMINI_CLI_HOME").as_deref(), &cfg.home_dir);
            out.push(Observed::make(GEMINI, root, Reader::Gemini(gemini_cli::GeminiCli::new()), vendor.clone()));
        }
        if on("copilot") {
            let root = copilot_cli::copilot_root(&cfg.home_dir);
            out.push(Observed::make(COPILOT, root, Reader::Copilot(copilot_cli::CopilotCli::new()), vendor.clone()));
        }
        if on("cursor") {
            let named = var("XENONET_CURSOR_EVENTS");
            let file = cursor::cursor_events_file(named.as_deref(), if named.is_some() { "" } else { &cfg.state_dir });
            out.push(Observed::make(CURSOR, file, Reader::Cursor(cursor::Cursor::new()), vendor.clone()));
        }
        if let Some(file) = cfg.numbat_file.clone() {
            out.push(Observed::make(NUMBAT, file, Reader::Numbat(Numbat::new()), vendor.clone()));
        }
        out
    }

    /// Reads what's new; a problem is the block's to say, never thrown.
    pub fn read(&self) {
        let now = now_ms();
        let mut g = lock(&self.inner);
        let (reader, life) = &mut *g;
        let vendor = &*self.vendor;
        let schema = self.kind.schema;
        let mut check = |rec: &Value, version: Option<&str>| -> Result<Checked, Threw> { Ok(checked(vendor, schema, rec, version)) };
        let failed: Option<String> = match reader {
            Reader::OpenCode(o) => {
                const DAY_MS: f64 = 24.0 * 3600e3;
                const EDIT_KEEP_MS: f64 = 30.0 * 60e3;
                let file = self.path.clone();
                let exists = std::path::Path::new(&file).exists();
                let mut open = || opencode::Sqlite::open(&file).map(|db| Box::new(db) as Box<dyn opencode::Db>);
                match opencode::read_db(exists, &mut open, &mut OpenCodeVendor { vendor }, now - DAY_MS, now - EDIT_KEEP_MS) {
                    Ok(got) => o.take(&got, now).err(),
                    Err(Threw) => Some("the vendor check failed".into()),
                }
            }
            Reader::Gemini(r) => {
                r.read(Some(&self.path), now, &mut StdFiles, &mut check);
                None
            }
            Reader::Copilot(r) => {
                r.read(Some(&self.path), now, &mut StdFiles, &mut check);
                None
            }
            Reader::Cursor(r) => r.read(Some(&self.path), now, &mut StdFiles, &mut check).err().map(|_| "the vendor check failed".to_string()),
            Reader::Numbat(r) => r.read(Some(&self.path), now, &mut StdFiles).err(),
        };
        match failed {
            Some(m) => life.problem(&m, now),
            None => life.ok(now),
        }
    }

    fn state(reader: &Reader) -> &'static str {
        match reader {
            Reader::OpenCode(o) => o.file_state,
            Reader::Gemini(r) => r.root_state,
            Reader::Copilot(r) => r.root_state,
            Reader::Cursor(r) => r.file_state,
            Reader::Numbat(r) => r.file_state,
        }
    }

    fn runs(reader: &Reader) -> Vec<Value> {
        let out = match reader {
            Reader::OpenCode(o) => o.runs_out(),
            Reader::Gemini(r) => r.runs_out(),
            Reader::Copilot(r) => r.runs_out(),
            Reader::Cursor(r) => r.runs_out(),
            Reader::Numbat(r) => r.runs_out(),
        };
        out.iter().filter_map(native_out).collect()
    }

    /// The runs as sessions, for the map.
    pub fn sessions(&self) -> Vec<Value> {
        let g = lock(&self.inner);
        let out = match &g.0 {
            Reader::OpenCode(o) => o.sessions(),
            Reader::Gemini(r) => r.sessions(),
            Reader::Copilot(r) => r.sessions(),
            Reader::Cursor(r) => r.sessions(),
            Reader::Numbat(r) => r.sessions(),
        };
        out.iter().filter_map(native_out).collect()
    }

    /// What the engine reads of a run, in the shape of a live session's:
    /// when it was last at work, its edits, and no turn XenoNet can see.
    pub fn live(&self, id: &str) -> Option<Value> {
        let g = lock(&self.inner);
        let r = Self::runs(&g.0).into_iter().find(|r| r.get("id").and_then(Value::as_str) == Some(id))?;
        let edits: Vec<Value> = crate::util::items(r.get("edits"))
            .iter()
            .filter_map(|kv| kv.get(1).and_then(Value::as_array).and_then(|l| l.last()).cloned())
            .map(|e| obj(vec![("path", e.get("path").cloned().unwrap_or(Value::Null)), ("ts", e.get("ts").cloned().unwrap_or(Value::Null))]))
            .collect();
        Some(obj(vec![
            ("observed", json!(self.kind.observed)),
            ("lastEventAt", r.get("updatedAt").cloned().unwrap_or(Value::Null)),
            ("endedAt", r.get("endedAt").cloned().unwrap_or(Value::Null)),
            ("status", json!("idle")),
            ("edits", Value::Array(edits)),
        ]))
    }

    /// Its status as a source of the map.
    pub fn status(&self) -> Value {
        let g = lock(&self.inner);
        xenonet_state::state::reader_status(self.kind.source, Self::state(&g.0), Self::runs(&g.0).len(), self.kind.what)
    }
}

impl Block for Observed {
    fn name(&self) -> String {
        self.kind.block.into()
    }
    fn health(&self) -> Value {
        let g = lock(&self.inner);
        health_of(self.kind.block, "gatherer", &g.1, &[], Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_schema::drift::DriftLog;

    #[test]
    fn numbats_records_are_runs_on_the_map() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-numbat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("events.ndjson");
        let now = xenonet_base::time::iso(now_ms());
        let ev = |kind: &str, file: Option<&str>| json!({ "schema_version": "0.5.0", "record_type": "event", "source_agent": "gemini-cli", "event_type": kind, "session_id": "g-0000001", "project_path": "/w/atlas", "file_path": file, "timestamp": now });
        let text: String = [ev("session.start", None), ev("file.write", Some("/w/atlas/src/a.js"))].iter().map(|e| xenonet_base::json::stringify(e, 0) + "\n").collect();
        std::fs::write(&file, text).expect("its records");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let o = Observed::make(NUMBAT, file.to_string_lossy().into_owned(), Reader::Numbat(Numbat::new()), vendor);
        o.read();
        assert_eq!((o.status()["name"].as_str(), o.status()["ok"].as_bool(), o.status()["count"].as_u64()), (Some("numbat"), Some(true), Some(1)));
        let sessions = o.sessions();
        assert_eq!(sessions.len(), 1);
        let id = sessions[0]["id"].as_str().unwrap_or_default().to_string();
        let live = o.live(&id).expect("its run");
        assert_eq!((live["observed"].as_bool(), live["edits"][0]["path"].as_str()), (Some(true), Some("/w/atlas/src/a.js")));
        assert_eq!(o.health()["name"], "numbat-sessions");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
