//! core/codex.js's runtime: the `codex app-server` XenoNet asks for Codex's
//! threads (a child process, started on the first request and again after
//! it exits, its requests answered or timed out here), Codex desktop's
//! projects from its state database, and a thread's items from the
//! app-server or from its rollout. What each line and answer means is
//! xenonet-sources' codex.rs.

use crate::blocks::{health_of, Block};
use crate::util::{codex_obj, lock, now_ms, num};
use crate::vendor::Vendor;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::oneshot;
use xenonet_blocks::lifecycle::{Lifecycle, State};
use xenonet_sources::codex::{self, Message, RolloutEntries, Stderr, Stdout};

/// A request's failure: its message, and the app-server's error when it
/// gave one.
#[derive(Clone, Debug)]
pub struct Failed {
    pub message: String,
    pub rpc: Option<Value>,
}

impl Failed {
    fn said(m: impl Into<String>) -> Failed {
        Failed { message: m.into(), rpc: None }
    }
}

type Waiting = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, Failed>>>>>;

struct Proc {
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Waiting,
    serial: u64,
}

struct Inner {
    life: Lifecycle,
    proc: Option<Proc>,
    ready: bool,
    next_id: u64,
    serial: u64,
    stderr: Arc<Mutex<Stderr>>,
    /// Codex's version, from initialize's user agent.
    version: Option<String>,
    /// The versions whose app-server said it can't list a thread's items.
    no_items: Vec<Option<String>>,
}

pub struct CodexClient {
    bin: String,
    version: String,
    vendor: Arc<Vendor>,
    inner: Mutex<Inner>,
    starting: tokio::sync::Mutex<()>,
    rollout: Mutex<Option<Rollout>>,
}

// The rollout read last, kept while it's paged through, and read on from
// where it ended as it grows.
struct Rollout {
    file: String,
    offset: u64,
    rest: Vec<u8>,
    entries: RolloutEntries,
}

fn fail_all(pending: &Waiting, why: &str) {
    for (_, tx) in lock(pending).drain() {
        let _ = tx.send(Err(Failed::said(why)));
    }
}

impl CodexClient {
    pub fn new(bin: String, version: String, vendor: Arc<Vendor>) -> Arc<CodexClient> {
        let mut life = Lifecycle::new(now_ms());
        for c in ["starts", "requests", "timeouts"] {
            life.counters.set(c, 0.0);
        }
        Arc::new(CodexClient {
            bin,
            version,
            vendor,
            inner: Mutex::new(Inner { life, proc: None, ready: false, next_id: 1, serial: 0, stderr: Arc::default(), version: None, no_items: Vec::new() }),
            starting: tokio::sync::Mutex::new(()),
            rollout: Mutex::new(None),
        })
    }

    fn spawn(self: &Arc<Self>) -> Result<(), Failed> {
        // Each start counts, and so does the initialize it was for, which a
        // start that fails fails too (as the JS's spawn's error event does).
        lock(&self.inner).life.count("starts", 1.0);
        let spawned = Command::new(&self.bin)
            .arg("app-server")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                lock(&self.inner).life.count("requests", 1.0);
                // As Node words it: spawn <file> <code>.
                let code = xenonet_base::fs::code_of(&e).map_or_else(|| e.to_string(), str::to_string);
                return Err(Failed::said(format!("spawn {} {code}", self.bin)));
            }
        };
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
            return Err(Failed::said("codex app-server has no pipes"));
        };
        let stdin = Arc::new(tokio::sync::Mutex::new(stdin));
        let pending: Waiting = Arc::default();
        let (serial, err_tail) = {
            let mut g = lock(&self.inner);
            g.serial += 1;
            g.proc = Some(Proc { stdin: stdin.clone(), pending: pending.clone(), serial: g.serial });
            (g.serial, g.stderr.clone())
        };
        // What it says: answers, and requests of its own, declined.
        let (p, w) = (pending.clone(), stdin.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut io = Stdout::default();
            while let Ok(Some(line)) = lines.next_line().await {
                for m in io.take(&format!("{line}\n")) {
                    match m {
                        Message::Decline(text) => {
                            let _ = w.lock().await.write_all(text.as_bytes()).await;
                        }
                        Message::Answer { id, result, error } => {
                            let Some(id) = id.as_u64() else { continue };
                            let Some(tx) = lock(&p).remove(&id) else { continue };
                            let _ = tx.send(match error {
                                Some(e) => Err(Failed { message: e.0, rpc: Some(e.1) }),
                                None => Ok(crate::util::codex_f(&result).unwrap_or(Value::Null)),
                            });
                        }
                        Message::Other => {}
                    }
                }
            }
        });
        let tail = err_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                lock(&tail).push(&format!("{line}\n"));
            }
        });
        // Its end fails what waits on it; one XenoNet stopped, or another
        // has replaced, is no longer this client's.
        let me = Arc::downgrade(self);
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            let why = lock(&err_tail).exited(Some(&code.map_or(Value::Null, |c| json!(c))));
            fail_all(&pending, &why);
            let Some(me) = me.upgrade() else { return };
            let mut g = lock(&me.inner);
            if g.proc.as_ref().is_some_and(|p| p.serial == serial) {
                g.proc = None;
                g.ready = false;
                g.life.fail(&why, now_ms());
            }
        });
        Ok(())
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, Failed> {
        let (stdin, pending, id, rx) = {
            let mut g = lock(&self.inner);
            g.life.count("requests", 1.0);
            let Some(p) = &g.proc else { return Err(Failed::said("codex app-server isn't running")) };
            let (stdin, pending) = (p.stdin.clone(), p.pending.clone());
            let id = g.next_id;
            g.next_id += 1;
            let (tx, rx) = oneshot::channel();
            lock(&pending).insert(id, tx);
            (stdin, pending, id, rx)
        };
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let _ = stdin.lock().await.write_all(format!("{}\n", xenonet_base::json::stringify(&msg, 0)).as_bytes()).await;
        match tokio::time::timeout(Duration::from_millis(codex::REQUEST_TIMEOUT_MS as u64), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(Failed::said("codex app-server stopped")),
            Err(_) => {
                lock(&pending).remove(&id);
                lock(&self.inner).life.count("timeouts", 1.0);
                Err(Failed::said(format!("{method} timed out")))
            }
        }
    }

    /// Starts the app-server, unless it's running.
    async fn connect(self: &Arc<Self>) -> Result<(), Failed> {
        let _one = self.starting.lock().await;
        if lock(&self.inner).ready {
            return Ok(());
        }
        if let Err(e) = self.spawn() {
            lock(&self.inner).life.fail(&e.message, now_ms());
            return Err(e);
        }
        let result = self
            .call("initialize", json!({ "clientInfo": { "name": "xenonet", "title": "XenoNet", "version": self.version }, "capabilities": { "experimentalApi": true } }))
            .await?;
        let stdin = lock(&self.inner).proc.as_ref().map(|p| p.stdin.clone());
        if let Some(w) = stdin {
            let _ = w.lock().await.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"initialized\"}\n").await;
        }
        let version = result.get("userAgent").and_then(Value::as_str).and_then(xenonet_schema::vendor::codex_version_of);
        self.vendor.check("openai/codex/app-server", json!({ "initialize": result }), version.as_deref(), None);
        let mut g = lock(&self.inner);
        g.version = version;
        g.ready = true;
        // Connected is started, as the JS's block counts it.
        g.life.started = true;
        g.life.ok(now_ms());
        Ok(())
    }

    pub async fn request(self: &Arc<Self>, method: &str, params: Value) -> Result<Value, Failed> {
        self.connect().await?;
        match self.call(method, params).await {
            Ok(r) => {
                lock(&self.inner).life.ok(now_ms());
                Ok(r)
            }
            Err(e) => {
                let mut g = lock(&self.inner);
                if g.proc.is_some() {
                    g.life.problem(&e.message, now_ms());
                }
                Err(e)
            }
        }
    }

    // An answer to `method`, read leniently: without what's of another
    // type, or {} when it isn't an object.
    fn answered(&self, method: &str, result: Value) -> Value {
        let version = lock(&self.inner).version.clone();
        self.vendor
            .check("openai/codex/app-server", json!({ method: result }), version.as_deref(), None)
            .and_then(|v| v.get(method).cloned())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    }

    /// Codex's threads, archived ones too, as sessions.
    pub async fn threads(self: &Arc<Self>) -> Result<Vec<Value>, String> {
        let mut sessions = Vec::new();
        for archived in [false, true] {
            let mut cursor = Value::Null;
            for _ in 0..20 {
                let res = self
                    .request("thread/list", json!({ "archived": archived, "limit": 100, "cursor": cursor, "sourceKinds": codex::SOURCE_KINDS, "useStateDbOnly": true }))
                    .await
                    .map_err(|e| e.message)?;
                let page = self.answered("thread/list", res);
                for t in crate::util::items(page.get("data")) {
                    sessions.push(codex_obj(&codex::thread_to_session(Some(t), archived).map_err(|e| e.0)?));
                }
                cursor = page.get("nextCursor").cloned().filter(|c| !c.is_null()).unwrap_or(Value::Null);
                if !xenonet_base::text::truthy(Some(&cursor)) {
                    break;
                }
            }
        }
        Ok(sessions)
    }

    /// Renames a thread, as the ChatGPT desktop app does.
    pub async fn rename(self: &Arc<Self>, raw_id: &str, name: &str) -> Result<(), String> {
        self.request("thread/name/set", json!({ "threadId": raw_id, "name": name })).await.map(|_| ()).map_err(|e| e.message)
    }

    /// A page of a thread's items, newest first; one whose app-server can't
    /// list them is read from its rollout (`rollout`, when found).
    pub async fn read_thread(self: &Arc<Self>, raw_id: &str, cursor: Option<&str>, limit: f64, rollout: Option<String>) -> Result<Value, String> {
        let from_rollout = |err: String| -> Result<Value, String> {
            match &rollout {
                Some(file) => self.rollout_page(file, cursor, limit),
                None => Err(err),
            }
        };
        if rollout.is_some() && cursor.is_some_and(|c| c.starts_with("rollout:")) {
            return from_rollout("XenoNet can't find this thread's rollout any more".into());
        }
        let version = lock(&self.inner).version.clone();
        if rollout.is_some() && lock(&self.inner).no_items.contains(&version) {
            let which = version.as_ref().map_or_else(|| "This Codex".to_string(), |v| format!("Codex {v}"));
            return from_rollout(format!("{which} can't list a thread's items, and XenoNet can't find this thread's rollout"));
        }
        let res = match self.request("thread/items/list", json!({ "threadId": raw_id, "limit": num(limit), "cursor": cursor, "sortDirection": "desc" })).await {
            Ok(r) => r,
            Err(e) => {
                if codex::says_unsupported(e.rpc.as_ref()) {
                    let mut g = lock(&self.inner);
                    let v = g.version.clone();
                    g.no_items.push(v);
                }
                if cursor.is_some() {
                    return Err(e.message);
                }
                return from_rollout(e.message);
            }
        };
        let page = self.answered("thread/items/list", res);
        let entries: Vec<Value> = codex::normalize_items(page.get("data")).map_err(|e| e.0)?.iter().map(codex_obj).collect();
        Ok(json!({ "entries": entries, "cursor": page.get("nextCursor").cloned().unwrap_or(Value::Null) }))
    }

    /// A page of a thread from its rollout file: newest first, `cursor`
    /// (`"rollout:<n>"`) continuing further back.
    #[expect(clippy::case_sensitive_file_extension_comparisons, reason = "as the JS's endsWith, which tells .zst from .ZST")]
    pub fn rollout_page(&self, file: &str, cursor: Option<&str>, limit: f64) -> Result<Value, String> {
        let mut slot = lock(&self.rollout);
        if slot.as_ref().is_none_or(|r| r.file != file) {
            *slot = Some(Rollout { file: file.into(), offset: 0, rest: Vec::new(), entries: RolloutEntries::default() });
        }
        let Some(r) = slot.as_mut() else { return Err("no rollout".into()) };
        if file.ends_with(".zst") {
            return Err("XenoNet can't read a compressed archive here yet".into());
        }
        let data = std::fs::read(file).map_err(|e| e.to_string())?;
        let (restarted, start) = xenonet_sources::lines::read_from(data.len() as u64, r.offset);
        if restarted {
            *r = Rollout { file: file.into(), offset: start, rest: Vec::new(), entries: RolloutEntries::default() };
        }
        if (data.len() as u64) > r.offset {
            let mut chunk = std::mem::take(&mut r.rest);
            chunk.extend_from_slice(&data[r.offset as usize..]);
            r.offset = data.len() as u64;
            let t = xenonet_sources::lines::take(&chunk);
            let mut sampling = xenonet_schema::vendor::Sampling::default();
            for l in &t.lines {
                if let Some(raw) = self.vendor.line(true, l, &mut sampling) {
                    let _ = r.entries.add(raw);
                }
            }
            match t.last {
                Some(last) => {
                    if let Some(raw) = self.vendor.record(true, last, &mut sampling) {
                        let _ = r.entries.add(raw);
                    }
                }
                None => r.rest = t.rest,
            }
        }
        let cursor = cursor.map(|c| Value::String(c.into()));
        let all = r.entries.list();
        let (a, b, next) = codex::rollout_page(all.len(), cursor.as_ref(), limit);
        Ok(json!({ "entries": all[a..b].iter().map(codex_obj).collect::<Vec<_>>(), "cursor": next, "total": all.len() }))
    }

    pub fn stop(&self) {
        let mut g = lock(&self.inner);
        if let Some(p) = g.proc.take() {
            fail_all(&p.pending, "codex app-server stopped");
        }
        g.ready = false;
        if g.life.started {
            g.life.begin_stop();
            g.life.set_state(State::Stopped, now_ms());
        }
    }
}

impl Block for CodexClient {
    fn name(&self) -> String {
        "codex-app-server".into()
    }
    fn health(&self) -> Value {
        health_of("codex-app-server", "client", &lock(&self.inner).life, &[], Value::Null)
    }
}

/// Codex desktop's projects, read-only from its state database. A
/// database of another number is a version the schema wasn't made for,
/// and a column it reads gone is drift; any failure only takes the
/// grouping away.
pub fn native_projects(codex_home: &str, vendor: &Vendor) -> Vec<Value> {
    let names: Vec<String> = std::fs::read_dir(codex_home).map(|d| d.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect()).unwrap_or_default();
    for n in codex::other_state_dbs(&names) {
        vendor.pick("openai/codex/state-db", Some(&n));
    }
    let file = std::path::Path::new(codex_home).join(format!("state_{}.sqlite", codex::STATE_DB));
    let Some(read) = codex::read_state_db(&file.to_string_lossy()) else { return Vec::new() };
    let rows = match read {
        codex::StateRows::Missing(m) => {
            for path in m {
                vendor.missing("openai/codex/state-db", &path, Some(&codex::STATE_DB.to_string()));
            }
            return Vec::new();
        }
        codex::StateRows::Rows { projects, roots } => {
            let cells = |rows: Vec<Vec<(&'static str, codex::Cell)>>| -> Value {
                Value::Array(
                    rows.into_iter()
                        .map(|r| {
                            Value::Object(
                                r.into_iter()
                                    .map(|(k, c)| {
                                        (
                                            k.to_string(),
                                            match c {
                                                codex::Cell::V(v) => v,
                                                codex::Cell::Bytes(b) => json!({ "type": "Buffer", "data": b }),
                                            },
                                        )
                                    })
                                    .collect::<Map<String, Value>>(),
                            )
                        })
                        .collect(),
                )
            };
            json!({ "projects": cells(projects), "project_roots": cells(roots) })
        }
    };
    let checked = vendor.check("openai/codex/state-db", rows, Some(&codex::STATE_DB.to_string()), None);
    match codex::native_projects(checked.as_ref()) {
        Ok(list) => list.iter().map(codex_obj).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_schema::drift::DriftLog;

    #[tokio::test]
    async fn an_app_server_that_isnt_there_fails_the_block() {
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let c = CodexClient::new("/nowhere/codex".into(), "0.0.0".into(), vendor.clone());
        assert!(c.threads().await.is_err());
        assert_eq!(c.health()["state"], "failed");
        assert_eq!(native_projects("/nowhere", &vendor), [] as [Value; 0]);
    }

    #[test]
    fn a_rollout_read_a_page_at_a_time() {
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let dir = std::env::temp_dir().join(format!("xenonet-wire-rollout-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let file = dir.join("rollout-x.jsonl");
        let line = |m: &str| {
            format!(
                "{}\n",
                xenonet_base::json::stringify(&json!({ "type": "event_msg", "timestamp": "2026-10-05T13:00:00Z", "payload": { "type": "user_message", "message": m } }), 0)
            )
        };
        std::fs::write(&file, format!("{}{}", line("one"), line("two"))).expect("a rollout");
        let c = CodexClient::new("codex".into(), "0.0.0".into(), vendor);
        let f = file.to_string_lossy().into_owned();
        let page = c.rollout_page(&f, None, 1.0).expect("a page");
        assert_eq!((page["entries"][0]["text"].as_str(), page["cursor"].as_str()), (Some("two"), Some("rollout:1")));
        let older = c.rollout_page(&f, Some("rollout:1"), 1.0).expect("a page");
        assert_eq!(older["entries"][0]["text"], "one");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}

#[cfg(all(test, unix))]
mod app_server {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use xenonet_schema::drift::DriftLog;

    // A stand-in for `codex app-server`: it answers initialize, a listing,
    // a thread's items and a rename, by the request's id.
    const FAKE: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf '{"id":%s,"result":{"userAgent":"codex_cli_rs/0.150.0 (Mac OS 15.0)"}}\n' "$id" ;;
    *'"method":"thread/list"'*'"archived":false'*) printf '{"id":%s,"result":{"data":[{"id":"0190d3e0-0000-7000-8000-00000000cccc","preview":"Fix the build\\nand more","createdAt":1790000000,"updatedAt":1790000100,"cwd":"/w/p","status":"idle","source":"cli"}],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/items/list"'*) printf '{"id":%s,"result":{"data":[{"item":{"id":"i1","type":"agentMessage","text":"Hello"}}],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/name/set"'*) printf '{"id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

    #[tokio::test(flavor = "multi_thread")]
    async fn threads_items_and_a_rename_through_the_app_server() {
        let dir = std::env::temp_dir().join(format!("xenonet-wire-app-server-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let bin = dir.join("codex");
        std::fs::write(&bin, FAKE).expect("the stand-in");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("runnable");
        let vendor = Vendor::new(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas"), Arc::new(Mutex::new(DriftLog::default())));
        let c = CodexClient::new(bin.to_string_lossy().into_owned(), "0.3.0".into(), vendor);
        let threads = c.threads().await.expect("the threads");
        assert_eq!(threads.len(), 1);
        assert_eq!((threads[0]["id"].as_str(), threads[0]["title"].as_str()), (Some("codex:0190d3e0-0000-7000-8000-00000000cccc"), Some("Fix the build")));
        assert_eq!(lock(&c.inner).version.as_deref(), Some("0.150.0"), "Codex's version, from its user agent");
        let page = c.read_thread("0190d3e0-0000-7000-8000-00000000cccc", None, 120.0, None).await.expect("a page");
        assert_eq!(page["entries"][0]["text"], "Hello");
        c.rename("0190d3e0-0000-7000-8000-00000000cccc", "Build fix").await.expect("renamed");
        let h = c.health();
        assert_eq!((h["state"].as_str(), h["counters"]["starts"].as_f64()), (Some("running"), Some(1.0)));
        c.stop();
        assert_eq!(c.health()["state"], "stopped");
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
