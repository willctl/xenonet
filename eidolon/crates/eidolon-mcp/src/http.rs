//! The tools over HTTP on this machine, for an agent that cannot start the
//! server as a child: MCP's Streamable HTTP in its simplest allowed form.
//! One path takes a POSTed JSON-RPC message and answers `application/json`;
//! there is no event stream.
//!
//! The listener is for XenoNet to publish as a service, so a session on
//! another machine reaches it through XenoNet's own door and credential.
//! The token here is the service's own and is checked on every request.
//!
//! Rules, each with a test:
//! - the address is `127.0.0.1` or `[::1]`; anything else is refused before
//!   a socket is opened;
//! - every request carries `Authorization: Bearer <token>`, compared in
//!   constant time, or it gets 401;
//! - a request with an `Origin` header gets 403, so a web page cannot call
//!   the tools;
//! - a body over [`MAX_BODY`] gets 413, and only [`PATH`] answers.
//!
//! The token is made by the caller and is never logged or put in an error.

use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, ORIGIN};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::error::{Error, Result};
use crate::server::{INSTRUCTIONS, NAME};
use crate::tools::{definitions, BrowserTools, Part, ToolDef};

/// The one path that answers.
pub const PATH: &str = "/mcp/";
/// The largest request body, as XenoNet's service limit.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// The protocol version answered when a client names none.
const PROTOCOL: &str = "2025-06-18";
const HEADER_WAIT: Duration = Duration::from_secs(10);

/// A bound listener that has not started answering.
pub struct HttpServer {
    listener: TcpListener,
    token: Arc<str>,
}

impl std::fmt::Debug for HttpServer {
    // The token stays out of any log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpServer").field("addr", &self.listener.local_addr().ok()).finish_non_exhaustive()
    }
}

/// Whether `addr` is one of the two loopback addresses we serve on.
fn loopback(addr: &SocketAddr) -> bool {
    match addr.ip() {
        IpAddr::V4(ip) => ip.octets() == [127, 0, 0, 1],
        IpAddr::V6(ip) => ip.is_loopback(),
    }
}

impl HttpServer {
    /// Binds `addr`, which must be `127.0.0.1` or `[::1]`; port 0 picks
    /// one, which [`HttpServer::local_addr`] tells.
    pub async fn bind(addr: SocketAddr, token: &str) -> Result<HttpServer> {
        if !loopback(&addr) {
            return Err(Error::Refused("The tools are served on 127.0.0.1 or [::1] only. Use one of those, and let XenoNet publish the service to other machines.".into()));
        }
        if token.len() < 16 {
            return Err(Error::Refused("The token is too short. Make it from 32 random bytes, base64url.".into()));
        }
        let listener = TcpListener::bind(addr).await.map_err(|e| Error::Io(format!("Could not listen on {addr}: {e}. Pick another port.")))?;
        Ok(HttpServer { listener, token: Arc::from(token) })
    }

    /// Where it listens.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(|e| Error::Io(format!("Could not read the listening address: {e}.")))
    }

    /// Answers until this future is dropped, which also ends the open
    /// connections.
    pub async fn serve(self, tools: BrowserTools) -> Result<()> {
        let state = Arc::new(State { tools, defs: definitions(), token_digest: digest(self.token.as_bytes()) });
        let mut conns = JoinSet::new();
        loop {
            let (stream, _) = self.listener.accept().await.map_err(|e| Error::Serve(format!("accepting a connection failed: {e}")))?;
            let state = state.clone();
            conns.spawn(async move {
                let service = service_fn(move |req| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(handle(&state, req).await) }
                });
                let _ = http1::Builder::new().timer(TokioTimer::new()).header_read_timeout(HEADER_WAIT).serve_connection(TokioIo::new(stream), service).await;
            });
            // Finished connections are collected as new ones arrive.
            while conns.try_join_next().is_some() {}
        }
    }
}

/// Serves the tools over HTTP on `addr` until the future is dropped.
pub async fn serve_http(tools: BrowserTools, addr: SocketAddr, token: &str) -> Result<()> {
    HttpServer::bind(addr, token).await?.serve(tools).await
}

struct State {
    tools: BrowserTools,
    defs: Vec<ToolDef>,
    token_digest: ring::digest::Digest,
}

fn digest(bytes: &[u8]) -> ring::digest::Digest {
    ring::digest::digest(&ring::digest::SHA256, bytes)
}

/// Whether the header carries the token. Both sides are hashed first, so
/// the comparison is over equal lengths and visits every byte whatever the
/// caller sent.
fn authorised(state: &State, header: Option<&HeaderValue>) -> bool {
    let Some(sent) = header.and_then(|h| h.to_str().ok()).and_then(|h| h.strip_prefix("Bearer ")) else { return false };
    let (a, b) = (digest(sent.as_bytes()), &state.token_digest);
    // Every byte is visited whether or not an earlier one differed.
    a.as_ref().iter().zip(b.as_ref()).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn reply(status: StatusCode, body: Option<&Value>) -> Response<Full<Bytes>> {
    let (bytes, json) = body.map_or((Bytes::new(), false), |b| (Bytes::from(b.to_string()), true));
    let mut r = Response::new(Full::new(bytes));
    *r.status_mut() = status;
    if json {
        r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    }
    r
}

fn refusal(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    reply(status, Some(&json!({ "error": message })))
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

async fn handle(state: &State, req: Request<Incoming>) -> Response<Full<Bytes>> {
    if req.uri().path() != PATH {
        return refusal(StatusCode::NOT_FOUND, "There is nothing here. The tools answer at /mcp/.");
    }
    // A browser adds Origin to the requests a web page makes; the tools are
    // for programs, so any request that has one is refused.
    if req.headers().contains_key(ORIGIN) {
        return refusal(StatusCode::FORBIDDEN, "Requests from a web page are refused.");
    }
    if !authorised(state, req.headers().get(AUTHORIZATION)) {
        let mut r = refusal(StatusCode::UNAUTHORIZED, "Send the service's token as Authorization: Bearer <token>.");
        r.headers_mut().insert(hyper::header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        return r;
    }
    if req.method() != Method::POST {
        let mut r = refusal(StatusCode::METHOD_NOT_ALLOWED, "POST a JSON-RPC message. There is no event stream here.");
        r.headers_mut().insert(hyper::header::ALLOW, HeaderValue::from_static("POST"));
        return r;
    }
    let too_big = || refusal(StatusCode::PAYLOAD_TOO_LARGE, "The request is over 8 MiB.");
    if req.headers().get(hyper::header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<usize>().ok()).is_some_and(|n| n > MAX_BODY) {
        return too_big();
    }
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return too_big(),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return reply(StatusCode::BAD_REQUEST, Some(&rpc_error(&Value::Null, -32700, "The body is not JSON.")));
    };
    if !message.is_object() {
        return reply(StatusCode::BAD_REQUEST, Some(&rpc_error(&Value::Null, -32600, "Send one JSON-RPC message, not a list.")));
    }
    match message.get("id").cloned() {
        // A notification, or a client's answer: nothing to say back.
        None => reply(StatusCode::ACCEPTED, None),
        Some(id) => reply(StatusCode::OK, Some(&dispatch(state, &id, &message).await)),
    }
}

async fn dispatch(state: &State, id: &Value, message: &Value) -> Value {
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let result = match message.get("method").and_then(Value::as_str) {
        Some("initialize") => json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": NAME, "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        }),
        Some("ping") => json!({}),
        Some("tools/list") => {
            json!({ "tools": state.defs.iter().map(|d| json!({ "name": d.name, "description": d.description, "inputSchema": d.schema })).collect::<Vec<_>>() })
        }
        Some("tools/call") => {
            let Some(name) = params.get("name").and_then(Value::as_str) else { return rpc_error(id, -32602, "Name the tool to call.") };
            if !state.defs.iter().any(|d| d.name == name) {
                return rpc_error(id, -32602, &format!("There is no tool named {name}."));
            }
            let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
            // A tool that fails answers with its reason, which the agent
            // reads; it is not a protocol error.
            match state.tools.call_tool(name, arguments).await {
                Ok(parts) => json!({ "content": parts.into_iter().map(content).collect::<Vec<_>>(), "isError": false }),
                Err(e) => json!({ "content": [{ "type": "text", "text": e.to_string() }], "isError": true }),
            }
        }
        Some(other) => return rpc_error(id, -32601, &format!("There is no method {other}.")),
        None => return rpc_error(id, -32600, "The message names no method."),
    };
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn content(part: Part) -> Value {
    match part {
        Part::Text(text) => json!({ "type": "text", "text": text }),
        Part::Image { data, mime } => json!({ "type": "image", "data": data, "mimeType": mime }),
    }
}
