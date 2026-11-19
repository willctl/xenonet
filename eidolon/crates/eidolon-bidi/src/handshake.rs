//! The HTTP/1.1 upgrade that starts a WebSocket (RFC 6455, section 4), for
//! both sides, so the client and the fake server share one reading of it.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ring::digest;
use tokio::io::{AsyncRead, AsyncReadExt};

use crate::error::Error;

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const MAX_HEAD: usize = 16 * 1024;

/// The `Sec-WebSocket-Accept` value for a key. The standard fixes SHA-1
/// here; it is not used for anything secret.
pub fn accept_key(key: &str) -> String {
    let mut input = String::with_capacity(key.len() + GUID.len());
    input.push_str(key);
    input.push_str(GUID);
    let hash = digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, input.as_bytes());
    STANDARD.encode(hash.as_ref())
}

/// A fresh random `Sec-WebSocket-Key`.
pub fn new_key() -> Result<String, Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| Error::Handshake(format!("no random bytes for the key: {e}")))?;
    Ok(STANDARD.encode(bytes))
}

/// The client's upgrade request.
pub fn request(path: &str, host: &str, key: &str) -> String {
    format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )
}

/// The server's `101` answer for a key.
pub fn response(key: &str) -> String {
    format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Accept: {}\r\n\r\n",
        accept_key(key)
    )
}

/// Reads up to the blank line that ends an HTTP head. Returns the head and
/// any bytes read past it, which belong to the first frames.
pub async fn read_head<R: AsyncRead + Unpin>(r: &mut R) -> Result<(String, Vec<u8>), Error> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = find_end(&buf) {
            let rest = buf.split_off(end);
            let head = String::from_utf8(buf).map_err(|_| Error::Handshake("the HTTP head is not text".into()))?;
            return Ok((head, rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(Error::Handshake("the HTTP head is over 16 KiB".into()));
        }
        let n = r.read(&mut chunk).await?;
        if n == 0 {
            return Err(Error::Handshake("the connection closed before the HTTP head ended".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().skip(1).find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.trim().eq_ignore_ascii_case(name).then(|| value.trim())
    })
}

fn has_token(value: &str, token: &str) -> bool {
    value.split(',').any(|t| t.trim().eq_ignore_ascii_case(token))
}

/// Checks the server's answer to our request: status 101, the upgrade
/// headers, and an accept value that matches `key`.
pub fn check_response(head: &str, key: &str) -> Result<(), Error> {
    let status = head.lines().next().unwrap_or_default();
    let mut parts = status.split_whitespace();
    let (version, code) = (parts.next().unwrap_or_default(), parts.next().unwrap_or_default());
    if !version.starts_with("HTTP/1.") || code != "101" {
        return Err(Error::Handshake(format!("it answered \"{status}\" instead of 101")));
    }
    if !header(head, "Upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        return Err(Error::Handshake("its answer has no \"Upgrade: websocket\"".into()));
    }
    if !header(head, "Connection").is_some_and(|v| has_token(v, "upgrade")) {
        return Err(Error::Handshake("its answer has no \"Connection: Upgrade\"".into()));
    }
    match header(head, "Sec-WebSocket-Accept") {
        Some(got) if got == accept_key(key) => Ok(()),
        Some(_) => Err(Error::Handshake("its Sec-WebSocket-Accept does not match our key, so it is not speaking WebSocket to us".into())),
        None => Err(Error::Handshake("its answer has no Sec-WebSocket-Accept".into())),
    }
}

/// Reads a client's upgrade request and returns its `Sec-WebSocket-Key`.
pub fn parse_request(head: &str) -> Result<String, Error> {
    let line = head.lines().next().unwrap_or_default();
    if !line.starts_with("GET ") {
        return Err(Error::Handshake(format!("expected a GET request, got \"{line}\"")));
    }
    if !header(head, "Upgrade").is_some_and(|v| v.eq_ignore_ascii_case("websocket")) {
        return Err(Error::Handshake("the request has no \"Upgrade: websocket\"".into()));
    }
    if header(head, "Sec-WebSocket-Version") != Some("13") {
        return Err(Error::Handshake("the request is not WebSocket version 13".into()));
    }
    header(head, "Sec-WebSocket-Key").map(str::to_owned).ok_or_else(|| Error::Handshake("the request has no Sec-WebSocket-Key".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_accept_example() {
        // RFC 6455 section 1.3.
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn request_and_response_agree() {
        let key = "dGhlIHNhbXBsZSBub25jZQ==";
        let req = request("/session", "127.0.0.1:9222", key);
        assert_eq!(parse_request(&req).as_deref(), Ok(key));
        assert_eq!(check_response(&response(key), key), Ok(()));
    }

    #[test]
    fn a_wrong_accept_is_refused() {
        let key = new_key().unwrap();
        let bad = response("another key");
        assert!(matches!(check_response(&bad, &key), Err(Error::Handshake(_))));
    }

    #[test]
    fn a_plain_http_answer_is_refused() {
        let key = new_key().unwrap();
        let head = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
        assert!(matches!(check_response(head, &key), Err(Error::Handshake(_))));
    }

    #[tokio::test]
    async fn read_head_keeps_the_bytes_after_it() {
        let mut data: &[u8] = b"HTTP/1.1 101 x\r\nA: b\r\n\r\n\x81\x00";
        let (head, rest) = read_head(&mut data).await.unwrap();
        assert!(head.ends_with("\r\n\r\n"));
        assert_eq!(rest, [0x81, 0x00]);
    }
}
