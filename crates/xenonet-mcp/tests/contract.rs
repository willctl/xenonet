// The contract (docs/design/rust-core.md, "The MCP servers"): the Rust
// servers answer as the JS ones do. Each case starts the JS server
// (node mcp/xenonet.js, node mcp/assistant.js) and the Rust one with the
// same environment, says the same to both (the 2026-07-28 probe, the 2025
// handshake, tools/list, calls that don't fit a tool's schema, calls with
// no XenoNet to reach, and the same in the 2026-07-28 revision) and
// compares the answers: the same JSON, and each tool's schemas written the
// same, key for key. A session, a Codex thread, an actor with its token
// file, and a channel session; and the assistant, for a grant and against
// the JS server's --fixture.
//
// It needs node and the checkout's node_modules; without them it says so
// and passes.

#![expect(clippy::unwrap_used, clippy::panic, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const SESSION: &str = "11111111-2222-3333-4444-555555555555";
const THREAD: &str = "019a0000-aaaa-bbbb-cccc-000000000001";
const GRANT: &str = "gr_000000000000000000000000";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

// Whether the JS servers can run here.
fn js_runs() -> bool {
    let ok = Command::new("node").arg("--version").output().is_ok_and(|o| o.status.success()) && root().join("node_modules/@modelcontextprotocol/server").exists();
    if !ok {
        eprintln!("contract: no node or node_modules here, so the JS servers can't run; skipped");
    }
    ok
}

fn meta26() -> Value {
    json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientCapabilities": {}, "io.modelcontextprotocol/clientInfo": { "name": "claude-code", "version": "2.1.302" } })
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    request(id, "tools/call", json!({ "name": name, "arguments": arguments }))
}

// The probe, the handshake and the list: what Claude Code says first.
fn opening() -> Vec<Value> {
    vec![
        request(1, "server/discover", json!({ "_meta": meta26() })),
        request(2, "initialize", json!({ "protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": { "name": "claude-code", "version": "2.1.302" } })),
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        request(3, "tools/list", json!({})),
    ]
}

// Says each message to a server, in turn, waiting for the answer to each
// request; the answers.
fn talk(program: &str, args: &[&str], env: &[(&str, String)], messages: &[Value]) -> Vec<Value> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(root()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    for v in [
        "CLAUDE_CODE_SESSION_ID",
        "CLAUDE_PID",
        "CLAUDE_PROJECT_DIR",
        "CODEX_THREAD_ID",
        "XENONET_ACTOR_TOKEN",
        "XENONET_ACTOR_TOKEN_FILE",
        "XENONET_CHANNEL",
        "XENONET_PORT",
        "XENONET_RUNTIME_DIR",
    ] {
        cmd.env_remove(v);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = cmd.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                let _ = tx.send(v);
            }
        }
    });
    let mut answers = vec![];
    for m in messages {
        writeln!(stdin, "{m}").unwrap();
        stdin.flush().unwrap();
        let Some(id) = m.get("id") else { continue };
        loop {
            let a: Value = rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("{program} didn't answer {m}"));
            if a.get("id") == Some(id) {
                answers.push(a);
                break;
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    answers
}

// The answers' contents, without what's random in them (an assistant
// request's id, the jitter in when to try again).
fn settled(mut v: Value) -> Value {
    if let Some(text) = v.pointer("/result/content/0/text").and_then(Value::as_str) {
        if let Ok(mut inner) = serde_json::from_str::<Value>(text) {
            if inner.get("schema").and_then(Value::as_str) == Some("xenonet.assistant.error.v1") {
                inner["request_id"] = json!("r_…");
                inner["retry_after_ms"] = json!(inner["retry_after_ms"].is_number());
                v["result"]["content"][0]["text"] = json!(inner.to_string());
            }
        }
    }
    v
}

// The same answers, and the tools' schemas written alike.
fn same(case: &str, messages: &[Value], js: Vec<Value>, rust: Vec<Value>) {
    assert_eq!(js.len(), rust.len(), "{case}: as many answers");
    for ((m, j), r) in messages.iter().filter(|m| m.get("id").is_some()).zip(js).zip(rust) {
        let (j, r) = (settled(j), settled(r));
        assert_eq!(j.get("result"), r.get("result"), "{case}: the answer to {m}");
        assert_eq!(j.get("error").map(|e| &e["code"]), r.get("error").map(|e| &e["code"]), "{case}: the error for {m}");
        if let (Some(jt), Some(rt)) = (j.pointer("/result/tools").and_then(Value::as_array), r.pointer("/result/tools").and_then(Value::as_array)) {
            for (a, b) in jt.iter().zip(rt) {
                for k in ["inputSchema", "outputSchema"] {
                    assert_eq!(a.get(k).map(Value::to_string), b.get(k).map(Value::to_string), "{case}: {}'s {k}, key for key", a["name"]);
                }
            }
        }
    }
}

fn compare(case: &str, js: (&str, &[&str]), rust: &[&str], env: &[(&str, String)], messages: &[Value]) {
    let j = talk("node", &[&[js.0], js.1].concat(), env, messages);
    let r = talk(env!("CARGO_BIN_EXE_xenonet-mcp"), rust, env, messages);
    same(case, messages, j, r);
}

// Calls that don't fit the sessions' tools, as zod and the SDK refuse them.
fn misfits(from: u64) -> Vec<Value> {
    [
        ("send", json!({ "text": 5 })),
        ("send", json!({})),
        ("claim", json!({ "paths": [], "minutes": 1.5 })),
        ("claim", json!({ "paths": ["a", 3], "minutes": 0 })),
        ("inbox", json!({ "wait_seconds": 121 })),
        ("inbox", json!({ "wait_seconds": "5" })),
        ("release", json!({ "paths": "src/" })),
        ("who_has", json!({ "path": null })),
        ("peers", json!({ "scope": ["repo"] })),
        ("focus", json!({ "text": "x", "extra": true })),
        ("handoff", json!({ "files": "src/" })),
        ("handoff", json!({ "accept": 5 })),
        ("nope", json!({})),
    ]
    .into_iter()
    .enumerate()
    .map(|(i, (name, args))| call(from + i as u64, name, args))
    .collect()
}

// The sessions' server with no XenoNet to reach (port 1), as each kind of
// caller.
fn sessions_case(case: &str, env: &[(&str, String)], extra: Vec<Value>) {
    let mut messages = opening();
    messages.extend(misfits(10));
    messages.extend([call(30, "whoami", json!({})), call(31, "peers", json!({ "scope": "machine" })), call(32, "ask_gateway_token", json!({ "actor": "actor:muse" }))]);
    messages.push(request(33, "tools/call", json!({ "name": "whoami", "arguments": {}, "_meta": { "threadId": THREAD } })));
    messages.extend([call(34, "handoff", json!({ "goal": "Finish the refunds", "files": ["src/refunds.ts"] })), call(35, "handoff", json!({ "accept": "hof_x" }))]);
    // A start, and a permission prompt, with no XenoNet to answer them.
    messages.extend([call(36, "start_session", json!({ "machine": "desk", "prompt": "Run the fixtures" })), call(37, "permission", json!({ "tool_name": "Bash", "input": { "command": "ls", "n": [1] } })), call(38, "permission", json!({ "tool_name": "Bash", "input": "ls" }))]);
    messages.extend(extra);
    let mut env = env.to_vec();
    env.push(("XENONET_PORT", "1".into()));
    compare(case, ("mcp/xenonet.js", &[]), &[], &env, &messages);
    // The 2026-07-28 revision, with no handshake: each request says it.
    let m26 = |id: u64, method: &str, mut params: Value| {
        params["_meta"] = meta26();
        request(id, method, params)
    };
    let mut thread = meta26();
    thread["threadId"] = json!(THREAD);
    let era = vec![
        m26(1, "server/discover", json!({})),
        m26(2, "tools/list", json!({})),
        m26(3, "tools/call", json!({ "name": "whoami", "arguments": {} })),
        m26(4, "tools/call", json!({ "name": "claim", "arguments": { "paths": [] } })),
        request(5, "tools/call", json!({ "name": "peers", "arguments": {}, "_meta": thread })),
    ];
    if !case.contains("channel") {
        compare(&format!("{case}, 2026-07-28"), ("mcp/xenonet.js", &[]), &[], &env, &era);
    }
}

#[test]
fn a_claude_code_session() {
    if !js_runs() {
        return;
    }
    sessions_case(
        "a session",
        &[("CLAUDE_CODE_SESSION_ID", SESSION.into()), ("XENONET_CHANNEL", "0".into()), ("CLAUDE_PROJECT_DIR", "/w/atlas".into()), ("CLAUDE_PID", "4242".into())],
        vec![],
    );
}

#[test]
fn a_codex_thread() {
    if !js_runs() {
        return;
    }
    sessions_case("a Codex thread", &[], vec![]);
    sessions_case("a Codex thread from its environment", &[("CODEX_THREAD_ID", THREAD.into())], vec![]);
}

#[test]
fn a_channel_session() {
    if !js_runs() {
        return;
    }
    sessions_case("a channel session", &[("CLAUDE_CODE_SESSION_ID", SESSION.into()), ("XENONET_CHANNEL", "1".into())], vec![]);
}

#[test]
fn an_actor() {
    if !js_runs() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("xenonet-mcp-contract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("xenonet.token");
    std::fs::write(&file, format!("xenonet_actor_{}\n", "ab".repeat(32))).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let reports = vec![
        call(40, "actor_heartbeat", json!({ "presence": "away" })),
        call(41, "actor_events", json!({ "events": [{ "kind": "said" }] })),
        call(42, "actor_events", json!({ "events": [] })),
        call(43, "actor_targets", json!({ "targets": [{ "relation": "owns" }], "replace": "yes" })),
        call(44, "actor_heartbeat", json!({ "presence": "working", "status": "Triaging" })),
        request(45, "tools/call", json!({ "name": "actor_heartbeat", "arguments": {}, "_meta": { "threadId": THREAD } })),
    ];
    sessions_case("an actor", &[("XENONET_ACTOR_TOKEN_FILE", file.to_string_lossy().into_owned())], reports.clone());
    // One whose file others can read: refused, and said so.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        sessions_case("an actor with a loose token file", &[("XENONET_ACTOR_TOKEN_FILE", file.to_string_lossy().into_owned())], reports);
    }
    sessions_case("an actor with a token that isn't one", &[("XENONET_ACTOR_TOKEN", "xenonet_actor_nope".into())], vec![]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_assistant() {
    if !js_runs() {
        return;
    }
    let runtime = std::env::temp_dir().join(format!("xenonet-mcp-contract-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).unwrap();
    let env = [("XENONET_RUNTIME_DIR", runtime.to_string_lossy().into_owned()), ("XENONET_PORT", "1".into())];
    let mut messages = opening();
    let session = "claude:0190fa00-0000-7000-8000-000000000001";
    messages.extend(
        [
            ("xenonet_sessions", json!({ "path": "/etc" })),
            ("xenonet_sessions", json!({ "statuses": [], "limit": 51, "updated_since": "yesterday", "cursor": "x.y" })),
            ("xenonet_sessions", json!({ "statuses": ["idle", "idle", "idle", "idle", "idle", "idle", "idle"], "project_id": "prj_nope" })),
            ("xenonet_attention", json!({ "kinds": ["waiting", "said"], "limit": 0 })),
            ("xenonet_session", json!({})),
            ("xenonet_session", json!({ "session_id": "claude:x", "include_stats": "yes" })),
            ("xenonet_actors", json!({ "project_id": 5, "cursor": null })),
            ("xenonet_status", json!({ "x": 1, "y": 2 })),
            ("terminal", json!({ "action": "write", "text": "rm -rf /" })),
            ("xenonet_status", json!({})),
            ("xenonet_session", json!({ "session_id": session, "include_stats": true })),
            ("xenonet_sessions", json!({ "statuses": ["idle"], "updated_since": "2026-10-07T00:00:00.000Z", "limit": 50 })),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (name, args))| call(10 + i as u64, name, args)),
    );
    compare("the assistant, for a grant", ("mcp/assistant.js", &["--grant", GRANT]), &["assistant", "--grant", GRANT], &env, &messages);
    // The JS server's --fixture lists the same tools.
    let listing = opening();
    let fixture = talk("node", &["mcp/assistant.js", "--fixture"], &env, &listing);
    let rust = talk(env!("CARGO_BIN_EXE_xenonet-mcp"), &["assistant", "--grant", GRANT], &env, &listing);
    same("the assistant, against the JS --fixture", &listing, fixture, rust);
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn the_console_operator() {
    if !js_runs() {
        return;
    }
    let mut messages = opening();
    // Calls that don't fit a tool, and calls each tool refuses before it
    // reaches an instance: neither drives a XenoNet.
    messages.extend(
        [
            ("select", json!({ "instance": "nope" })),
            ("navigate", json!({})),
            ("find", json!({ "limit": 0 })),
            ("find", json!({ "kind": "planet" })),
            ("rename", json!({ "title": "" })),
            ("input", json!({ "move": { "x": 1 } })),
            ("input", json!({ "keys": 5 })),
            ("terminal", json!({ "lines": 1001 })),
            ("feed", json!({ "limit": 1.5 })),
            ("root", json!({ "path": 5 })),
            ("logs", json!({ "since": "x" })),
            ("simulate", json!({ "kind": "nope" })),
            ("nope", json!({})),
            ("stats", json!({})),
            ("rename", json!({ "title": "x" })),
            ("theme", json!({ "action": "set" })),
            ("mcp_info", json!({})),
            ("environment", json!({ "action": "preview" })),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (name, args))| call(10 + i as u64, name, args)),
    );
    compare("the console operator", ("mcp/console.js", &[]), &["console"], &[], &messages);
    let m26 = |id: u64, method: &str, mut params: Value| {
        params["_meta"] = meta26();
        request(id, method, params)
    };
    let era = vec![m26(1, "server/discover", json!({})), m26(2, "tools/list", json!({})), m26(3, "tools/call", json!({ "name": "theme", "arguments": { "action": "focus" } }))];
    compare("the console operator, 2026-07-28", ("mcp/console.js", &[]), &["console"], &[], &era);
}
