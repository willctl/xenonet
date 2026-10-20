//! Calls to the XenoNet on this machine, as core/client.js makes them
//! (xenonet_base::client is the same for the command), but on tokio: the
//! servers wait on XenoNet (inbox waits up to two minutes, a channel's
//! poll 25 seconds) while they go on answering their client, and a call
//! still waiting when the client goes is dropped with the process, not
//! waited for. HTTP/1.1 to 127.0.0.1, with the API's version header.

use std::fmt::Write as _;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use xenonet_base::client::{Refused, Server};
use xenonet_base::json::{self, number_string, Value};
use xenonet_base::text::js_string;

struct Answer {
    status: u16,
    reason: String,
    body: std::io::Result<Vec<u8>>,
}

// A port fetch would make a URL of.
fn real_port(n: f64) -> Option<u16> {
    (n.fract() == 0.0 && (0.0..=65535.0).contains(&n)).then_some(n as u16)
}

async fn send(port: u16, method: &str, path: &str, headers: &[(String, String)], body: Option<&[u8]>) -> std::io::Result<Answer> {
    let s = TcpStream::connect(("127.0.0.1", port)).await?;
    s.set_nodelay(true)?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in headers {
        let _ = write!(head, "{k}: {v}\r\n");
    }
    if let Some(b) = body {
        let _ = write!(head, "Content-Length: {}\r\n", b.len());
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut r = BufReader::new(s);
    r.get_mut().write_all(head.as_bytes()).await?;
    if let Some(b) = body {
        r.get_mut().write_all(b).await?;
    }
    r.get_mut().flush().await?;
    let mut line = String::new();
    if r.read_line(&mut line).await? == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the answer ended early"));
    }
    let mut first = line.trim_end().splitn(3, ' ');
    let status = first.nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let reason = first.next().unwrap_or("").to_string();
    let (mut length, mut chunked) = (None, false);
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the answer ended early"));
        }
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let v = v.trim();
            if k.eq_ignore_ascii_case("content-length") {
                length = v.parse::<u64>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let body = async {
        let mut data = Vec::new();
        if chunked {
            loop {
                line.clear();
                r.read_line(&mut line).await?;
                let size = u64::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a bad chunk"))?;
                if size == 0 {
                    break;
                }
                take(&mut r, size, &mut data).await?;
                // The CRLF after the chunk.
                take(&mut r, 2, &mut Vec::new()).await?;
            }
        } else if let Some(n) = length {
            take(&mut r, n, &mut data).await?;
        } else {
            r.read_to_end(&mut data).await?;
        }
        Ok(data)
    }
    .await;
    Ok(Answer { status, reason, body })
}

// `n` bytes onto `data`, or an early end. Whatever answers on the port
// says how big its body is, so only the bytes that come take room.
async fn take(r: &mut BufReader<TcpStream>, n: u64, data: &mut Vec<u8>) -> std::io::Result<()> {
    if (&mut *r).take(n).read_to_end(data).await? as u64 != n {
        return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "the answer ended early"));
    }
    Ok(())
}

// An answer's body as res.json() reads it.
fn json_of(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);
    json::parse(text.strip_prefix('\u{feff}').unwrap_or(&text)).ok()
}

/// A call to the API on `port`, for XenoNet `name` (as core/client.js's
/// request): its answer, or an error with XenoNet's message and the
/// status, or with no status when nothing answered. Headers given replace
/// the ones it sends by itself.
pub async fn request(port: f64, name: &str, method: &str, route: &str, body: Option<&Value>, headers: &[(String, String)]) -> Result<Value, Refused> {
    let mut all: Vec<(String, String)> = vec![("x-xenonet-api-version".into(), "1".into())];
    if body.is_some() {
        all.push(("content-type".into(), "application/json".into()));
    }
    for (k, v) in headers {
        all.retain(|(a, _)| !a.eq_ignore_ascii_case(k));
        all.push((k.clone(), v.clone()));
    }
    let text = body.map(|b| json::stringify(b, 0));
    let stopped = || Refused { message: format!("{name} stopped answering on 127.0.0.1:{} (restarting, or quit); try again in a moment", number_string(port)), status: None };
    let Some(p) = real_port(port) else { return Err(stopped()) };
    let answer = send(p, method, route, &all, text.as_deref().map(str::as_bytes)).await.map_err(|_| stopped())?;
    let ok = (200..300).contains(&answer.status);
    let data = answer.body.as_ref().ok().and_then(|b| json_of(b)).unwrap_or_else(|| Value::Object(serde_json::Map::default()));
    if ok {
        return Ok(data);
    }
    if data.is_null() {
        return Err(Refused { message: "Cannot read properties of null (reading 'error')".into(), status: None });
    }
    let message = data
        .get("error")
        .filter(|e| !e.is_null())
        .map_or_else(|| format!("{route}: {} {}", answer.status, answer.reason), |e| e.as_str().map_or_else(|| js_string(e), str::to_string));
    Err(Refused { message, status: Some(answer.status) })
}

/// The first of these ports where a XenoNet answers /api/health within
/// `answer` (core/client.js's find), in their order.
pub async fn find(ports: &[f64], answer: Duration) -> Option<Server> {
    let probes = ports.iter().map(|&n| async move {
        let port = real_port(n)?;
        let a = tokio::time::timeout(answer, send(port, "GET", "/api/health", &[], None)).await.ok()?.ok()?;
        let health = json_of(a.body.as_ref().ok()?)?;
        (200..300).contains(&a.status).then_some(Server { port, health })
    });
    let mut found = vec![];
    for p in probes {
        found.push(p.await);
    }
    found.into_iter().flatten().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    // A stand-in XenoNet answering each request with `answer`'s status and
    // body for its path.
    fn stand_in(answer: fn(&str) -> (u16, &'static str, &'static str)) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut first, mut line, mut length) = (String::new(), String::new(), 0);
                r.read_line(&mut first).unwrap();
                loop {
                    line.clear();
                    r.read_line(&mut line).unwrap();
                    if line.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                r.read_exact(&mut body).unwrap();
                let (status, reason, text) = answer(first.split(' ').nth(1).unwrap());
                write!(s, "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\n\r\n{text}", text.len()).unwrap();
            }
        });
        port
    }

    #[tokio::test(flavor = "current_thread")]
    async fn calls_and_their_errors_as_core_client_words_them() {
        let port = stand_in(|path| match path {
            "/api/health" => (200, "OK", r#"{"ok":true,"name":"Stand-in"}"#),
            "/api/refused" => (403, "Forbidden", r#"{"error":"an agent can't do that"}"#),
            "/api/broken" => (500, "Internal Server Error", "<html>"),
            "/api/null" => (500, "Internal Server Error", "null"),
            _ => (200, "OK", r#"{"n":1e21}"#),
        });
        let p = f64::from(port);
        assert_eq!(json::stringify(&request(p, "XenoNet", "POST", "/api/x", Some(&json::parse("{}").unwrap()), &[]).await.unwrap(), 0), r#"{"n":1e+21}"#);
        assert_eq!(request(p, "XenoNet", "GET", "/api/refused", None, &[]).await, Err(Refused { message: "an agent can't do that".into(), status: Some(403) }));
        assert_eq!(request(p, "XenoNet", "GET", "/api/broken", None, &[]).await.unwrap_err().message, "/api/broken: 500 Internal Server Error");
        assert_eq!(request(p, "XenoNet", "GET", "/api/null", None, &[]).await.unwrap_err().status, None);
        let gone = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert_eq!(
            request(gone.into(), "XenoNet Dev", "GET", "/api/x", None, &[]).await.unwrap_err(),
            Refused { message: format!("XenoNet Dev stopped answering on 127.0.0.1:{gone} (restarting, or quit); try again in a moment"), status: None }
        );
        let found = find(&[gone.into(), p], Duration::from_secs(2)).await.unwrap();
        assert_eq!((found.port, found.health["name"].as_str()), (port, Some("Stand-in")));
    }

    // Whatever listens on the port can say any size: one too big to hold is
    // an answer that ended early, not an overflow or a huge allocation.
    #[tokio::test(flavor = "current_thread")]
    async fn sizes_too_big_to_hold_end_early() {
        for raw in [
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\n{}",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\nfffffffffffffffe\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 18446744073709551615\r\n\r\n{}",
        ] {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = l.local_addr().unwrap().port();
            std::thread::spawn(move || {
                let (mut s, _) = l.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                s.write_all(raw.as_bytes()).unwrap();
            });
            let answer = send(port, "GET", "/api/x", &[], None).await.unwrap();
            assert_eq!(answer.body.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof, "{raw}");
        }
    }
}
