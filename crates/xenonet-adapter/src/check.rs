//! A doctor check an adapter brings: its row (id, group and title, as the
//! doctor lists them), what it may ask of the machine while it runs
//! (`CheckCtx`), and the verdict it gives. With them, what every app's
//! checks share: the facts as they read them, a folder's state, the
//! privacy fix, and the verdict on whether an app's sessions are connected.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
pub use xenonet_model::doctor::Status;
use xenonet_text::text::{display, truthy};

/// Where an adapter's rows go in the doctor's list, which keeps one order
/// for everyone: the app's own checks (installed, signed in, readable)
/// after XenoNet's own, and the sessions' checks (connected) after the
/// machine's; and the actors' checks (a product's plugin) among the
/// actors', after XenoNet's own check of its observers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    App,
    Sessions,
    Actors,
}

/// A check's place in the doctor's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckRow {
    /// `<group>.<check>`: `claude.auth`, `agents.codex`.
    pub id: &'static str,
    /// The group it shows under: the app's name, "Sessions", "Actors".
    pub group: &'static str,
    pub title: &'static str,
    pub slot: Slot,
}

/// Why a check ended without a verdict: an error of its own, or the error
/// its last ask of the machine threw, passed on as it was.
#[derive(Clone, Debug, PartialEq)]
pub enum Stop {
    Error { message: String, code: Option<String> },
    Rethrow,
}

impl Stop {
    pub fn msg(message: impl Into<String>) -> Self {
        Stop::Error { message: message.into(), code: None }
    }
}

/// What a check may ask of the machine it runs on, and the facts the
/// doctor gathered before it ran (its home folder, platform, paths, what
/// a running XenoNet says of connect and of its sources). A check runs
/// where the doctor's tasks do, on one thread, so nothing here is Send.
#[async_trait(?Send)]
pub trait CheckCtx {
    /// The doctor's facts, as its JSON object.
    fn facts(&self) -> &Value;
    /// A command on PATH.
    async fn which(&self, cmd: &str) -> Result<Option<String>, Stop>;
    async fn exists(&self, path: &str) -> Result<bool, Stop>;
    /// A program run with these arguments, for at most `ms`: what the
    /// doctor's runner answers (`{ stdout, stderr, code, error? }`).
    async fn run(&self, file: &str, args: &[&str], ms: f64) -> Result<Value, Stop>;
    /// A small settings file's text, or "" when it can't be read.
    async fn read_text(&self, path: &str) -> String;
    /// A folder's entries: None when the machine can't list folders here,
    /// Err with the error's code when the folder can't be read.
    async fn readdir(&self, path: &str) -> Result<Option<Vec<String>>, Option<String>>;
    async fn now(&self) -> Result<f64, Stop>;
    /// A time in ms, written in the machine's local time.
    async fn local_time(&self, ms: &Value) -> Result<String, Stop>;
    /// A path with the home folder as ~.
    fn tilde(&self, path: &str) -> String;
}

/// One check an adapter brings.
#[async_trait(?Send)]
pub trait Check {
    fn row(&self) -> CheckRow;
    async fn run(&self, ctx: &dyn CheckCtx) -> Result<Verdict, Stop>;
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    pub status: Status,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
}

impl Verdict {
    fn new(status: Status, detail: impl Into<String>, fix: Option<&str>) -> Self {
        Verdict { status, detail: detail.into(), fix: fix.filter(|f| !f.is_empty()).map(String::from) }
    }
    pub fn ok(detail: impl Into<String>) -> Self {
        Self::new(Status::Ok, detail, None)
    }
    /// A fact, not a problem.
    pub fn info(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Info, detail, fix)
    }
    pub fn warn(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Warn, detail, fix)
    }
    pub fn fail(detail: impl Into<String>, fix: Option<&str>) -> Self {
        Self::new(Status::Fail, detail, fix)
    }
    /// The check couldn't run here.
    pub fn skip(detail: impl Into<String>) -> Self {
        Self::new(Status::Skip, detail, None)
    }
}

/// The doctor's facts, read as the checks read them: a missing or null
/// one is absent.
#[derive(Clone, Copy)]
pub struct Facts<'a>(pub &'a Value);

impl Facts<'_> {
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.0.get(k).filter(|v| !v.is_null())
    }
    pub fn home(&self) -> String {
        self.get("home").map(display).unwrap_or_default()
    }
    pub fn platform(&self) -> String {
        self.get("platform").map(display).unwrap_or_default()
    }
    /// One of the doctor's paths, when it's set.
    pub fn path(&self, k: &str) -> Option<String> {
        self.get("paths").and_then(|p| p.get(k)).filter(|v| truthy(Some(v))).map(display)
    }
}

/// A value as `String(v)` writes it, "undefined" for a missing one.
pub fn s(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), display)
}

/// An array's items, or none.
pub fn list(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array).cloned().unwrap_or_default()
}

/// A folder a check looks at: not there, readable with this many entries,
/// or unreadable with the error's code.
pub enum Folder {
    Missing,
    Ok(usize),
    Denied(String),
}

pub async fn folder(c: &dyn CheckCtx, dir: Option<&str>) -> Result<Folder, Stop> {
    let Some(dir) = dir.filter(|d| !d.is_empty()) else { return Ok(Folder::Missing) };
    if !c.exists(dir).await? {
        return Ok(Folder::Missing);
    }
    Ok(match c.readdir(dir).await {
        Ok(Some(entries)) => Folder::Ok(entries.len()),
        Ok(None) => Folder::Denied("undefined".into()),
        Err(code) => Folder::Denied(code.unwrap_or_else(|| "undefined".into())),
    })
}

/// How to let XenoNet read a folder it can't, on this system.
pub fn privacy_fix(c: &dyn CheckCtx, what: &str) -> String {
    if Facts(c.facts()).platform() == "darwin" {
        format!("Let XenoNet read {what}: System Settings › Privacy & Security › Files and Folders (or Full Disk Access), then restart XenoNet.")
    } else {
        format!("Check the permissions on {what}.")
    }
}

/// Whether an app's sessions are connected to XenoNet, from what a running
/// XenoNet says of connect for the app (`app` its registry id, `name` as
/// the verdict names it).
pub fn connected(c: &dyn CheckCtx, app: &str, name: &str) -> Verdict {
    let f = Facts(c.facts());
    let connect = f.get("connect").filter(|v| truthy(Some(v)));
    let a = connect.and_then(|cn| cn.get("apps")).and_then(Value::as_array).and_then(|apps| apps.iter().find(|x| x.get("app").and_then(Value::as_str) == Some(app)));
    let Some(a) = a else { return Verdict::skip("needs XenoNet running") };
    let changes = list(a.get("changes"));
    let unconnected = changes.iter().all(|x| x.get("state").and_then(Value::as_str) == Some("missing"));
    if unconnected && a.get("installed") == Some(&json!(false)) {
        return Verdict::skip(format!("{name} isn't installed"));
    }
    if unconnected {
        return Verdict::info(
            "not connected, so these sessions can't see or message each other through XenoNet",
            Some("Connect them from XenoNet's palette (Connect your agents to XenoNet), or run xenonet connect."),
        );
    }
    // XenoNet's plugin for Claude Code carries what connect would add;
    // connect's own entries beside it are to come out.
    let removing = |x: &&Value| x.get("state").and_then(Value::as_str) == Some("remove");
    if changes.iter().any(|x| removing(&x)) {
        let tools = changes.iter().any(|x| removing(&x) && x.get("id").and_then(Value::as_str) == Some("mcp"));
        let tools = if tools { ", but sessions get XenoNet's tools twice" } else { "" };
        return Verdict::warn(
            format!("connected twice, through XenoNet's plugin and through xenonet connect's own entries: each event still arrives once{tools}"),
            Some("Run xenonet connect claude, which takes connect's own entries out and keeps the plugin."),
        );
    }
    let plugin = changes.iter().any(|x| x.get("via").and_then(Value::as_str) == Some("plugin"));
    let stale: Vec<String> =
        changes.iter().filter(|x| x.get("state").and_then(Value::as_str) != Some("present")).map(|x| format!("{} {}", s(x.get("id")), s(x.get("state")))).collect();
    if !stale.is_empty() {
        return Verdict::warn(format!("partly connected ({})", stale.join(", ")), Some("Run xenonet connect again."));
    }
    let broken = list(a.get("broken"));
    if !broken.is_empty() {
        let fix = if plugin { "Install XenoNet, or put its xenonet command on your PATH." } else { "Run xenonet connect again." };
        return Verdict::fail(format!("connected, but {}", broken.iter().map(display).collect::<Vec<_>>().join("; ")), Some(fix));
    }
    let through = if plugin { ", through its plugin for Claude Code" } else { "" };
    Verdict::ok(format!("connected to XenoNet on port {}{through}", s(connect.and_then(|cn| cn.get("port")))))
}

/// A machine for a check's tests: its files, folders and commands by
/// path, the programs' answers by their command line, and a clock.
#[cfg(any(test, feature = "testing"))]
pub mod fake {
    use super::{CheckCtx, Facts, Stop};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    /// Runs a check to its end against a fake, which answers at once, so
    /// nothing ever waits.
    pub fn block_on<T>(fut: impl Future<Output = T>) -> T {
        let mut fut = std::pin::pin!(fut);
        let mut cx = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
                return v;
            }
        }
    }

    #[derive(Default)]
    pub struct FakeCtx {
        pub facts: Value,
        /// Readable files, by path.
        pub files: BTreeMap<String, String>,
        /// Folders, by path: their entries, or the code they can't be read
        /// for.
        pub dirs: BTreeMap<String, Result<Vec<String>, String>>,
        /// The commands on PATH, each with where it is.
        pub clis: BTreeMap<String, String>,
        /// What a program answers, by its command line ("file arg arg").
        pub runs: BTreeMap<String, Value>,
        pub now: f64,
        /// Every program run, in order.
        pub ran: RefCell<Vec<String>>,
    }

    impl FakeCtx {
        pub fn new(facts: Value) -> Self {
            FakeCtx { facts, ..Default::default() }
        }
        #[must_use]
        pub fn file(mut self, path: &str, text: &str) -> Self {
            self.files.insert(path.into(), text.into());
            self
        }
        #[must_use]
        pub fn dir(mut self, path: &str, entries: &[&str]) -> Self {
            self.dirs.insert(path.into(), Ok(entries.iter().map(|e| (*e).to_string()).collect()));
            self
        }
        #[must_use]
        pub fn denied(mut self, path: &str, code: &str) -> Self {
            self.dirs.insert(path.into(), Err(code.into()));
            self
        }
        #[must_use]
        pub fn cli(mut self, name: &str, path: &str) -> Self {
            self.clis.insert(name.into(), path.into());
            self
        }
        /// What `file args…` prints.
        #[must_use]
        pub fn answers(mut self, line: &str, stdout: &str) -> Self {
            self.runs.insert(line.into(), json!({ "stdout": stdout, "stderr": "", "code": 0 }));
            self
        }
    }

    #[async_trait(?Send)]
    impl CheckCtx for FakeCtx {
        fn facts(&self) -> &Value {
            &self.facts
        }
        async fn which(&self, cmd: &str) -> Result<Option<String>, Stop> {
            Ok(self.clis.get(cmd).cloned())
        }
        async fn exists(&self, path: &str) -> Result<bool, Stop> {
            Ok(self.files.contains_key(path) || self.dirs.contains_key(path) || self.clis.values().any(|p| p == path))
        }
        async fn run(&self, file: &str, args: &[&str], _ms: f64) -> Result<Value, Stop> {
            let line = format!("{file} {}", args.join(" "));
            self.ran.borrow_mut().push(line.clone());
            Ok(self.runs.get(&line).cloned().unwrap_or_else(|| json!({ "stdout": "", "stderr": "", "code": 0 })))
        }
        async fn read_text(&self, path: &str) -> String {
            self.files.get(path).cloned().unwrap_or_default()
        }
        async fn readdir(&self, path: &str) -> Result<Option<Vec<String>>, Option<String>> {
            match self.dirs.get(path) {
                Some(Ok(entries)) => Ok(Some(entries.clone())),
                Some(Err(code)) => Err(Some(code.clone())),
                None => Err(Some("ENOENT".into())),
            }
        }
        async fn now(&self) -> Result<f64, Stop> {
            Ok(self.now)
        }
        async fn local_time(&self, ms: &Value) -> Result<String, Stop> {
            Ok(format!("local time {}", xenonet_text::text::display(ms)))
        }
        fn tilde(&self, path: &str) -> String {
            xenonet_os::paths::tilde(path, &Facts(&self.facts).home())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{block_on, FakeCtx};
    use super::*;

    fn connect(app: &str, over: Value) -> Value {
        let mut a = json!({ "app": app, "installed": true, "changes": [{ "id": "hooks", "state": "present" }, { "id": "mcp", "state": "present" }], "broken": [] });
        for (k, v) in over.as_object().into_iter().flatten() {
            a[k] = v.clone();
        }
        json!({ "home": "/Users/ada", "platform": "darwin", "connect": { "port": 47840, "apps": [a] } })
    }

    #[test]
    fn folders_and_the_privacy_fix() {
        let c = FakeCtx::new(json!({ "home": "/Users/ada", "platform": "darwin" })).dir("/Users/ada/a", &["x", "y"]).denied("/Users/ada/b", "EACCES");
        assert!(matches!(block_on(folder(&c, Some("/Users/ada/a"))), Ok(Folder::Ok(2))));
        assert!(matches!(block_on(folder(&c, Some("/Users/ada/b"))), Ok(Folder::Denied(code)) if code == "EACCES"));
        assert!(matches!(block_on(folder(&c, Some("/Users/ada/c"))), Ok(Folder::Missing)));
        assert!(matches!(block_on(folder(&c, None)), Ok(Folder::Missing)));
        assert!(privacy_fix(&c, "~/a").starts_with("Let XenoNet read ~/a: System Settings"));
        let linux = FakeCtx::new(json!({ "platform": "linux" }));
        assert_eq!(privacy_fix(&linux, "~/a"), "Check the permissions on ~/a.");
        assert_eq!(c.tilde("/Users/ada/a"), "~/a");
    }

    #[test]
    fn connected_reads_what_connect_says() {
        let on = |facts: Value, app: &str, name: &str| connected(&FakeCtx::new(facts), app, name);
        let v = on(json!({}), "claude", "Claude Code");
        assert_eq!((v.status, v.detail.as_str()), (Status::Skip, "needs XenoNet running"));
        let v = on(connect("claude", json!({})), "claude", "Claude Code");
        assert_eq!((v.status, v.detail.as_str()), (Status::Ok, "connected to XenoNet on port 47840"));
        let missing = json!({ "installed": false, "changes": [{ "id": "hooks", "state": "missing" }] });
        let v = on(connect("codex", missing), "codex", "Codex");
        assert_eq!((v.status, v.detail.as_str()), (Status::Skip, "Codex isn't installed"));
        let partly = json!({ "changes": [{ "id": "hooks", "state": "present" }, { "id": "mcp", "state": "stale" }] });
        let v = on(connect("grok", partly), "grok", "Grok Build");
        assert_eq!((v.status, v.detail.as_str(), v.fix.as_deref()), (Status::Warn, "partly connected (mcp stale)", Some("Run xenonet connect again.")));
        let plugin = json!({ "changes": [{ "id": "hooks", "state": "present", "via": "plugin" }], "broken": ["the hook's command is gone"] });
        let v = on(connect("claude", plugin), "claude", "Claude Code");
        assert_eq!((v.status, v.fix.as_deref()), (Status::Fail, Some("Install XenoNet, or put its xenonet command on your PATH.")));
    }
}
