//! A2A 1.0's JSON-RPC binding (spec sections 9 and 5.4): the request's
//! shape, the version it asks for, and the errors, each a2a-lf's
//! `JsonRpcError` with the code the spec gives it and a google.rpc detail
//! saying why. The words are XenoNet's own: a2a-lf's `to_jsonrpc_error`
//! isn't used, since it stamps every error with the time.

use crate::http::{Reply, ReplyBody};
use ::a2a::error_code as code;
use ::a2a::errordetails::{FieldViolation, TypedDetail, PROTOCOL_DOMAIN};
use ::a2a::JsonRpcError;
use bytes::Bytes;
use serde_json::{json, Value};
use std::collections::HashMap;

/// The one version XenoNet speaks.
pub const VERSION: &str = ::a2a::VERSION;
/// What an empty or missing A2A-Version means (spec 3.6.2).
pub const UNSAID: &str = "0.3";

/// A JSON-RPC error, and the HTTP status its answer goes with.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub error: JsonRpcError,
    pub status: u16,
}

// The reasons are A2A's own, as the spec's example names them.
fn info(reason: &str, metadata: &[(&str, &str)]) -> Value {
    let metadata: HashMap<String, String> = metadata.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    json!([TypedDetail::error_info(reason, PROTOCOL_DOMAIN, Some(metadata))])
}

impl RpcError {
    fn new(code: i32, message: impl Into<String>, data: Option<Value>) -> RpcError {
        RpcError { error: JsonRpcError { code, message: message.into(), data }, status: 200 }
    }

    pub fn code(&self) -> i32 {
        self.error.code
    }

    pub fn message(&self) -> &str {
        &self.error.message
    }

    #[must_use]
    pub fn with_status(mut self, status: u16) -> RpcError {
        self.status = status;
        self
    }

    pub fn parse() -> RpcError {
        RpcError::new(code::PARSE_ERROR, "Invalid JSON payload", None)
    }

    pub fn invalid_request(why: &str) -> RpcError {
        RpcError::new(code::INVALID_REQUEST, format!("Request payload validation error: {why}"), None)
    }

    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::new(code::METHOD_NOT_FOUND, "Method not found", Some(info("METHOD_NOT_FOUND", &[("method", method)])))
    }

    /// A parameter that isn't right: which one, and why.
    pub fn invalid_params(field: &str, why: &str) -> RpcError {
        let violation = FieldViolation { field: field.to_string(), description: why.to_string() };
        RpcError::new(code::INVALID_PARAMS, format!("Invalid parameters: {field}: {why}"), Some(json!([TypedDetail::bad_request(vec![violation])])))
    }

    pub fn internal(why: &str) -> RpcError {
        RpcError::new(code::INTERNAL_ERROR, format!("Internal error: {why}"), None)
    }

    pub fn task_not_found(id: &str) -> RpcError {
        RpcError::new(code::TASK_NOT_FOUND, "Task not found", Some(info("TASK_NOT_FOUND", &[("taskId", id)])))
    }

    pub fn not_cancelable(id: &str, state: &str) -> RpcError {
        RpcError::new(code::TASK_NOT_CANCELABLE, format!("Task cannot be canceled: it is {state}"), Some(info("TASK_NOT_CANCELABLE", &[("taskId", id), ("state", state)])))
    }

    pub fn push_not_supported() -> RpcError {
        RpcError::new(
            code::PUSH_NOTIFICATION_NOT_SUPPORTED,
            "Push Notification is not supported: XenoNet makes no outbound calls; poll GetTask or stream instead",
            Some(info("PUSH_NOTIFICATION_NOT_SUPPORTED", &[])),
        )
    }

    pub fn unsupported(why: &str) -> RpcError {
        RpcError::new(code::UNSUPPORTED_OPERATION, format!("This operation is not supported: {why}"), Some(info("UNSUPPORTED_OPERATION", &[])))
    }

    pub fn content_type(why: &str) -> RpcError {
        RpcError::new(code::CONTENT_TYPE_NOT_SUPPORTED, format!("Incompatible content types: {why}"), Some(info("CONTENT_TYPE_NOT_SUPPORTED", &[("supported", "text/plain")])))
    }

    pub fn version(asked: &str) -> RpcError {
        let said = if asked.is_empty() { format!("no A2A-Version means {UNSAID}") } else { format!("it asked for {asked}") };
        RpcError::new(
            code::VERSION_NOT_SUPPORTED,
            format!("Protocol version not supported: XenoNet speaks A2A {VERSION} ({said}); send the header A2A-Version: {VERSION}"),
            Some(info("VERSION_NOT_SUPPORTED", &[("requested", if asked.is_empty() { UNSAID } else { asked }), ("supported", VERSION)])),
        )
    }

    /// The JSON-RPC answer for this error, to request `id`. The id goes
    /// back as it came: a2a-lf's `JsonRpcId` holds integers only.
    pub fn to_json(&self, id: &Value) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "error": self.error })
    }
}

/// A result's answer to request `id`.
pub fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The version a request asks for: its A2A-Version header, else the
/// parameter of that name, trimmed ("" when it says none).
pub fn asked_version(headers: &[(String, String)], query: &[(String, String)]) -> String {
    let named = |list: &'_ [(String, String)]| list.iter().find(|(k, _)| k.eq_ignore_ascii_case(::a2a::SVC_PARAM_VERSION)).map(|(_, v)| v.clone());
    named(headers).or_else(|| named(query)).unwrap_or_default().trim().to_string()
}

/// Whether XenoNet speaks the version asked for: major and minor match,
/// and a patch number is ignored (spec 3.6).
pub fn speaks(asked: &str) -> bool {
    let mut parts = asked.split('.');
    matches!((parts.next(), parts.next()), (Some("1"), Some("0")))
}

/// The request, checked as JSON-RPC 2.0: its id, method and params.
pub fn envelope(v: &Value) -> Result<(Value, String, Value), (Value, RpcError)> {
    let Some(o) = v.as_object() else {
        let why = if v.is_array() { "send one request at a time, not a batch" } else { "the body isn't a JSON-RPC request object" };
        return Err((Value::Null, RpcError::invalid_request(why)));
    };
    let id = o.get("id").cloned().unwrap_or(Value::Null);
    let id_ok = matches!(id, Value::String(_) | Value::Number(_));
    let reply_id = if id_ok { id.clone() } else { Value::Null };
    if o.get("jsonrpc") != Some(&json!("2.0")) {
        return Err((reply_id, RpcError::invalid_request("jsonrpc must be \"2.0\"")));
    }
    let Some(method) = o.get("method").and_then(Value::as_str) else {
        return Err((reply_id, RpcError::invalid_request("method must be a string")));
    };
    if !id_ok {
        return Err((reply_id, RpcError::invalid_request("an A2A request carries an id, a string or a number")));
    }
    let params = match o.get("params") {
        None | Some(Value::Null) => json!({}),
        Some(p @ Value::Object(_)) => p.clone(),
        Some(_) => return Err((reply_id, RpcError::invalid_params("params", "must be an object"))),
    };
    Ok((id, method.to_string(), params))
}

/// `params` as one of a2a-lf's request types, once XenoNet's own checks
/// passed: what they let through and the type still refuses (a number past
/// its 32 bits) is an invalid parameter, named by its path.
pub fn typed<T: serde::de::DeserializeOwned>(params: &Value) -> Result<T, RpcError> {
    serde_path_to_error::deserialize(params).map_err(|e| {
        let path = e.path().to_string();
        RpcError::invalid_params(if path == "." { "params" } else { &path }, &e.into_inner().to_string())
    })
}

/// A JSON answer.
pub fn json_reply(status: u16, body: &Value, extra: &[(&str, &str)]) -> Reply {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string()), ("cache-control".to_string(), "no-store".to_string())];
    for (k, v) in extra {
        headers.push(((*k).to_string(), (*v).to_string()));
    }
    Reply { status, headers, body: ReplyBody::Full(Bytes::from(xenonet_text::json::stringify(body, 0))) }
}

/// One server-sent event carrying a JSON-RPC answer.
pub fn sse_frame(body: &Value) -> Bytes {
    Bytes::from(format!("data: {}\n\n", xenonet_text::json::stringify(body, 0)))
}

/// The headers of an event stream.
pub fn sse_headers() -> Vec<(String, String)> {
    vec![
        ("content-type".to_string(), "text/event-stream".to_string()),
        ("cache-control".to_string(), "no-store".to_string()),
        // Proxies in front of the gateway pass each event on at once.
        ("x-accel-buffering".to_string(), "no".to_string()),
    ]
}

/// An error, as one event of a stream that then ends: a client that asked
/// for a stream reads its answer as one.
pub fn sse_error(id: &Value, e: &RpcError) -> Reply {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let _ = tx.send(sse_frame(&e.to_json(id)));
    drop(tx);
    Reply { status: 200, headers: sse_headers(), body: ReplyBody::Stream(rx) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_by_major_and_minor() {
        assert!(speaks("1.0"));
        assert!(speaks("1.0.3"), "a patch number doesn't count");
        assert!(!speaks("0.3"));
        assert!(!speaks("1.1"));
        assert!(!speaks(""));
        let h = vec![("a2a-version".to_string(), " 1.0 ".to_string())];
        assert_eq!(asked_version(&h, &[]), "1.0");
        assert_eq!(asked_version(&[], &[("A2A-Version".into(), "1.0".into())]), "1.0", "or as a parameter");
        assert_eq!(asked_version(&[], &[]), "");
    }

    #[test]
    fn requests_are_json_rpc_two() {
        assert_eq!(envelope(&json!({ "jsonrpc": "2.0", "id": 1, "method": "GetTask" })).unwrap(), (json!(1), "GetTask".into(), json!({})));
        let (id, e) = envelope(&json!([{ "jsonrpc": "2.0" }])).unwrap_err();
        assert_eq!((id, e.code()), (Value::Null, -32600));
        assert_eq!(envelope(&json!({ "jsonrpc": "1.0", "id": "a", "method": "GetTask" })).unwrap_err().0, json!("a"));
        assert_eq!(envelope(&json!({ "jsonrpc": "2.0", "method": "GetTask" })).unwrap_err().1.code(), -32600, "a notification gets no task");
        assert_eq!(envelope(&json!({ "jsonrpc": "2.0", "id": 2, "method": "GetTask", "params": [1] })).unwrap_err().1.code(), -32602);
    }

    #[test]
    fn errors_carry_the_specs_codes() {
        assert_eq!(RpcError::task_not_found("t").code(), -32001);
        assert_eq!(RpcError::not_cancelable("t", "TASK_STATE_COMPLETED").code(), -32002);
        assert_eq!(RpcError::push_not_supported().code(), -32003);
        assert_eq!(RpcError::unsupported("x").code(), -32004);
        assert_eq!(RpcError::content_type("x").code(), -32005);
        assert_eq!(RpcError::version("").code(), -32009);
        let v = RpcError::version("").to_json(&json!(7));
        assert_eq!(v["error"]["data"][0]["metadata"], json!({ "requested": "0.3", "supported": "1.0" }));
        assert_eq!(v["error"]["data"][0]["@type"], "type.googleapis.com/google.rpc.ErrorInfo");
        assert_eq!(v["id"], 7);
        let p = RpcError::invalid_params("id", "is required").to_json(&json!(1));
        assert_eq!(p["error"]["data"], json!([{ "@type": "type.googleapis.com/google.rpc.BadRequest", "fieldViolations": [{ "field": "id", "description": "is required" }] }]));
    }

    #[test]
    fn typed_requests_name_the_field() {
        let t: ::a2a::GetTaskRequest = typed(&json!({ "id": "t", "historyLength": 3 })).unwrap();
        assert_eq!((t.id.as_str(), t.history_length), ("t", Some(3)));
        let e = typed::<::a2a::GetTaskRequest>(&json!({ "id": "t", "historyLength": 1u64 << 40 })).unwrap_err();
        assert_eq!(e.code(), -32602);
        assert!(e.message().contains("historyLength"), "{}", e.message());
    }
}
