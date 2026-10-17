//! `serve` over stdio: one JSON-RPC message per line on stdin, responses on stdout (and nothing else
//! there: logging goes to stderr). Requests run concurrently on a few worker threads, as the Node
//! server's async handlers do, so a slow call never holds up a quick one. Ends when stdin closes.
//!
//! The app that starts it is this machine's, so it works as you do. With XENOLITH_TOKEN set (an
//! actor started with its own token), it works as that token's holder, checked again on each call.
//!
//! A connection lasts, so it can subscribe to pages (resources.rs): once the client has
//! initialized, a watcher looks once a second and sends what changed.

use crate::mcp::{self, Ctx, log};
use xenolith_core::access::Principal;
use xenolith_core::reqlog::{RequestLog, clip_str};
use xenolith_core::wiki;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, mpsc};

const WORKERS: usize = 8;

/// Who this connection works as: you, or the holder of the token in XENOLITH_TOKEN. A token
/// Xenolith doesn't know ends the server at once, so the app says it failed to start.
fn principal() -> Principal {
    let Some(token) = std::env::var("XENOLITH_TOKEN").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) else {
        return Principal::Local;
    };
    match xenolith_core::access::load(&xenolith_core::paths::AppPaths::current()).principal_for(&token, xenolith_core::text::now_ms()) {
        Some(p) => p,
        None => {
            log("XENOLITH_TOKEN is not a token Xenolith knows: it is unknown, expired or revoked");
            std::process::exit(1)
        }
    }
}

pub fn run(read_only: bool) -> ! {
    let principal = principal();
    let st = Arc::new(if read_only { mcp::setup_read_only() } else { mcp::setup() });
    let proc_name = if read_only { std::env::var("XENOLITH_PROCESS").unwrap_or_else(|_| "read-only".into()) } else { "stdio".into() };
    let reqlog: Arc<RequestLog> = Arc::new(mcp::open_request_log(&proc_name, &st.config));
    let client_info: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let stdout = Arc::new(Mutex::new(std::io::stdout()));
    let mode = if read_only { "read-only".to_string() } else { format!("{} writes", if st.curated { "curated" } else { "direct" }) };
    let shown = st.wiki_dir.as_ref().map(|d| wiki::disp(d)).unwrap_or_else(|| "(unavailable)".into());
    log(&format!("v{} ready (stdio, {mode}); wiki at {shown}", xenolith_core::VERSION));

    let subs = Arc::new(crate::resources::Subscriptions::default());
    let (tx, rx) = mpsc::channel::<Value>();
    let rx = Arc::new(Mutex::new(rx));
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (rx, st, reqlog, client_info, stdout, principal, subs) = (rx.clone(), st.clone(), reqlog.clone(), client_info.clone(), stdout.clone(), principal.clone(), subs.clone());
            std::thread::spawn(move || {
                loop {
                    let Ok(m) = rx.lock().unwrap().recv() else { break };
                    let client = client_info.lock().unwrap().clone().unwrap_or_default();
                    let ctx = Ctx::new("stdio", None, client, &reqlog, None, principal.clone());
                    let resp = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match m.get("method").and_then(Value::as_str) {
                        // Subscriptions belong to this connection, so they are answered here.
                        Some(method @ ("resources/subscribe" | "resources/unsubscribe")) if !st.read_only => {
                            let id = m.get("id").cloned()?;
                            let params = m.get("params").cloned().unwrap_or(Value::Null);
                            let r = if method == "resources/subscribe" { subs.subscribe(&st, &ctx, &params) } else { subs.unsubscribe(&params) };
                            Some(match r {
                                Ok(v) => json!({ "result": v, "jsonrpc": "2.0", "id": id }),
                                Err((code, msg)) => mcp::rpc_error(id, code, &msg),
                            })
                        }
                        _ => mcp::handle(&st, &ctx, &m, &client_info),
                    }))
                    .unwrap_or_else(|_| m.get("id").cloned().map(|id| mcp::rpc_error(id, -32603, "Internal error")));
                    if let Some(resp) = resp {
                        let mut out = stdout.lock().unwrap();
                        let _ = writeln!(out, "{resp}");
                        let _ = out.flush();
                    }
                }
            })
        })
        .collect();

    let mut watching = false;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            log("ignored a line that is not JSON");
            continue;
        };
        let messages = match msg {
            Value::Array(a) => a,
            other => vec![other],
        };
        for m in messages {
            match m.get("method").and_then(Value::as_str) {
                Some("notifications/initialized") => {
                    let client = client_info.lock().unwrap().clone().unwrap_or_default();
                    reqlog.write(vec![("kind", json!("session")), ("transport", json!("stdio")), ("client", json!(clip_str(&client, 80))), ("result", json!("initialized"))]);
                    if !read_only && !watching {
                        watching = true;
                        let (st, subs, stdout, principal) = (st.clone(), subs.clone(), stdout.clone(), principal.clone());
                        std::thread::spawn(move || {
                            loop {
                                for n in subs.poll(&st, &principal) {
                                    let mut out = stdout.lock().unwrap();
                                    let _ = writeln!(out, "{n}");
                                    let _ = out.flush();
                                }
                                std::thread::sleep(std::time::Duration::from_secs(1));
                            }
                        });
                    }
                }
                // initialize sets the client name the later calls are logged with: answer it in order.
                Some("initialize") => {
                    let ctx = Ctx::new("stdio", None, String::new(), &reqlog, None, principal.clone());
                    if let Some(resp) = mcp::handle(&st, &ctx, &m, &client_info) {
                        let mut out = stdout.lock().unwrap();
                        let _ = writeln!(out, "{resp}");
                        let _ = out.flush();
                    }
                }
                _ => {
                    let _ = tx.send(m);
                }
            }
        }
    }
    drop(tx);
    for w in workers {
        let _ = w.join();
    }
    let _ = stdout.lock().unwrap().flush();
    std::process::exit(0)
}
