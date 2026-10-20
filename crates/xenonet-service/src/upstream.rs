//! XenoNet Dev reading another XenoNet (server/upstream.js): with
//! XENONET_UPSTREAM set (a URL, such as a forwarded port to the XenoNet on
//! the computer the agents run on), the dev service passes the page's reads
//! to that XenoNet, so a page in development shows real sessions. Only reads
//! go through: anything that would change something there is refused. The
//! dev service's own routes (its health, the window's control and the
//! page's error reports) stay here.

use crate::http::{Reply, ReplyBody};
use bytes::Bytes;
use serde_json::json;
use std::fmt::Write as _;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

// What the page's request says about itself, which the other XenoNet has no
// use for and might take as a page of its own.
const DROP: [&str; 7] = ["host", "origin", "referer", "cookie", "x-xenonet-window", "authorization", "connection"];

pub struct Upstream {
    /// The host to connect to, without brackets.
    pub hostname: String,
    pub port: u16,
    /// url.host: the name and port, as the Host header names it.
    pub host: String,
}

/// The upstream XENONET_UPSTREAM names, None when it's not set, or why it
/// can't be one.
pub fn upstream_of(value: Option<&str>) -> Result<Option<Upstream>, String> {
    let Some(value) = value.filter(|v| !v.is_empty()) else { return Ok(None) };
    let (scheme, rest) = value.split_once("://").ok_or_else(|| format!("XENONET_UPSTREAM isn't a URL: {value}"))?;
    if !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) || scheme.is_empty() {
        return Err(format!("XENONET_UPSTREAM isn't a URL: {value}"));
    }
    if !scheme.eq_ignore_ascii_case("http") {
        return Err("XENONET_UPSTREAM must be an http:// address, such as a forwarded port".into());
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("").to_lowercase();
    let (hostname, port) = if let Some(after) = authority.strip_prefix('[') {
        let (h, p) = after.split_once(']').ok_or_else(|| format!("XENONET_UPSTREAM isn't a URL: {value}"))?;
        (format!("[{h}]"), p.strip_prefix(':').unwrap_or(""))
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_string(), p),
            None => (authority.clone(), ""),
        }
    };
    if hostname.is_empty() {
        return Err(format!("XENONET_UPSTREAM isn't a URL: {value}"));
    }
    if !["127.0.0.1", "localhost", "[::1]"].contains(&hostname.as_str()) {
        return Err("XENONET_UPSTREAM must be on this computer (127.0.0.1): forward the other XenoNet's port here, over SSH, rather than reach it across a network".into());
    }
    let port_n = if port.is_empty() { 80 } else { port.parse::<u16>().map_err(|_| format!("XENONET_UPSTREAM isn't a URL: {value}"))? };
    // url.host leaves out the scheme's own port.
    let host = if port.is_empty() || port_n == 80 { hostname.clone() } else { format!("{hostname}:{port_n}") };
    Ok(Some(Upstream { hostname: hostname.trim_start_matches('[').trim_end_matches(']').to_string(), port: port_n, host }))
}

fn local(route: &str) -> bool {
    route == "/api/health" || route.starts_with("/api/dev/") || route == "/api/logs/page"
}

fn answer(status: u16, body: serde_json::Value) -> Reply {
    let mut r = Reply::raw(status, xenonet_base::json::stringify(&body, 0), &[]);
    r.headers = vec![("content-type".into(), "application/json".into())];
    r
}

impl Upstream {
    /// Whether this request is the other XenoNet's to answer.
    pub fn handles(&self, route: &str) -> bool {
        route.starts_with("/api/") && !local(route)
    }

    /// A read passed on, its answer streamed back as it comes.
    pub async fn pass(&self, method: &str, target: &str, headers: &[(String, String)]) -> Reply {
        if method != "GET" && method != "HEAD" {
            return answer(403, json!({ "error": format!("this XenoNet Dev reads {} and changes nothing there", self.host), "code": "read-only" }));
        }
        let didnt = |why: String| answer(502, json!({ "error": format!("{} didn't answer: {why}", self.host), "code": "upstream" }));
        let stream = match TcpStream::connect((self.hostname.as_str(), self.port)).await {
            Ok(s) => s,
            Err(e) => {
                let code = if e.kind() == std::io::ErrorKind::ConnectionRefused { "ECONNREFUSED".to_string() } else { e.to_string() };
                return didnt(format!("connect {code} {}:{}", self.hostname, self.port));
            }
        };
        // HTTP/1.1, as Node's request writes it, the answer read as it comes.
        let mut head = format!("{method} {target} HTTP/1.1\r\n");
        for (k, v) in headers.iter().filter(|(k, _)| !DROP.contains(&k.as_str())) {
            let _ = write!(head, "{k}: {v}\r\n");
        }
        let _ = write!(head, "host: {}\r\nconnection: close\r\n\r\n", self.host);
        let mut stream = stream;
        if let Err(e) = stream.write_all(head.as_bytes()).await {
            return didnt(e.to_string());
        }
        let mut r = BufReader::new(stream);
        let mut line = String::new();
        if r.read_line(&mut line).await.is_err() {
            return didnt("socket hang up".into());
        }
        let Some(status) = line.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()) else { return didnt("socket hang up".into()) };
        let mut headers: Vec<(String, String)> = Vec::new();
        let (mut length, mut chunked) = (None, false);
        loop {
            line.clear();
            if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                break;
            }
            let h = line.trim_end();
            if h.is_empty() {
                break;
            }
            let Some((k, v)) = h.split_once(':') else { continue };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            match k.as_str() {
                "content-length" => length = v.parse::<usize>().ok(),
                "transfer-encoding" if v.to_ascii_lowercase().contains("chunked") => chunked = true,
                "connection" | "keep-alive" | "transfer-encoding" => {}
                _ => headers.push((k, v)),
            }
        }
        // XenoNet answers only to its own address (against DNS rebinding),
        // so a port forwarded to a different number here is turned away.
        if status == 421 {
            return answer(
                502,
                json!({ "error": format!("{} turned the request away: forward the other XenoNet's port to the same port here (47840 to 47840), since XenoNet answers only to its own address", self.host), "code": "upstream-port" }),
            );
        }
        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
        let head_only = method == "HEAD";
        tokio::spawn(async move {
            if head_only {
                return;
            }
            let mut buf = vec![0u8; 16 << 10];
            if chunked {
                let mut size = String::new();
                loop {
                    size.clear();
                    if r.read_line(&mut size).await.unwrap_or(0) == 0 {
                        return;
                    }
                    let Ok(n) = u64::from_str_radix(size.trim().split(';').next().unwrap_or(""), 16) else { return };
                    if n == 0 {
                        return;
                    }
                    // The chunk a piece at a time, as it comes: its size is
                    // the other end's word, and only what arrives takes room.
                    let mut left = n;
                    while left > 0 {
                        let want = buf.len().min(usize::try_from(left).unwrap_or(usize::MAX));
                        match r.read(&mut buf[..want]).await {
                            Ok(0) | Err(_) => return,
                            Ok(got) => {
                                left -= got as u64;
                                // The page going away ends the read there too.
                                if tx.send(Bytes::copy_from_slice(&buf[..got])).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                    // The CRLF after it.
                    let mut crlf = [0u8; 2];
                    if r.read_exact(&mut crlf).await.is_err() {
                        return;
                    }
                }
            }
            let mut left = length.unwrap_or(usize::MAX);
            while left > 0 {
                let want = buf.len().min(left);
                match r.read(&mut buf[..want]).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        left -= n;
                        if tx.send(Bytes::copy_from_slice(&buf[..n])).is_err() {
                            return;
                        }
                    }
                }
            }
        });
        Reply { status, headers, body: ReplyBody::Stream(rx) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_http_address_on_this_computer() {
        assert!(upstream_of(Some("")).unwrap().is_none());
        assert_eq!(upstream_of(Some("http://127.0.0.1:47850")).unwrap().unwrap().port, 47850);
        assert_eq!(upstream_of(Some("http://localhost:47850")).unwrap().unwrap().hostname, "localhost");
        assert!(upstream_of(Some("not a url")).err().unwrap().contains("isn't a URL"));
        assert!(upstream_of(Some("https://127.0.0.1:47850")).err().unwrap().contains("http://"));
        assert!(upstream_of(Some("http://192.0.2.10:47840")).err().unwrap().contains("on this computer"));
        let up = upstream_of(Some("http://127.0.0.1:47850")).unwrap().unwrap();
        assert!(up.handles("/api/state") && !up.handles("/api/health") && !up.handles("/api/dev/control") && !up.handles("/src/main.js"));
    }
}

#[cfg(test)]
mod passing {
    use super::*;
    use tokio::net::TcpListener;

    async fn body(r: Reply) -> String {
        let mut out = Vec::new();
        match r.body {
            ReplyBody::Full(b) => out.extend_from_slice(&b),
            ReplyBody::Stream(mut rx) => {
                while let Some(b) = rx.recv().await {
                    out.extend_from_slice(&b);
                }
            }
        }
        String::from_utf8(out).unwrap()
    }

    // A made-up XenoNet that says what it heard, and streams its events.
    async fn other() -> (u16, mpsc::UnboundedReceiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap();
                let head = String::from_utf8_lossy(&buf[..n]).to_string();
                let _ = tx.send(head.clone());
                if head.starts_with("GET /api/events") {
                    s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n").await.unwrap();
                    for d in ["event: live\ndata: {\"n\":1}\n\n", "event: live\ndata: {\"n\":2}\n\n"] {
                        s.write_all(format!("{:x}\r\n{d}\r\n", d.len()).as_bytes()).await.unwrap();
                    }
                    s.write_all(b"0\r\n\r\n").await.unwrap();
                } else if head.starts_with("GET /api/turned-away") {
                    s.write_all(b"HTTP/1.1 421 Misdirected\r\ncontent-type: application/json\r\ncontent-length: 2\r\n\r\n{}").await.unwrap();
                } else {
                    s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nx-from: other\r\ncontent-length: 14\r\n\r\n{\"sessions\":3}").await.unwrap();
                }
            }
        });
        (port, rx)
    }

    #[tokio::test]
    async fn reads_go_through_without_the_pages_own_headers_and_changes_dont() {
        let (port, mut heard) = other().await;
        let up = upstream_of(Some(&format!("http://127.0.0.1:{port}"))).unwrap().unwrap();
        let h = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<Vec<_>>();
        let r =
            up.pass("GET", "/api/state?scope=7d", &h(&[("origin", "http://x"), ("cookie", "a=b"), ("x-xenonet-window", "the-window-key"), ("x-xenonet-api-version", "1")])).await;
        assert_eq!((r.status, r.header("x-from").map(str::to_string)), (200, Some("other".into())));
        assert_eq!(body(r).await, "{\"sessions\":3}");
        let got = heard.recv().await.unwrap();
        assert!(got.starts_with("GET /api/state?scope=7d HTTP/1.1"));
        assert!(got.contains(&format!("host: 127.0.0.1:{port}")) && got.contains("x-xenonet-api-version: 1"));
        assert!(!got.contains("cookie") && !got.contains("origin") && !got.contains("the-window-key"));
        let events = body(up.pass("GET", "/api/events", &[]).await).await;
        assert!(events.contains("data: {\"n\":1}") && events.contains("data: {\"n\":2}"), "{events}");
        for m in ["POST", "PUT", "DELETE"] {
            let r = up.pass(m, "/api/acks", &[]).await;
            assert_eq!((r.status, r.body_json()["code"].clone()), (403, json!("read-only")));
        }
        let r = up.pass("GET", "/api/turned-away", &[]).await;
        assert_eq!((r.status, r.body_json()["code"].clone()), (502, json!("upstream-port")));
    }

    // A chunk size too big to hold, from the other end, ends the stream
    // where the bytes do, rather than taking the service's memory.
    #[tokio::test]
    async fn a_chunk_too_big_to_hold_ends_the_stream() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = s.read(&mut buf).await.unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n5\r\nfirst\r\nffffffffffffffff\r\nsecond").await.unwrap();
        });
        let up = upstream_of(Some(&format!("http://127.0.0.1:{port}"))).unwrap().unwrap();
        assert_eq!(body(up.pass("GET", "/api/events", &[]).await).await, "firstsecond");
    }

    #[tokio::test]
    async fn another_xenonet_that_doesnt_answer_is_a_502() {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let up = upstream_of(Some(&format!("http://127.0.0.1:{port}"))).unwrap().unwrap();
        let r = up.pass("GET", "/api/state", &[]).await;
        assert_eq!(r.status, 502);
        assert!(r.body_json()["error"].as_str().unwrap().contains("didn't answer"));
    }
}
