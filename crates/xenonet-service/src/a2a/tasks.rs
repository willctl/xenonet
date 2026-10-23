//! How A2A's objects map to XenoNet's: a client's Message to the text of a
//! XenoNet message and where it goes, and a kept task back to A2A's Task,
//! its status, history and artifacts (spec section 4.1), and to the events
//! of a stream (4.2).

use super::rpc::RpcError;
use super::store::{ReplyRow, TaskRow, CANCELED, COMPLETED, FAILED, REJECTED, WORKING};
use serde_json::{json, Map, Value};
use xenonet_base::time::iso;
use xenonet_engine::delivery::selector::KEYS as SELECTOR_KEYS;

/// The longest id XenoNet keeps of a client's (message ids, contexts).
pub const ID_MAX: usize = 200;
/// The longest target a message may name.
const TARGET_MAX: usize = 300;

/// A client's message, as XenoNet takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Incoming {
    pub message_id: String,
    pub context_id: Option<String>,
    pub task_id: Option<String>,
    pub text: String,
    /// The target it names, if any (metadata.target).
    pub target: Option<String>,
    /// The message as it came, to give back in the task's history.
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

/// A SendMessageRequest's message, checked: text parts only.
pub fn incoming(params: &Value) -> Result<Incoming, RpcError> {
    let Some(m) = params.get("message").filter(|m| m.is_object()) else {
        return Err(RpcError::invalid_params("message", "a SendMessageRequest carries a message"));
    };
    let Some(message_id) = short_id(m.get("messageId"), "message.messageId")? else {
        return Err(RpcError::invalid_params("message.messageId", "is required"));
    };
    match m.get("role").and_then(Value::as_str) {
        Some("ROLE_USER") => {}
        Some(other) => return Err(RpcError::invalid_params("message.role", &format!("a client's message is ROLE_USER, not {other}"))),
        None => return Err(RpcError::invalid_params("message.role", "is required (ROLE_USER)")),
    }
    let Some(parts) = m.get("parts").and_then(Value::as_array).filter(|p| !p.is_empty()) else {
        return Err(RpcError::invalid_params("message.parts", "At least one part is required"));
    };
    let mut texts = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        let field = format!("message.parts[{i}]");
        let Some(o) = p.as_object() else { return Err(RpcError::invalid_params(&field, "a part is an object")) };
        let kinds: Vec<&str> = ["text", "raw", "url", "data"].into_iter().filter(|k| o.contains_key(*k)).collect();
        match kinds.as_slice() {
            ["text"] => match o.get("text") {
                Some(Value::String(t)) => {
                    let media = o.get("mediaType").and_then(Value::as_str).unwrap_or("text/plain");
                    if !media.to_ascii_lowercase().starts_with("text/") {
                        return Err(RpcError::content_type(&format!("{field} says it's {media}; XenoNet takes text")));
                    }
                    texts.push(t.as_str());
                }
                _ => return Err(RpcError::invalid_params(&format!("{field}.text"), "must be a string")),
            },
            [one] => return Err(RpcError::content_type(&format!("{field} is a {} part; XenoNet takes text parts only", if *one == "data" { "data" } else { "file" }))),
            [] => return Err(RpcError::invalid_params(&field, "a part holds one of text, raw, url or data")),
            _ => return Err(RpcError::invalid_params(&field, "a part holds exactly one of text, raw, url or data")),
        }
    }
    let text = xenonet_base::text::js_trim(&texts.join("\n")).to_string();
    if text.is_empty() {
        return Err(RpcError::invalid_params("message.parts", "the text is empty"));
    }
    let target = target_of(m.get("metadata")).or_else(|| target_of(params.get("metadata")));
    let target = match target {
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return Err(e),
        None => None,
    };
    Ok(Incoming {
        message_id,
        context_id: short_id(m.get("contextId"), "message.contextId")?,
        task_id: short_id(m.get("taskId"), "message.taskId")?,
        text,
        target,
        message: m.clone(),
    })
}

fn target_of(metadata: Option<&Value>) -> Option<Result<String, RpcError>> {
    let t = metadata?.get("target")?;
    Some(match t.as_str().map(xenonet_base::text::js_trim) {
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

/// How much history a task is shown with: None for all of it.
pub fn history_length(v: Option<&Value>, field: &str) -> Result<Option<usize>, RpcError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(n) => match n.as_i64() {
            Some(n) if n >= 0 => Ok(Some(usize::try_from(n).unwrap_or(usize::MAX))),
            _ => Err(RpcError::invalid_params(field, "must be a non-negative integer")),
        },
    }
}

/// A reply's id as A2A shows it: its mid, or its XenoNet id.
fn reply_id(r: &ReplyRow) -> String {
    r.mid.clone().unwrap_or_else(|| format!("xenonet-{}", r.message))
}

/// A message from XenoNet's side, for a task.
fn agent_message(t: &TaskRow, id: &str, text: &str, metadata: Option<Value>) -> Value {
    let mut m = json!({ "messageId": id, "contextId": t.context, "taskId": t.id, "role": "ROLE_AGENT", "parts": [{ "text": text, "mediaType": "text/plain" }] });
    if let Some(md) = metadata {
        m["metadata"] = md;
    }
    m
}

fn reply_message(t: &TaskRow, r: &ReplyRow) -> Value {
    agent_message(t, &reply_id(r), &r.text, Some(from_of(r)))
}

fn from_of(r: &ReplyRow) -> Value {
    let mut md = Map::new();
    if let Some(s) = &r.sender {
        md.insert("from".into(), json!(s));
    }
    if let Some(n) = &r.sender_name {
        md.insert("fromName".into(), json!(n));
    }
    Value::Object(md)
}

/// A reply as an artifact.
pub fn artifact(r: &ReplyRow) -> Value {
    let who = r.sender_name.clone().or_else(|| r.sender.clone()).unwrap_or_else(|| "the session".into());
    json!({ "artifactId": format!("reply-{}", r.message), "name": format!("Reply from {who}"), "parts": [{ "text": r.text, "mediaType": "text/plain" }], "metadata": from_of(r) })
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

/// The task's status: its state, when it changed, and its message.
pub fn status(t: &TaskRow, replies: &[ReplyRow]) -> Value {
    let mut s = json!({ "state": t.state, "timestamp": iso(t.updated_at) });
    if t.state == COMPLETED {
        if let Some(r) = replies.first() {
            s["message"] = reply_message(t, r);
        }
    } else if let Some(text) = status_text(t) {
        s["message"] = agent_message(t, &format!("{}-{}", t.id, t.state.trim_start_matches("TASK_STATE_").to_ascii_lowercase()), &text, None);
    }
    s
}

/// The client's message, with its task and context filled in.
fn user_message(t: &TaskRow) -> Value {
    let mut m = t.request.clone();
    if m.is_object() {
        m["taskId"] = json!(t.id);
        m["contextId"] = json!(t.context);
    }
    m
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

/// A kept task as A2A's Task.
pub fn task(t: &TaskRow, replies: &[ReplyRow], shown: Shown) -> Value {
    let mut out = json!({ "id": t.id, "contextId": t.context, "status": status(t, replies) });
    if shown.artifacts {
        out["artifacts"] = Value::Array(replies.iter().map(artifact).collect());
    }
    let mut history: Vec<Value> = std::iter::once(user_message(t)).chain(replies.iter().map(|r| reply_message(t, r))).collect();
    match shown.history {
        Some(0) => {}
        Some(n) => {
            let skip = history.len().saturating_sub(n);
            history.drain(..skip);
            out["history"] = Value::Array(history);
        }
        None => out["history"] = Value::Array(history),
    }
    let mut x = json!({ "recipients": t.recipients });
    if let Some(target) = &t.target {
        x["target"] = json!(target);
    }
    out["metadata"] = json!({ "xenonet": x });
    out
}

/// A stream's event: a status change.
pub fn status_event(t: &TaskRow, replies: &[ReplyRow]) -> Value {
    json!({ "statusUpdate": { "taskId": t.id, "contextId": t.context, "status": status(t, replies) } })
}

/// A stream's event: a reply, as an artifact.
pub fn artifact_event(t: &TaskRow, r: &ReplyRow) -> Value {
    json!({ "artifactUpdate": { "taskId": t.id, "contextId": t.context, "artifact": artifact(r), "append": false, "lastChunk": true } })
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
        assert_eq!(incoming(&msg(json!([{ "url": "https://x/a.pdf" }]))).unwrap_err().code, -32005);
        assert_eq!(incoming(&msg(json!([{ "data": { "a": 1 } }]))).unwrap_err().code, -32005);
        assert_eq!(incoming(&msg(json!([{ "raw": "aGk=" }]))).unwrap_err().code, -32005);
        assert_eq!(incoming(&msg(json!([{ "text": "{}", "mediaType": "application/json" }]))).unwrap_err().code, -32005);
        assert_eq!(incoming(&msg(json!([{ "text": "a", "data": 1 }]))).unwrap_err().code, -32602);
        assert_eq!(incoming(&msg(json!([]))).unwrap_err().code, -32602);
        assert_eq!(incoming(&msg(json!([{ "text": "  " }]))).unwrap_err().code, -32602);
    }

    #[test]
    fn a_clients_message_is_the_users() {
        let e = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_AGENT", "parts": [{ "text": "x" }] } })).unwrap_err();
        assert!(e.message.contains("ROLE_USER"));
        assert_eq!(incoming(&json!({ "message": { "role": "ROLE_USER", "parts": [{ "text": "x" }] } })).unwrap_err().code, -32602, "a message id is required");
        assert_eq!(incoming(&json!({})).unwrap_err().code, -32602);
    }

    #[test]
    fn targets_from_the_messages_metadata_or_the_requests() {
        let i = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }], "metadata": { "target": " claude:abc " } } })).unwrap();
        assert_eq!(i.target.as_deref(), Some("claude:abc"));
        let i = incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }] }, "metadata": { "target": "repo:atlas" } })).unwrap();
        assert_eq!(i.target.as_deref(), Some("repo:atlas"));
        assert_eq!(incoming(&json!({ "message": { "messageId": "m", "role": "ROLE_USER", "parts": [{ "text": "x" }], "metadata": { "target": 5 } } })).unwrap_err().code, -32602);
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
    fn a_refusal_says_why() {
        let mut t = row(REJECTED);
        t.reason = Some("no session or actor you can reach is claude:x".into());
        assert_eq!(status(&t, &[])["message"]["parts"][0]["text"], "no session or actor you can reach is claude:x");
        assert_eq!(status(&row(WORKING), &[])["message"]["parts"][0]["text"], "Sent to claude:x. Its reply completes this task.");
    }
}
