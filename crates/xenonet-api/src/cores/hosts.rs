//! This machine's own API, read as any local program reads it: what the
//! window reads of another machine (docs/design/network.md, section 7,
//! over svc/2) reads on the machine it's for.

use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// This machine's own API, read as any local program reads it: a GET with
/// no key, so its routes' own checks apply. A window read from another
/// machine reads through it.
#[async_trait]
pub trait SelfApi: Send + Sync {
    /// The status and JSON of `path`, or why there was none and the status
    /// to answer with.
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)>;
}

/// The API over HTTP on 127.0.0.1, as a window read reads it. It sends
/// no window key and no token. It carries the API's credential, as any
/// program here does, and with `own`, the service's own key: the network
/// decided what the other machine may read, so the read of this service
/// is the service's own.
pub struct HttpApi {
    pub port: u16,
    /// A read's longest wait, and its biggest answer.
    pub timeout: Duration,
    pub max_bytes: usize,
    /// The service's own key (x-xenonet-self), when the service reads
    /// itself.
    pub own: Option<String>,
}

impl HttpApi {
    /// The headers a read sends: nothing but what it accepts.
    pub const HEADERS: [(&'static str, &'static str); 1] = [("accept", "application/json")];
}

#[async_trait]
impl SelfApi for HttpApi {
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)> {
        let (port, route, timeout, max) = (self.port, path.to_string(), self.timeout, self.max_bytes);
        let mut headers: Vec<(String, String)> = Self::HEADERS.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        if let Some(own) = &self.own {
            headers.push(("x-xenonet-self".into(), own.clone()));
        }
        let r = tokio::task::spawn_blocking(move || crate::http::request(port, "GET", &route, &headers, None, Some(timeout))).await.map_err(|e| (e.to_string(), 502))?;
        let a = r.map_err(|e| match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => (format!("no answer in {} s", timeout.as_secs()), 504),
            _ => (e.to_string(), 502),
        })?;
        let body = a.body.map_err(|e| (e.to_string(), 502))?;
        if body.len() > max {
            return Err(("too big to send to another machine".into(), 413));
        }
        let text = String::from_utf8_lossy(&body);
        if text.is_empty() {
            return Ok((a.status, Value::Null));
        }
        Ok(match xenonet_text::json::parse(&text) {
            Ok(v) => (a.status, v),
            Err(_) => (502, serde_json::json!({ "error": "XenoNet answered with something that isn't JSON" })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // A read of this machine's own API carries no window key and no
    // token, and the service's own key only when it's given one.
    #[tokio::test]
    async fn own_reads_carry_no_key() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let body = "{\"nodes\":[]}";
            write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let api = HttpApi { port, timeout: Duration::from_secs(5), max_bytes: 1 << 20, own: None };
        assert_eq!(api.get("/api/graph?hosts=0").await.unwrap(), (200, serde_json::json!({ "nodes": [] })));
        let request = seen.join().unwrap();
        assert!(request.starts_with("get /api/graph?hosts=0 http/1.1"), "{request}");
        assert!(
            !request.contains("x-xenonet-window") && !request.contains("authorization") && !request.contains("x-xenonet-agent") && !request.contains("x-xenonet-self"),
            "{request}"
        );
    }
}
