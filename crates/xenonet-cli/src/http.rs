//! A request to a XenoNet on this machine: HTTP/1.1 to 127.0.0.1, which is
//! all the command needs, within a deadline that covers connecting,
//! sending and the whole answer.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub struct Answer {
    pub status: u16,
    pub body: Vec<u8>,
}

fn left(deadline: Instant) -> io::Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "out of time"))
}

/// POSTs a JSON body to `path` on 127.0.0.1:`port`.
pub fn post_json(port: u16, path: &str, body: &[u8], timeout: Duration) -> io::Result<Answer> {
    let deadline = Instant::now() + timeout;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut s = TcpStream::connect_timeout(&addr, left(deadline)?)?;
    s.set_nodelay(true)?;
    s.set_write_timeout(Some(left(deadline)?))?;
    // XenoNet answers only to its own address in Host, and to no Origin.
    let head = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    s.flush()?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    let read_line = |r: &mut BufReader<TcpStream>, line: &mut String| -> io::Result<()> {
        r.get_ref().set_read_timeout(Some(left(deadline)?))?;
        line.clear();
        if r.read_line(line)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the answer ended early"));
        }
        Ok(())
    };
    read_line(&mut r, &mut line)?;
    let status = line.split(' ').nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "not an HTTP answer"))?;
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
    let mut body = Vec::new();
    if chunked {
        loop {
            read_line(&mut r, &mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a bad chunk"))?;
            if size == 0 {
                break;
            }
            let mut chunk = vec![0; size + 2];
            r.get_ref().set_read_timeout(Some(left(deadline)?))?;
            r.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(n) = length {
        body.resize(n, 0);
        r.get_ref().set_read_timeout(Some(left(deadline)?))?;
        r.read_exact(&mut body)?;
    } else {
        loop {
            r.get_ref().set_read_timeout(Some(left(deadline)?))?;
            let mut buf = [0u8; 8192];
            match r.read(&mut buf)? {
                0 => break,
                n => body.extend_from_slice(&buf[..n]),
            }
        }
    }
    Ok(Answer { status, body })
}
