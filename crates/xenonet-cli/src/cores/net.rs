//! What the command's async work runs on: a runtime of its own for a
//! command that waits on it, and fetch, as the components and the console
//! ask for it.

use serde_json::{json, Value};
use std::time::Duration;
use xenonet_os::error::Failure;

fn failure(v: Value) -> Failure {
    Failure::from_value(&v)
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

/// A fetch: to a XenoNet on this machine over its own HTTP (never through
/// a proxy), anything else through the service's HTTPS client. It fails
/// as "fetch failed", with its cause's code (`causeCode`) and message
/// (`causeMessage`), or as a TimeoutError.
pub async fn fetch(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>, timeout_ms: Option<f64>) -> Result<Fetched, Failure> {
    fetched(method, url, headers, body, timeout_ms).await.map_err(failure)
}

async fn fetched(method: &str, url: &str, headers: &[(String, String)], body: Option<Vec<u8>>, timeout_ms: Option<f64>) -> Result<Fetched, Value> {
    let parsed = url::Url::parse(url).map_err(|_| json!({ "message": "Failed to parse URL", "name": "TypeError" }))?;
    let timeout = timeout_ms.filter(|t| t.is_finite() && *t > 0.0).map(|t| Duration::from_millis(t as u64));
    if is_local(&parsed) {
        let port = parsed.port().unwrap_or(80);
        let path = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_string(),
        };
        let (method, headers) = (method.to_string(), headers.to_vec());
        let answer = tokio::task::spawn_blocking(move || xenonet_api::http::request(port, &method, &path, &headers, body.as_deref(), timeout))
            .await
            .map_err(|e| json!({ "message": e.to_string() }))?;
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
    let r = xenonet_os::net::fetch(method, url, headers, body, timeout.unwrap_or(Duration::from_secs(300))).await.map_err(|m| {
        if m.contains("timeout") {
            json!({ "message": m, "name": "TimeoutError" })
        } else {
            json!({ "message": "fetch failed", "name": "TypeError", "causeMessage": m })
        }
    })?;
    // Its words go unread: only a XenoNet's own answers are worded back.
    Ok(Fetched { status: r.status, status_text: String::new(), body: r.body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fetch_to_nothing_is_refused_with_its_cause() {
        let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).expect("a port");
        // Windows retries a refused connection for about two seconds before
        // it says so, so the timeout leaves it time; elsewhere it's at once.
        let r = block_on(fetch("GET", &format!("http://127.0.0.1:{port}/api/health"), &[], None, Some(10000.0))).expect("a runtime");
        let e = r.err().expect("nothing listens");
        assert_eq!((e.message.as_str(), e.field("causeCode")), ("fetch failed", Some(&json!("ECONNREFUSED"))));
    }
}
