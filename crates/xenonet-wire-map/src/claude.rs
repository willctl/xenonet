//! core/claude.js's runtime: Claude Code's sessions and transcripts, with
//! Claude desktop's records of them. The JS has them from the Agent SDK
//! (listSessions, getSessionMessages, renameSession), which reads Claude
//! Code's transcripts under ~/.claude/projects; the binary reads the same
//! files (sdk.rs). Claude desktop's records, merged into the listing, and
//! a transcript's messages as the console shows them are xenonet-sources'
//! claude.rs.

use crate::util::{codex_obj, lock};
use crate::vendor::Vendor;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use xenonet_sources::claude as core;

/// Where a renamed session's name went.
pub struct Renamed {
    pub desktop: bool,
    pub desktop_error: Option<String>,
}

pub struct Claude {
    projects_dir: String,
    desktop_dir: Option<String>,
    vendor: Arc<Vendor>,
    /// Where each desktop session's file is, by its desktop id, as the last
    /// listing found them, for renaming one.
    desktop_files: Mutex<HashMap<String, String>>,
}

impl Claude {
    pub fn new(projects_dir: &str, desktop_dir: Option<&str>, vendor: Arc<Vendor>) -> Arc<Claude> {
        Arc::new(Claude { projects_dir: projects_dir.into(), desktop_dir: desktop_dir.map(str::to_string), vendor, desktop_files: Mutex::default() })
    }

    // Claude desktop's records, by the CLI session each is of. Sessions sit
    // in claude-code-sessions/<account>/<organization>/, so the folders say
    // which Claude account each ran as.
    fn desktop(&self) -> Vec<Value> {
        let Some(root) = self.desktop_dir.as_ref().map(|d| Path::new(d).join("claude-code-sessions").to_string_lossy().into_owned()) else { return Vec::new() };
        let mut records = Vec::new();
        for f in core::desktop_files(&root) {
            // A file that isn't JSON is passed over.
            let Some(data) = f.data else { continue };
            if let Some(d) = self.vendor.sample("anthropic/claude-desktop/session", data, None, None, 16) {
                records.push((f.file, f.parts, d));
            }
        }
        let (desktop, files) = core::desktop_index(records);
        lock(&self.desktop_files).extend(files);
        desktop
    }

    /// Claude Code's sessions, with what Claude desktop says of them.
    pub async fn sessions(self: &Arc<Self>) -> Result<Vec<Value>, String> {
        let me = self.clone();
        crate::util::blocking(move || {
            let version = crate::sdk::VERSION;
            let listed: Vec<Value> = crate::sdk::list_sessions(&me.projects_dir)
                .into_iter()
                .filter_map(|s| me.vendor.sample("anthropic/agent-sdk/session-listing", s, Some(version), None, 16))
                .collect();
            let desktop = me.desktop();
            core::sessions(&listed, &desktop).map(|list| list.iter().map(codex_obj).collect()).map_err(|e| e.0)
        })
        .await
    }

    /// A page of a session's transcript, newest first; `cursor` is the
    /// count of messages already shown from the end.
    pub async fn read(self: &Arc<Self>, raw_id: &str, cursor: Option<&str>, limit: f64) -> Result<Value, String> {
        let me = self.clone();
        let (raw, cursor) = (raw_id.to_string(), cursor.map(|c| Value::String(c.into())));
        crate::util::blocking(move || {
            let all = crate::sdk::session_messages(&me.projects_dir, &raw);
            let (start, end, next) = core::page(all.len(), cursor.as_ref(), limit);
            let version = crate::sdk::VERSION;
            let page: Vec<Value> = all[start..end]
                .iter()
                .filter_map(|m| {
                    let kind = m.get("type").and_then(Value::as_str).map(str::to_string);
                    me.vendor.sample("anthropic/agent-sdk/session-message", m.clone(), Some(version), kind.as_deref(), 16)
                })
                .collect();
            let entries: Vec<Value> = core::entries(&page).map_err(|e| e.0)?.iter().map(codex_obj).collect();
            Ok(json!({ "entries": entries, "cursor": next, "total": all.len() }))
        })
        .await
    }

    /// Renames a session where Claude keeps its name: Claude Code's
    /// transcript, and Claude desktop's record too when the session is in
    /// the desktop.
    pub async fn rename(self: &Arc<Self>, node: &Value, title: &str) -> Result<Renamed, String> {
        let raw = node.get("rawId").and_then(Value::as_str).unwrap_or_default().to_string();
        let desktop_id = node.get("desktopId").and_then(Value::as_str).filter(|d| !d.is_empty()).map(str::to_string);
        let me = self.clone();
        let t = title.to_string();
        crate::util::blocking(move || {
            crate::sdk::rename_session(&me.projects_dir, &raw, &t)?;
            let Some(desktop_id) = desktop_id else { return Ok(Renamed { desktop: false, desktop_error: None }) };
            Ok(match me.rename_desktop(&desktop_id, &raw, &t) {
                Ok(()) => Renamed { desktop: true, desktop_error: None },
                Err(e) => Renamed { desktop: false, desktop_error: Some(e) },
            })
        })
        .await
    }

    // The desktop's record of the session, with only its title changed and
    // the title marked as yours. The file is undocumented: it's read again
    // just before the write, and left alone unless it's still the same
    // session's.
    fn rename_desktop(&self, desktop_id: &str, cli_id: &str, title: &str) -> Result<(), String> {
        let file = lock(&self.desktop_files).get(desktop_id).cloned().ok_or_else(|| core::NO_RECORD.to_string())?;
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let record = xenonet_base::json::parse(&text).map_err(|e| e.to_string())?;
        let out = core::renamed_record(record, Some(&json!(desktop_id)), Some(&json!(cli_id)), Some(&json!(title))).ok_or_else(|| core::OTHER_SESSION.to_string())?;
        let tmp = format!("{file}.xenonet-{}.tmp", std::process::id());
        std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
        // Windows refuses the rename for a moment while the desktop has the
        // file open.
        let mut attempt = 0;
        loop {
            match std::fs::rename(&tmp, &file) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let busy = matches!(xenonet_base::fs::code_of(&e), Some("EPERM" | "EBUSY" | "EACCES"));
                    if attempt >= 4 || !busy {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(e.to_string());
                    }
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            }
        }
    }
}
