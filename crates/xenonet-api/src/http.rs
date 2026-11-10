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
    read_answer(&mut r, &mut |r: &mut BufReader<TcpStream>| r.get_ref().set_read_timeout(left(deadline)?))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_string())
}

type Wait<'a, R> = dyn FnMut(&mut R) -> io::Result<()> + 'a;

fn read_line<R: BufRead>(r: &mut R, wait: &mut Wait<'_, R>, line: &mut String) -> io::Result<()> {
    wait(r)?;
    line.clear();
    if r.read_line(line)? == 0 {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the answer ended early"));
    }
    Ok(())
}

// `n` bytes onto `body`, or an early end. Only the bytes that come take
// room, whatever `n` says.
fn take<R: BufRead>(r: &mut R, wait: &mut Wait<'_, R>, n: u64, body: &mut Vec<u8>) -> io::Result<()> {
    wait(r)?;
    if r.by_ref().take(n).read_to_end(body)? as u64 != n {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the answer ended early"));
    }
    Ok(())
}

fn read_body<R: BufRead>(r: &mut R, wait: &mut Wait<'_, R>, length: Option<u64>, chunked: bool) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    if chunked {
        let mut line = String::new();
        loop {
            read_line(r, wait, &mut line)?;
            let size = u64::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| bad("a bad chunk"))?;
            if size == 0 {
                break;
            }
            take(r, wait, size, &mut body)?;
            // The CRLF after the chunk.
            take(r, wait, 2, &mut Vec::new())?;
        }
    } else if let Some(n) = length {
        take(r, wait, n, &mut body)?;
    } else {
        loop {
            wait(r)?;
            let mut buf = [0u8; 8192];
            match r.read(&mut buf)? {
                0 => break,
                n => body.extend_from_slice(&buf[..n]),
            }
        }
    }
    Ok(body)
}

/// Reads an answer from `r`: its status line, its headers, and its body by
/// Content-Length, in chunks, or to the end. `wait` runs before each read
/// (to set how long it may take). The answer comes from whatever listens on
/// the port, so a size it declares is believed only as far as the bytes
/// that come.
pub fn read_answer<R: BufRead>(r: &mut R, wait: &mut Wait<'_, R>) -> io::Result<Answer> {
    let mut line = String::new();
    read_line(r, wait, &mut line)?;
    let mut status_line = line.trim_end().splitn(3, ' ');
    let status = status_line.nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| bad("not an HTTP answer"))?;
    let reason = status_line.next().unwrap_or("").to_string();
    let (mut length, mut chunked) = (None, false);
    loop {
        read_line(r, wait, &mut line)?;
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
    let body = read_body(r, wait, length, chunked);
    Ok(Answer { status, reason, body })
}

/// POSTs a JSON body to `path` on 127.0.0.1:`port`.
pub fn post_json(port: u16, path: &str, body: &[u8], timeout: Duration) -> io::Result<Answer> {
    request(port, "POST", path, &[("Content-Type".into(), "application/json".into())], Some(body), Some(timeout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &[u8]) -> io::Result<Answer> {
        read_answer(&mut io::Cursor::new(text), &mut |_| Ok(()))
    }

    #[test]
    fn reads_answers() {
        let a = answer(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi").unwrap();
        assert_eq!((a.status, a.reason.as_str(), a.body.unwrap()), (200, "OK", b"hi".to_vec()));
        let a = answer(b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nhi\r\n3;x=1\r\n th\r\n0\r\n\r\n").unwrap();
        assert_eq!(a.body.unwrap(), b"hi th");
        assert_eq!(answer(b"HTTP/1.1 204 No Content\r\n\r\nrest").unwrap().body.unwrap(), b"rest");
    }

    #[test]
    fn a_size_is_believed_only_as_far_as_the_bytes() {
        // A chunk or a length too big to hold, from whatever answers on the
        // port, is an early end, not an overflow or a huge allocation.
        for odd in [
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nffffffffffffffff\r\nhi"[..],
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nfffffffffffffffe\r\n",
            b"HTTP/1.1 200 OK\r\nContent-Length: 18446744073709551615\r\n\r\nhi",
        ] {
            assert_eq!(answer(odd).unwrap().body.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
        }
        assert_eq!(answer(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1ffffffffffffffff\r\n").unwrap().body.unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(answer(b"nothing").is_err());
    }
}
