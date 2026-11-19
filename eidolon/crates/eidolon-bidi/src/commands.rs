//! The BiDi commands the browser facet needs, each a thin wrapper over
//! [`Session::send`]. Names and parameters follow
//! <https://w3c.github.io/webdriver-bidi/>.

use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::Error;
use crate::session::Session;

/// When `navigate` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    None,
    Interactive,
    Complete,
}

impl Wait {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Interactive => "interactive",
            Self::Complete => "complete",
        }
    }
}

/// A browsing context (a tab, a window or a frame) and its children.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ContextInfo {
    pub context: String,
    pub url: String,
    #[serde(default)]
    pub children: Vec<ContextInfo>,
    #[serde(default)]
    pub parent: Option<String>,
}

/// What a navigation reports.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Navigation {
    #[serde(default)]
    pub navigation: Option<String>,
    pub url: String,
}

/// A DOM node the browser found, by the id it can be named with later.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Node {
    #[serde(rename = "sharedId")]
    pub shared_id: String,
    #[serde(default)]
    pub value: Value,
}

fn decode<T: for<'de> Deserialize<'de>>(what: &str, value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|e| Error::Protocol(format!("the answer to {what} has an unexpected shape: {e}")))
}

fn script_result(what: &str, answer: &Value) -> Result<Value, Error> {
    match answer.get("type").and_then(Value::as_str) {
        Some("success") => Ok(answer.get("result").cloned().unwrap_or(Value::Null)),
        Some("exception") => {
            let text = answer.pointer("/exceptionDetails/text").and_then(Value::as_str).unwrap_or("an exception without text");
            Err(Error::Script(text.to_owned()))
        }
        _ => Err(Error::Protocol(format!("the answer to {what} is neither a result nor an exception"))),
    }
}

/// The actions for a mouse click at the middle of an element.
pub fn click_actions(shared_id: &str) -> Value {
    json!([{
        "type": "pointer",
        "id": "mouse",
        "parameters": { "pointerType": "mouse" },
        "actions": [
            { "type": "pointerMove", "x": 0, "y": 0, "origin": { "type": "element", "element": { "sharedId": shared_id } } },
            { "type": "pointerDown", "button": 0 },
            { "type": "pointerUp", "button": 0 },
        ],
    }])
}

/// The actions for typing `text`, one key down and up per character.
/// Special keys use their WebDriver code points (Enter is `\u{E007}`).
pub fn type_actions(text: &str) -> Value {
    let mut actions = Vec::new();
    for c in text.chars() {
        let key = c.to_string();
        actions.push(json!({ "type": "keyDown", "value": key }));
        actions.push(json!({ "type": "keyUp", "value": key }));
    }
    json!([{ "type": "key", "id": "keyboard", "actions": actions }])
}

impl Session {
    /// `session.status`: whether the browser can take a session.
    pub async fn status(&self) -> Result<Value, Error> {
        self.send("session.status", json!({})).await
    }

    /// Waits for the browser to answer `session.status` with `ready`, for
    /// at most `limit`. The protocol answering is the readiness test; there
    /// is no sleeping and retrying here.
    pub async fn wait_ready(&self, limit: Duration) -> Result<(), Error> {
        let status = self.send_within("session.status", json!({}), limit).await?;
        if status.get("ready").and_then(Value::as_bool) == Some(true) {
            Ok(())
        } else {
            let why = status.get("message").and_then(Value::as_str).unwrap_or("it gave no reason");
            Err(Error::NotReady(why.to_owned()))
        }
    }

    /// `session.new` with no capabilities; returns the answer.
    pub async fn new_session(&self) -> Result<Value, Error> {
        self.send("session.new", json!({ "capabilities": {} })).await
    }

    /// `session.end`.
    pub async fn session_end(&self) -> Result<(), Error> {
        self.send("session.end", json!({})).await.map(drop)
    }

    /// `browsingContext.getTree`, from `root` or from every top level.
    pub async fn browsing_context_get_tree(&self, root: Option<&str>) -> Result<Vec<ContextInfo>, Error> {
        let params = root.map_or_else(|| json!({}), |r| json!({ "root": r }));
        let answer = self.send("browsingContext.getTree", params).await?;
        decode("browsingContext.getTree", answer.get("contexts").cloned().unwrap_or(Value::Null))
    }

    /// `browsingContext.create` for a new tab; returns its context id.
    pub async fn browsing_context_create(&self) -> Result<String, Error> {
        let answer = self.send("browsingContext.create", json!({ "type": "tab" })).await?;
        answer.get("context").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| Error::Protocol("browsingContext.create gave no context".into()))
    }

    /// `browsingContext.navigate`.
    pub async fn navigate(&self, context: &str, url: &str, wait: Wait) -> Result<Navigation, Error> {
        let params = json!({ "context": context, "url": url, "wait": wait.as_str() });
        decode("browsingContext.navigate", self.send("browsingContext.navigate", params).await?)
    }

    /// `browsingContext.captureScreenshot`: the PNG bytes.
    pub async fn capture_screenshot(&self, context: &str) -> Result<Vec<u8>, Error> {
        let answer = self.send("browsingContext.captureScreenshot", json!({ "context": context })).await?;
        let data = answer.get("data").and_then(Value::as_str).ok_or_else(|| Error::Protocol("captureScreenshot gave no data".into()))?;
        STANDARD.decode(data).map_err(|e| Error::Protocol(format!("the screenshot is not base64: {e}")))
    }

    /// `browsingContext.locateNodes` with a CSS selector.
    pub async fn locate_nodes(&self, context: &str, css: &str) -> Result<Vec<Node>, Error> {
        let params = json!({ "context": context, "locator": { "type": "css", "value": css } });
        let answer = self.send("browsingContext.locateNodes", params).await?;
        decode("browsingContext.locateNodes", answer.get("nodes").cloned().unwrap_or(Value::Null))
    }

    /// `browsingContext.close`.
    pub async fn browsing_context_close(&self, context: &str) -> Result<(), Error> {
        self.send("browsingContext.close", json!({ "context": context })).await.map(drop)
    }

    /// `script.evaluate`: the remote value it returned. A throw is an
    /// [`Error::Script`].
    pub async fn script_evaluate(&self, context: &str, expression: &str, await_promise: bool) -> Result<Value, Error> {
        let params = json!({
            "expression": expression,
            "target": { "context": context },
            "awaitPromise": await_promise,
        });
        script_result("script.evaluate", &self.send("script.evaluate", params).await?)
    }

    /// `script.callFunction` with already-serialized arguments (remote
    /// values such as `{"type":"string","value":"x"}`).
    pub async fn script_call_function(&self, context: &str, function_declaration: &str, arguments: Vec<Value>, await_promise: bool) -> Result<Value, Error> {
        let params = json!({
            "functionDeclaration": function_declaration,
            "arguments": arguments,
            "target": { "context": context },
            "awaitPromise": await_promise,
        });
        script_result("script.callFunction", &self.send("script.callFunction", params).await?)
    }

    /// `input.performActions` with action sources as the spec writes them.
    pub async fn input_perform_actions(&self, context: &str, actions: Value) -> Result<(), Error> {
        self.send("input.performActions", json!({ "context": context, "actions": actions })).await.map(drop)
    }

    /// A mouse click at an element (found with `locate_nodes`).
    pub async fn click(&self, context: &str, shared_id: &str) -> Result<(), Error> {
        self.input_perform_actions(context, click_actions(shared_id)).await
    }

    /// Types `text` into whatever has focus.
    pub async fn type_text(&self, context: &str, text: &str) -> Result<(), Error> {
        self.input_perform_actions(context, type_actions(text)).await
    }

    /// `input.setFiles`: chooses files, by path in the browser's own
    /// filesystem, in a file input.
    pub async fn input_set_files(&self, context: &str, shared_id: &str, paths: &[&str]) -> Result<(), Error> {
        let params = json!({ "context": context, "element": { "sharedId": shared_id }, "files": paths });
        self.send("input.setFiles", params).await.map(drop)
    }
}
