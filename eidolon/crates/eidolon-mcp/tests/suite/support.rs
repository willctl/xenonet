//! A fake driver over a temp folder, a BiDi double that answers the page
//! scripts itself and lets everything else reach `FakeBidi`, and a ready
//! environment.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use adytum_driver::{Driver, DriverError, ExecOpts, Listed};
use adytum_exec::{Duplex, Output};
use adytum_model::{CellName, CellSpec};
use async_trait::async_trait;
use eidolon_bidi::Session;
use eidolon_mcp::{Bidi, BrowserTools};
use eidolon_testkit::FakeBidi;
use serde_json::{json, Value};

pub const DOWNLOADS: &str = "/home/eidolon/files/downloads";

pub fn cell() -> CellName {
    "adytum-test-mcp".parse().unwrap()
}

/// A temp folder that goes away with the test.
pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new() -> Temp {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!("eidolon-mcp-test-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        // Resolved, so a test can compare paths even where the temp folder is a link.
        Temp(dir.canonicalize().unwrap())
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the fake driver was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    CopyIn { host: PathBuf, guest: String },
    CopyOut { guest: String, host: PathBuf },
    Exec(Vec<String>),
}

/// A driver whose cell is a folder on the host: guest paths map under `root`.
pub struct FakeDriver {
    root: PathBuf,
    pub calls: Mutex<Vec<Call>>,
}

impl FakeDriver {
    pub fn new(root: &Path) -> Arc<FakeDriver> {
        Arc::new(FakeDriver { root: root.to_path_buf(), calls: Mutex::default() })
    }

    pub fn guest(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    pub fn copies(&self) -> usize {
        self.calls().iter().filter(|c| !matches!(c, Call::Exec(_))).count()
    }
}

fn unsupported(what: &str) -> DriverError {
    DriverError::Unsupported { what: what.to_owned() }
}

fn io(what: &'static str, source: std::io::Error) -> DriverError {
    DriverError::Io { what, source }
}

#[async_trait]
impl Driver for FakeDriver {
    async fn create(&self, _spec: &CellSpec) -> Result<(), DriverError> {
        Err(unsupported("create"))
    }
    async fn start(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("start"))
    }
    async fn exec(&self, _name: &CellName, argv: &[String], _opts: ExecOpts) -> Result<Output, DriverError> {
        self.calls.lock().unwrap().push(Call::Exec(argv.to_vec()));
        // Only `find <dir> -maxdepth 1 -type f -printf '%f\0%s\0'` is answered.
        assert_eq!(argv.first().map(String::as_str), Some("find"), "the fake answers find only");
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(self.guest(&argv[1])) {
            for entry in entries.flatten() {
                let meta = entry.metadata().unwrap();
                if meta.is_file() {
                    out.extend_from_slice(entry.file_name().to_str().unwrap().as_bytes());
                    out.push(0);
                    out.extend_from_slice(meta.len().to_string().as_bytes());
                    out.push(0);
                }
            }
        }
        Ok(Output { status: Some(0), stdout: out, ..Output::default() })
    }
    async fn stream(&self, _name: &CellName, _argv: &[String]) -> Result<Duplex, DriverError> {
        Err(unsupported("stream"))
    }
    async fn copy_in(&self, _name: &CellName, host: &Path, guest: &str) -> Result<(), DriverError> {
        self.calls.lock().unwrap().push(Call::CopyIn { host: host.to_path_buf(), guest: guest.to_owned() });
        let to = self.guest(guest);
        std::fs::create_dir_all(to.parent().unwrap()).map_err(|e| io("make the folder", e))?;
        std::fs::copy(host, to).map(drop).map_err(|e| io("copy in", e))
    }
    async fn copy_out(&self, _name: &CellName, guest: &str, host: &Path) -> Result<(), DriverError> {
        self.calls.lock().unwrap().push(Call::CopyOut { guest: guest.to_owned(), host: host.to_path_buf() });
        std::fs::copy(self.guest(guest), host).map(drop).map_err(|e| io("copy out", e))
    }
    async fn stop(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("stop"))
    }
    async fn remove(&self, _name: &CellName) -> Result<(), DriverError> {
        Err(unsupported("remove"))
    }
    async fn list(&self) -> Result<Vec<Listed>, DriverError> {
        Ok(Vec::new())
    }
}

/// The page `FakeBidi` models has no DOM to walk, so this stands in for the
/// browser's script engine: it answers the three page scripts with a canned
/// page whose elements are the fake's own nodes, and passes every other
/// command to the fake.
pub struct Proxy {
    inner: Session,
    pub scripts: Mutex<Vec<String>>,
    pub methods: Mutex<Vec<String>>,
}

pub const PAGE_TEXT: &str = "Sign up\nName\nGo";

fn tree() -> Value {
    json!({
        "url": "https://example.test/signup",
        "title": "Sign up",
        "truncated": false,
        "items": [
            { "n": 0, "role": "heading", "tag": "h1", "name": "Sign up", "level": 1 },
            { "c": "form", "name": "", "children": [
                { "t": "Your details" },
                { "n": 1, "role": "textbox", "tag": "input", "name": "Name", "attrs": { "id": "name", "type": "text" } },
                { "n": 2, "role": "file", "tag": "input", "name": "Avatar", "attrs": { "id": "upload", "type": "file" } },
                { "n": 3, "role": "button", "tag": "button", "name": "Go", "attrs": { "id": "go" } },
            ] },
        ],
    })
}

fn remote_snapshot() -> Value {
    let nodes: Vec<Value> = ["node-9", "node-2", "node-3", "node-1"].iter().map(|id| json!({ "type": "node", "sharedId": id })).collect();
    json!({
        "type": "object",
        "value": [
            ["tree", { "type": "string", "value": tree().to_string() }],
            ["nodes", { "type": "array", "value": nodes }],
        ],
    })
}

#[async_trait]
impl Bidi for Proxy {
    async fn command(&self, method: &str, params: Value) -> eidolon_mcp::Result<Value> {
        self.methods.lock().unwrap().push(method.to_owned());
        if method == "script.callFunction" {
            let declaration = params["functionDeclaration"].as_str().unwrap_or_default();
            self.scripts.lock().unwrap().push(declaration.to_owned());
            let result = if declaration.contains("eidolon:snapshot") {
                remote_snapshot()
            } else if declaration.contains("eidolon:text") {
                json!({ "type": "string", "value": PAGE_TEXT })
            } else {
                json!({ "type": "string", "value": "typing" })
            };
            return Ok(json!({ "type": "success", "realm": "realm-1", "result": result }));
        }
        Ok(self.inner.send(method, params).await?)
    }
}

pub struct Env {
    pub tools: BrowserTools,
    pub page: eidolon_testkit::Page,
    pub driver: Arc<FakeDriver>,
    pub proxy: Arc<Proxy>,
    /// The host files folder.
    pub files: PathBuf,
    /// The folder holding `files` and the guest root.
    pub dir: Temp,
}

impl Env {
    /// Another set of tools on the same browser, driver and folders, as a
    /// server of its own would have.
    pub fn tools_again(&self) -> BrowserTools {
        BrowserTools::new(self.proxy.clone(), self.driver.clone(), cell(), self.files.clone())
    }
}

/// An environment on a fake with the default page nodes plus a hidden file
/// input.
pub async fn env() -> Env {
    let dir = Temp::new();
    let files = dir.0.join("files");
    std::fs::create_dir_all(&files).unwrap();
    let guest = dir.0.join("guest");
    std::fs::create_dir_all(&guest).unwrap();
    let fake = FakeBidi::new();
    let page = fake.page();
    let (client, server) = tokio::io::duplex(1 << 16);
    tokio::spawn(fake.serve(server));
    let session = Session::connect(client, "127.0.0.1:9222").await.unwrap();
    let proxy = Arc::new(Proxy { inner: session, scripts: Mutex::default(), methods: Mutex::default() });
    let driver = FakeDriver::new(&guest);
    let tools = BrowserTools::new(proxy.clone(), driver.clone(), cell(), files.clone());
    Env { tools, page, driver, proxy, files, dir }
}

/// Writes a file, making its folders.
pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
