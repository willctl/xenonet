//! Claude Code's sessions and transcripts for the map: the transcripts
//! under ~/.claude/projects (`sdk`), merged with Claude desktop's records
//! of them (`desktop`), a page of a transcript (`transcript`), and a rename
//! written where Claude keeps a session's name.

use super::desktop::{self, key_of};
use super::{sdk, transcript};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use xenonet_adapter::reader::{Lister, Renamed, VendorCheck};
use xenonet_sources::lines::{opt_prop, prop, Js, TypeError};
use xenonet_sources::record::{nn, obj_json, Obj, F};
use xenonet_sources::session::{ClaudeSession, Parent, ParentKind};
use xenonet_text::text::{display, to_number, truthy};
use xenonet_text::{json as js, utf16};

const DESKTOP_SOURCE: &str = "anthropic/claude-desktop/session";
const LISTING_SOURCE: &str = "anthropic/agent-sdk/session-listing";
const MESSAGE_SOURCE: &str = "anthropic/agent-sdk/session-message";

/// A lock, whichever thread held it last: a panic elsewhere while holding
/// it leaves what it guards as it was, which the lister can still use.
fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The lister: where it reads, and what its last listing found.
pub struct Claude {
    projects_dir: String,
    desktop_dir: Option<String>,
    /// Where each desktop session's file is, by its desktop id, as the last
    /// listing found them, for renaming one.
    desktop_files: Mutex<HashMap<String, String>>,
    /// What the last listing read of each transcript.
    listings: Mutex<sdk::Listings>,
}

impl Claude {
    pub fn new(projects_dir: &str, desktop_dir: Option<&str>) -> Claude {
        Claude { projects_dir: projects_dir.into(), desktop_dir: desktop_dir.map(str::to_string), desktop_files: Mutex::default(), listings: Mutex::default() }
    }

    // Claude desktop's records, by the CLI session each is of. Sessions sit
    // in claude-code-sessions/<account>/<organization>/, so the folders say
    // which Claude account each ran as.
    fn desktop(&self, vendor: &dyn VendorCheck) -> Vec<Value> {
        let Some(root) = self.desktop_dir.as_ref().map(|d| Path::new(d).join("claude-code-sessions").to_string_lossy().into_owned()) else { return Vec::new() };
        let mut records = Vec::new();
        for f in desktop::desktop_files(&root) {
            // A file that isn't JSON is passed over.
            let Some(data) = f.data else { continue };
            if let Some(d) = vendor.sample(DESKTOP_SOURCE, data, None, None, 16) {
                records.push((f.file, f.parts, d));
            }
        }
        let (desktop, files) = desktop::desktop_index(records);
        lock(&self.desktop_files).extend(files);
        desktop
    }

    // The desktop's record of the session, with only its title changed and
    // the title marked as yours. The file is undocumented: it's read again
    // just before the write, and left alone unless it's still the same
    // session's.
    fn rename_desktop(&self, desktop_id: &str, cli_id: &str, title: &str) -> Result<(), String> {
        let file = lock(&self.desktop_files).get(desktop_id).cloned().ok_or_else(|| desktop::NO_RECORD.to_string())?;
        let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        let record = js::parse(&text).map_err(|e| e.to_string())?;
        let out = desktop::renamed_record(record, Some(&json!(desktop_id)), Some(&json!(cli_id)), Some(&json!(title))).ok_or_else(|| desktop::OTHER_SESSION.to_string())?;
        let tmp = format!("{file}.xenonet-{}.tmp", std::process::id());
        std::fs::write(&tmp, out).map_err(|e| e.to_string())?;
        // Windows refuses the rename for a moment while the desktop has the
        // file open.
        let mut attempt = 0;
        loop {
            match std::fs::rename(&tmp, &file) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    let busy = matches!(xenonet_os::fs::code_of(&e), Some("EPERM" | "EBUSY" | "EACCES"));
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

impl Lister for Claude {
    /// Claude Code's sessions, with what Claude desktop says of them.
    fn sessions(&self, vendor: &dyn VendorCheck) -> Result<Vec<Value>, String> {
        let listed: Vec<Value> = sdk::list_sessions_with(&self.projects_dir, &mut lock(&self.listings))
            .into_iter()
            .filter_map(|s| vendor.sample(LISTING_SOURCE, s, Some(sdk::VERSION), None, 16))
            .collect();
        let desktop = self.desktop(vendor);
        sessions(&listed, &desktop).map(|list| list.iter().map(obj_json).collect()).map_err(|e| e.0)
    }

    /// A page of a session's transcript, newest first; `cursor` is the
    /// count of messages already shown from the end.
    fn transcript(&self, vendor: &dyn VendorCheck, raw_id: &str, cursor: Option<&str>, limit: f64, _files: &dyn Fn() -> Vec<String>) -> Result<Value, String> {
        // Only the page's messages are read whole.
        let all = sdk::messages(&self.projects_dir, raw_id);
        let cursor = cursor.map(|c| Value::String(c.into()));
        let (start, end, next) = transcript::page(all.len(), cursor.as_ref(), limit);
        let page: Vec<Value> = all
            .page(start, end)
            .into_iter()
            .filter_map(|m| {
                let kind = m.get("type").and_then(Value::as_str).map(str::to_string);
                vendor.sample(MESSAGE_SOURCE, m, Some(sdk::VERSION), kind.as_deref(), 16)
            })
            .collect();
        let entries: Vec<Value> = transcript::entries(&page).map_err(|e| e.0)?.iter().map(obj_json).collect();
        Ok(json!({ "entries": entries, "cursor": next, "total": all.len() }))
    }

    /// Renames a session where Claude keeps its name: Claude Code's
    /// transcript, and Claude desktop's record too when the session is in
    /// the desktop.
    fn rename(&self, _vendor: &dyn VendorCheck, node: &Value, title: &str) -> Result<Renamed, String> {
        let raw = node.get("rawId").and_then(Value::as_str).unwrap_or_default();
        let desktop_id = node.get("desktopId").and_then(Value::as_str).filter(|d| !d.is_empty());
        sdk::rename_session(&self.projects_dir, raw, title)?;
        let mut saved = vec!["Claude Code"];
        let Some(desktop_id) = desktop_id else { return Ok(Renamed { saved, desktop: None }) };
        let desktop = self.rename_desktop(desktop_id, raw, title);
        if desktop.is_ok() {
            saved.push("Claude desktop");
        }
        Ok(Renamed { saved, desktop: Some(desktop) })
    }
}

/// The Agent SDK's sessions (after the lenient reader), with what Claude
/// desktop's records say of them: its title first, the repo a worktree
/// session branched from, starred and archived, its model, the session
/// that spawned it, and the account its folders name.
pub fn sessions(listed: &[Value], desktop: &[Value]) -> Result<Vec<Obj>, TypeError> {
    let by_cli: Vec<(String, &Value)> = desktop.iter().enumerate().map(|(i, d)| (key_of(opt_prop(Some(d), "cliSessionId"), i), d)).collect();
    // desktop id → its CLI session id; the last of each desktop id wins.
    let mut desktop_to_cli: Vec<(String, Js<'_>)> = Vec::new();
    for (i, d) in desktop.iter().enumerate() {
        let k = key_of(opt_prop(Some(d), "sessionId"), i + 1_000_000);
        let v = opt_prop(Some(d), "cliSessionId");
        match desktop_to_cli.iter_mut().find(|(x, _)| *x == k) {
            Some((_, at)) => *at = v,
            None => desktop_to_cli.push((k, v)),
        }
    }
    let mut out = Vec::with_capacity(listed.len());
    for (n, s) in listed.iter().enumerate() {
        let s = Some(s);
        let get = |k: &str| prop(s, k);
        let sid = get("sessionId")?;
        let want = key_of(sid, usize::MAX - n);
        let d = by_cli.iter().find(|(k, _)| *k == want).map(|(_, d)| *d);
        let dp = |k: &str| opt_prop(d, k);
        let spawned = opt_prop(dp("spawnedFrom"), "sessionId");
        let parent_cli = if truthy(spawned) {
            let k = key_of(spawned, usize::MAX);
            desktop_to_cli.iter().find(|(x, _)| *x == k).and_then(|(_, v)| *v)
        } else {
            None
        };
        let parent_cli = parent_cli.filter(|p| truthy(Some(p)));
        let first = get("firstPrompt")?;
        let title = [dp("title"), get("customTitle")?, get("summary")?].into_iter().flatten().find(|v| truthy(Some(v))).cloned();
        let title = match title {
            Some(t) => t,
            None => {
                let cut = match nn(first) {
                    None => None,
                    Some(Value::String(p)) => Some(Value::String(utf16::head(p, 80))),
                    Some(Value::Array(a)) => Some(Value::Array(a.iter().take(80).cloned().collect())),
                    Some(_) => return Err(TypeError("s.firstPrompt?.slice is not a function".into())),
                };
                cut.filter(|c| truthy(Some(c))).unwrap_or_else(|| Value::String("(untitled session)".into()))
            }
        };
        let cwd = [dp("originCwd"), get("cwd")?].into_iter().flatten().find(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null);
        let updated = {
            let a = to_number(Some(nn(get("lastModified")?).unwrap_or(&Value::from(0))));
            let b = to_number(Some(nn(dp("lastActivityAt")).unwrap_or(&Value::from(0))));
            let m = if a.is_nan() || b.is_nan() { f64::NAN } else { a.max(b) };
            if m == 0.0 || m.is_nan() {
                F::null()
            } else {
                F::V(js::number(m))
            }
        };
        let created = nn(get("createdAt")?).or(nn(dp("createdAt")));
        let or_null = |v: Js<'_>| nn(v).cloned().unwrap_or(Value::Null);
        out.push(
            ClaudeSession {
                raw_id: sid.cloned(),
                desktop_id: or_null(dp("sessionId")),
                in_desktop: d.is_some(),
                title,
                preview: or_null(first),
                cwd,
                worktree: or_null(dp("worktreePath")),
                created_at: or_null(created),
                updated_at: updated,
                archived: truthy(dp("isArchived")),
                starred: truthy(dp("isStarred")),
                model: or_null(dp("model")),
                effort: or_null(dp("effort")),
                permission_mode: or_null(dp("permissionMode")),
                branch: or_null(nn(dp("branch")).or(nn(get("gitBranch")?))),
                tag: or_null(get("tag")?),
                size_bytes: or_null(get("fileSize")?),
                parent: parent_cli.map(|p| Parent { id: format!("claude:{}", display(p)), kind: ParentKind::Spawn }),
                claude_account: or_null(dp("account")),
            }
            .record(),
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::reader::Unchecked;

    fn get<'a>(o: &'a Obj, k: &str) -> Option<&'a F> {
        o.iter().find(|(n, _)| *n == k).map(|(_, f)| f)
    }

    #[test]
    fn merges_the_desktops_records() {
        let a = "0190d3e0-0000-7000-a000-000000000001".to_string();
        let o = "0190d3e0-0000-7000-b000-000000000001".to_string();
        let recs = vec![
            ("/d/local_1.json".to_string(), vec![a.clone(), o.clone()], json!({ "sessionId": "local_1", "cliSessionId": "c1", "title": "Refunds", "lastActivityAt": 50 })),
            (
                "/d/local_2.json".to_string(),
                vec![a.clone(), o.clone()],
                json!({ "sessionId": "local_2", "cliSessionId": "c2", "title": "", "spawnedFrom": { "sessionId": "local_1" } }),
            ),
            ("/d/x/local_3.json".to_string(), vec!["x".into()], json!({ "sessionId": "local_3", "cliSessionId": "c3" })),
        ];
        let (desktop, files) = desktop::desktop_index(recs);
        assert_eq!(files.len(), 3);
        assert_eq!(desktop[0]["account"], json!({ "account": a, "org": o }));
        let listed =
            [json!({ "sessionId": "c1", "lastModified": 10, "firstPrompt": "Add refunds" }), json!({ "sessionId": "c2", "firstPrompt": "Review" }), json!({ "sessionId": "c9" })];
        let s = sessions(&listed, &desktop).unwrap();
        assert_eq!(get(&s[0], "title"), Some(&F::s("Refunds")));
        assert_eq!(get(&s[0], "updatedAt"), Some(&F::V(json!(50))));
        assert_eq!(get(&s[1], "title"), Some(&F::s("Review")));
        assert_eq!(get(&s[1], "parentId"), Some(&F::s("claude:c1")));
        assert_eq!(get(&s[2], "inDesktop"), Some(&F::V(json!(false))));
        assert_eq!(get(&s[2], "title"), Some(&F::s("(untitled session)")));
    }

    #[test]
    fn lists_reads_and_renames_a_transcript() {
        let home = std::env::temp_dir().join(format!("xenonet-claude-lister-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let projects = home.join("projects");
        let folder = projects.join("-w-atlas");
        std::fs::create_dir_all(&folder).expect("a folder");
        let id = "0190d3e0-0000-7000-8000-00000000aaaa";
        let lines = [
            json!({ "type": "user", "uuid": "u1", "parentUuid": null, "sessionId": id, "cwd": "/w/atlas", "version": "2.0.1", "timestamp": "2026-10-03T12:00:00Z", "message": { "role": "user", "content": "Add a health check route" } }),
            json!({ "type": "assistant", "uuid": "a1", "parentUuid": "u1", "sessionId": id, "cwd": "/w/atlas", "version": "2.0.1", "timestamp": "2026-10-03T12:00:05Z", "message": { "id": "m1", "role": "assistant", "model": "claude-sonnet-4-5", "stop_reason": "end_turn", "usage": { "input_tokens": 10, "output_tokens": 20 }, "content": [{ "type": "text", "text": "Added GET /health." }] } }),
        ];
        let text: String = lines.iter().map(|l| js::stringify(l, 0) + "\n").collect();
        std::fs::write(folder.join(format!("{id}.jsonl")), text).expect("a transcript");
        let c = Claude::new(&projects.to_string_lossy(), None);
        let list = c.sessions(&Unchecked).expect("the sessions");
        assert_eq!((list.len(), list[0]["id"].as_str(), list[0]["inDesktop"].as_bool()), (1, Some(&*format!("claude:{id}")), Some(false)));
        let page = c.transcript(&Unchecked, id, None, 120.0, &Vec::new).expect("a page");
        assert_eq!((page["entries"][0]["text"].as_str(), page["total"].as_u64()), (Some("Add a health check route"), Some(2)));
        let renamed = c.rename(&Unchecked, &json!({ "rawId": id }), "Health check").expect("renamed");
        assert_eq!((renamed.saved, renamed.desktop), (vec!["Claude Code"], None));
        assert_eq!(c.sessions(&Unchecked).expect("the sessions")[0]["title"], "Health check");
        assert!(c.rename(&Unchecked, &json!({ "rawId": id, "desktopId": "local_9" }), "Again").expect("renamed").desktop.is_some_and(|d| d.is_err()), "no desktop record");
        std::fs::remove_dir_all(&home).expect("cleaned up");
    }
}
