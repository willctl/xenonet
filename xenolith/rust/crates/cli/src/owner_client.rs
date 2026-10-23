//! `xenolith` acting for the owner through the running service: it reads the owner key the service
//! wrote (xenolith_core::owner), checks that the service answering on the key's port is the one
//! that wrote it, and makes the call in the window's API with `X-Xenolith: cli`. The service then
//! decides, and records the decision in the audit chain as made by "cli".

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
use xenolith_core::owner::{self, OwnerKey};
use xenolith_core::paths::AppPaths;
use xenolith_core::wiki;

/// The running service's owner key, once that service has shown it is the one that wrote it.
fn key() -> Result<OwnerKey, String> {
    let paths = AppPaths::current();
    let file = owner::key_file(&paths);
    let key = owner::read(&paths).ok_or_else(|| format!("No running Xenolith service has left an owner key you can read ({}). Start the service, then try again.", wiki::disp(&file)))?;
    let (code, status) = request(key.port, "GET", "/status", None, None).map_err(|e| format!("The Xenolith service on port {} is not answering ({e}). Start it, then try again.", key.port))?;
    if code != 200 || status["pid"].as_u64() != Some(u64::from(key.pid)) {
        return Err(format!("The owner key in {} is from a service that is no longer running (pid {}). Start the service again, then try again.", wiki::disp(&file), key.pid));
    }
    Ok(key)
}

/// POST `body` to the window's API at `path` as the owner: (status, JSON answer).
pub fn post(path: &str, body: &Value) -> Result<(u16, Value), String> {
    let key = key()?;
    request(key.port, "POST", path, Some(&key.key), Some(body))
}

/// One HTTP/1.1 request to 127.0.0.1:`port`, with the owner key when given.
fn request(port: u16, method: &str, path: &str, key: Option<&str>, body: Option<&Value>) -> Result<(u16, Value), String> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let t = Duration::from_secs(30);
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(4)).map_err(|e| e.to_string())?;
    let _ = s.set_read_timeout(Some(t));
    let _ = s.set_write_timeout(Some(t));
    let data = body.map(Value::to_string).unwrap_or_default();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: application/json\r\nConnection: close\r\nX-Xenolith: cli\r\n");
    if let Some(k) = key {
        head.push_str(&format!("{}: {k}\r\n", owner::HEADER));
    }
    if body.is_some() {
        head.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", data.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes()).and_then(|_| s.write_all(data.as_bytes())).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    let code = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("malformed status line")?;
    Ok((code, serde_json::from_str(body.trim()).unwrap_or_else(|_| json!({ "error": body.trim() }))))
}
