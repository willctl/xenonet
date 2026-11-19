//! The server over an in-memory duplex, spoken to in JSON-RPC lines.

use std::path::PathBuf;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, ReadHalf, WriteHalf};

use crate::support::env;

struct Client {
    read: BufReader<ReadHalf<tokio::io::DuplexStream>>,
    write: WriteHalf<tokio::io::DuplexStream>,
    next: u64,
}

impl Client {
    async fn send(&mut self, message: Value) {
        self.write.write_all(format!("{message}\n").as_bytes()).await.unwrap();
        self.write.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
        loop {
            let mut line = String::new();
            assert!(self.read.read_line(&mut line).await.unwrap() > 0, "the server ended");
            let message: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("not JSON on the protocol stream: {e}: {line}"));
            if message["id"] == json!(id) {
                return message;
            }
        }
    }
}

async fn started() -> (Client, crate::support::Env) {
    let e = env().await;
    // The server gets its own tools on the same fake page and folders.
    let tools = e.tools_again();
    let (client, server) = tokio::io::duplex(1 << 20);
    tokio::spawn(eidolon_mcp::serve_transport(tools, tokio::io::split(server)));
    let (read, write) = tokio::io::split(client);
    let mut c = Client { read: BufReader::new(read), write, next: 0 };
    let init = c.request("initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "0" } })).await;
    assert_eq!(init["result"]["serverInfo"]["name"], "eidolon-browser", "{init}");
    c.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
    (c, e)
}

fn golden() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/tools.json")
}

#[tokio::test]
async fn tools_list_matches_the_golden_file() {
    let (mut c, _e) = started().await;
    let listed = c.request("tools/list", json!({})).await;
    let tools: Vec<Value> =
        listed["result"]["tools"].as_array().unwrap().iter().map(|t| json!({ "name": t["name"], "description": t["description"], "inputSchema": t["inputSchema"] })).collect();
    if std::env::var("EIDOLON_UPDATE_GOLDEN").is_ok() {
        std::fs::write(golden(), serde_json::to_string_pretty(&tools).unwrap() + "\n").unwrap();
    }
    let text = std::fs::read_to_string(golden()).expect("tests/golden/tools.json; run with EIDOLON_UPDATE_GOLDEN=1 to write it");
    // Compared as values, so the order of an object's keys does not matter.
    let expected: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(Value::Array(tools), expected, "the tools changed; run with EIDOLON_UPDATE_GOLDEN=1 if that is on purpose");
}

#[tokio::test]
async fn the_tool_names_are_the_firefox_servers_and_there_is_no_script_tool() {
    let (mut c, _e) = started().await;
    let listed = c.request("tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for needed in [
        "list_pages",
        "select_page",
        "navigate_page",
        "take_snapshot",
        "click_by_uid",
        "fill_by_uid",
        "type_text",
        "press_key",
        "upload_file_by_uid",
        "screenshot_page",
        "get_page_text",
    ] {
        assert!(names.contains(&needed), "{needed} is missing");
    }
    assert!(!names.iter().any(|n| n.contains("evaluate") || n.contains("script")), "{names:?}");
    for t in listed["result"]["tools"].as_array().unwrap() {
        let d = t["description"].as_str().unwrap();
        assert!(d.ends_with('.') && !d.contains('\n') && !d.contains("AI"), "{d}");
    }
}

#[tokio::test]
async fn a_tool_call_runs_and_answers_as_content() {
    let (mut c, e) = started().await;
    let snapshot = c.request("tools/call", json!({ "name": "take_snapshot", "arguments": {} })).await;
    let text = snapshot["result"]["content"][0]["text"].as_str().unwrap().to_owned();
    assert!(text.contains("button \"Go\""), "{text}");
    let uid = text.lines().find(|l| l.contains("\"Go\"")).unwrap().split('[').nth(1).unwrap().split(']').next().unwrap().to_owned();
    let clicked = c.request("tools/call", json!({ "name": "click_by_uid", "arguments": { "uid": uid } })).await;
    assert_eq!(clicked["result"]["content"][0]["text"], format!("Clicked {uid}."));
    assert_eq!(e.page.clicks_on("#go"), 1);

    let shot = c.request("tools/call", json!({ "name": "screenshot_page", "arguments": {} })).await;
    assert_eq!(shot["result"]["content"][0]["type"], "image");
    assert_eq!(shot["result"]["content"][0]["mimeType"], "image/png");
    assert_eq!(shot["result"]["content"][0]["data"], eidolon_testkit::SCREENSHOT_BASE64);
}

#[tokio::test]
async fn a_refused_call_is_a_tool_error_that_says_why() {
    let (mut c, e) = started().await;
    let r = c.request("tools/call", json!({ "name": "navigate_page", "arguments": { "url": "file:///etc/passwd" } })).await;
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("cannot be opened"));
    assert!(e.proxy.methods.lock().unwrap().is_empty());

    let r = c.request("tools/call", json!({ "name": "evaluate_script", "arguments": {} })).await;
    assert!(r["error"].is_object(), "an unknown tool is a protocol error: {r}");
}
