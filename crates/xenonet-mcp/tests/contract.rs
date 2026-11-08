// The contract (docs/design/rust-core.md, "The MCP servers"): the Rust
// servers answer as the JS ones did. Each file in tests/golden/ holds one
// case, recorded from the JS servers (node mcp/xenonet.js, mcp/assistant.js
// and mcp/console.js) on 2026-11-08: the arguments and environment the
// server starts with, the messages said to it (the 2026-07-28 probe, the
// 2025 handshake, tools/list, calls that don't fit a tool's schema, calls
// with no XenoNet to reach, and the same in the 2026-07-28 revision) and
// the JS answers. Those answers are now the contract. Each case starts the
// Rust server the same way, says the same and compares: the same result,
// the same error code, and each tool's schemas written the same, key for
// key. A session, a Codex thread, an actor with its token file, and a
// channel session; the assistant, for a grant and against the listing of
// the JS server's --fixture; and the console operator.
//
// What differs from one machine or run to the next is written in the
// goldens as a placeholder: {token_file} and {runtime_dir} for the paths
// the cases make, {version} for the server's version, and the assistant's
// request id and retry jitter as settled() leaves them.

#![expect(clippy::unwrap_used, clippy::panic, reason = "a test fails where its helpers unwrap")]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

// Says each message to the Rust server, in turn, waiting for the answer to
// each request; the answers.
fn talk(args: &[String], env: &[(String, String)], messages: &[Value]) -> Vec<Value> {
    let program = env!("CARGO_BIN_EXE_xenonet-mcp");
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

// An answer's contents, without what's random in it (an assistant
// request's id, the jitter in when to try again). The goldens were
// recorded through this, so it applies to the Rust answers only.
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

// The server's version isn't part of the contract: the JS servers said
// package.json's, the Rust ones say their crate's.
fn unversioned(mut v: Value) -> Value {
    for p in ["/result/_meta/io.modelcontextprotocol~1serverInfo/version", "/result/serverInfo/version"] {
        if let Some(version) = v.pointer_mut(p) {
            *version = json!("{version}");
        }
    }
    v
}

// Puts each placeholder in place of what it stands for, wherever it's
// written, inside JSON-in-a-string too.
fn placeheld(v: &mut Value, subs: &[(&str, String)]) {
    match v {
        Value::String(s) => {
            for (placeholder, actual) in subs {
                if s.contains(actual.as_str()) {
                    *s = s.replace(actual.as_str(), placeholder);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| placeheld(x, subs)),
        Value::Object(o) => o.values_mut().for_each(|x| placeheld(x, subs)),
        _ => {}
    }
}

// Starts the Rust server as the golden says, with each placeholder in its
// environment filled in from subs, and compares its answers to the JS ones.
fn replay(file: &str, subs: &[(&str, String)]) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{file}.json"));
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap();
    let case = golden["case"].as_str().unwrap();
    let args: Vec<String> = golden["args"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_owned()).collect();
    let env: Vec<(String, String)> = golden["env"]
        .as_array()
        .unwrap()
        .iter()
        .map(|kv| {
            let mut v = kv[1].as_str().unwrap().to_owned();
            for (placeholder, actual) in subs {
                v = v.replace(placeholder, actual);
            }
            (kv[0].as_str().unwrap().to_owned(), v)
        })
        .collect();
    let messages = golden["messages"].as_array().unwrap();
    let js = golden["answers"].as_array().unwrap();
    let rust = talk(&args, &env, messages);
    assert_eq!(js.len(), rust.len(), "{case}: as many answers");
    for ((m, j), r) in messages.iter().filter(|m| m.get("id").is_some()).zip(js).zip(rust) {
        let mut r = unversioned(settled(r));
        placeheld(&mut r, subs);
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

// The sessions' server, and the same in the 2026-07-28 revision, which a
// channel session doesn't speak.
fn sessions_case(file: &str, subs: &[(&str, String)]) {
    replay(file, subs);
    if !file.contains("channel") {
        replay(&format!("{file}-2026-07-28"), subs);
    }
}

#[test]
fn a_claude_code_session() {
    sessions_case("a-session", &[]);
}

#[test]
fn a_codex_thread() {
    sessions_case("a-codex-thread", &[]);
    sessions_case("a-codex-thread-from-its-environment", &[]);
}

#[test]
fn a_channel_session() {
    sessions_case("a-channel-session", &[]);
}

#[test]
fn an_actor() {
    let dir = std::env::temp_dir().join(format!("xenonet-mcp-contract-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("xenonet.token");
    std::fs::write(&file, format!("xenonet_actor_{}\n", "ab".repeat(32))).unwrap();
    let subs = [("{token_file}", file.to_string_lossy().into_owned())];
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    sessions_case("an-actor", &subs);
    // One whose file others can read: refused, and said so.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        sessions_case("an-actor-with-a-loose-token-file", &subs);
    }
    sessions_case("an-actor-with-a-token-that-isn-t-one", &[]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_assistant() {
    let runtime = std::env::temp_dir().join(format!("xenonet-mcp-contract-runtime-{}", std::process::id()));
    std::fs::create_dir_all(&runtime).unwrap();
    let subs = [("{runtime_dir}", runtime.to_string_lossy().into_owned())];
    replay("the-assistant-for-a-grant", &subs);
    // The JS server's --fixture listed the same tools as a grant does.
    replay("the-assistant-against-the-js-fixture", &subs);
    let _ = std::fs::remove_dir_all(&runtime);
}

#[test]
fn the_console_operator() {
    // Calls that don't fit a tool, and calls each tool refuses before it
    // reaches an instance: neither drives a XenoNet.
    replay("the-console-operator", &[]);
    replay("the-console-operator-2026-07-28", &[]);
}
