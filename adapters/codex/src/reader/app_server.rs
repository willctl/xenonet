//! The `codex app-server` XenoNet asks for Codex's threads: a child
//! process, started on the first request and again after it exits, its
//! requests answered or timed out here, and its stdio (its lines, the
//! requests it makes of XenoNet, its answers and errors). The lister over
//! it: Codex's threads as sessions, a thread's items (from the app-server,
//! or from its rollout when the app-server can't list them), a rename,
//! Codex desktop's projects, and the client's block.

use super::rollouts::Rollouts;
use super::state_db;
use super::threads::{normalize_items, thread_to_session};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::runtime::Handle;
use tokio::sync::oneshot;
use xenonet_adapter::reader::{BlockHealth, Failed, Lister, ListerEnv, Renamed, VendorCheck};
use xenonet_base::text::{js_string, js_trim, truthy};
use xenonet_base::time::now_ms;
use xenonet_base::{dirs, json as js};
use xenonet_blocks::lifecycle::{Lifecycle, State};
use xenonet_sources::lines::{opt_prop, Js};
use xenonet_sources::record::{nn, obj_json, tpl, F};

/// The kinds of thread the listing asks for.
pub const SOURCE_KINDS: [&str; 9] = ["cli", "vscode", "exec", "appServer", "subAgent", "subAgentReview", "subAgentCompact", "subAgentThreadSpawn", "subAgentOther"];

pub const REQUEST_TIMEOUT_MS: f64 = 20000.0;

const SOURCE: &str = "openai/codex/app-server";

fn lock<T: ?Sized>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------- the app-server's stdio ----------

/// Whether the app-server's error says it can't list a thread's items:
/// an unknown method, or words to say it isn't supported.
pub fn says_unsupported(rpc: Js<'_>) -> bool {
    if !truthy(rpc) {
        return false;
    }
    if matches!(opt_prop(rpc, "code"), Some(Value::Number(n)) if n.as_f64() == Some(-32601.0)) {
        return true;
    }
    let m = nn(opt_prop(rpc, "message")).map_or_else(String::new, js_string).to_ascii_lowercase();
    ["not supported", "not yet supported", "unsupported", "not implemented", "unknown method", "method not found"].iter().any(|w| m.contains(w))
}

/// What a line from the app-server is.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    /// A request of its own (an approval): XenoNet declines, with this line.
    Decline(String),
    /// An answer to request `id`: its result, or its error's message and
    /// the error itself.
    Answer { id: Value, result: Box<F>, error: Option<Box<(String, Value)>> },
    /// A notification, or what isn't JSON.
    Other,
}

/// One app-server's stdout, read into lines.
#[derive(Default, Clone, Debug)]
pub struct Stdout {
    buf: String,
}

impl Stdout {
    /// What a chunk of stdout completes: each whole line, trimmed, that has
    /// something in it, as a message.
    pub fn take(&mut self, chunk: &str) -> Vec<Message> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.find('\n') {
            let line = js_trim(&self.buf[..nl]).to_string();
            self.buf.drain(..=nl);
            if !line.is_empty() {
                out.push(message(&line));
            }
        }
        out
    }
}

/// The last 20 lines the client's app-servers wrote to stderr.
#[derive(Default, Clone, Debug)]
pub struct Stderr {
    pub lines: Vec<String>,
}

impl Stderr {
    pub fn push(&mut self, chunk: &str) {
        for l in xenonet_sources::git::split_lines(chunk).into_iter().filter(|l| !l.is_empty()) {
            self.lines.push(l.to_string());
        }
        if self.lines.len() > 20 {
            self.lines.drain(..self.lines.len() - 20);
        }
    }

    /// Why the app-server's end failed what waited on it.
    pub fn exited(&self, code: Js<'_>) -> String {
        let tail = &self.lines[self.lines.len().saturating_sub(3)..];
        format!("codex app-server exited ({}): {}", tpl(code), tail.join(" | "))
    }
}

/// What one line from the app-server is.
pub fn message(line: &str) -> Message {
    let Ok(msg) = js::parse(line) else { return Message::Other };
    let m = Some(&msg);
    let id = opt_prop(m, "id");
    let Some(id) = id else { return Message::Other };
    if truthy(opt_prop(m, "method")) {
        // A request of the app-server's own (approvals and the like):
        // XenoNet never starts turns, so it declines.
        let mut o = Map::new();
        o.insert("jsonrpc".into(), Value::String("2.0".into()));
        o.insert("id".into(), id.clone());
        o.insert("error".into(), json!({ "code": -32601, "message": "XenoNet is read-only" }));
        return Message::Decline(format!("{}\n", js::stringify(&Value::Object(o), 0)));
    }
    let error = opt_prop(m, "error");
    if truthy(error) {
        let message = nn(opt_prop(error, "message")).map_or_else(|| "app-server error".to_string(), js_string);
        return Message::Answer { id: id.clone(), result: Box::new(F::Undef), error: Some(Box::new((message, error.cloned().unwrap_or(Value::Null)))) };
    }
    Message::Answer { id: id.clone(), result: Box::new(F::of(opt_prop(m, "result"))), error: None }
}

/// The next line of the app-server's output, without its `\n` or `\r\n`;
/// None at the end, or when the read fails. A byte that isn't UTF-8 is
/// U+FFFD, and the reading goes on: a reader that stopped there would lose
/// every line after it, while the app-server's pipe filled.
async fn next_line<R: AsyncBufRead + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> Option<String> {
    buf.clear();
    match r.read_until(b'\n', buf).await {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            if buf.last() == Some(&b'\n') {
                buf.pop();
                if buf.last() == Some(&b'\r') {
                    buf.pop();
                }
            }
            Some(String::from_utf8_lossy(buf).into_owned())
        }
    }
}

// ---------- where Codex is ----------

struct Disk;
impl dirs::Look for Disk {
    fn exists(&mut self, p: &str) -> bool {
        std::path::Path::new(p).exists()
    }
    fn readdir(&mut self, p: &str) -> Result<Vec<String>, dirs::Unreadable> {
        let names = std::fs::read_dir(p).map_err(|_| dirs::Unreadable)?;
        let mut out: Vec<String> = names.filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned())).collect();
        out.sort();
        Ok(out)
    }
    fn mtime(&mut self, p: &str) -> f64 {
        std::fs::metadata(p).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok()).unwrap_or(f64::NAN)
    }
}

/// The codex binary XenoNet runs: the one XENONET_CODEX_BIN names, else
/// ChatGPT's own, else codex.
pub fn binary(env: &ListerEnv<'_>) -> String {
    if let Some(b) = env.var("XENONET_CODEX_BIN") {
        return b;
    }
    let platform = if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(windows) {
        "win32"
    } else {
        "linux"
    };
    let desktop = dirs::Env {
        claude_desktop_dir: env.var("XENONET_CLAUDE_DESKTOP_DIR"),
        local_app_data: env.env.get("LOCALAPPDATA").and_then(Value::as_str).map(str::to_string),
        app_data: env.env.get("APPDATA").and_then(Value::as_str).map(str::to_string),
        xdg_config_home: env.env.get("XDG_CONFIG_HOME").and_then(Value::as_str).map(str::to_string),
    };
    let home = std::env::var("HOME").unwrap_or_default();
    dirs::bundled_codex_binaries(&desktop, platform, &home, &mut Disk).into_iter().next().unwrap_or_else(|| "codex".into())
}

// ---------- the client ----------

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

/// The client of one `codex app-server`, which runs on the service's
/// runtime (`handle`).
pub struct Client {
    me: Weak<Client>,
    bin: String,
    version: String,
    handle: Handle,
    inner: Mutex<Inner>,
    starting: tokio::sync::Mutex<()>,
}

fn fail_all(pending: &Waiting, why: &str) {
    for (_, tx) in lock(pending).drain() {
        let _ = tx.send(Err(Failed::said(why)));
    }
}

impl Client {
    /// A client of the app-server `bin` runs, which says it is XenoNet
    /// `version`.
    pub fn new(bin: String, version: String, handle: Handle) -> Arc<Client> {
        let mut life = Lifecycle::new(now_ms());
        for c in ["starts", "requests", "timeouts"] {
            life.counters.set(c, 0.0);
        }
        Arc::new_cyclic(|me| Client {
            me: me.clone(),
            bin,
            version,
            handle,
            inner: Mutex::new(Inner { life, proc: None, ready: false, next_id: 1, serial: 0, stderr: Arc::default(), version: None, no_items: Vec::new() }),
            starting: tokio::sync::Mutex::new(()),
        })
    }

    fn spawn(&self) -> Result<(), Failed> {
        // Each start counts, and so does the initialize it was for, which a
        // start that fails fails too.
        lock(&self.inner).life.count("starts", 1.0);
        let mut cmd = Command::new(&self.bin);
        xenonet_base::process::Background::no_window(cmd.as_std_mut());
        let spawned =
            cmd.arg("app-server").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true).spawn();
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
        let (p, w) = (pending.clone(), stdin);
        self.handle.spawn(async move {
            let (mut out, mut buf) = (BufReader::new(stdout), Vec::new());
            let mut io = Stdout::default();
            while let Some(line) = next_line(&mut out, &mut buf).await {
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
                                None => Ok(result.json().unwrap_or(Value::Null)),
                            });
                        }
                        Message::Other => {}
                    }
                }
            }
        });
        let tail = err_tail.clone();
        self.handle.spawn(async move {
            let (mut err, mut buf) = (BufReader::new(stderr), Vec::new());
            while let Some(line) = next_line(&mut err, &mut buf).await {
                lock(&tail).push(&format!("{line}\n"));
            }
        });
        // Its end fails what waits on it; one XenoNet stopped, or another
        // has replaced, is no longer this client's.
        let me = self.me.clone();
        self.handle.spawn(async move {
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
        let _ = stdin.lock().await.write_all(format!("{}\n", js::stringify(&msg, 0)).as_bytes()).await;
        match tokio::time::timeout(Duration::from_millis(REQUEST_TIMEOUT_MS as u64), rx).await {
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
    async fn connect(&self, vendor: &dyn VendorCheck) -> Result<(), Failed> {
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
        vendor.check(SOURCE, json!({ "initialize": result }), version.as_deref(), None);
        let mut g = lock(&self.inner);
        g.version = version;
        g.ready = true;
        // Connected is started, as the block counts it.
        g.life.started = true;
        g.life.ok(now_ms());
        Ok(())
    }

    async fn request_async(&self, vendor: &dyn VendorCheck, method: &str, params: Value) -> Result<Value, Failed> {
        self.connect(vendor).await?;
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

    /// A request of the app-server, started when it isn't running, waited
    /// for here.
    pub fn request(&self, vendor: &dyn VendorCheck, method: &str, params: Value) -> Result<Value, Failed> {
        self.handle.block_on(self.request_async(vendor, method, params))
    }

    // An answer to `method`, read leniently: without what's of another
    // type, or {} when it isn't an object.
    fn answered(&self, vendor: &dyn VendorCheck, method: &str, result: Value) -> Value {
        let version = lock(&self.inner).version.clone();
        vendor.check(SOURCE, json!({ method: result }), version.as_deref(), None).and_then(|v| v.get(method).cloned()).filter(Value::is_object).unwrap_or_else(|| json!({}))
    }

    /// Codex's threads, archived ones too, as sessions.
    pub fn threads(&self, vendor: &dyn VendorCheck) -> Result<Vec<Value>, String> {
        let mut sessions = Vec::new();
        for archived in [false, true] {
            let mut cursor = Value::Null;
            for _ in 0..20 {
                let res = self
                    .request(vendor, "thread/list", json!({ "archived": archived, "limit": 100, "cursor": cursor, "sourceKinds": SOURCE_KINDS, "useStateDbOnly": true }))
                    .map_err(|e| e.message)?;
                let page = self.answered(vendor, "thread/list", res);
                for t in page.get("data").and_then(Value::as_array).map_or(&[][..], Vec::as_slice) {
                    sessions.push(obj_json(&thread_to_session(Some(t), archived).map_err(|e| e.0)?));
                }
                cursor = page.get("nextCursor").cloned().filter(|c| !c.is_null()).unwrap_or(Value::Null);
                if !truthy(Some(&cursor)) {
                    break;
                }
            }
        }
        Ok(sessions)
    }

    /// Renames a thread, as the ChatGPT desktop app does.
    pub fn rename(&self, vendor: &dyn VendorCheck, raw_id: &str, name: &str) -> Result<(), String> {
        self.request(vendor, "thread/name/set", json!({ "threadId": raw_id, "name": name })).map(|_| ()).map_err(|e| e.message)
    }

    /// A page of a thread's items, newest first; one whose app-server can't
    /// list them is read from its rollout (`rollout`, when found).
    pub fn read_thread(&self, vendor: &dyn VendorCheck, rollouts: &Rollouts, raw_id: &str, cursor: Option<&str>, limit: f64, rollout: Option<String>) -> Result<Value, String> {
        let from_rollout = |err: String| -> Result<Value, String> {
            match &rollout {
                Some(file) => rollouts.page(vendor, file, cursor, limit),
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
        let res = match self.request(vendor, "thread/items/list", json!({ "threadId": raw_id, "limit": js::number(limit), "cursor": cursor, "sortDirection": "desc" })) {
            Ok(r) => r,
            Err(e) => {
                if says_unsupported(e.rpc.as_ref()) {
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
        let page = self.answered(vendor, "thread/items/list", res);
        let entries: Vec<Value> = normalize_items(page.get("data")).map_err(|e| e.0)?.iter().map(obj_json).collect();
        Ok(json!({ "entries": entries, "cursor": page.get("nextCursor").cloned().unwrap_or(Value::Null) }))
    }

    /// Stops the app-server, failing what waits on it.
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

    /// Its block: the codex-app-server client.
    pub fn block(&self) -> BlockHealth {
        BlockHealth { name: "codex-app-server", kind: "client", life: lock(&self.inner).life.clone() }
    }

    #[cfg(test)]
    fn codex_version(&self) -> Option<String> {
        lock(&self.inner).version.clone()
    }
}

/// Codex on the map: its app-server's client, the rollouts a thread is
/// read from when the app-server can't list it, and its home, where
/// Codex desktop's state database is.
pub struct Codex {
    client: Arc<Client>,
    rollouts: Rollouts,
    home: String,
}

impl Codex {
    pub fn new(env: &ListerEnv<'_>) -> Codex {
        Codex { client: Client::new(binary(env), env.version.to_string(), env.handle.clone()), rollouts: Rollouts::default(), home: env.root.to_string() }
    }

    /// Over a client already made (a test's).
    pub fn over(client: Arc<Client>, home: &str) -> Codex {
        Codex { client, rollouts: Rollouts::default(), home: home.into() }
    }
}

impl Lister for Codex {
    fn sessions(&self, vendor: &dyn VendorCheck) -> Result<Vec<Value>, String> {
        self.client.threads(vendor)
    }

    fn transcript(&self, vendor: &dyn VendorCheck, raw_id: &str, cursor: Option<&str>, limit: f64, files: &dyn Fn() -> Vec<String>) -> Result<Value, String> {
        let rollout = files().into_iter().next();
        self.client.read_thread(vendor, &self.rollouts, raw_id, cursor, limit, rollout)
    }

    fn rename(&self, vendor: &dyn VendorCheck, node: &Value, title: &str) -> Result<Renamed, String> {
        self.client.rename(vendor, node.get("rawId").and_then(Value::as_str).unwrap_or_default(), title)?;
        Ok(Renamed { saved: vec!["Codex"], desktop: None })
    }

    fn projects(&self, vendor: &dyn VendorCheck) -> Option<Vec<Value>> {
        Some(state_db::projects(&self.home, vendor))
    }

    fn block(&self) -> Option<BlockHealth> {
        Some(self.client.block())
    }

    fn request(&self, vendor: &dyn VendorCheck, method: &str, params: Value) -> Result<Value, Failed> {
        self.client.request(vendor, method, params)
    }

    fn stop(&self) {
        self.client.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_adapter::reader::Unchecked;

    #[test]
    fn reads_the_app_servers_lines() {
        let mut io = Stdout::default();
        let ms = io.take("not json\n{\"id\":\"a\",\"method\":\"x\"}\n{\"id\":1,\"result\":{}}\n{\"id\":2,\"error\":{\"code\":1}}\n{\"method\":\"n\"}\n{\"id\":3");
        assert_eq!(ms.len(), 5);
        assert_eq!(ms[0], Message::Other);
        assert_eq!(ms[1], Message::Decline("{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"error\":{\"code\":-32601,\"message\":\"XenoNet is read-only\"}}\n".into()));
        assert!(matches!(&ms[3], Message::Answer { error: Some(e), .. } if e.0 == "app-server error"));
        assert_eq!(io.take(",\"result\":5}\n").len(), 1);
        let mut err = Stderr::default();
        err.push("a\nb\r\n\nc");
        assert_eq!(err.exited(Some(&json!(2))), "codex app-server exited (2): a | b | c");
        err.push("d\r");
        assert_eq!(err.lines.last().map(String::as_str), Some("d\r"));
        assert!(says_unsupported(Some(&json!({ "code": -32603, "message": "thread/items/list is Not Supported yet" }))));
        assert!(!says_unsupported(Some(&json!({ "code": "-32601" }))));
    }

    #[test]
    fn an_app_server_that_isnt_there_fails_the_block() {
        let rt = tokio::runtime::Runtime::new().expect("a runtime");
        let c = Codex::over(Client::new("/nowhere/codex".into(), "0.0.0".into(), rt.handle().clone()), "/nowhere");
        assert_eq!(c.sessions(&Unchecked), Err("spawn /nowhere/codex ENOENT".into()));
        assert_eq!(c.block().map(|b| b.life.state), Some(State::Failed));
        assert_eq!(c.projects(&Unchecked), Some(Vec::new()));
    }
}

#[cfg(all(test, unix))]
mod through_a_stand_in {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use xenonet_adapter::reader::Unchecked;

    // A stand-in for `codex app-server`: it answers initialize, a listing,
    // a thread's items and a rename, by the request's id. Before its first
    // answer it writes a line that isn't UTF-8, which the client reads past.
    const FAKE: &str = r#"#!/bin/sh
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*) printf 'not UTF-8: \377\376\n{"id":%s,"result":{"userAgent":"codex_cli_rs/0.150.0 (Mac OS 15.0)"}}\n' "$id" ;;
    *'"method":"thread/list"'*'"archived":false'*) printf '{"id":%s,"result":{"data":[{"id":"0190d3e0-0000-7000-8000-00000000cccc","preview":"Fix the build\\nand more","createdAt":1790000000,"updatedAt":1790000100,"cwd":"/w/p","status":"idle","source":"cli"}],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/list"'*) printf '{"id":%s,"result":{"data":[],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/items/list"'*) printf '{"id":%s,"result":{"data":[{"item":{"id":"i1","type":"agentMessage","text":"Hello"}}],"nextCursor":null}}\n' "$id" ;;
    *'"method":"thread/name/set"'*) printf '{"id":%s,"result":{}}\n' "$id" ;;
  esac
done
"#;

    #[test]
    fn threads_items_and_a_rename_through_the_app_server() {
        let dir = std::env::temp_dir().join(format!("xenonet-codex-app-server-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a folder");
        let bin = dir.join("codex");
        std::fs::write(&bin, FAKE).expect("the stand-in");
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).expect("runnable");
        let rt = tokio::runtime::Runtime::new().expect("a runtime");
        let client = Client::new(bin.to_string_lossy().into_owned(), "0.3.0".into(), rt.handle().clone());
        let c = Codex::over(client.clone(), &dir.to_string_lossy());
        let threads = c.sessions(&Unchecked).expect("the threads");
        assert_eq!(threads.len(), 1);
        assert_eq!((threads[0]["id"].as_str(), threads[0]["title"].as_str()), (Some("codex:0190d3e0-0000-7000-8000-00000000cccc"), Some("Fix the build")));
        assert_eq!(client.codex_version().as_deref(), Some("0.150.0"), "Codex's version, from its user agent");
        let page = c.transcript(&Unchecked, "0190d3e0-0000-7000-8000-00000000cccc", None, 120.0, &Vec::new).expect("a page");
        assert_eq!(page["entries"][0]["text"], "Hello");
        let renamed = c.rename(&Unchecked, &json!({ "rawId": "0190d3e0-0000-7000-8000-00000000cccc" }), "Build fix").expect("renamed");
        assert_eq!(renamed.saved, ["Codex"]);
        let h = c.block().expect("its block");
        assert_eq!((h.name, h.kind, h.life.state, h.life.counters.get("starts")), ("codex-app-server", "client", State::Running, Some(1.0)));
        c.stop();
        assert_eq!(c.block().map(|b| b.life.state), Some(State::Stopped));
        std::fs::remove_dir_all(&dir).expect("cleaned up");
    }
}
