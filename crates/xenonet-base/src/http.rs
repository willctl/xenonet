//! A request to a XenoNet on this machine: HTTP/1.1 to 127.0.0.1, which is
//! all the command and its clients need, within a deadline (when there's
//! one) that covers connecting, sending and the whole answer.

use std::fmt::Write as _;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

/// An answer: its status and reason, and its body, or how reading the body
/// failed (fetch's res.json() fails then, after the status came).
pub struct Answer {
    pub status: u16,
    pub reason: String,
    pub body: io::Result<Vec<u8>>,
}

impl Answer {
    /// fetch's res.ok.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

// What's left of the time, or None for no deadline.
fn left(deadline: Option<Instant>) -> io::Result<Option<Duration>> {
    match deadline {
        None => Ok(None),
        Some(d) => d.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).map(Some).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "out of time")),
    }
}

/// Sends `method` `path` to 127.0.0.1:`port` with these headers (and a
/// Content-Length for a body), and reads the answer.
pub fn request(port: u16, method: &str, path: &str, headers: &[(String, String)], body: Option<&[u8]>, timeout: Option<Duration>) -> io::Result<Answer> {
    let deadline = timeout.map(|t| Instant::now() + t);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = match left(deadline)? {
        Some(t) => TcpStream::connect_timeout(&addr, t)?,
        None => TcpStream::connect(addr)?,
    };
    s.set_nodelay(true)?;
    s.set_write_timeout(left(deadline)?)?;
    // XenoNet answers only to its own address in Host, and to no Origin.
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (k, v) in headers {
        let _ = write!(head, "{k}: {v}\r\n");
    }
    if let Some(b) = body {
        let _ = write!(head, "Content-Length: {}\r\n", b.len());
    }
    head.push_str("Connection: close\r\n\r\n");
    s.write_all(head.as_bytes())?;
    if let Some(b) = body {
        s.write_all(b)?;
    }
    s.flush()?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    let read_line = |r: &mut BufReader<TcpStream>, line: &mut String| -> io::Result<()> {
        r.get_ref().set_read_timeout(left(deadline)?)?;
        line.clear();
        if r.read_line(line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the answer ended early"));
        }
        Ok(())
    };
    read_line(&mut r, &mut line)?;
    let mut status_line = line.trim_end().splitn(3, ' ');
    let status = status_line.nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let reason = status_line.next().unwrap_or("").to_string();
    let (mut length, mut chunked) = (None, false);
    loop {
        read_line(&mut r, &mut line)?;
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let v = v.trim();
            if k.eq_ignore_ascii_case("content-length") {
                length = v.parse::<usize>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let mut read_body = || -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        if chunked {
            loop {
                read_line(&mut r, &mut line)?;
                let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a bad chunk"))?;
                if size == 0 {
                    break;
                }
                let mut chunk = vec![0; size + 2];
                r.get_ref().set_read_timeout(left(deadline)?)?;
                r.read_exact(&mut chunk)?;
                body.extend_from_slice(&chunk[..size]);
            }
        } else if let Some(n) = length {
            body.resize(n, 0);
            r.get_ref().set_read_timeout(left(deadline)?)?;
            r.read_exact(&mut body)?;
        } else {
            loop {
                r.get_ref().set_read_timeout(left(deadline)?)?;
                let mut buf = [0u8; 8192];
                match r.read(&mut buf)? {
                    0 => break,
                    n => body.extend_from_slice(&buf[..n]),
                }
            }
        }
        Ok(body)
    };
    let body = read_body();
    Ok(Answer { status, reason, body })
}

/// POSTs a JSON body to `path` on 127.0.0.1:`port`.
pub fn post_json(port: u16, path: &str, body: &[u8], timeout: Duration) -> io::Result<Answer> {
    request(port, "POST", path, &[("Content-Type".into(), "application/json".into())], Some(body), Some(timeout))
}
