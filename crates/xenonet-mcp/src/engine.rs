//! Requests to the XenoNet on this machine, as the session calling: HTTP/1.1
//! to 127.0.0.1, with the API's version header and the session's
//! x-xenonet-agent.

use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub struct Answer {
    pub status: u16,
    pub body: Value,
}

/// POSTs JSON to `path` on 127.0.0.1:`port`, within `timeout`.
pub async fn post(port: u16, path: &str, agent: &str, body: &Value, timeout: Duration) -> std::io::Result<Answer> {
    tokio::time::timeout(timeout, post_now(port, path, agent, body)).await.map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "XenoNet didn't answer in time"))?
}

async fn post_now(port: u16, path: &str, agent: &str, body: &Value) -> std::io::Result<Answer> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
    let body = serde_json::to_vec(body)?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nx-xenonet-api-version: 1\r\nx-xenonet-agent: {agent}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).await?;
    s.write_all(&body).await?;
    let mut r = BufReader::new(s);
    let mut line = String::new();
    r.read_line(&mut line).await?;
    let status = line.split(' ').nth(1).and_then(|c| c.parse().ok()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not an HTTP answer"))?;
    let (mut length, mut chunked) = (None, false);
    loop {
        line.clear();
        r.read_line(&mut line).await?;
        let h = line.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                length = v.trim().parse::<usize>().ok();
            } else if k.eq_ignore_ascii_case("transfer-encoding") && v.to_ascii_lowercase().contains("chunked") {
                chunked = true;
            }
        }
    }
    let mut data = Vec::new();
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line).await?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "a bad chunk"))?;
            if size == 0 {
                break;
            }
            let mut chunk = vec![0; size + 2];
            r.read_exact(&mut chunk).await?;
            data.extend_from_slice(&chunk[..size]);
        }
    } else if let Some(n) = length {
        data.resize(n, 0);
        r.read_exact(&mut data).await?;
    } else {
        r.read_to_end(&mut data).await?;
    }
    let body = if data.is_empty() { Value::Null } else { serde_json::from_slice(&data).unwrap_or(Value::Null) };
    Ok(Answer { status, body })
}
