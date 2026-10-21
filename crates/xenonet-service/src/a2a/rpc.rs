//! A2A 1.0's JSON-RPC binding (spec sections 9 and 5.4): the request's
//! shape, the version it asks for, and the errors, each with the code the
//! spec gives it and a google.rpc detail saying why.

use crate::http::{Reply, ReplyBody};
use bytes::Bytes;
use serde_json::{json, Value};

/// The one version XenoNet speaks.
pub const VERSION: &str = "1.0";
/// What an empty or missing A2A-Version means (spec 3.6.2).
pub const UNSAID: &str = "0.3";
// The reasons are A2A's own, as the spec's example names them.
const DOMAIN: &str = "a2a-protocol.org";
const INFO: &str = "type.googleapis.com/google.rpc.ErrorInfo";
const BAD_REQUEST: &str = "type.googleapis.com/google.rpc.BadRequest";

/// A JSON-RPC error, and the HTTP status its answer goes with.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
    pub status: u16,
}

fn info(reason: &str, metadata: Value) -> Value {
    json!([{ "@type": INFO, "reason": reason, "domain": DOMAIN, "metadata": metadata }])
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>, data: Option<Value>) -> RpcError {
        RpcError { code, message: message.into(), data, status: 200 }
    }

    #[must_use]
    pub fn with_status(mut self, status: u16) -> RpcError {
        self.status = status;
        self
    }

    pub fn parse() -> RpcError {
        RpcError::new(-32700, "Invalid JSON payload", None)
    }

    pub fn invalid_request(why: &str) -> RpcError {
        RpcError::new(-32600, format!("Request payload validation error: {why}"), None)
    }

    pub fn method_not_found(method: &str) -> RpcError {
        RpcError::new(-32601, "Method not found", Some(info("METHOD_NOT_FOUND", json!({ "method": method }))))
    }

    /// A parameter that isn't right: which one, and why.
    pub fn invalid_params(field: &str, why: &str) -> RpcError {
        RpcError::new(-32602, format!("Invalid parameters: {field}: {why}"), Some(json!([{ "@type": BAD_REQUEST, "fieldViolations": [{ "field": field, "description": why }] }])))
    }

    pub fn internal(why: &str) -> RpcError {
        RpcError::new(-32603, format!("Internal error: {why}"), None)
    }

    pub fn task_not_found(id: &str) -> RpcError {
        RpcError::new(-32001, "Task not found", Some(info("TASK_NOT_FOUND", json!({ "taskId": id }))))
    }

    pub fn not_cancelable(id: &str, state: &str) -> RpcError {
        RpcError::new(-32002, format!("Task cannot be canceled: it is {state}"), Some(info("TASK_NOT_CANCELABLE", json!({ "taskId": id, "state": state }))))
    }

    pub fn push_not_supported() -> RpcError {
        RpcError::new(
            -32003,
            "Push Notification is not supported: XenoNet makes no outbound calls; poll GetTask or stream instead",
            Some(info("PUSH_NOTIFICATION_NOT_SUPPORTED", json!({}))),
        )
    }

    pub fn unsupported(why: &str) -> RpcError {
        RpcError::new(-32004, format!("This operation is not supported: {why}"), Some(info("UNSUPPORTED_OPERATION", json!({}))))
    }

    pub fn content_type(why: &str) -> RpcError {
        RpcError::new(-32005, format!("Incompatible content types: {why}"), Some(info("CONTENT_TYPE_NOT_SUPPORTED", json!({ "supported": "text/plain" }))))
    }

    pub fn version(asked: &str) -> RpcError {
        let said = if asked.is_empty() { format!("no A2A-Version means {UNSAID}") } else { format!("it asked for {asked}") };
        RpcError::new(
            -32009,
            format!("Protocol version not supported: XenoNet speaks A2A {VERSION} ({said}); send the header A2A-Version: {VERSION}"),
            Some(info("VERSION_NOT_SUPPORTED", json!({ "requested": if asked.is_empty() { UNSAID } else { asked }, "supported": VERSION }))),
        )
    }

    /// The JSON-RPC answer for this error, to request `id`.
    pub fn to_json(&self, id: &Value) -> Value {
        let mut e = json!({ "code": self.code, "message": self.message });
        if let Some(d) = &self.data {
            e["data"] = d.clone();
        }
        json!({ "jsonrpc": "2.0", "id": id, "error": e })
    }
}

/// A result's answer to request `id`.
pub fn result(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// The version a request asks for: its A2A-Version header, else the
/// parameter of that name, trimmed ("" when it says none).
pub fn asked_version(headers: &[(String, String)], query: &[(String, String)]) -> String {
    let header = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("a2a-version")).map(|(_, v)| v.as_str());
    let param = query.iter().find(|(k, _)| k.eq_ignore_ascii_case("a2a-version")).map(|(_, v)| v.as_str());
    header.or(param).unwrap_or("").trim().to_string()
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

/// A JSON answer.
pub fn json_reply(status: u16, body: &Value, extra: &[(&str, &str)]) -> Reply {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string()), ("cache-control".to_string(), "no-store".to_string())];
    for (k, v) in extra {
        headers.push(((*k).to_string(), (*v).to_string()));
    }
    Reply { status, headers, body: ReplyBody::Full(Bytes::from(xenonet_base::json::stringify(body, 0))) }
}

/// One server-sent event carrying a JSON-RPC answer.
pub fn sse_frame(body: &Value) -> Bytes {
    Bytes::from(format!("data: {}\n\n", xenonet_base::json::stringify(body, 0)))
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
        assert_eq!((id, e.code), (Value::Null, -32600));
        assert_eq!(envelope(&json!({ "jsonrpc": "1.0", "id": "a", "method": "GetTask" })).unwrap_err().0, json!("a"));
        assert_eq!(envelope(&json!({ "jsonrpc": "2.0", "method": "GetTask" })).unwrap_err().1.code, -32600, "a notification gets no task");
        assert_eq!(envelope(&json!({ "jsonrpc": "2.0", "id": 2, "method": "GetTask", "params": [1] })).unwrap_err().1.code, -32602);
    }

    #[test]
    fn errors_carry_the_specs_codes() {
        assert_eq!(RpcError::task_not_found("t").code, -32001);
        assert_eq!(RpcError::not_cancelable("t", "TASK_STATE_COMPLETED").code, -32002);
        assert_eq!(RpcError::push_not_supported().code, -32003);
        assert_eq!(RpcError::unsupported("x").code, -32004);
        assert_eq!(RpcError::content_type("x").code, -32005);
        assert_eq!(RpcError::version("").code, -32009);
        let v = RpcError::version("").to_json(&json!(7));
        assert_eq!(v["error"]["data"][0]["metadata"], json!({ "requested": "0.3", "supported": "1.0" }));
        assert_eq!(v["id"], 7);
    }
}
