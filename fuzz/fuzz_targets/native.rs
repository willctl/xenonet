#![no_main]

// The native readers: Gemini CLI's chats, Copilot CLI's event logs,
// OpenCode's database rows, Cursor's hook log and numbat's event stream,
// each read from a stand-in folder whose files hold the input. The first
// byte picks the reader.

use libfuzzer_sys::fuzz_target;
use serde_json::{json, Value};
use xenonet_base::fs::FsError;
use xenonet_sources::native::files::{Entry, Files, Stat};
use xenonet_sources::native::opencode::{self, Db, OpenCode};
use xenonet_sources::native::{copilot_cli, cursor, gemini_cli, no_check, Checked, NativeReader, Threw, Vendor};
use xenonet_sources::numbat::{self, Numbat};

const NOW: f64 = 1_791_230_000_000.0;

// A folder where every file holds `data`, laid out the way each reader
// looks for its files.
struct Stand {
    data: Vec<u8>,
}

impl Files for Stand {
    fn read_dir(&mut self, dir: &str) -> Result<Vec<Entry>, FsError> {
        let file = |name: &str| Entry { name: name.into(), is_dir: false, is_file: true };
        let folder = |name: &str| Entry { name: name.into(), is_dir: true, is_file: false };
        Ok(if dir.ends_with("/chats") {
            vec![file("session-2026-10-05T12-00-0190d3e0.json"), file("session-2026-10-05T12-01-0190d3e1.jsonl")]
        } else if dir.ends_with("/root") {
            vec![folder("atlas"), folder("0190d3e0-0000-7000-8000-000000000001"), file("0190d3e0-0000-7000-8000-000000000002.jsonl")]
        } else {
            vec![folder("chats"), file("events.jsonl"), file(".project_root")]
        })
    }
    fn stat(&mut self, _: &str) -> Result<Stat, FsError> {
        Ok(Stat { mtime_ms: NOW - 1000.0, size: self.data.len() as f64, ino: Some(1.0) })
    }
    fn read_text(&mut self, file: &str) -> Result<String, FsError> {
        if file.ends_with(".project_root") {
            return Ok("/home/nova/Projects/atlas".into());
        }
        Ok(String::from_utf8_lossy(&self.data).into_owned())
    }
    fn read_at(&mut self, _: &str, offset: f64, len: usize) -> Result<Vec<u8>, FsError> {
        let mut out = vec![0u8; len.min(1 << 20)];
        let from = (offset.max(0.0) as usize).min(self.data.len());
        let n = (self.data.len() - from).min(out.len());
        out[..n].copy_from_slice(&self.data[from..from + n]);
        Ok(out)
    }
    fn exists(&mut self, _: &str) -> bool {
        true
    }
}

// OpenCode's database, as rows a JSON document gives: its tables'
// columns, the newest session's version, its sessions and its parts.
struct Rows(Value);

impl Db for Rows {
    fn all(&mut self, sql: &str, _: Option<f64>) -> Result<Vec<Value>, String> {
        let key = if sql.contains("table_info(session)") {
            "s"
        } else if sql.contains("table_info(part)") {
            "p"
        } else if sql.contains("FROM session") {
            "sessions"
        } else {
            "parts"
        };
        match self.0.get(key) {
            Some(Value::Array(rows)) => Ok(rows.clone()),
            Some(Value::String(e)) => Err(e.clone()),
            _ => Ok(Vec::new()),
        }
    }
    fn get(&mut self, _: &str) -> Result<Option<Value>, String> {
        Ok(self.0.get("v").cloned())
    }
    fn close(&mut self) {}
}

struct Lenient;

impl Vendor for Lenient {
    fn ready(&mut self) -> Result<(), Threw> {
        Ok(())
    }
    fn missing(&mut self, _: &str, _: Option<&str>) -> Result<(), Threw> {
        Ok(())
    }
    fn check(&mut self, _: &Value, _: Option<&str>) -> Result<Checked, Threw> {
        Ok(Checked::Same)
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let text = xenonet_fuzz::text(rest);
    xenonet_fuzz::run(rest, || {
        let mut files = Stand { data: rest.to_vec() };
        let mut check = no_check;
        match pick % 5 {
            0 => {
                if let Ok(Some(chat)) = gemini_cli::read_chat(&text, &mut |_| Ok(Checked::Same)) {
                    let _ = gemini_cli::chat_out(&chat);
                }
                let mut g = gemini_cli::GeminiCli::new();
                g.read(Some("/home/nova/.gemini/tmp/root"), NOW, &mut files, &mut check);
                let _ = (g.sessions(), g.runs_out());
                let _ = gemini_cli::session_id_of(&text);
            }
            1 => {
                if let Ok(Some(log)) = copilot_cli::read_events(&text, &mut |_| Ok(Checked::Same)) {
                    let _ = copilot_cli::log_out(&log);
                }
                let mut c = copilot_cli::CopilotCli::new();
                c.read(Some("/home/nova/.copilot/session-state/root"), NOW, &mut files, &mut check);
                let _ = (c.sessions(), c.runs_out());
                if let Some(v) = xenonet_fuzz::json(rest) {
                    let _ = copilot_cli::edited_by(v.get("toolName"), v.get("arguments"));
                }
            }
            2 => {
                let doc = xenonet_fuzz::json(rest).unwrap_or(Value::Null);
                let _ = opencode::patch_files(Some(&doc));
                let read = opencode::read_db(true, &mut || Ok(Box::new(Rows(doc.clone())) as Box<dyn Db>), &mut Lenient, NOW - 86_400_000.0, NOW - 3_600_000.0);
                let mut o = OpenCode::new();
                if let Ok(read) = read {
                    let _ = read.out();
                    let _ = o.take(&read, NOW);
                    let _ = (o.sessions(), o.runs_out());
                }
                // Rows another way: a read whose sessions and edits are the input's.
                let read = opencode::Read {
                    state: "ok",
                    version: Some("1.0.0".into()),
                    sessions: doc.get("sessions").cloned().unwrap_or(json!([])),
                    edits: doc.get("parts").cloned().unwrap_or(json!([])),
                };
                let _ = o.take(&read, NOW);
                let _ = (o.sessions(), o.runs_out());
            }
            3 => {
                let _ = cursor::tail("/home/nova/.cursor/hooks.jsonl", &mut files);
                let mut c = cursor::Cursor::new();
                let _ = c.read(Some("/home/nova/.cursor/hooks.jsonl"), NOW, &mut files, &mut check);
                let _ = (c.sessions(), c.runs_out());
                let _ = cursor::session_id_of(&text);
            }
            _ => {
                for line in text.split('\n') {
                    if let Ok(e) = numbat::read_record(line) {
                        let _ = numbat::event_out(&e);
                    }
                }
                let mut n = Numbat::new();
                let _ = n.read(Some("/home/nova/.numbat/events.ndjson"), NOW, &mut files);
                // Again, as the file grew.
                files.data.extend_from_slice(b"\n");
                let _ = n.read(Some("/home/nova/.numbat/events.ndjson"), NOW + 1000.0, &mut files);
                let _ = (n.sessions(), n.runs_out());
            }
        }
    });
});
