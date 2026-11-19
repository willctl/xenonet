//! The tools: what an agent can do in the cell's browser.
//!
//! Each public method is one tool. They take no arbitrary script: the agent
//! gets pages, a snapshot, clicks, typing, files and screenshots, and the
//! scripts that run are ours (`snapshot.js`, `pagetext.js`,
//! `prepare_fill.js`).
//!
//! Files pass through one host folder (`files`). An upload must be inside it
//! once symlinks are resolved; a fetched download lands in `files/downloads`.
//! Downloads are found by listing the cell's downloads folder, which is the
//! truth whatever events the browser sends.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use adytum_driver::{Driver, ExecOpts};
use adytum_model::CellName;
use eidolon_bidi::commands::{click_actions, type_actions};
use eidolon_bidi::{ContextInfo, Error as BidiError};
use eidolon_browser::units::{DOWNLOADS_PATH, UPLOADS_PATH};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::args;
use crate::bidi::Bidi;
use crate::error::{Error, Result};
use crate::files;
use crate::keys;
use crate::snapshot::{self, PAGE_TEXT_JS, PREPARE_FILL_JS, SNAPSHOT_JS};

/// The longest page text returned, in characters.
const MAX_TEXT: usize = 100_000;

/// A piece of a tool's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    /// An image, base64 as the protocol carries it.
    Image {
        data: String,
        mime: &'static str,
    },
}

/// One tool: its name, a one-line description and the JSON Schema of its
/// arguments.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: serde_json::Map<String, Value>,
}

fn schema_of<T: schemars::JsonSchema>() -> serde_json::Map<String, Value> {
    let mut map = match serde_json::to_value(schemars::schema_for!(T)) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    map.remove("$schema");
    map.remove("title");
    // Clients read `properties` on every object schema, so a tool with no
    // arguments says so with an empty one.
    map.entry("properties").or_insert_with(|| json!({}));
    map
}

/// Every tool, in the order they are listed.
pub fn definitions() -> Vec<ToolDef> {
    fn def<T: schemars::JsonSchema>(name: &'static str, description: &'static str) -> ToolDef {
        ToolDef { name, description, schema: schema_of::<T>() }
    }
    vec![
        def::<args::Empty>("list_pages", "List the open pages with their ids, addresses and titles."),
        def::<args::NewPage>("new_page", "Open a new page, optionally at an address, and select it."),
        def::<args::PageId>("select_page", "Make a page the one the other tools act on."),
        def::<args::PageId>("close_page", "Close a page."),
        def::<args::NavigatePage>("navigate_page", "Open an address in the selected page and wait for it to finish loading."),
        def::<args::NavigateHistory>("navigate_history", "Go back or forward in the selected page's history."),
        def::<args::Empty>("take_snapshot", "List the elements on the selected page with a uid for each; the uids work until the page navigates."),
        def::<args::Uid>("click_by_uid", "Click an element from the last snapshot."),
        def::<args::Uid>("hover_by_uid", "Move the pointer over an element from the last snapshot."),
        def::<args::FillByUid>("fill_by_uid", "Replace a field's content with a value, or choose an option in a select."),
        def::<args::TypeText>("type_text", "Type text into the element that has focus."),
        def::<args::PressKey>("press_key", "Press a key or a combination such as Control+a."),
        def::<args::Empty>("get_page_text", "Return the visible text of the selected page."),
        def::<args::Empty>("screenshot_page", "Take a PNG screenshot of the selected page."),
        def::<args::UploadFileByUid>("upload_file_by_uid", "Choose a file from the shared files folder in a file input from the last snapshot."),
        def::<args::Empty>("list_downloads", "List the files the browser has downloaded in the cell."),
        def::<args::FetchDownload>("fetch_download", "Copy a finished download to the downloads folder inside the shared files folder."),
    ]
}

#[derive(Default)]
struct State {
    /// The context the tools act on.
    selected: Option<String>,
    /// The number the next uid gets. Never reused, so an old uid cannot name
    /// a different element after a new snapshot.
    next_uid: u64,
    /// Per page: uid to the browser's id for the element.
    uids: HashMap<String, HashMap<String, String>>,
}

/// A file the browser saved in the cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub name: String,
    pub size: u64,
    /// False while the browser is still writing it (`.part`).
    pub finished: bool,
}

/// The browser facet's tools for one cell.
pub struct BrowserTools {
    bidi: Arc<dyn Bidi>,
    driver: Arc<dyn Driver>,
    cell: CellName,
    files: PathBuf,
    timeout: Duration,
    state: Mutex<State>,
}

impl BrowserTools {
    /// Tools on `bidi` (a supervised [`eidolon_browser::Browser`] in
    /// practice), moving files through `driver` to `cell`, with `files` as
    /// the one host folder files pass through.
    pub fn new(bidi: Arc<dyn Bidi>, driver: Arc<dyn Driver>, cell: CellName, files: PathBuf) -> Self {
        Self { bidi, driver, cell, files, timeout: Duration::from_secs(60), state: Mutex::new(State::default()) }
    }

    /// Changes how long one browser command may take (60 seconds).
    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn state(&self) -> MutexGuard<'_, State> {
        // Nothing panics while the lock is held, so a poisoned lock still
        // holds consistent data.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    // ---- plumbing ----

    /// One BiDi command, with a time limit, and the errors an agent should
    /// act on told apart from the rest.
    async fn cmd(&self, method: &str, params: Value) -> Result<Value> {
        let answer = tokio::time::timeout(self.timeout, self.bidi.command(method, params)).await.map_err(|_| Error::Bidi(BidiError::Timeout(self.timeout)))?;
        answer.map_err(|e| match &e {
            Error::Bidi(BidiError::Remote { code, .. }) if code == "no such frame" => {
                let mut state = self.state();
                state.selected = None;
                Error::PageGone
            }
            Error::Bidi(BidiError::Remote { code, .. }) if matches!(code.as_str(), "no such node" | "no such element" | "stale element reference") => Error::StaleElement,
            _ => e,
        })
    }

    async fn call_function(&self, context: &str, declaration: &str, arguments: Vec<Value>) -> Result<Value> {
        let params = json!({
            "functionDeclaration": declaration,
            "arguments": arguments,
            "target": { "context": context },
            "awaitPromise": false,
            "resultOwnership": "none",
        });
        let answer = self.cmd("script.callFunction", params).await?;
        match answer.get("type").and_then(Value::as_str) {
            Some("success") => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
            Some("exception") => {
                let text = answer.pointer("/exceptionDetails/text").and_then(Value::as_str).unwrap_or("an exception without text");
                Err(Error::Bidi(BidiError::Script(text.to_owned())))
            }
            _ => Err(Error::Bidi(BidiError::Protocol("the answer to script.callFunction is neither a result nor an exception".to_owned()))),
        }
    }

    async fn get_tree(&self) -> Result<Vec<ContextInfo>> {
        let answer = self.cmd("browsingContext.getTree", json!({})).await?;
        serde_json::from_value(answer.get("contexts").cloned().unwrap_or(Value::Null))
            .map_err(|e| Error::Bidi(BidiError::Protocol(format!("the answer to browsingContext.getTree has an unexpected shape: {e}"))))
    }

    /// The selected page, or the first open one.
    async fn page(&self) -> Result<String> {
        if let Some(selected) = self.state().selected.clone() {
            return Ok(selected);
        }
        let first = self.get_tree().await?.into_iter().next().ok_or(Error::NoPage)?.context;
        self.state().selected = Some(first.clone());
        Ok(first)
    }

    /// The browser's id for the element a uid names, on the selected page.
    fn element(&self, uid: &str) -> Result<(String, String)> {
        let state = self.state();
        // No page selected means no snapshot was taken: the same advice applies.
        let context = state.selected.clone().ok_or_else(|| Error::UnknownUid(uid.to_owned()))?;
        let id = state.uids.get(&context).and_then(|m| m.get(uid)).cloned().ok_or_else(|| Error::UnknownUid(uid.to_owned()))?;
        Ok((context, id))
    }

    fn forget_uids(&self, context: &str) {
        self.state().uids.remove(context);
    }

    // ---- pages ----

    /// `list_pages`.
    pub async fn list_pages(&self) -> Result<String> {
        let contexts = self.get_tree().await?;
        if contexts.is_empty() {
            return Ok("No pages are open.".to_owned());
        }
        let selected = self.state().selected.clone();
        let mut out = String::new();
        for c in contexts {
            // A title is a nicety: a page still loading may not answer yet.
            let title = self.title_of(&c.context).await.unwrap_or_default();
            let mark = if selected.as_deref() == Some(c.context.as_str()) { " (selected)" } else { "" };
            let _ = writeln!(out, "{}{mark} {} {title:?}", c.context, c.url);
        }
        Ok(out)
    }

    async fn title_of(&self, context: &str) -> Result<String> {
        let params = json!({ "expression": "document.title", "target": { "context": context }, "awaitPromise": false });
        let answer = self.cmd("script.evaluate", params).await?;
        Ok(answer.pointer("/result/value").and_then(Value::as_str).unwrap_or_default().to_owned())
    }

    /// `new_page`.
    pub async fn new_page(&self, url: Option<&str>) -> Result<String> {
        if let Some(url) = url {
            check_url(url)?;
        }
        let answer = self.cmd("browsingContext.create", json!({ "type": "tab" })).await?;
        let id = answer.get("context").and_then(Value::as_str).ok_or_else(|| Error::Bidi(BidiError::Protocol("browsingContext.create gave no context".to_owned())))?.to_owned();
        self.state().selected = Some(id.clone());
        match url {
            Some(url) => {
                let at = self.navigate(&id, url).await?;
                Ok(format!("Opened page {id} at {at}. It is selected."))
            }
            None => Ok(format!("Opened page {id}. It is selected.")),
        }
    }

    /// `select_page`.
    pub async fn select_page(&self, id: &str) -> Result<String> {
        if !self.get_tree().await?.iter().any(|c| c.context == id) {
            return Err(Error::NoSuchPage(id.to_owned()));
        }
        self.state().selected = Some(id.to_owned());
        // Brings the tab to the front for a person watching. The page can be
        // driven without it, so a browser that cannot do this is not an error.
        let _ = self.cmd("browsingContext.activate", json!({ "context": id })).await;
        Ok(format!("Page {id} is selected."))
    }

    /// `close_page`.
    pub async fn close_page(&self, id: &str) -> Result<String> {
        self.cmd("browsingContext.close", json!({ "context": id })).await.map_err(|e| match e {
            Error::PageGone => Error::NoSuchPage(id.to_owned()),
            other => other,
        })?;
        let mut state = self.state();
        state.uids.remove(id);
        if state.selected.as_deref() == Some(id) {
            state.selected = None;
        }
        Ok(format!("Closed page {id}."))
    }

    async fn navigate(&self, context: &str, url: &str) -> Result<String> {
        let answer = self.cmd("browsingContext.navigate", json!({ "context": context, "url": url, "wait": "complete" })).await?;
        self.forget_uids(context);
        Ok(answer.get("url").and_then(Value::as_str).unwrap_or(url).to_owned())
    }

    /// `navigate_page`.
    pub async fn navigate_page(&self, url: &str) -> Result<String> {
        check_url(url)?;
        let context = self.page().await?;
        let at = self.navigate(&context, url).await?;
        Ok(format!("Loaded {at}. Call take_snapshot to see it."))
    }

    /// `navigate_history`.
    pub async fn navigate_history(&self, direction: args::Direction) -> Result<String> {
        let context = self.page().await?;
        let delta = if direction == args::Direction::Back { -1 } else { 1 };
        self.cmd("browsingContext.traverseHistory", json!({ "context": context, "delta": delta })).await?;
        self.forget_uids(&context);
        Ok("Done. Call take_snapshot to see the page.".to_owned())
    }

    // ---- reading ----

    /// `take_snapshot`.
    pub async fn take_snapshot(&self) -> Result<String> {
        let context = self.page().await?;
        let remote = self.call_function(&context, SNAPSHOT_JS, Vec::new()).await?;
        let first = self.state().next_uid.max(1);
        let (snap, next) = snapshot::parse(&remote, first)?;
        let mut state = self.state();
        state.next_uid = next;
        state.uids.insert(context, snap.uids);
        Ok(snap.text)
    }

    /// `get_page_text`.
    pub async fn get_page_text(&self) -> Result<String> {
        let context = self.page().await?;
        let remote = self.call_function(&context, PAGE_TEXT_JS, Vec::new()).await?;
        let text = remote.get("value").and_then(Value::as_str).unwrap_or_default();
        if text.chars().count() > MAX_TEXT {
            let cut: String = text.chars().take(MAX_TEXT).collect();
            return Ok(format!("{cut}\n(the text is longer; the rest is left out.)"));
        }
        Ok(text.to_owned())
    }

    /// `screenshot_page`: the PNG, base64.
    pub async fn screenshot_page(&self) -> Result<String> {
        let context = self.page().await?;
        let answer = self.cmd("browsingContext.captureScreenshot", json!({ "context": context })).await?;
        answer.get("data").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| Error::Bidi(BidiError::Protocol("captureScreenshot gave no data".to_owned())))
    }

    // ---- acting ----

    async fn perform(&self, context: &str, actions: Value) -> Result<()> {
        self.cmd("input.performActions", json!({ "context": context, "actions": actions })).await.map(drop)
    }

    /// `click_by_uid`.
    pub async fn click_by_uid(&self, uid: &str) -> Result<String> {
        let (context, id) = self.element(uid)?;
        self.perform(&context, click_actions(&id)).await?;
        Ok(format!("Clicked {uid}."))
    }

    /// `hover_by_uid`.
    pub async fn hover_by_uid(&self, uid: &str) -> Result<String> {
        let (context, id) = self.element(uid)?;
        let actions = json!([{
            "type": "pointer",
            "id": "mouse",
            "parameters": { "pointerType": "mouse" },
            "actions": [{ "type": "pointerMove", "x": 0, "y": 0, "origin": { "type": "element", "element": { "sharedId": id } } }],
        }]);
        self.perform(&context, actions).await?;
        Ok(format!("Pointer is over {uid}."))
    }

    /// `fill_by_uid`: a select takes the option; anything else is clicked,
    /// has its content selected and is typed into.
    pub async fn fill_by_uid(&self, uid: &str, value: &str) -> Result<String> {
        let (context, id) = self.element(uid)?;
        let element = json!({ "sharedId": id });
        let text = json!({ "type": "string", "value": value });
        let phase = |p: &str| vec![element.clone(), text.clone(), json!({ "type": "string", "value": p })];
        let probe = self.call_function(&context, PREPARE_FILL_JS, phase("probe")).await?;
        match probe.get("value").and_then(Value::as_str) {
            Some("selected") => return Ok(format!("Chose {value:?} in {uid}.")),
            Some("no-option") => return Err(Error::BadArguments(format!("{uid} has no option {value:?}. Call take_snapshot or get_page_text to see the options."))),
            _ => {}
        }
        self.perform(&context, click_actions(&id)).await?;
        self.call_function(&context, PREPARE_FILL_JS, phase("focus")).await?;
        self.perform(&context, type_actions(value)).await?;
        Ok(format!("Filled {uid}."))
    }

    /// `type_text`.
    pub async fn type_text(&self, text: &str) -> Result<String> {
        let context = self.page().await?;
        self.perform(&context, type_actions(text)).await?;
        Ok("Typed.".to_owned())
    }

    /// `press_key`.
    pub async fn press_key(&self, key: &str) -> Result<String> {
        let actions = keys::press_actions(key)?;
        let context = self.page().await?;
        self.perform(&context, actions).await?;
        Ok(format!("Pressed {key}."))
    }

    // ---- files ----

    /// `upload_file_by_uid`: the file must be inside the files folder.
    pub async fn upload_file_by_uid(&self, uid: &str, file_path: &str) -> Result<String> {
        // The path rule first, so a refused path never reaches the cell.
        let source = files::upload_source(&self.files, file_path)?;
        let name = files::upload_name(&source)?;
        let (context, id) = self.element(uid)?;
        let guest = format!("{UPLOADS_PATH}/{name}");
        self.driver.copy_in(&self.cell, &source, &guest).await?;
        let params = json!({ "context": context, "element": { "sharedId": id }, "files": [guest] });
        self.cmd("input.setFiles", params).await?;
        Ok(format!("Chose {name} in {uid}."))
    }

    /// The files in the cell's downloads folder, by name.
    pub async fn downloads(&self) -> Result<Vec<Download>> {
        let argv: Vec<String> = ["find", DOWNLOADS_PATH, "-maxdepth", "1", "-type", "f", "-printf", "%f\\0%s\\0"].iter().map(|s| (*s).to_owned()).collect();
        let out = self.driver.exec(&self.cell, &argv, ExecOpts::default()).await?;
        if out.status != Some(0) {
            let stderr = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Driver(format!("The downloads folder could not be listed: {}", stderr.trim())));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let mut fields = text.split('\0');
        let mut found = Vec::new();
        while let (Some(name), Some(size)) = (fields.next(), fields.next()) {
            if name.is_empty() {
                continue;
            }
            found.push(Download { name: name.to_owned(), size: size.parse().unwrap_or(0), finished: !Path::new(name).extension().is_some_and(|e| e.eq_ignore_ascii_case("part")) });
        }
        found.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(found)
    }

    /// `list_downloads`.
    pub async fn list_downloads(&self) -> Result<String> {
        let found = self.downloads().await?;
        if found.is_empty() {
            return Ok("There are no downloads.".to_owned());
        }
        let mut out = String::new();
        for d in found {
            let state = if d.finished { "" } else { " (still downloading)" };
            let _ = writeln!(out, "{} {} bytes{state}", d.name, d.size);
        }
        Ok(out)
    }

    /// `fetch_download`: copies one finished download out of the cell into
    /// `files/downloads`, never over an earlier file.
    pub async fn fetch_download(&self, name: &str) -> Result<String> {
        files::check_name(name)?;
        let found = self.downloads().await?;
        let download = found.iter().find(|d| d.name == name).ok_or_else(|| Error::NoSuchDownload(name.to_owned()))?;
        if !download.finished {
            return Err(Error::Refused(format!("{name} is still downloading. Wait and call list_downloads again.")));
        }
        let dir = files::downloads_dir(&self.files)?;
        let target = files::free_path(&dir, name);
        self.driver.copy_out(&self.cell, &format!("{DOWNLOADS_PATH}/{name}"), &target).await?;
        let saved = target.file_name().and_then(|n| n.to_str()).unwrap_or(name);
        Ok(format!("Saved {} bytes as {}/{saved} in the files folder.", download.size, files::DOWNLOADS_DIR))
    }

    // ---- dispatch ----

    /// Runs the tool `name` with JSON `arguments`, as the MCP server does.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<Vec<Part>> {
        let text = |s: String| vec![Part::Text(s)];
        let arguments = if arguments.is_null() { json!({}) } else { arguments };
        match name {
            "list_pages" => {
                parse::<args::Empty>(name, arguments)?;
                Ok(text(self.list_pages().await?))
            }
            "new_page" => Ok(text(self.new_page(parse::<args::NewPage>(name, arguments)?.url.as_deref()).await?)),
            "select_page" => Ok(text(self.select_page(&parse::<args::PageId>(name, arguments)?.id).await?)),
            "close_page" => Ok(text(self.close_page(&parse::<args::PageId>(name, arguments)?.id).await?)),
            "navigate_page" => Ok(text(self.navigate_page(&parse::<args::NavigatePage>(name, arguments)?.url).await?)),
            "navigate_history" => Ok(text(self.navigate_history(parse::<args::NavigateHistory>(name, arguments)?.direction).await?)),
            "take_snapshot" => {
                parse::<args::Empty>(name, arguments)?;
                Ok(text(self.take_snapshot().await?))
            }
            "click_by_uid" => Ok(text(self.click_by_uid(&parse::<args::Uid>(name, arguments)?.uid).await?)),
            "hover_by_uid" => Ok(text(self.hover_by_uid(&parse::<args::Uid>(name, arguments)?.uid).await?)),
            "fill_by_uid" => {
                let a = parse::<args::FillByUid>(name, arguments)?;
                Ok(text(self.fill_by_uid(&a.uid, &a.value).await?))
            }
            "type_text" => Ok(text(self.type_text(&parse::<args::TypeText>(name, arguments)?.text).await?)),
            "press_key" => Ok(text(self.press_key(&parse::<args::PressKey>(name, arguments)?.key).await?)),
            "get_page_text" => {
                parse::<args::Empty>(name, arguments)?;
                Ok(text(self.get_page_text().await?))
            }
            "screenshot_page" => {
                parse::<args::Empty>(name, arguments)?;
                Ok(vec![Part::Image { data: self.screenshot_page().await?, mime: "image/png" }])
            }
            "upload_file_by_uid" => {
                let a = parse::<args::UploadFileByUid>(name, arguments)?;
                Ok(text(self.upload_file_by_uid(&a.uid, &a.file_path).await?))
            }
            "list_downloads" => {
                parse::<args::Empty>(name, arguments)?;
                Ok(text(self.list_downloads().await?))
            }
            "fetch_download" => Ok(text(self.fetch_download(&parse::<args::FetchDownload>(name, arguments)?.name).await?)),
            other => Err(Error::BadArguments(format!("There is no tool named {other}."))),
        }
    }
}

fn parse<T: DeserializeOwned>(tool: &str, arguments: Value) -> Result<T> {
    serde_json::from_value(arguments).map_err(|e| Error::BadArguments(format!("The arguments for {tool} are not right: {e}.")))
}

/// Only web addresses and the blank page may be opened. `file:` would let a
/// page read the browser's own profile in the cell, `about:` pages such as
/// logins show saved passwords, and `javascript:` and `data:` run script.
pub fn check_url(url: &str) -> Result<()> {
    let lower = url.trim().to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower == "about:blank" {
        return Ok(());
    }
    Err(Error::Refused(format!("{url:?} cannot be opened. Only http and https addresses and about:blank are allowed.")))
}
