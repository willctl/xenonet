//! Calling an outside agent: a small JSON-RPC and SSE client on a2a-lf's
//! types. A delegation is one SendStreamingMessage; its events come as
//! they happen, and when the connection drops before the task stops,
//! SubscribeToTask picks it up again (three times at most). Nothing polls
//! GetTask, and no push-notification config is ever sent.
//!
//! Each call fetches the card again and checks it against its pin first,
//! and goes only to the pinned interface, under the guard's rules.

use crate::bearer::{self, Bearer};
use crate::card::{self, Pinned};
use crate::guard::{self, Private};
use crate::outcome::{self, Update};
use crate::A2aError;
use ::a2a::{
    methods, CancelTaskRequest, JsonRpcId, JsonRpcRequest, JsonRpcResponse, Message, Part, Role, SendMessageConfiguration, SendMessageRequest, StreamResponse,
    SubscribeToTaskRequest, Task,
};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc::UnboundedSender;
use url::Url;

/// The most of one event, or of a plain JSON answer, XenoNet reads.
pub const EVENT_MAX: usize = 256 << 10;
/// How long a stream may go without a byte before XenoNet takes it as
/// dropped.
pub const QUIET: Duration = Duration::from_secs(120);
/// How long a plain call (CancelTask) may take.
pub const CALL: Duration = Duration::from_secs(30);
/// How many times a dropped stream is picked up again.
pub const RESUBSCRIBE: u32 = 3;

/// An outbound actor, ready to call: what was pinned at registration, the
/// bearer from its secret card (or none, when its card asks for none),
/// and, for the tests, the only certificates to trust.
pub struct Agent {
    pinned: Pinned,
    bearer: Option<Bearer>,
    roots: Option<Vec<reqwest::Certificate>>,
}

/// How a delegation ended: the task's id and context, when it had one,
/// and whether it stopped (done, ended or waiting on someone) or its
/// stream was lost.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delegated {
    pub task: Option<String>,
    pub context: Option<String>,
    pub stopped: bool,
}

/// The events of one stream, as they're cut from its bytes.
#[derive(Default)]
pub(crate) struct Sse {
    buf: Vec<u8>,
}

impl Sse {
    /// Takes `bytes`; gives each whole event's data.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Result<Vec<String>, A2aError> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let ends = [&b"\n\n"[..], b"\r\n\r\n"].iter().filter_map(|sep| self.buf.windows(sep.len()).position(|w| w == *sep).map(|i| (i, sep.len()))).min();
            let Some((at, len)) = ends else { break };
            let event: Vec<u8> = self.buf.drain(..at + len).collect();
            let text = String::from_utf8_lossy(&event[..at]).into_owned();
            let data: Vec<&str> = text.lines().filter_map(|l| l.strip_prefix("data:")).map(|d| d.strip_prefix(' ').unwrap_or(d)).collect();
            if !data.is_empty() {
                out.push(data.join("\n"));
            }
        }
        if self.buf.len() > EVENT_MAX {
            return Err(A2aError::new("protocol", format!("one of the agent's events is over {} KiB", EVENT_MAX >> 10)));
        }
        Ok(out)
    }
}

/// A JSON-RPC answer's result, or the agent's error in its own words, the
/// bearer taken out.
fn result(text: &str, bearer: Option<&Bearer>) -> Result<Value, A2aError> {
    let r: JsonRpcResponse = serde_json::from_str(text).map_err(|e| A2aError::new("protocol", format!("the agent answered something that isn't JSON-RPC: {e}")))?;
    if let Some(e) = r.error {
        return Err(A2aError::new("remote", format!("the agent answered error {}: {}", e.code, bearer::redact(bearer, &e.message))));
    }
    r.result.ok_or_else(|| A2aError::new("protocol", "the agent's answer has neither a result nor an error"))
}

fn rpc(method: &str, params: Value) -> JsonRpcRequest {
    JsonRpcRequest::new(JsonRpcId::String(::a2a::new_message_id()), method, Some(params))
}

impl Agent {
    pub fn new(pinned: Pinned, bearer: Option<Bearer>) -> Agent {
        Agent { pinned, bearer, roots: None }
    }

    /// The same, trusting only `roots` (a test's own certificate).
    #[must_use]
    pub fn trusting(mut self, roots: Vec<reqwest::Certificate>) -> Agent {
        self.roots = Some(roots);
        self
    }

    fn private(&self) -> &Private {
        &self.pinned.private
    }

    /// The pinned interface, once the card fetched now still says what was
    /// pinned.
    async fn checked(&self) -> Result<(reqwest::Client, Url), A2aError> {
        let card = card::fetch(&self.pinned.card_url, self.private(), self.roots.as_deref()).await?;
        self.pinned.check(&card)?;
        let url = guard::https(&self.pinned.interface_url)?;
        guard::check(&url, self.private())?;
        Ok((guard::client(&self.private().for_url(&url), self.roots.as_deref(), false)?, url))
    }

    async fn post(&self, client: &reqwest::Client, url: &Url, request: &JsonRpcRequest, stream: bool, timeout: Option<Duration>) -> Result<reqwest::Response, A2aError> {
        let mut b = client
            .post(url.clone())
            .header("content-type", "application/json")
            .header("accept", if stream { "text/event-stream" } else { "application/json" })
            .header(::a2a::SVC_PARAM_VERSION, ::a2a::VERSION)
            .body(serde_json::to_vec(request).unwrap_or_default());
        if let Some(h) = self.bearer.as_ref().and_then(Bearer::header) {
            b = b.header("authorization", h);
        }
        if let Some(t) = timeout {
            b = b.timeout(t);
        }
        let r = b.send().await.map_err(|e| crate::transport(&e))?;
        let status = r.status();
        if status.is_redirection() {
            return Err(A2aError::new("redirect", "the agent's interface redirects, which XenoNet doesn't follow"));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(A2aError::new("remote", format!("the agent refused its token ({}): put the right one in its secret card in XenoNet's window", status.as_u16())));
        }
        if !status.is_success() {
            return Err(A2aError::new("remote", format!("the agent answered {}", status.as_u16())));
        }
        Ok(r)
    }

    /// Reads one stream to its end, sending each update; whether the task
    /// stopped, and the task and context it named.
    async fn follow(&self, r: reqwest::Response, artifacts: &mut Vec<String>, said: &mut Delegated, tx: &UnboundedSender<Update>) -> Result<bool, A2aError> {
        let is_stream = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.to_ascii_lowercase().starts_with("text/event-stream"));
        let bearer = self.bearer.as_ref();
        let mut handle = |data: &str| -> Result<bool, A2aError> {
            let event: StreamResponse =
                serde_json::from_value(result(data, bearer)?).map_err(|e| A2aError::new("protocol", format!("the agent sent an event A2A doesn't say: {e}")))?;
            if let StreamResponse::Task(Task { id, context_id, .. }) = &event {
                said.task.get_or_insert_with(|| id.clone());
                said.context.get_or_insert_with(|| context_id.clone());
            }
            let update = outcome::of(event, &artifacts.join("\n\n"), bearer);
            if let Update::Artifact { text, .. } = &update {
                artifacts.push(text.read().clone());
            }
            if let (Some(t), None) = (update.task(), &said.task) {
                said.task = Some(t.to_string());
            }
            if let Update::Working { context, .. } = &update {
                said.context.get_or_insert_with(|| context.clone());
            }
            let stops = update.stops();
            let _ = tx.send(update);
            Ok(stops)
        };
        if !is_stream {
            // An agent may answer a streaming call with one JSON answer: its
            // error, or the task or message as it is.
            let text = card::body(r, EVENT_MAX, "the agent's answer").await?;
            return handle(&String::from_utf8_lossy(&text));
        }
        let mut sse = Sse::default();
        let mut r = r;
        loop {
            let chunk = match tokio::time::timeout(QUIET, r.chunk()).await {
                Err(_) => return Ok(false),
                Ok(Err(_)) => return Ok(false),
                Ok(Ok(None)) => return Ok(false),
                Ok(Ok(Some(c))) => c,
            };
            for data in sse.feed(&chunk)? {
                if handle(&data)? {
                    return Ok(true);
                }
            }
        }
    }

    /// Hands `text` to the agent, in `context` when it goes on from an
    /// earlier task, as more for `task` when that one waits for it
    /// (input-required), and sends each update to `tx` as it comes. The
    /// text is the delegating agent's own: XenoNet adds none of its
    /// authority, only the remote's bearer.
    pub async fn delegate(&self, text: &str, context: Option<&str>, task: Option<&str>, tx: &UnboundedSender<Update>) -> Result<Delegated, A2aError> {
        let (client, url) = self.checked().await?;
        let message = Message {
            context_id: context.map(str::to_string),
            task_id: task.map(str::to_string),
            ..Message::new(Role::User, vec![Part::text(text).with_media_type("text/plain")])
        };
        let config = SendMessageConfiguration {
            accepted_output_modes: Some(vec!["text/plain".into()]),
            task_push_notification_config: None,
            history_length: Some(0),
            return_immediately: None,
        };
        let params = serde_json::to_value(SendMessageRequest { message, configuration: Some(config), metadata: None, tenant: None }).unwrap_or(Value::Null);
        let mut said = Delegated { task: task.map(str::to_string), context: context.map(str::to_string), stopped: false };
        let mut artifacts = Vec::new();
        let r = self.post(&client, &url, &rpc(methods::SEND_STREAMING_MESSAGE, params), true, None).await?;
        if self.follow(r, &mut artifacts, &mut said, tx).await? {
            said.stopped = true;
            return Ok(said);
        }
        // The stream dropped before the task stopped: pick it up again.
        for n in 0..RESUBSCRIBE {
            let Some(task) = said.task.clone() else { break };
            tokio::time::sleep(Duration::from_millis(500 << n)).await;
            let params = serde_json::to_value(SubscribeToTaskRequest { id: task, tenant: None }).unwrap_or(Value::Null);
            let Ok(r) = self.post(&client, &url, &rpc(methods::SUBSCRIBE_TO_TASK, params), true, None).await else { continue };
            if self.follow(r, &mut artifacts, &mut said, tx).await? {
                said.stopped = true;
                return Ok(said);
            }
        }
        Ok(said)
    }

    /// Cancels `task` there: the task as the agent answers it.
    pub async fn cancel(&self, task: &str) -> Result<Update, A2aError> {
        let (client, url) = self.checked().await?;
        let params = serde_json::to_value(CancelTaskRequest { id: task.to_string(), metadata: None, tenant: None }).unwrap_or(Value::Null);
        let r = self.post(&client, &url, &rpc(methods::CANCEL_TASK, params), false, Some(CALL)).await?;
        let text = card::body(r, EVENT_MAX, "the agent's answer").await?;
        let t: Task = serde_json::from_value(result(&String::from_utf8_lossy(&text), self.bearer.as_ref())?)
            .map_err(|e| A2aError::new("protocol", format!("the agent answered CancelTask with something that isn't a task: {e}")))?;
        Ok(outcome::of(StreamResponse::Task(t), "", self.bearer.as_ref()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_cut_whole() {
        let mut s = Sse::default();
        assert_eq!(s.feed(b"data: {\"a\":").unwrap(), Vec::<String>::new());
        assert_eq!(s.feed(b"1}\n\n: keep-alive\n\ndata: x\r\n\r\n").unwrap(), vec!["{\"a\":1}".to_string(), "x".to_string()]);
        assert_eq!(s.feed(b"event: m\ndata: one\ndata: two\n\n").unwrap(), vec!["one\ntwo".to_string()]);
        let mut big = Sse::default();
        assert!(big.feed(&vec![b'a'; EVENT_MAX + 1]).is_err());
    }

    #[test]
    fn a_remote_error_keeps_its_words_without_the_token() {
        let b = Bearer::new("tok-0123456789abcdef".into()).unwrap();
        let e = result(r#"{"jsonrpc":"2.0","id":"1","error":{"code":-32001,"message":"no task for tok-0123456789abcdef"}}"#, Some(&b)).unwrap_err();
        assert_eq!(e.code, "remote");
        assert!(!e.message.contains("tok-0123"), "{}", e.message);
    }
}
