//! The tools over localhost HTTP, on a real socket: every refusal first,
//! then a session of `initialize`, `tools/list` and `tools/call`.

use std::net::SocketAddr;

use eidolon_mcp::{definitions, Error, HttpServer, MAX_BODY, PATH};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::support::env;

const TOKEN: &str = "k3Jm9wQ2xZ8vB1nR5tY7uC4dF6gH0aLs-_PeOiXqVbM";

struct Answer {
    status: u16,
    headers: String,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("not JSON ({e}): {}", self.body))
    }
}

/// One request on its own connection, with exactly these header lines.
async fn raw(addr: SocketAddr, method: &str, path: &str, headers: &[String], body: &[u8], claimed_length: Option<usize>) -> Answer {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n", claimed_length.unwrap_or(body.len()));
    for h in headers {
        head.push_str(h);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).await.unwrap();
    // The body may be refused before it is read, so a failed write is fine.
    let _ = s.write_all(body).await;
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out).await;
    let text = String::from_utf8_lossy(&out).into_owned();
    let (headers, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = headers.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Answer { status, headers: headers.to_ascii_lowercase(), body: body.to_owned() }
}

fn bearer(token: &str) -> String {
    format!("Authorization: Bearer {token}")
}

async fn post(addr: SocketAddr, message: &Value) -> Answer {
    raw(addr, "POST", PATH, &[bearer(TOKEN), "Content-Type: application/json".into()], message.to_string().as_bytes(), None).await
}

async fn started() -> (SocketAddr, crate::support::Env) {
    let e = env().await;
    let server = HttpServer::bind("127.0.0.1:0".parse().unwrap(), TOKEN).await.unwrap();
    let addr = server.local_addr().unwrap();
    tokio::spawn(server.serve(e.tools_again()));
    (addr, e)
}

#[tokio::test]
async fn an_address_that_is_not_loopback_is_refused_before_a_socket_opens() {
    for addr in ["0.0.0.0:0", "192.168.1.5:0", "127.0.0.2:0", "[::]:0", "[fe80::1]:0"] {
        let err = HttpServer::bind(addr.parse().unwrap(), TOKEN).await.unwrap_err();
        assert!(matches!(err, Error::Refused(_)), "{addr}: {err}");
        assert!(err.to_string().contains("127.0.0.1 or [::1]"), "{err}");
    }
    // The other loopback address binds where the machine has it.
    if let Ok(server) = HttpServer::bind("[::1]:0".parse().unwrap(), TOKEN).await {
        assert!(server.local_addr().unwrap().ip().is_loopback());
    }
}

#[tokio::test]
async fn a_token_that_is_missing_or_short_is_refused_at_the_start() {
    for token in ["", "short"] {
        let err = HttpServer::bind("127.0.0.1:0".parse().unwrap(), token).await.unwrap_err();
        assert!(matches!(err, Error::Refused(_)), "{err}");
    }
}

#[tokio::test]
async fn a_request_without_the_token_gets_401() {
    let (addr, _e) = started().await;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string();
    let wrong = format!("{TOKEN}x");
    let shorter = &TOKEN[..TOKEN.len() - 1];
    let cases: Vec<Vec<String>> = vec![
        vec![],
        vec![bearer("")],
        vec![bearer(&wrong)],
        vec![bearer(shorter)],
        vec![format!("Authorization: Basic {TOKEN}")],
        vec![format!("Authorization: {TOKEN}")],
        vec![format!("Authorization: bearer {TOKEN}")],
    ];
    for headers in cases {
        let a = raw(addr, "POST", PATH, &headers, body.as_bytes(), None).await;
        assert_eq!(a.status, 401, "{headers:?}: {}", a.body);
        assert!(a.headers.contains("www-authenticate: bearer"), "{}", a.headers);
        assert!(!a.body.contains("result") && !a.body.contains(TOKEN), "{}", a.body);
    }
    // The token in the address is no way in either.
    let a = raw(addr, "POST", &format!("{PATH}?token={TOKEN}"), &[], body.as_bytes(), None).await;
    assert_eq!(a.status, 401);
}

#[tokio::test]
async fn a_request_with_an_origin_is_refused_even_with_the_token() {
    let (addr, _e) = started().await;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }).to_string();
    for origin in ["https://example.com", "null", "http://127.0.0.1:3000"] {
        let a = raw(addr, "POST", PATH, &[bearer(TOKEN), format!("Origin: {origin}")], body.as_bytes(), None).await;
        assert_eq!(a.status, 403, "{origin}: {}", a.body);
        assert!(!a.body.contains("tools"), "{}", a.body);
    }
}

#[tokio::test]
async fn only_one_path_and_post_answer_and_the_body_is_capped() {
    let (addr, _e) = started().await;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string();
    for path in ["/", "/mcp", "/mcp/x", "/other/", "/api/svc/desk/browser/mcp/"] {
        let a = raw(addr, "POST", path, &[bearer(TOKEN)], body.as_bytes(), None).await;
        assert_eq!(a.status, 404, "{path}");
    }
    let a = raw(addr, "GET", PATH, &[bearer(TOKEN)], b"", None).await;
    assert_eq!(a.status, 405);
    // A body claimed over the cap is refused before it is read.
    let a = raw(addr, "POST", PATH, &[bearer(TOKEN)], b"{}", Some(MAX_BODY + 1)).await;
    assert_eq!(a.status, 413);
    // So is one that is sent chunked and runs over. The server may answer 413
    // and close while the client is still writing, and on some systems the
    // reset then discards the unread answer. So the answer is read while the
    // body is written, and "413" or "no answer at all" both count as refused.
    // The padded message is a valid ping: if the cap let it through, the
    // answer would be a 200 with a result, which fails the check below.
    let mut padded = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" }).to_string().into_bytes();
    padded.resize(MAX_BODY + 1024, b' ');
    let s = TcpStream::connect(addr).await.unwrap();
    let (mut rd, mut wr) = s.into_split();
    let reader = tokio::spawn(async move {
        let mut out = Vec::new();
        let _ = rd.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).into_owned()
    });
    let head = format!("POST {PATH} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n", padded.len());
    let _ = wr.write_all(head.as_bytes()).await;
    let _ = wr.write_all(&padded).await;
    let _ = wr.write_all(b"\r\n0\r\n\r\n").await;
    let out = tokio::time::timeout(std::time::Duration::from_secs(20), reader).await.expect("the server closed the connection").unwrap();
    assert!(out.is_empty() || out.starts_with("HTTP/1.1 413"), "{}", out.lines().next().unwrap_or(""));
    assert!(!out.contains("result"), "the over-cap body reached the handler: {out}");
    // The server is still healthy on a new connection.
    let a = post(addr, &json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" })).await;
    assert_eq!(a.status, 200, "{}", a.body);
}

#[tokio::test]
async fn text_that_is_not_json_is_a_protocol_error_not_a_crash() {
    let (addr, _e) = started().await;
    let a = raw(addr, "POST", PATH, &[bearer(TOKEN)], b"not json", None).await;
    assert_eq!((a.status, a.json()["error"]["code"].clone()), (400, json!(-32700)));
    let a = raw(addr, "POST", PATH, &[bearer(TOKEN)], b"[{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}]", None).await;
    assert_eq!((a.status, a.json()["error"]["code"].clone()), (400, json!(-32600)));
    let a = post(addr, &json!({ "jsonrpc": "2.0", "id": 7, "method": "nope" })).await;
    assert_eq!((a.status, a.json()["error"]["code"].clone(), a.json()["id"].clone()), (200, json!(-32601), json!(7)));
}

#[tokio::test]
async fn a_session_initialises_lists_the_tools_and_calls_one() {
    let (addr, _e) = started().await;
    let a = post(addr, &json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } } })).await;
    assert_eq!(a.status, 200);
    assert!(a.headers.contains("content-type: application/json"), "{}", a.headers);
    let init = a.json();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["serverInfo"]["name"], "eidolon-browser");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let a = post(addr, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    assert_eq!((a.status, a.body.as_str()), (202, ""));

    let listed = post(addr, &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" })).await.json();
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), definitions().len());
    assert!(tools.iter().any(|t| t["name"] == "list_pages" && t["inputSchema"].is_object()));

    let called = post(addr, &json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "list_pages", "arguments": {} } })).await.json();
    assert_eq!(called["result"]["isError"], false, "{called}");
    assert_eq!(called["result"]["content"][0]["type"], "text");

    // A tool that fails says why and is not a protocol error.
    let failed = post(addr, &json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "click_by_uid", "arguments": { "uid": "nope" } } })).await.json();
    assert_eq!(failed["result"]["isError"], true, "{failed}");
    let unknown = post(addr, &json!({ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": { "name": "run_script", "arguments": {} } })).await.json();
    assert_eq!(unknown["error"]["code"], -32602);
}
