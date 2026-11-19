//! A fake WebDriver BiDi server.
//!
//! It speaks the server side of the WebSocket handshake and frames that
//! `eidolon-bidi` uses, so a client test needs no browser. It refuses
//! unmasked client frames, as a real server must: a client that sends them
//! has a bug the test should catch. Behind the commands is a model of one
//! page: a URL, a title, nodes found by CSS selector, text fields that
//! typing changes, a click count and the files chosen in file inputs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use eidolon_bidi::frame::{Opcode, MAX_PAYLOAD};
use eidolon_bidi::handshake;
use eidolon_bidi::ws::{Message, Reader, Role, Writer};
use eidolon_bidi::Error;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};

/// A 1x1 PNG, which every screenshot returns (base64).
pub const SCREENSHOT_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// What a node does when it is used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// Counts clicks.
    Button,
    /// Takes focus when clicked; typing then changes its value.
    TextField,
    /// Records the files `input.setFiles` chooses.
    FileInput,
    /// Found, and nothing more.
    Plain,
}

#[derive(Clone, Debug)]
struct FakeNode {
    shared_id: String,
    selector: String,
    kind: NodeKind,
    value: String,
    clicks: u32,
    files: Vec<String>,
}

#[derive(Debug)]
struct PageState {
    url: String,
    title: String,
    nodes: Vec<FakeNode>,
    focused: Option<usize>,
    contexts: Vec<String>,
    next_context: u32,
    next_navigation: u32,
    subscribed: Vec<String>,
    methods: Vec<String>,
    pongs: u32,
    rejected_unmasked: bool,
}

/// A handle on the page model, for assertions. Clones share the page.
#[derive(Clone, Debug)]
pub struct Page(Arc<Mutex<PageState>>);

impl Page {
    fn lock(&self) -> MutexGuard<'_, PageState> {
        self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn url(&self) -> String {
        self.lock().url.clone()
    }

    pub fn title(&self) -> String {
        self.lock().title.clone()
    }

    /// Clicks on all nodes together.
    pub fn clicks(&self) -> u32 {
        self.lock().nodes.iter().map(|n| n.clicks).sum()
    }

    /// Clicks on the node with this selector.
    pub fn clicks_on(&self, selector: &str) -> u32 {
        self.lock().nodes.iter().find(|n| n.selector == selector).map_or(0, |n| n.clicks)
    }

    /// The value of the text field with this selector.
    pub fn field_value(&self, selector: &str) -> Option<String> {
        self.lock().nodes.iter().find(|n| n.selector == selector).map(|n| n.value.clone())
    }

    /// The files chosen in the file input with this selector.
    pub fn files(&self, selector: &str) -> Vec<String> {
        self.lock().nodes.iter().find(|n| n.selector == selector).map(|n| n.files.clone()).unwrap_or_default()
    }

    /// Every command method received, in order.
    pub fn methods(&self) -> Vec<String> {
        self.lock().methods.clone()
    }

    /// How many pongs the client answered pings with.
    pub fn pongs(&self) -> u32 {
        self.lock().pongs
    }

    /// Whether the client ever sent an unmasked frame.
    pub fn rejected_unmasked(&self) -> bool {
        self.lock().rejected_unmasked
    }
}

#[derive(Default)]
struct Script {
    failures: HashMap<String, (String, String)>,
    drop_on_message: Option<usize>,
    fragment: bool,
    ping_first: bool,
    bad_accept: bool,
}

/// The fake server. Build it, keep a [`Page`] handle, then `serve` a stream.
pub struct FakeBidi {
    script: Script,
    page: Page,
}

impl Default for FakeBidi {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeBidi {
    /// A fake with one blank tab (`ctx-1`) and three nodes: a button
    /// `#go`, a text field `#name` and a file input `#upload`.
    pub fn new() -> Self {
        let node =
            |n: u32, selector: &str, kind| FakeNode { shared_id: format!("node-{n}"), selector: selector.to_owned(), kind, value: String::new(), clicks: 0, files: Vec::new() };
        let state = PageState {
            url: "about:blank".into(),
            title: String::new(),
            nodes: vec![node(1, "#go", NodeKind::Button), node(2, "#name", NodeKind::TextField), node(3, "#upload", NodeKind::FileInput)],
            focused: None,
            contexts: vec!["ctx-1".into()],
            next_context: 2,
            next_navigation: 1,
            subscribed: Vec::new(),
            methods: Vec::new(),
            pongs: 0,
            rejected_unmasked: false,
        };
        Self { script: Script::default(), page: Page(Arc::new(Mutex::new(state))) }
    }

    /// The page model, to assert on after the client has acted.
    pub fn page(&self) -> Page {
        self.page.clone()
    }

    /// Adds a node found by `selector`.
    #[must_use]
    pub fn with_node(self, selector: &str, kind: NodeKind) -> Self {
        {
            let mut page = self.page.lock();
            let n = page.nodes.len() + 1;
            page.nodes.push(FakeNode { shared_id: format!("node-{n}"), selector: selector.to_owned(), kind, value: String::new(), clicks: 0, files: Vec::new() });
        }
        self
    }

    /// Sets the page title (a long one makes a large answer).
    #[must_use]
    pub fn with_title(self, title: &str) -> Self {
        title.clone_into(&mut self.page.lock().title);
        self
    }

    /// Answers every call of `method` with a BiDi error.
    #[must_use]
    pub fn fail(mut self, method: &str, code: &str, message: &str) -> Self {
        self.script.failures.insert(method.to_owned(), (code.to_owned(), message.to_owned()));
        self
    }

    /// Drops the connection when the `n`th command arrives, unanswered.
    #[must_use]
    pub fn drop_on_message(mut self, n: usize) -> Self {
        self.script.drop_on_message = Some(n);
        self
    }

    /// Sends every answer in three fragments with a ping between them.
    #[must_use]
    pub fn fragment_answers(mut self) -> Self {
        self.script.fragment = true;
        self
    }

    /// Sends a ping as soon as the upgrade is done.
    #[must_use]
    pub fn ping_first(mut self) -> Self {
        self.script.ping_first = true;
        self
    }

    /// Answers the upgrade with a wrong `Sec-WebSocket-Accept`.
    #[must_use]
    pub fn bad_accept(mut self) -> Self {
        self.script.bad_accept = true;
        self
    }

    /// Serves one connection until it ends. `Ok` when the client closed it
    /// (or a scripted drop happened); `Err` when the client broke the rules,
    /// such as sending an unmasked frame.
    pub async fn serve<S>(self, stream: S) -> Result<(), Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut read, mut write) = tokio::io::split(stream);
        let (head, leftover) = handshake::read_head(&mut read).await?;
        let key = handshake::parse_request(&head)?;
        let answer = handshake::response(if self.script.bad_accept { "not the client's key" } else { &key });
        write.write_all(answer.as_bytes()).await?;
        write.flush().await?;

        let mut reader = Reader::new(Box::new(read), Role::Server, leftover, MAX_PAYLOAD);
        let mut writer = Writer::new(Box::new(write), Role::Server);
        if self.script.ping_first {
            writer.send_frame(true, Opcode::Ping, b"fake").await?;
        }
        let mut count = 0usize;
        loop {
            match reader.next().await {
                Ok(Message::Text(text)) => {
                    count += 1;
                    if self.script.drop_on_message == Some(count) {
                        writer.shutdown().await;
                        return Ok(());
                    }
                    for out in self.handle(&text) {
                        self.send(&mut writer, &out).await?;
                    }
                }
                Ok(Message::Ping(payload)) => writer.send_pong(&payload).await?,
                Ok(Message::Pong(_)) => self.page.lock().pongs += 1,
                Ok(Message::Binary(_)) => {}
                Ok(Message::Close { .. }) => {
                    writer.send_close(1000, "").await?;
                    writer.shutdown().await;
                    return Ok(());
                }
                Err(Error::Closed(_)) => return Ok(()),
                Err(e) => {
                    if matches!(&e, Error::Protocol(p) if p.contains("unmasked")) {
                        self.page.lock().rejected_unmasked = true;
                    }
                    // Best effort: the peer broke the rules and may be gone.
                    let _ = writer.send_close(1002, "protocol error").await;
                    writer.shutdown().await;
                    return Err(e);
                }
            }
        }
    }

    async fn send(&self, writer: &mut Writer, text: &str) -> Result<(), Error> {
        if !self.script.fragment {
            return writer.send_text(text).await;
        }
        let bytes = text.as_bytes();
        let (a, b) = (bytes.len() / 3, 2 * bytes.len() / 3);
        writer.send_frame(false, Opcode::Text, &bytes[..a]).await?;
        writer.send_frame(true, Opcode::Ping, b"between").await?;
        writer.send_frame(false, Opcode::Continuation, &bytes[a..b]).await?;
        writer.send_frame(true, Opcode::Continuation, &bytes[b..]).await
    }

    /// The messages to send back for one command, answer first.
    fn handle(&self, text: &str) -> Vec<String> {
        let Ok(command) = serde_json::from_str::<Value>(text) else {
            return vec![error_message(Value::Null, "invalid argument", "the command is not JSON")];
        };
        let id = command.get("id").cloned().unwrap_or(Value::Null);
        let method = command.get("method").and_then(Value::as_str).unwrap_or_default().to_owned();
        let params = command.get("params").cloned().unwrap_or(Value::Null);
        self.page.lock().methods.push(method.clone());
        if let Some((code, message)) = self.script.failures.get(&method) {
            return vec![error_message(id, code, message)];
        }
        match self.run(&method, &params) {
            Ok((result, events)) => {
                let mut out = vec![json!({ "type": "success", "id": id, "result": result }).to_string()];
                out.extend(events.into_iter().map(|e| e.to_string()));
                out
            }
            Err((code, message)) => vec![error_message(id, code, &message)],
        }
    }

    fn run(&self, method: &str, params: &Value) -> Result<(Value, Vec<Value>), (&'static str, String)> {
        let mut page = self.page.lock();
        let text = |key: &str| params.get(key).and_then(Value::as_str);
        let known_context = |page: &PageState| match text("context") {
            Some(c) if page.contexts.iter().any(|x| x == c) => Ok(c.to_owned()),
            Some(c) => Err(("no such frame", format!("there is no browsing context {c}"))),
            None => Err(("invalid argument", "context is missing".to_owned())),
        };
        match method {
            "fake.echo" => Ok((params.clone(), vec![])),
            "session.status" => Ok((json!({ "ready": true, "message": "fake browser" }), vec![])),
            "session.new" => Ok((json!({ "sessionId": "fake-session", "capabilities": { "browserName": "fake", "acceptInsecureCerts": false } }), vec![])),
            "session.end" => Ok((json!({}), vec![])),
            "session.subscribe" => {
                let events = params.get("events").and_then(Value::as_array).ok_or(("invalid argument", "events is missing".to_owned()))?;
                page.subscribed.extend(events.iter().filter_map(Value::as_str).map(str::to_owned));
                Ok((json!({ "subscription": "sub-1" }), vec![]))
            }
            "browsingContext.getTree" => {
                let root = text("root");
                let contexts: Vec<Value> = page
                    .contexts
                    .iter()
                    .filter(|c| root.is_none_or(|r| r == c.as_str()))
                    .map(|c| json!({ "context": c, "url": page.url, "children": [], "userContext": "default", "originalOpener": null }))
                    .collect();
                Ok((json!({ "contexts": contexts }), vec![]))
            }
            "browsingContext.create" => {
                let context = format!("ctx-{}", page.next_context);
                page.next_context += 1;
                page.contexts.push(context.clone());
                Ok((json!({ "context": context, "userContext": "default" }), vec![]))
            }
            "browsingContext.close" => {
                let context = known_context(&page)?;
                page.contexts.retain(|c| *c != context);
                Ok((json!({}), vec![]))
            }
            "browsingContext.navigate" => {
                let context = known_context(&page)?;
                let url = text("url").ok_or(("invalid argument", "url is missing".to_owned()))?.to_owned();
                let navigation = format!("nav-{}", page.next_navigation);
                page.next_navigation += 1;
                page.url.clone_from(&url);
                page.title = format!("Page at {url}");
                page.focused = None;
                let loaded = page.subscribed.iter().any(|s| s == "browsingContext.load" || s == "browsingContext");
                let events = if loaded {
                    vec![json!({
                        "type": "event",
                        "method": "browsingContext.load",
                        "params": { "context": context, "navigation": navigation, "url": url, "timestamp": 0 },
                    })]
                } else {
                    vec![]
                };
                Ok((json!({ "navigation": navigation, "url": url }), events))
            }
            "browsingContext.captureScreenshot" => {
                known_context(&page)?;
                Ok((json!({ "data": SCREENSHOT_BASE64 }), vec![]))
            }
            "browsingContext.locateNodes" => {
                known_context(&page)?;
                let locator = params.get("locator");
                if locator.and_then(|l| l.get("type")).and_then(Value::as_str) != Some("css") {
                    return Err(("invalid argument", "only css locators are supported".into()));
                }
                let css = locator.and_then(|l| l.get("value")).and_then(Value::as_str).unwrap_or_default();
                let nodes: Vec<Value> = page
                    .nodes
                    .iter()
                    .filter(|n| n.selector == css)
                    .map(|n| json!({ "type": "node", "sharedId": n.shared_id, "value": { "nodeType": 1, "childNodeCount": 0 } }))
                    .collect();
                Ok((json!({ "nodes": nodes }), vec![]))
            }
            "script.evaluate" => {
                known_target(&page, params)?;
                let expression = text("expression").unwrap_or_default();
                Ok((evaluate(&page, expression), vec![]))
            }
            "script.callFunction" => {
                known_target(&page, params)?;
                Ok((json!({ "type": "success", "realm": "realm-1", "result": { "type": "undefined" } }), vec![]))
            }
            "input.performActions" => {
                known_context(&page)?;
                perform(&mut page, params)?;
                Ok((json!({}), vec![]))
            }
            "input.setFiles" => {
                known_context(&page)?;
                let id = element_id(params)?;
                let node = page.nodes.iter_mut().find(|n| n.shared_id == id).ok_or(("no such element", format!("there is no node {id}")))?;
                if node.kind != NodeKind::FileInput {
                    return Err(("unable to set file input", format!("{} is not a file input", node.selector)));
                }
                node.files = params.get("files").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
                Ok((json!({}), vec![]))
            }
            other => Err(("unknown command", format!("the fake does not know {other}"))),
        }
    }
}

fn known_target(page: &PageState, params: &Value) -> Result<(), (&'static str, String)> {
    match params.pointer("/target/context").and_then(Value::as_str) {
        Some(c) if page.contexts.iter().any(|x| x == c) => Ok(()),
        Some(c) => Err(("no such frame", format!("there is no browsing context {c}"))),
        None => Err(("invalid argument", "target.context is missing".to_owned())),
    }
}

fn element_id(params: &Value) -> Result<String, (&'static str, String)> {
    params.pointer("/element/sharedId").and_then(Value::as_str).map(str::to_owned).ok_or(("invalid argument", "element.sharedId is missing".to_owned()))
}

fn string_value(s: &str) -> Value {
    json!({ "type": "success", "realm": "realm-1", "result": { "type": "string", "value": s } })
}

fn evaluate(page: &PageState, expression: &str) -> Value {
    if let Some(thrown) = expression.strip_prefix("throw ") {
        return json!({
            "type": "exception",
            "realm": "realm-1",
            "exceptionDetails": { "text": thrown, "lineNumber": 0, "columnNumber": 0 },
        });
    }
    match expression {
        "document.title" => string_value(&page.title),
        "location.href" => string_value(&page.url),
        "document.querySelector('#name').value" => string_value(page.nodes.iter().find(|n| n.selector == "#name").map_or("", |n| n.value.as_str())),
        _ => json!({ "type": "success", "realm": "realm-1", "result": { "type": "undefined" } }),
    }
}

/// Runs pointer and key action sources against the model.
fn perform(page: &mut PageState, params: &Value) -> Result<(), (&'static str, String)> {
    let sources = params.get("actions").and_then(Value::as_array).ok_or(("invalid argument", "actions is missing".to_owned()))?;
    for source in sources {
        let steps = source.get("actions").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        let mut target: Option<usize> = None;
        for step in steps {
            match step.get("type").and_then(Value::as_str) {
                Some("pointerMove") => {
                    let id = step.pointer("/origin/element/sharedId").and_then(Value::as_str).ok_or(("invalid argument", "only element origins are supported".to_owned()))?;
                    target = Some(page.nodes.iter().position(|n| n.shared_id == id).ok_or(("no such element", format!("there is no node {id}")))?);
                }
                Some("pointerUp") => {
                    if let Some(i) = target {
                        page.nodes[i].clicks += 1;
                        page.focused = Some(i);
                    }
                }
                Some("keyDown") => {
                    let key = step.get("value").and_then(Value::as_str).unwrap_or_default();
                    if let Some(i) = page.focused.filter(|&i| page.nodes[i].kind == NodeKind::TextField) {
                        if key == "\u{E003}" {
                            page.nodes[i].value.pop();
                        } else {
                            page.nodes[i].value.push_str(key);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn error_message(id: Value, code: &str, message: &str) -> String {
    json!({ "type": "error", "id": id, "error": code, "message": message, "stacktrace": "" }).to_string()
}

/// The screenshot the fake returns, decoded.
pub fn screenshot_bytes() -> Vec<u8> {
    STANDARD.decode(SCREENSHOT_BASE64).unwrap_or_default()
}
