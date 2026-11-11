//! How A2A's objects map to XenoNet's, on a2a-lf's types: a client's
//! Message (checked first by XenoNet's own rules, then read as a2a-lf's
//! `SendMessageRequest`) to the text of a XenoNet message and where it
//! goes, and a kept task back to a2a-lf's Task, its status, history and
//! artifacts (spec section 4.1), and to the events of a stream (4.2).
//!
//! One thing is mapped at the edge: a status's timestamp is written as
//! XenoNet writes every time (three digits of milliseconds always), where
//! chrono's form, which a2a-lf's `TaskStatus` carries, drops them at a
//! whole second.

use super::rpc::{self, RpcError};
use super::store::{ReplyRow, TaskRow, CANCELED, COMPLETED, FAILED, REJECTED, WORKING};
use ::a2a::{Artifact, Message, Part, Role, SendMessageRequest, StreamResponse, Task, TaskArtifactUpdateEvent, TaskState, TaskStatus, TaskStatusUpdateEvent};
use serde_json::{json, Value};
use std::collections::HashMap;
use xenonet_engine::delivery::selector::KEYS as SELECTOR_KEYS;
use xenonet_text::time::iso;

/// The longest id XenoNet keeps of a client's (message ids, contexts).
pub const ID_MAX: usize = 200;
/// The longest target a message may name.
const TARGET_MAX: usize = 300;
const TEXT: &str = "text/plain";

/// A client's message, as XenoNet takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub message_id: String,
    pub context_id: Option<String>,
    pub task_id: Option<String>,
    pub text: String,
    /// The target it names, if any (metadata.target).
    pub target: Option<String>,
    /// The message, as a2a-lf's Message reads it, to give back in the
    /// task's history.
    pub message: Value,
}

fn short_id(v: Option<&Value>, field: &str) -> Result<Option<String>, RpcError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) if s.len() <= ID_MAX && !s.chars().any(char::is_control) => Ok(Some(s.clone())),
        Some(Value::String(_)) => Err(RpcError::invalid_params(field, &format!("must be at most {ID_MAX} bytes, without control characters"))),
        Some(_) => Err(RpcError::invalid_params(field, "must be a string")),
    }
}

/// XenoNet's own checks of a SendMessageRequest's message, on the JSON as
/// it came: a2a-lf's Part takes the first of text, raw, url and data and
/// drops the rest, where XenoNet refuses a part that holds two.
fn check(params: &Value) -> Result<(), RpcError> {
    let Some(m) = params.get("message").filter(|m| m.is_object()) else {
        return Err(RpcError::invalid_params("message", "a SendMessageRequest carries a message"));
    };
    if short_id(m.get("messageId"), "message.messageId")?.is_none() {
        return Err(RpcError::invalid_params("message.messageId", "is required"));
    }
    match m.get("role").and_then(Value::as_str) {
        Some("ROLE_USER") => {}
        Some(other) => return Err(RpcError::invalid_params("message.role", &format!("a client's message is ROLE_USER, not {other}"))),
        None => return Err(RpcError::invalid_params("message.role", "is required (ROLE_USER)")),
    }
    let Some(parts) = m.get("parts").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
        return Err(RpcError::invalid_params("message.parts", "At least one part is required"));
    };
    for (i, p) in parts.iter().enumerate() {
        let field = format!("message.parts[{i}]");
        let Some(o) = p.as_object() else { return Err(RpcError::invalid_params(&field, "a part is an object")) };
        let kinds: Vec<&str> = ["text", "raw", "url", "data"].into_iter().filter(|k| o.contains_key(*k)).collect();
        match kinds.as_slice() {
            ["text"] => match o.get("text") {
                Some(Value::String(_)) => {
                    let media = o.get("mediaType").and_then(Value::as_str).unwrap_or(TEXT);
                    if !media.to_ascii_lowercase().starts_with("text/") {
                        return Err(RpcError::content_type(&format!("{field} says it's {media}; XenoNet takes text")));
                    }
                }
                _ => return Err(RpcError::invalid_params(&format!("{field}.text"), "must be a string")),
            },
            [one] => return Err(RpcError::content_type(&format!("{field} is a {} part; XenoNet takes text parts only", if *one == "data" { "data" } else { "file" }))),
            [] => return Err(RpcError::invalid_params(&field, "a part holds one of text, raw, url or data")),
            _ => return Err(RpcError::invalid_params(&field, "a part holds exactly one of text, raw, url or data")),
        }
    }
    short_id(m.get("contextId"), "message.contextId")?;
    short_id(m.get("taskId"), "message.taskId")?;
    Ok(())
}

/// A SendMessageRequest's message, checked: text parts only.
pub fn incoming(params: &Value) -> Result<Incoming, RpcError> {
    check(params)?;
    let request: SendMessageRequest = rpc::typed(params)?;
    let m = request.message;
    let texts: Vec<&str> = m.parts.iter().filter_map(Part::as_text).collect();
    let text = xenonet_text::text::trim(&texts.join("\n")).to_string();
    if text.is_empty() {
        return Err(RpcError::invalid_params("message.parts", "the text is empty"));
    }
    let target = target_of(m.metadata.as_ref()).or_else(|| target_of(request.metadata.as_ref())).transpose()?;
    let nonempty = |s: &Option<String>| s.clone().filter(|s| !s.is_empty());
    Ok(Incoming {
        message_id: m.message_id.clone(),
        context_id: nonempty(&m.context_id),
        task_id: nonempty(&m.task_id),
        text,
        target,
        message: serde_json::to_value(&m).unwrap_or(Value::Null),
    })
}

fn target_of(metadata: Option<&HashMap<String, Value>>) -> Option<Result<String, RpcError>> {
    let t = metadata?.get("target")?;
    Some(match t.as_str().map(xenonet_text::text::trim) {
        Some(s) if !s.is_empty() && s.len() <= TARGET_MAX => Ok(s.to_string()),
        Some("") | None => Err(RpcError::invalid_params("metadata.target", "a session's id (claude:…), repo:<name>, or a selector such as project:atlas")),
        Some(_) => Err(RpcError::invalid_params("metadata.target", &format!("must be at most {TARGET_MAX} bytes"))),
    })
}

/// A target as the engine's `to` takes it: `repo:<name>` is the sessions in
/// that repo, which the engine selects as `project:<name>` (the repo's
/// folder); a remote's path (`repo:github.com/x/atlas`) names it by its last
/// part.
pub fn engine_to(target: &str) -> String {
    match target.strip_prefix("repo:") {
        Some(key) => {
            let name = key.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or(key);
            format!("project:{}", name.strip_suffix(".git").unwrap_or(name))
        }
        None => target.to_string(),
    }
}

/// Whether a target reaches many at once (a repo, a selector, all), which
/// an agent may send to less often.
pub fn is_wide(to: &str) -> bool {
    if matches!(to, "all" | "repo" | "project") {
        return true;
    }
    let first = to.split(|c: char| c.is_whitespace() || c == '+').find(|p| !p.is_empty()).unwrap_or("");
    first.split_once(':').is_some_and(|(k, _)| SELECTOR_KEYS.iter().any(|s| s.eq_ignore_ascii_case(k)))
}

/// Whether the client accepts what XenoNet answers in (text).
pub fn accepts_text(modes: Option<&Value>) -> bool {
    let Some(list) = modes.and_then(Value::as_array) else { return true };
    let modes: Vec<String> = list.iter().filter_map(Value::as_str).map(str::to_ascii_lowercase).collect();
    modes.is_empty() || modes.iter().any(|m| m.starts_with("text/") || m == "*/*" || m == "*")
}

/// How much history a task is shown with: None for all of it. a2a-lf
/// counts it in 32 bits.
pub fn history_length(v: Option<&Value>, field: &str) -> Result<Option<usize>, RpcError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(n) => match n.as_i64().and_then(|n| i32::try_from(n).ok()) {
            Some(n) if n >= 0 => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
            _ => Err(RpcError::invalid_params(field, "must be a non-negative integer")),
        },
    }
}

/// A kept state (`TASK_STATE_…`) as a2a-lf's TaskState.
pub fn state_of(s: &str) -> TaskState {
    serde_json::from_value(json!(s)).unwrap_or_default()
}

/// A reply's id as A2A shows it: its mid, or its XenoNet id.
fn reply_id(r: &ReplyRow) -> String {
    r.mid.clone().unwrap_or_else(|| format!("xenonet-{}", r.message))
}

fn text_part(text: &str) -> Part {
    Part::text(text).with_media_type(TEXT)
}

/// A message from XenoNet's side, for a task.
fn agent_message(t: &TaskRow, id: &str, text: &str, metadata: Option<HashMap<String, Value>>) -> Message {
    Message { message_id: id.to_string(), context_id: Some(t.context.clone()), task_id: Some(t.id.clone()), metadata, ..Message::new(Role::Agent, vec![text_part(text)]) }
}

fn reply_message(t: &TaskRow, r: &ReplyRow) -> Message {
    agent_message(t, &reply_id(r), &r.text, Some(from_of(r)))
}

fn from_of(r: &ReplyRow) -> HashMap<String, Value> {
    let mut md = HashMap::new();
    if let Some(s) = &r.sender {
        md.insert("from".into(), json!(s));
    }
    if let Some(n) = &r.sender_name {
        md.insert("fromName".into(), json!(n));
    }
    md
}

/// A reply as an artifact.
pub fn artifact(r: &ReplyRow) -> Artifact {
    let who = r.sender_name.clone().or_else(|| r.sender.clone()).unwrap_or_else(|| "the session".into());
    Artifact {
        artifact_id: format!("reply-{}", r.message),
        name: Some(format!("Reply from {who}")),
        description: None,
        parts: vec![text_part(&r.text)],
        metadata: Some(from_of(r)),
        extensions: None,
    }
}

/// The words of a task's status.
fn status_text(t: &TaskRow) -> Option<String> {
    match t.state.as_str() {
        WORKING => Some(match t.recipients.as_slice() {
            [] => "Sent. Its reply completes this task.".to_string(),
            [one] => format!("Sent to {one}. Its reply completes this task."),
            many => format!("Sent to {} sessions and actors. The first reply completes this task.", many.len()),
        }),
        REJECTED | FAILED | CANCELED => t.reason.clone(),
        _ => None,
    }
}

/// The task's status: its state and its message. The timestamp is put in
/// as XenoNet writes it, by `stamp`.
fn typed_status(t: &TaskRow, replies: &[ReplyRow]) -> TaskStatus {
    let message = if t.state == COMPLETED {
        replies.first().map(|r| reply_message(t, r))
    } else {
        status_text(t).map(|text| agent_message(t, &format!("{}-{}", t.id, t.state.trim_start_matches("TASK_STATE_").to_ascii_lowercase()), &text, None))
    };
    TaskStatus { state: state_of(&t.state), message, timestamp: None }
}

/// The status at `path` in `v`, given the time it changed.
fn stamp(mut v: Value, path: &[&str], at: f64) -> Value {
    let mut s = &mut v;
    for k in path {
        s = &mut s[*k];
    }
    if let Value::Object(o) = s {
        o.insert("timestamp".into(), json!(iso(at)));
    }
    v
}

fn wire<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The task's status: its state, when it changed, and its message.
pub fn status(t: &TaskRow, replies: &[ReplyRow]) -> Value {
    stamp(wire(&typed_status(t, replies)), &[], t.updated_at)
}

/// The client's message, with its task and context filled in.
fn user_message(t: &TaskRow) -> Option<Message> {
    let mut m: Message = serde_json::from_value(t.request.clone()).ok()?;
    m.task_id = Some(t.id.clone());
    m.context_id = Some(t.context.clone());
    Some(m)
}

/// How a task is shown.
#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub history: Option<usize>,
    pub artifacts: bool,
}

impl Shown {
    pub const ALL: Shown = Shown { history: None, artifacts: true };
}

fn typed_task(t: &TaskRow, replies: &[ReplyRow], shown: Shown) -> Task {
    let mut history: Vec<Message> = user_message(t).into_iter().chain(replies.iter().map(|r| reply_message(t, r))).collect();
    let history = match shown.history {
        Some(0) => None,
        Some(n) => {
            let skip = history.len().saturating_sub(n);
            history.drain(..skip);
            Some(history)
        }
        None => Some(history),
    };
    let mut x = json!({ "recipients": t.recipients });
    if let Some(target) = &t.target {
        x["target"] = json!(target);
    }
    Task {
        id: t.id.clone(),
        context_id: t.context.clone(),
        status: typed_status(t, replies),
        artifacts: shown.artifacts.then(|| replies.iter().map(artifact).collect()),
        history,
        metadata: Some(HashMap::from([("xenonet".to_string(), x)])),
    }
}

/// A kept task as A2A's Task.
pub fn task(t: &TaskRow, replies: &[ReplyRow], shown: Shown) -> Value {
    stamp(wire(&typed_task(t, replies, shown)), &["status"], t.updated_at)
}

/// A stream's first event, and SendMessage's answer: the task.
pub fn task_event(t: &TaskRow, replies: &[ReplyRow], shown: Shown) -> Value {
    stamp(wire(&StreamResponse::Task(typed_task(t, replies, shown))), &["task", "status"], t.updated_at)
}

/// A stream's event: a status change.
pub fn status_event(t: &TaskRow, replies: &[ReplyRow]) -> Value {
    let e = TaskStatusUpdateEvent { task_id: t.id.clone(), context_id: t.context.clone(), status: typed_status(t, replies), metadata: None };
    stamp(wire(&StreamResponse::StatusUpdate(e)), &["statusUpdate", "status"], t.updated_at)
}

/// A stream's event: a reply, as an artifact.
pub fn artifact_event(t: &TaskRow, r: &ReplyRow) -> Value {
    let e = TaskArtifactUpdateEvent { task_id: t.id.clone(), context_id: t.context.clone(), artifact: artifact(r), append: Some(false), last_chunk: Some(true), metadata: None };
    wire(&StreamResponse::ArtifactUpdate(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(parts: Value) -> Value {
        json!({ "message": { "messageId": "m1", "role": "ROLE_USER", "parts": parts } })
    }

    #[test]
    fn text_parts_only() {
        let i = incoming(&msg(json!([{ "text": "Which branch" }, { "text": "has the fix?", "mediaType": "text/markdown" }]))).unwrap();
        assert_eq!(i.text, "Which branch\nhas the fix?");
        assert_eq!(incoming(&msg(json!([{ "url": "https://x/a.pdf" }]))).unwrap_err().code(), -32005);
        assert_eq!(incoming(&msg(json!([{ "data": { "a": 1 } }]))).unwrap_err().code(), -32005);
        assert_eq!(incoming(&msg(json!([{ "raw": "aGk=" }]))).unwrap_err().code(), -32005);
        assert_eq!(incoming(&msg(json!([{ "text": "{}", "mediaType": "application/json" }]))).unwrap_err().code(), -32005);
        assert_eq!(incoming(&msg(json!([{ "text": "a", "data": 1 }]))).unwrap_err().code(), -32602);
        assert_eq!(incoming(&msg(json!([]))).unwrap_err().code(), -32602);
        assert_eq!(incoming(&msg(json!([{ "text": "  " }]))).unwrap_err().code(), -32602);
    }

    #[test]
    fn a_clients_message_is_the_users() {
        let e = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_AGENT", "parts": [{ "text": "x" }] } })).unwrap_err();
        assert!(e.message().contains("ROLE_USER"));
        assert_eq!(incoming(&json!({ "message": { "role": "ROLE_USER", "parts": [{ "text": "x" }] } })).unwrap_err().code(), -32602, "a message id is required");
        assert_eq!(incoming(&json!({})).unwrap_err().code(), -32602);
    }

    #[test]
    fn targets_from_the_messages_metadata_or_the_requests() {
        let i = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }], "metadata": { "target": " claude:abc " } } })).unwrap();
        assert_eq!(i.target.as_deref(), Some("claude:abc"));
        let i = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }] }, "metadata": { "target": "repo:atlas" } })).unwrap();
        assert_eq!(i.target.as_deref(), Some("repo:atlas"));
        assert_eq!(incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }], "metadata": { "target": 5 } } })).unwrap_err().code(), -32602);
    }

    #[test]
    fn repos_and_selectors_as_the_engine_takes_them() {
        assert_eq!(engine_to("repo:atlas"), "project:atlas");
        assert_eq!(engine_to("repo:github.com/willctl/atlas.git"), "project:atlas");
        assert_eq!(engine_to("repo:git@github.com:x/atlas"), "project:atlas");
        assert_eq!(engine_to("claude:abc"), "claude:abc");
        assert!(is_wide("project:atlas"));
        assert!(is_wide("all"));
        assert!(is_wide("App:claude status:idle"));
        assert!(!is_wide("claude:abc"));
        assert!(!is_wide("actor:wren"));
    }

    #[test]
    fn output_modes() {
        assert!(accepts_text(None));
        assert!(accepts_text(Some(&json!([]))));
        assert!(accepts_text(Some(&json!(["application/json", "text/plain"]))));
        assert!(!accepts_text(Some(&json!(["application/json", "image/png"]))));
    }

    fn row(state: &str) -> TaskRow {
        TaskRow {
            seq: 1,
            id: "msg_t".into(),
            actor: "actor:wren".into(),
            context: "ctx_1".into(),
            client_message: "m1".into(),
            message: Some(41),
            target: Some("claude:x".into()),
            recipients: vec!["claude:x".into()],
            state: state.into(),
            reason: None,
            request: json!({ "messageId": "m1", "role": "ROLE_USER", "parts": [{ "text": "hi" }] }),
            created_at: 0.0,
            updated_at: 1_792_512_345_678.0,
        }
    }

    #[test]
    fn a_completed_task_carries_its_reply() {
        let r = ReplyRow { message: 42, mid: Some("msg_r".into()), sender: Some("claude:x".into()), sender_name: Some("Fix login".into()), text: "On fix/login.".into(), at: 0.0 };
        let t = task(&row(COMPLETED), std::slice::from_ref(&r), Shown::ALL);
        assert_eq!(t["status"]["state"], "TASK_STATE_COMPLETED");
        assert_eq!(t["status"]["timestamp"], "2026-10-20T16:05:45.678Z");
        assert_eq!(t["status"]["message"]["parts"][0]["text"], "On fix/login.");
        assert_eq!(t["status"]["message"]["role"], "ROLE_AGENT");
        assert_eq!(t["artifacts"][0]["name"], "Reply from Fix login");
        assert_eq!(t["history"].as_array().unwrap().len(), 2);
        assert_eq!(t["history"][0]["taskId"], "msg_t");
        let none = task(&row(COMPLETED), &[r], Shown { history: Some(0), artifacts: false });
        assert!(none.get("history").is_none() && none.get("artifacts").is_none(), "left out, not empty");
    }

    #[test]
    fn times_as_xenonet_writes_them() {
        let mut t = row(WORKING);
        t.updated_at = 1_792_512_345_000.0;
        assert_eq!(status(&t, &[])["timestamp"], "2026-10-20T16:05:45.000Z", "chrono would drop the milliseconds");
        assert_eq!(status_event(&t, &[])["statusUpdate"]["status"]["timestamp"], "2026-10-20T16:05:45.000Z");
        assert_eq!(task_event(&t, &[], Shown::ALL)["task"]["status"]["timestamp"], "2026-10-20T16:05:45.000Z");
    }

    #[test]
    fn a_refusal_says_why() {
        let mut t = row(REJECTED);
        t.reason = Some("no session or actor you can reach is claude:x".into());
        assert_eq!(status(&t, &[])["message"]["parts"][0]["text"], "no session or actor you can reach is claude:x");
        assert_eq!(status(&row(WORKING), &[])["message"]["parts"][0]["text"], "Sent to claude:x. Its reply completes this task.");
    }
}
