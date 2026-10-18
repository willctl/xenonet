//! Requests and answers as server/index.js handles them: Node's view of a
//! request (its headers, the URL `new URL(req.url, base)` makes, the
//! socket a process check looks at), the JSON bodies readJson reads, and
//! what send() writes.

use bytes::Bytes;
use http_body_util::BodyExt;
use serde_json::Value;
use std::fmt::Write as _;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use xenonet_base::json;

/// The two ends of a request's connection: what core/engine/verify.js reads
/// off `req.socket` to find the process that sent it.
#[derive(Clone, Copy, Debug)]
pub struct Peer {
    pub remote: SocketAddr,
    pub local: SocketAddr,
}

/// Why a body couldn't be read: too big (413) or not JSON (400), with the
/// words readJson uses.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyError {
    pub status: u16,
    pub message: &'static str,
}

pub const TOO_BIG: BodyError = BodyError { status: 413, message: "the body is too big" };
pub const NOT_JSON: BodyError = BodyError { status: 400, message: "the body isn't JSON" };

/// The body a route reads once, as Node's request stream is.
pub enum Body {
    Incoming(hyper::body::Incoming),
    Bytes(Vec<u8>),
    Taken,
}

pub struct Req {
    pub method: String,
    /// `url.pathname`, as WHATWG URL parsing leaves it.
    pub path: String,
    /// `url.searchParams`, decoded, in order.
    pub query: Vec<(String, String)>,
    /// The headers, names lowercased, each value as Node joins or keeps it.
    pub headers: Vec<(String, String)>,
    pub peer: Peer,
    pub body: Body,
}

impl Req {
    /// `req.headers[name]` as Node has it: undefined when absent.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    /// `url.searchParams.get(name)`: the first value.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.query.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The body's bytes, up to `limit`; a second read gets nothing, as a
    /// stream read twice does.
    pub async fn bytes(&mut self, limit: usize) -> Result<Vec<u8>, BodyError> {
        match std::mem::replace(&mut self.body, Body::Taken) {
            Body::Taken => Ok(Vec::new()),
            Body::Bytes(b) => {
                if b.len() > limit {
                    Err(TOO_BIG)
                } else {
                    Ok(b)
                }
            }
            Body::Incoming(mut incoming) => {
                let mut out = Vec::new();
                while let Some(frame) = incoming.frame().await {
                    // A connection that broke off mid-body is a body that
                    // isn't JSON, as JSON.parse of what came would find.
                    let Ok(frame) = frame else { return Err(NOT_JSON) };
                    if let Ok(data) = frame.into_data() {
                        if out.len() + data.len() > limit {
                            return Err(TOO_BIG);
                        }
                        out.extend_from_slice(&data);
                    }
                }
                Ok(out)
            }
        }
    }

    /// readJson(req, limit): the body as JSON, {} when there's none.
    pub async fn json(&mut self, limit: usize) -> Result<Value, BodyError> {
        let bytes = self.bytes(limit).await?;
        if bytes.is_empty() {
            return Ok(Value::Object(serde_json::Map::default()));
        }
        // Buffer.toString("utf8") puts U+FFFD for bytes that aren't UTF-8.
        let text = String::from_utf8_lossy(&bytes);
        json::parse(&text).map_err(|_| NOT_JSON)
    }

    /// The default limit, 1 MiB.
    pub async fn json_body(&mut self) -> Result<Value, BodyError> {
        self.json(1 << 20).await
    }

    /// A request made by hand, for tests.
    pub fn fake(method: &str, target: &str, headers: &[(&str, &str)], body: Option<&str>) -> Req {
        let (path, query) = parse_target(target);
        let addr = SocketAddr::from(([127, 0, 0, 1], 1));
        Req {
            method: method.to_string(),
            path,
            query,
            headers: headers.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.to_string())).collect(),
            peer: Peer { remote: addr, local: addr },
            body: body.map_or(Body::Taken, |b| Body::Bytes(b.as_bytes().to_vec())),
        }
    }
}

/// What a route answers.
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: ReplyBody,
}

pub enum ReplyBody {
    Full(Bytes),
    /// A stream (the event streams), written as it goes; it ends when the
    /// sender is dropped.
    Stream(mpsc::UnboundedReceiver<Bytes>),
}

impl Reply {
    /// send(res, status, body, headers): JSON as JS writes it, unless it's a
    /// string already, with the API's headers and then `headers` over them.
    pub fn send(status: u16, body: &Value, headers: &[(&str, &str)]) -> Reply {
        let data = match body {
            Value::String(s) => s.clone().into_bytes(),
            v => json::stringify(v, 0).into_bytes(),
        };
        Reply::raw(status, data, headers)
    }

    /// send() with a string or a Buffer as it is.
    pub fn raw(status: u16, data: impl Into<Bytes>, headers: &[(&str, &str)]) -> Reply {
        let mut all = vec![("content-type".to_string(), "application/json; charset=utf-8".to_string()), ("cache-control".to_string(), "no-store".to_string())];
        for (k, v) in headers {
            let k = k.to_ascii_lowercase();
            match all.iter_mut().find(|(name, _)| *name == k) {
                Some(slot) => slot.1 = v.to_string(),
                None => all.push((k, v.to_string())),
            }
        }
        Reply { status, headers: all, body: ReplyBody::Full(data.into()) }
    }

    pub fn json(status: u16, body: Value) -> Reply {
        Reply::send(status, &body, &[])
    }

    /// `{ error }`.
    pub fn error(status: u16, error: impl Into<String>) -> Reply {
        Reply::json(status, serde_json::json!({ "error": error.into() }))
    }

    /// `{ error, code }`.
    pub fn coded(status: u16, code: &str, error: impl Into<String>) -> Reply {
        Reply::json(status, serde_json::json!({ "error": error.into(), "code": code }))
    }

    /// res.writeHead(status).end(): no headers, no body.
    pub fn empty(status: u16) -> Reply {
        Reply { status, headers: Vec::new(), body: ReplyBody::Full(Bytes::new()) }
    }

    /// A stream with these headers.
    pub fn stream(status: u16, headers: &[(&str, &str)], rx: mpsc::UnboundedReceiver<Bytes>) -> Reply {
        Reply { status, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: ReplyBody::Stream(rx) }
    }

    /// The body as JSON, for tests.
    pub fn body_json(&self) -> Value {
        match &self.body {
            ReplyBody::Full(b) => json::parse(&String::from_utf8_lossy(b)).unwrap_or(Value::Null),
            ReplyBody::Stream(_) => Value::Null,
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

// ---------- headers as Node has them ----------

/// Headers Node keeps only the first of; the rest are joined with ", "
/// (http.IncomingMessage's headers, set-cookie aside).
const FIRST_ONLY: [&str; 18] = [
    "age",
    "authorization",
    "content-length",
    "content-type",
    "etag",
    "expires",
    "from",
    "host",
    "if-modified-since",
    "if-unmodified-since",
    "last-modified",
    "location",
    "max-forwards",
    "proxy-authorization",
    "referer",
    "retry-after",
    "server",
    "user-agent",
];

pub fn node_headers(map: &hyper::HeaderMap) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (name, value) in map {
        let k = name.as_str().to_ascii_lowercase();
        // Node reads header values as latin1.
        let v: String = value.as_bytes().iter().map(|&b| b as char).collect();
        match out.iter_mut().find(|(n, _)| *n == k) {
            Some(slot) if !FIRST_ONLY.contains(&k.as_str()) => {
                slot.1.push_str(", ");
                slot.1.push_str(&v);
            }
            Some(_) => {}
            None => out.push((k, v)),
        }
    }
    out
}

// ---------- the URL, as `new URL(req.url, "http://<host>")` reads it ----------

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decoding as the WHATWG URL standard does it: a % without two
/// hex digits after it stays as it is.
pub fn percent_decode(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(a), Some(b)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(a * 16 + b);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// decodeURIComponent: an error where a % isn't followed by two hex digits
/// or the bytes aren't UTF-8 (a URIError in JS).
pub fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let (Some(x), Some(y)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) else { return None };
            out.push(x * 16 + y);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// application/x-www-form-urlencoded, as URLSearchParams reads a query.
pub fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let dec = |s: &str| String::from_utf8_lossy(&percent_decode(s.replace('+', " ").as_bytes())).into_owned();
            (dec(k), dec(v))
        })
        .collect()
}

// The path percent-encode set: C0 controls, space, ", #, <, >, ?, `, {, }
// and everything above ~. (# and ? never reach here.)
fn path_encode(b: u8) -> bool {
    b <= 0x20 || b >= 0x7f || matches!(b, b'"' | b'#' | b'<' | b'>' | b'?' | b'`' | b'{' | b'}')
}

fn single_dot(seg: &str) -> bool {
    seg == "." || seg.eq_ignore_ascii_case("%2e")
}

fn double_dot(seg: &str) -> bool {
    matches!(seg.to_ascii_lowercase().as_str(), ".." | ".%2e" | "%2e." | "%2e%2e")
}

/// A path as the URL parser leaves it for a special scheme: \ is /, dot
/// segments go, and what isn't URL-safe is percent-encoded.
fn normalize_path(raw: &str) -> String {
    let raw = raw.replace('\\', "/");
    let mut segments: Vec<String> = Vec::new();
    let parts: Vec<&str> = raw.strip_prefix('/').unwrap_or(&raw).split('/').collect();
    let last = parts.len().saturating_sub(1);
    for (i, seg) in parts.iter().enumerate() {
        if double_dot(seg) {
            segments.pop();
            if i == last {
                segments.push(String::new());
            }
        } else if single_dot(seg) {
            if i == last {
                segments.push(String::new());
            }
        } else {
            let mut s = String::new();
            for &b in seg.as_bytes() {
                if path_encode(b) {
                    let _ = write!(s, "%{b:02X}");
                } else {
                    s.push(b as char);
                }
            }
            segments.push(s);
        }
    }
    format!("/{}", segments.join("/"))
}

/// The pathname and the search params of a request's target, resolved
/// against XenoNet's own base: an origin-form path (/api/…), a path
/// starting with // (another host's, whose path is what's left), or an
/// absolute URL (its path).
pub fn parse_target(target: &str) -> (String, Vec<(String, String)>) {
    let target = target.split('#').next().unwrap_or("");
    let (before, query) = match target.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (target, None),
    };
    let lower = before.to_ascii_lowercase();
    let path = if let Some(rest) = lower.strip_prefix("http://").or_else(|| lower.strip_prefix("https://")).map(|r| &before[before.len() - r.len()..]) {
        rest.find(['/', '\\']).map_or("/", |i| &rest[i..]).to_string()
    } else if let Some(rest) = before.strip_prefix("//").or_else(|| before.strip_prefix("\\\\")) {
        rest.find(['/', '\\']).map_or("/", |i| &rest[i..]).to_string()
    } else if before.starts_with('/') || before.starts_with('\\') {
        before.to_string()
    } else {
        format!("/{before}")
    };
    (normalize_path(&path), query.map(parse_query).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_as_new_url_reads_them() {
        assert_eq!(parse_target("/api/graph").0, "/api/graph");
        assert_eq!(parse_target("/api/../x").0, "/x");
        assert_eq!(parse_target("/api/%2e%2e/x").0, "/x");
        assert_eq!(parse_target("/a/./b/.").0, "/a/b/");
        assert_eq!(parse_target("/a\\b").0, "/a/b");
        assert_eq!(parse_target("/a b\"<>").0, "/a%20b%22%3C%3E");
        assert_eq!(parse_target("//evil/x").0, "/x");
        assert_eq!(parse_target("http://other:1/y?z=1").0, "/y");
        assert_eq!(parse_target("/api/session/claude%3Aabc/stats").0, "/api/session/claude%3Aabc/stats");
        assert_eq!(parse_target("/é").0, "/%C3%A9");
    }

    #[test]
    fn queries_as_url_search_params_read_them() {
        let (_, q) = parse_target("/x?a=1&b=two+words&c=%E2%9C%93&d&a=2&e=%zz");
        assert_eq!(
            q,
            vec![
                ("a".into(), "1".into()),
                ("b".into(), "two words".into()),
                ("c".into(), "✓".into()),
                ("d".into(), String::new()),
                ("a".into(), "2".into()),
                ("e".into(), "%zz".into())
            ]
        );
    }

    #[test]
    fn decode_uri_component_refuses_what_js_refuses() {
        assert_eq!(decode_uri_component("claude%3Aabc").as_deref(), Some("claude:abc"));
        assert_eq!(decode_uri_component("%"), None);
        assert_eq!(decode_uri_component("%ff"), None);
    }

    #[tokio::test]
    async fn bodies_as_read_json_reads_them() {
        let mut r = Req::fake("POST", "/", &[], Some(""));
        assert_eq!(r.json_body().await, Ok(serde_json::json!({})));
        let mut r = Req::fake("POST", "/", &[], Some("{\"a\":1}"));
        assert_eq!(r.json(4).await, Err(TOO_BIG));
        let mut r = Req::fake("POST", "/", &[], Some("{"));
        assert_eq!(r.json_body().await, Err(NOT_JSON));
        let mut r = Req::fake("POST", "/", &[], Some("[1]"));
        assert_eq!(r.json_body().await, Ok(serde_json::json!([1])));
    }

    #[test]
    fn send_writes_json_with_the_api_headers() {
        let r = Reply::send(200, &serde_json::json!({ "a": 1.5 }), &[("Content-Type", "text/plain")]);
        assert_eq!(r.header("content-type"), Some("text/plain"));
        assert_eq!(r.header("cache-control"), Some("no-store"));
        assert_eq!(r.body_json(), serde_json::json!({ "a": 1.5 }));
    }
}
