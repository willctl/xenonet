//! A probe of a real agent's ACP mode, through this crate as XenoNet runs
//! it (revision 25 of phase 6: a fake agent built from the same SDK can't
//! catch a protocol mismatch). It starts the agent with no window and a
//! clean environment (PATH, HOME, USER, LANG, TMPDIR and SHELL), opens a
//! session in a fresh folder, sends one framed turn asking for a one-word
//! answer, denies any permission it asks for, and says what happened.
//!
//!   cargo run -p xenonet-acp --example probe -- <program> [args…]
//!   cargo run -p xenonet-acp --example probe -- grok agent --no-leader stdio
//!
//! It spends one short turn of the agent's plan. A sign-in the agent asks
//! for is reported, never answered.
#![expect(clippy::unwrap_used, clippy::print_stdout, clippy::print_stderr, reason = "a developer's probe, run by hand: it prints what it saw")]

use futures::future::BoxFuture;
use futures::FutureExt as _;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenonet_acp::schema::{ContentBlock, SessionId, SessionUpdate};
use xenonet_acp::{Answer, Client, Host, Launch, Prompt, Turn};
use xenonet_authz::Information;

#[derive(Default)]
struct Probe {
    said: Mutex<String>,
    updates: Mutex<Vec<String>>,
}

impl Host for Probe {
    fn ask(&self, prompt: Prompt) -> BoxFuture<'static, Answer> {
        println!("asked: {} ({:?}), denied", prompt.title.read(), prompt.kind);
        async { Answer::Deny }.boxed()
    }

    fn update(&self, _session: &SessionId, update: SessionUpdate) {
        let kind = serde_json::to_value(&update).ok().and_then(|v| v.get("sessionUpdate").and_then(|k| k.as_str()).map(str::to_string)).unwrap_or_default();
        if let SessionUpdate::AgentMessageChunk(c) = &update {
            if let ContentBlock::Text(t) = &c.content {
                self.said.lock().unwrap().push_str(&t.text);
            }
        }
        let mut u = self.updates.lock().unwrap();
        if u.last() != Some(&kind) {
            u.push(kind);
        }
    }

    fn refused(&self, method: &'static str) {
        println!("refused: {method}");
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((program, rest)) = args.split_first() else {
        eprintln!("usage: probe <program> [args…]");
        std::process::exit(2);
    };
    let keep = ["PATH", "HOME", "USER", "LANG", "TMPDIR", "SHELL"];
    let base: Vec<(String, String)> = std::env::vars().filter(|(k, _)| keep.contains(&k.as_str())).collect();
    let cwd = std::env::temp_dir().join(format!("xenonet-acp-probe-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let launch = Launch::new(program, rest.to_vec(), &cwd, base, &[]);
    let host = Arc::new(Probe::default());
    let t0 = Instant::now();
    let client = match tokio::time::timeout(Duration::from_secs(60), Client::start(&launch, host.clone())).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            println!("start: {e}");
            std::process::exit(1);
        }
        Err(_) => {
            println!("start: no answer to initialize in 60 s");
            std::process::exit(1);
        }
    };
    let info = client.info();
    println!(
        "initialized in {} ms: agent {:?}, auth methods {}, load session {}",
        t0.elapsed().as_millis(),
        info.agent_info.as_ref().map(|i| format!("{} {}", i.name, i.version)),
        info.auth_methods.len(),
        info.agent_capabilities.load_session
    );
    let session = match client.open(&cwd, None).await {
        Ok(s) => s,
        Err(e) => {
            println!("session/new: {e}");
            client.close().await;
            std::process::exit(1);
        }
    };
    println!("session {} in {} ms", session.id().0, t0.elapsed().as_millis());
    let by = json!({ "kind": "you", "id": "you", "title": "the probe" });
    let turn = Turn::started(&by, None, &Information::new("Reply with the single word ok, and nothing else. Use no tools.".to_string()), None);
    let ended = tokio::time::timeout(Duration::from_secs(120), session.prompt(turn)).await;
    match ended {
        Ok(Ok(stop)) => println!("turn ended {stop:?} in {} ms", t0.elapsed().as_millis()),
        Ok(Err(e)) => println!("session/prompt: {e}"),
        Err(_) => {
            println!("session/prompt: no end in 120 s, cancelled");
            let _ = session.cancel();
        }
    }
    println!("updates: {}", host.updates.lock().unwrap().join(", "));
    println!("said: {:?}", host.said.lock().unwrap().trim());
    client.close().await;
    let _ = std::fs::remove_dir_all(&cwd);
}
