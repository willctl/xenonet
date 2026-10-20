//! The slices' async Rust, run in the binary: a task (xenonet-app's
//! task.rs) awaits `io.ask(request)` wherever its JS awaited a fetch, a
//! child process or a timer, and here tokio does the asking, as the twins'
//! drive() did it with Node's calls. A sleep is done here for every task;
//! the rest each caller's `Perform` answers.

use serde_json::{json, Map, Value};
use std::time::Duration;
use xenonet_app::task::{Failure, Outcome, Reply, Task};

/// What a task asks for that its caller answers.
pub trait Perform {
    async fn ask(&mut self, q: &Value) -> Reply;
}

/// Why a task ended without its value: its message, and the error's other
/// fields (code…).
#[derive(Debug)]
pub struct Failed {
    pub message: String,
    pub fields: Map<String, Value>,
}

/// An error as a task reads it (task.js's describe): its message, and its
/// code and cause's code when it has them.
pub fn error(message: impl Into<String>) -> Value {
    json!({ "message": message.into() })
}

pub fn coded(message: impl Into<String>, code: &str) -> Value {
    json!({ "message": message.into(), "code": code })
}

/// Runs a task to its end, answering what it asks with `p`.
pub async fn drive(mut task: Task, p: &mut impl Perform) -> Result<Value, Failed> {
    let mut reply = None;
    let mut last: Option<Value> = None;
    loop {
        match task.step(reply.take()) {
            Outcome::Done(v) => return Ok(v),
            Outcome::Failed(Failure::Error { message, fields, .. }) => return Err(Failed { message, fields }),
            Outcome::Failed(Failure::Rethrow) => {
                let e = last.unwrap_or_else(|| error("a request failed"));
                let mut fields = e.as_object().cloned().unwrap_or_default();
                let message = fields.shift_remove("message").map(|m| xenonet_base::text::js_string(&m)).unwrap_or_default();
                return Err(Failed { message, fields });
            }
            Outcome::Ask(q) => {
                let r = if q.get("op").and_then(Value::as_str) == Some("sleep") {
                    let ms = q.get("ms").and_then(Value::as_f64).unwrap_or(0.0);
                    tokio::time::sleep(Duration::from_millis(if ms.is_finite() && ms > 0.0 { ms as u64 } else { 0 })).await;
                    Ok(Value::Null)
                } else {
                    p.ask(&q).await
                };
                if let Err(e) = &r {
                    last = Some(e.clone());
                }
                reply = Some(r);
            }
        }
    }
}

/// Runs `f` on a runtime of its own, for a command that waits on it.
pub fn block_on<F: std::future::Future>(f: F) -> Result<F::Output, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    Ok(rt.block_on(f))
}

/// What a fetch answered: its status, words and body.
pub struct Fetched {
    pub status: u16,
    pub status_text: String,
    pub body: Vec<u8>,
}

impl Fetched {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

fn is_local(url: &url::Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("127.0.0.1" | "localhost"))
}

/// fetch(url, { method, headers, body, signal: timeout }): to a XenoNet on
/// this machine over its own HTTP (never through a proxy, as Node's fetch
/// goes), anything else through the service's HTTPS client. Its error as
/// fetch's: "fetch failed", with the cause's code.
pub async fn fetch(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>, timeout_ms: Option<f64>) -> Result<Fetched, Value> {
    let parsed = url::Url::parse(url).map_err(|_| json!({ "message": "Failed to parse URL", "name": "TypeError" }))?;
    let timeout = timeout_ms.filter(|t| t.is_finite() && *t > 0.0).map(|t| Duration::from_millis(t as u64));
    if is_local(&parsed) {
        let port = parsed.port().unwrap_or(80);
        let path = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_string(),
        };
        let (method, headers) = (method.to_string(), headers.to_vec());
        let answer =
            tokio::task::spawn_blocking(move || xenonet_base::http::request(port, &method, &path, &headers, body.as_deref(), timeout)).await.map_err(|e| error(e.to_string()))?;
        return match answer {
            Ok(a) => {
                let body = a.body.map_err(|e| json!({ "message": "fetch failed", "name": "TypeError", "causeMessage": e.to_string() }))?;
                Ok(Fetched { status: a.status, status_text: a.reason, body })
            }
            Err(e) if timeout.is_some() && matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                Err(json!({ "message": "The operation was aborted due to timeout", "name": "TimeoutError", "code": 23 }))
            }
            Err(e) => {
                let code = match e.kind() {
                    std::io::ErrorKind::ConnectionRefused => "ECONNREFUSED",
                    std::io::ErrorKind::ConnectionReset => "ECONNRESET",
                    _ => "UND_ERR_SOCKET",
                };
                Err(json!({ "message": "fetch failed", "name": "TypeError", "causeCode": code, "causeMessage": e.to_string() }))
            }
        };
    }
    let r = xenonet_service::net::fetch(method, url, headers, body, timeout.unwrap_or(Duration::from_secs(300))).await.map_err(|m| {
        if m.contains("timeout") {
            json!({ "message": m, "name": "TimeoutError" })
        } else {
            json!({ "message": "fetch failed", "name": "TypeError", "causeMessage": m })
        }
    })?;
    // Its words go unread: only a XenoNet's own answers are worded back.
    Ok(Fetched { status: r.status, status_text: String::new(), body: r.body })
}

/// fetchAsked (task.js): a fetch as a task asks for it, its `read` "none",
/// "json" (a body that isn't JSON said so, as bodyError) or "json!" (one
/// that isn't throws).
pub async fn fetch_asked(q: &Value) -> Reply {
    let url = q.get("url").map(xenonet_base::text::js_string).unwrap_or_default();
    let method = q.get("method").and_then(Value::as_str).unwrap_or("GET");
    let headers: Vec<(String, String)> = q.get("headers").and_then(Value::as_object).into_iter().flatten().map(|(k, v)| (k.clone(), xenonet_base::text::js_string(v))).collect();
    let body = q.get("body").filter(|b| !b.is_null()).map(|b| xenonet_base::text::js_string(b).into_bytes());
    let res = fetch(method, &url, &headers, body, q.get("timeoutMs").and_then(Value::as_f64)).await?;
    let mut out = json!({ "ok": res.ok(), "status": res.status, "statusText": res.status_text });
    let parse = || xenonet_base::json::parse(&String::from_utf8_lossy(&res.body));
    match q.get("read").and_then(Value::as_str) {
        Some("json") => match parse() {
            Ok(v) => out["body"] = v,
            Err(_) => out["bodyError"] = json!(true),
        },
        Some("json!") => out["body"] = parse().map_err(|e| json!({ "message": e.to_string(), "name": "SyntaxError" }))?,
        _ => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Twice;
    impl Perform for Twice {
        #[expect(clippy::unused_async_trait_impl, reason = "the trait's ask is async, and this one answers at once")]
        async fn ask(&mut self, q: &Value) -> Reply {
            match q["n"].as_i64() {
                Some(n) if n < 100 => Ok(json!(n * 2)),
                _ => Err(coded("too big", "ERANGE")),
            }
        }
    }

    #[test]
    fn a_task_asks_and_carries_on_with_each_answer() {
        let t = Task::new(|io| async move {
            io.sleep(1.0).await;
            let a = io.ask(json!({ "op": "twice", "n": 21 })).await.map_err(|_| Failure::Rethrow)?;
            io.ask(json!({ "op": "twice", "n": a.as_i64().unwrap_or(0) * 10 })).await.map_err(|_| Failure::Rethrow)
        });
        let out = block_on(async { drive(t, &mut Twice).await }).expect("a runtime");
        let e = out.expect_err("the second is too big");
        assert_eq!((e.message.as_str(), e.fields.get("code")), ("too big", Some(&json!("ERANGE"))));
    }

    #[test]
    fn a_fetch_to_nothing_is_refused_as_fetch_says_it() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).expect("a port");
        // Windows retries a refused connection for about two seconds before
        // it says so, so the timeout leaves it time; elsewhere it's at once.
        let r = block_on(fetch_asked(&json!({ "url": format!("http://127.0.0.1:{port}/api/health"), "timeoutMs": 10000 }))).expect("a runtime");
        let e = r.expect_err("nothing listens");
        assert_eq!((e["message"].clone(), e["causeCode"].clone()), (json!("fetch failed"), json!("ECONNREFUSED")));
    }
}
