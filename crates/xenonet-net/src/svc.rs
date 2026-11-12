//! Services through the network (docs/design/network.md, "Services through
//! the network"): a host's owner publishes a local HTTP service by name,
//! and any program on another host in the network reaches it through its
//! own XenoNet's local API, at `/api/svc/<host>/<name>/<path>`. XenoNet
//! knows nothing of what the service is: it carries the request and the
//! answer between the two hosts on svc/2 (`svc2`), one QUIC stream for
//! each, and checks them at both ends.
//!
//! - **Publishing** is the owner's (`xenonet svc publish`): a name, a
//!   target that is a loopback IP literal and a port over plain http, the
//!   path prefixes callers may reach, and, if the owner narrows it, the
//!   hosts that may call. It's kept with the network it's published in
//!   (`Persist`), and answers members of that network alone.
//! - **The caller's host** opens a stream to the publishing host's
//!   endpoint and waits on it for the answer, 90 s at most.
//! - **The publishing host** takes the caller to be the member whose
//!   endpoint id the connection authenticated, in the network the request
//!   names, by its own chain. It checks the name, the caller, a request id
//!   it hasn't seen from that caller (a stream sent again is answered 409
//!   and goes nowhere), its rate (300 a minute for each caller and
//!   service), the method, the path once normalized against the
//!   allowlist, and the body's size. It rebuilds the request with four of
//!   the caller's headers, `Host`, and a `Forwarded` naming the caller,
//!   and passes it to the target with no proxy and no redirects followed.
//! - **The answer** comes back without `Set-Cookie` and hop-by-hop headers.
//!
//! The service's own credential (`Authorization`) passes untouched, and
//! nothing here logs a header.

use crate::codec::random;
use crate::gateway::Bucket;
use crate::persist::Persist;
use crate::{NetError, R};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;
use xenonet_os::sync::{Locked, ReadWrite};

pub const SCHEMA: &str = "network/svc/v2";
/// A request's body, at most.
pub const REQUEST_BODY: usize = xenonet_model::network::SVC_REQUEST_BODY;
/// An answer's body, at most.
pub const ANSWER_BODY: usize = 8 << 20;
/// Requests a caller host may make to one service in a minute.
pub const A_MINUTE: f64 = 300.0;
/// How long a caller waits for the answer.
pub const TIMEOUT_MS: u64 = 90_000;
/// How long the publishing host waits on its target: under the caller's
/// wait, so the caller hears why.
const TARGET_MS: u64 = 75_000;
/// How long a request id is remembered from its caller: a stream that
/// names one again in that time is a replay.
pub const SEEN_MS: f64 = 15.0 * 60e3;
const SEEN_MAX: usize = 65_536;
/// Requests a host forwards to its services at once, at most, and from one
/// caller host: no one host holds every slot.
const IN_FLIGHT: usize = 32;
const IN_FLIGHT_EACH: usize = 8;
/// Requests a host waits on at once, at most, and to one host: one that
/// never answers holds a quarter of them at most, for 90 s each.
const WAITING: usize = 256;
const WAITING_EACH: usize = 64;
const MAX_SERVICES: usize = 32;
const MAX_PATHS: usize = 16;
const MAX_HOSTS: usize = 64;
const MAX_HEADERS: usize = 64;
const HEADER_VALUE: usize = 8 << 10;
const PATH_MAX: usize = 4096;
/// The caller's headers that reach the service; every other one is left
/// behind, `Forwarded`, `X-Forwarded-*`, `Via` and cookies among them.
pub const PASSED: [&str; 4] = ["authorization", "content-type", "accept", "user-agent"];
/// The methods a service is reached with.
pub const METHODS: [&str; 7] = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"];
/// Headers about one connection, never passed on (RFC 9110, 7.6.1).
const HOP_BY_HOP: [&str; 10] =
    ["connection", "keep-alive", "proxy-authenticate", "proxy-authorization", "proxy-connection", "te", "trailer", "trailers", "transfer-encoding", "upgrade"];
/// The header XenoNet's own refusals carry, with their code, so a caller
/// can tell them from the service's answers.
pub const REFUSED_HEADER: &str = xenonet_model::network::SVC_REFUSED_HEADER;

/// A service's name: 2 to 32 lowercase letters, digits and dashes,
/// starting with a letter, as a host id is.
pub fn is_name(name: &str) -> bool {
    xenonet_model::ids::is_host_id(name)
}

fn is_request_id(id: &str) -> bool {
    id.len() == 36 && id.starts_with("svq_") && id[4..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A new request's id: `svq_` and 32 hex digits.
pub fn request_id() -> R<String> {
    let bytes = random::<16>()?;
    Ok(bytes.iter().fold(String::from("svq_"), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

// ---------- the target ----------

/// Where a published service listens: a loopback IP literal, a port, and
/// a path its requests go under (empty, or `/base`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub ip: IpAddr,
    pub port: u16,
    pub prefix: String,
}

impl Target {
    /// `http://127.0.0.1:<port>[/prefix]` or `http://[::1]:<port>[/prefix]`:
    /// plain http, on this machine, by its IP. No name is looked up.
    pub fn parse(url: &str) -> Result<Target, String> {
        let shape = "a service's address is http://127.0.0.1:<port> or http://[::1]:<port>, with a path after it if it has one";
        let rest = url.trim().strip_prefix("http://").ok_or_else(|| format!("{shape}: XenoNet passes requests on to this machine only, over plain http"))?;
        let (auth, path) = rest.find('/').map_or((rest, ""), |i| (&rest[..i], &rest[i..]));
        if auth.contains('@') {
            return Err(format!("{shape}, with no user name or password in it"));
        }
        let (host, port) = if let Some(v6) = auth.strip_prefix('[') {
            let (h, p) = v6.split_once("]:").ok_or_else(|| shape.to_string())?;
            (h, p)
        } else {
            let (h, p) = auth.rsplit_once(':').ok_or_else(|| format!("{shape}: say the port"))?;
            if h.contains(':') {
                return Err(format!("{shape}: an IPv6 address in brackets"));
            }
            (h, p)
        };
        let ip = match host {
            "127.0.0.1" => IpAddr::V4(Ipv4Addr::LOCALHOST),
            "::1" => IpAddr::V6(Ipv6Addr::LOCALHOST),
            "localhost" => return Err(format!("{shape}: by its IP, since XenoNet looks no names up")),
            _ => return Err(format!("{shape}: a service is published from this machine's loopback address only")),
        };
        let port = Some(port)
            .filter(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse::<u16>().ok())
            .filter(|p| *p > 0)
            .ok_or_else(|| format!("{shape}: the port is 1 to 65535"))?;
        if path.contains(['?', '#']) {
            return Err(format!("{shape}: with no query or fragment"));
        }
        let prefix = path.trim_end_matches('/');
        if !prefix.is_empty() {
            if prefix.contains('%') || !prefix.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err(format!("{shape}: its path in plain letters, digits and punctuation"));
            }
            check_decoded(prefix).map_err(|why| format!("{shape}: {why}"))?;
        }
        Ok(Target { ip, port, prefix: prefix.to_string() })
    }

    /// `127.0.0.1:<port>` or `[::1]:<port>`: the `Host` it's sent.
    pub fn authority(&self) -> String {
        match self.ip {
            IpAddr::V4(ip) => format!("{ip}:{}", self.port),
            IpAddr::V6(ip) => format!("[{ip}]:{}", self.port),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}{}", self.authority(), self.prefix)
    }
}

// ---------- paths ----------

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decoding, strictly: a % without two hex digits is refused, and
/// so is what isn't UTF-8 once decoded.
fn decode(raw: &str) -> Result<String, &'static str> {
    let b = raw.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let (Some(x), Some(y)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) else {
                return Err("a % in the path isn't followed by two hex digits");
            };
            out.push(x * 16 + y);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| "the path isn't UTF-8 once decoded")
}

/// What a decoded path may not hold: anything that a server might read as
/// another path than the allowlist matched.
fn check_decoded(p: &str) -> Result<(), &'static str> {
    if !p.starts_with('/') {
        return Err("a path starts with /");
    }
    if p.chars().any(char::is_control) {
        return Err("the path has a control character");
    }
    if p.contains('\\') {
        return Err("the path has a \\");
    }
    if p.contains(';') {
        return Err("the path has a ;");
    }
    if p.contains("//") {
        return Err("the path has //");
    }
    if p.split('/').any(|s| s == "." || s == "..") {
        return Err("the path has a . or .. segment");
    }
    Ok(())
}

/// The decoded path, percent-encoded again the one way: what the service
/// is sent is what the allowlist matched.
fn encode(decoded: &str) -> String {
    let mut out = String::with_capacity(decoded.len());
    for b in decoded.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/!$&'()*+,=:@".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// A request's path, checked: decoded for matching, encoded again for the
/// service, and its query as it came.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normal {
    pub decoded: String,
    pub path: String,
    pub query: Option<String>,
}

impl Normal {
    /// The path and query to send the service, under its prefix.
    pub fn target(&self, prefix: &str) -> String {
        match self.query.as_deref().filter(|q| !q.is_empty()) {
            Some(q) => format!("{prefix}{}?{q}", self.path),
            None => format!("{prefix}{}", self.path),
        }
    }
}

/// The path after `/api/svc/<host>/<name>`, with its query: the query is
/// split off first and passed on as it is; the path is percent-decoded,
/// refused with `..`, `.`, `//`, `;`, `\` or a control character in it,
/// and encoded again.
pub fn normalize(rest: &str) -> Result<Normal, &'static str> {
    if rest.len() > PATH_MAX {
        return Err("the path is over 4 KiB");
    }
    if !rest.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("the path has a space, a control character or a letter outside ASCII: percent-encode it");
    }
    if rest.contains('#') {
        return Err("the path has a #");
    }
    let (raw, query) = match rest.split_once('?') {
        Some((p, q)) => (p, Some(q.to_string())),
        None => (rest, None),
    };
    let decoded = decode(raw)?;
    check_decoded(&decoded)?;
    let path = encode(&decoded);
    // Encoding again can make a path longer (a " is sent as %22): what's
    // sent is held to the same 4 KiB.
    if path.len() + query.as_ref().map_or(0, |q| q.len() + 1) > PATH_MAX {
        return Err("the path is over 4 KiB once encoded");
    }
    Ok(Normal { decoded, path, query })
}

/// An allowlist entry: a path prefix that starts and ends with /.
fn check_allowed(p: &str) -> Result<(), String> {
    if !(p.starts_with('/') && p.ends_with('/')) || p.len() > 200 {
        return Err(format!("{p}: a path a service may be reached at starts and ends with /, as /api/v1/"));
    }
    if p.contains('%') || !p.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(format!("{p}: say the path in plain letters, digits and punctuation"));
    }
    check_decoded(p).map_err(|why| format!("{p}: {why}"))
}

// ---------- headers ----------

fn valid_name(n: &str) -> bool {
    !n.is_empty() && n.len() <= 64 && n.bytes().all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// A value hyper takes as it is: visible ASCII, spaces and tabs.
fn valid_value(v: &str) -> bool {
    v.len() <= HEADER_VALUE && v.bytes().all(|b| b == b'\t' || (0x20..=0x7e).contains(&b))
}

/// The caller's headers that cross: the four in `PASSED`, the first of
/// each, as they came.
pub fn passed(given: &[(String, String)]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for name in PASSED {
        if let Some((_, v)) = given.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
            if valid_value(v) {
                out.push((name.to_string(), v.clone()));
            }
        }
    }
    out
}

/// The headers the service is sent: the four the caller may set, then
/// `Host` for the target and `Forwarded` naming the caller's host, as the
/// frame's signature proved it. Nothing else the caller sent crosses.
pub fn request_headers(given: &[(String, String)], authority: &str, caller: &str) -> Vec<(String, String)> {
    let mut out = passed(given);
    out.push(("host".into(), authority.into()));
    out.push(("forwarded".into(), format!("by=xenonet;for={caller}")));
    out
}

/// An answer's headers, as they go back: no `Set-Cookie`, nothing about
/// one connection (hop-by-hop, and what `Connection` names), no length,
/// which is the body's own, and only what hyper takes as it is.
pub fn answer_headers(given: &[(String, String)]) -> Vec<(String, String)> {
    let named: Vec<String> =
        given.iter().filter(|(k, _)| k.eq_ignore_ascii_case("connection")).flat_map(|(_, v)| v.split(',').map(|t| t.trim().to_ascii_lowercase()).collect::<Vec<_>>()).collect();
    given
        .iter()
        .map(|(k, v)| (k.to_ascii_lowercase(), v))
        .filter(|(k, v)| {
            valid_name(k) && valid_value(v) && !HOP_BY_HOP.contains(&k.as_str()) && !matches!(k.as_str(), "set-cookie" | "set-cookie2" | "content-length") && !named.contains(k)
        })
        .take(MAX_HEADERS)
        .map(|(k, v)| (k, v.clone()))
        .collect()
}

fn pairs(v: Option<&Value>) -> Result<Vec<(String, String)>, String> {
    let list = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => return Err("headers are a list of [name, value] pairs".into()),
    };
    if list.len() > MAX_HEADERS {
        return Err(format!("at most {MAX_HEADERS} headers"));
    }
    list.iter()
        .map(|p| match p.as_array().map(Vec::as_slice) {
            Some([Value::String(k), Value::String(v)]) => Ok((k.clone(), v.clone())),
            _ => Err("headers are a list of [name, value] pairs".to_string()),
        })
        .collect()
}

fn pairs_json(h: &[(String, String)]) -> Value {
    Value::Array(h.iter().map(|(k, v)| json!([k, v])).collect())
}

// ---------- the heads ----------

/// What a program on the caller's host asked for: the path is the part
/// after `/api/svc/<host>/<name>`, with its query.
#[derive(Debug, Clone, Default)]
pub struct Call {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// A request on a svc/2 stream (`network/svc/v2`, op `request`): its head,
/// then `length` bytes of body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: String,
    pub name: String,
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The head that goes before the body, in `network`.
    pub fn head(&self, network: &str) -> Value {
        json!({
            "schema": SCHEMA,
            "op": "request",
            "network": network,
            "id": self.id,
            "name": self.name,
            "method": self.method,
            "path": self.path,
            "headers": pairs_json(&self.headers),
            "length": self.body.len(),
        })
    }

    /// A request's head: the request with no body yet, and the body's
    /// length, which is at most `REQUEST_BODY`.
    pub fn parse_head(v: &Value) -> Result<(Request, usize), String> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        if text("schema") != Some(SCHEMA) || text("op") != Some("request") {
            return Err("that isn't a service's request".into());
        }
        let id = text("id").filter(|i| is_request_id(i)).ok_or("a request's id is svq_ and 32 hex digits")?;
        let name = text("name").filter(|n| is_name(n)).ok_or("a service's name is 2 to 32 lowercase letters, digits and dashes")?;
        let method = text("method").filter(|m| !m.is_empty() && m.len() <= 16 && m.bytes().all(|b| b.is_ascii_uppercase())).ok_or("a request's method is a word in capitals")?;
        let path = text("path").filter(|p| p.len() <= PATH_MAX).ok_or("a request's path is at most 4 KiB")?;
        let headers = pairs(v.get("headers"))?;
        let length = length_of(v)?;
        Ok((Request { id: id.into(), name: name.into(), method: method.into(), path: path.into(), headers, body: Vec::new() }, length))
    }
}

/// The body length a head names: a whole number of bytes.
fn length_of(v: &Value) -> Result<usize, String> {
    v.get("length").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).ok_or_else(|| "a head's length is a whole number of bytes".to_string())
}

/// The network a svc/2 head names.
pub fn network_of(v: &Value) -> Option<&str> {
    v.get("network").and_then(Value::as_str).filter(|n| crate::codec::is_network_id(n))
}

/// A service's answer, or XenoNet's refusal (with its `code`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Why XenoNet answered rather than the service.
    pub code: Option<String>,
}

impl Answer {
    /// XenoNet's own answer: `{ error, code }`, with `X-XenoNet-Refused`.
    pub fn refused(status: u16, code: &str, message: impl Into<String>) -> Answer {
        let (headers, body) = xenonet_model::network::svc_refusal(code, &message.into());
        Answer { status, headers, body, code: Some(code.into()) }
    }

    /// A JSON answer, as a window read's is.
    pub fn json(status: u16, body: &Value) -> Answer {
        Answer { status, headers: vec![("content-type".into(), "application/json; charset=utf-8".into())], body: body.to_string().into_bytes(), code: None }
    }

    /// The words of a refusal, for the log.
    pub fn message(&self) -> String {
        serde_json::from_slice::<Value>(&self.body).ok().and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string)).unwrap_or_default()
    }

    /// The head that goes before the body.
    pub fn head(&self) -> Value {
        let mut v = json!({
            "schema": SCHEMA,
            "op": "answer",
            "status": self.status,
            "headers": pairs_json(&self.headers),
            "length": self.body.len(),
        });
        if let Some(c) = &self.code {
            v["code"] = json!(c);
        }
        v
    }

    /// An answer's head, as it may reach a program here, cleaned again,
    /// with no body yet, and the body's length.
    pub fn parse_head(v: &Value) -> Result<(Answer, usize), String> {
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        if text("schema") != Some(SCHEMA) || text("op") != Some("answer") {
            return Err("that isn't a service's answer".into());
        }
        let status = v.get("status").and_then(Value::as_u64).filter(|s| (200..=599).contains(s)).ok_or("an answer's status is 200 to 599")? as u16;
        let headers = answer_headers(&pairs(v.get("headers"))?);
        let length = length_of(v)?;
        let code = text("code").filter(|c| c.len() <= 32 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')).map(str::to_string);
        Ok((Answer { status, headers, body: Vec::new(), code }, length))
    }
}

// ---------- what's published ----------

/// A service this host published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub name: String,
    pub target: Target,
    /// Path prefixes callers may reach, each starting and ending with /.
    pub paths: Vec<String>,
    /// The hosts that may call it; every host in the roster when empty.
    pub hosts: Vec<String>,
    pub at: i64,
}

impl Published {
    pub fn new(name: &str, target: &str, paths: &[String], hosts: &[String], at: i64) -> Result<Published, String> {
        if !is_name(name) {
            return Err("a service's name is 2 to 32 lowercase letters, digits and dashes, starting with a letter".into());
        }
        let target = Target::parse(target)?;
        if paths.is_empty() {
            return Err("say the paths callers may reach, as --paths /api/v1/: nothing else of the service is reachable".into());
        }
        if paths.len() > MAX_PATHS {
            return Err(format!("at most {MAX_PATHS} paths"));
        }
        for p in paths {
            check_allowed(p)?;
        }
        if hosts.len() > MAX_HOSTS {
            return Err(format!("at most {MAX_HOSTS} hosts"));
        }
        if let Some(h) = hosts.iter().find(|h| !xenonet_model::ids::is_host_id(h)) {
            return Err(format!("{h} isn't a host id"));
        }
        let mut paths = paths.to_vec();
        paths.sort();
        paths.dedup();
        let mut hosts = hosts.to_vec();
        hosts.sort();
        hosts.dedup();
        Ok(Published { name: name.into(), target, paths, hosts, at })
    }

    pub fn to_json(&self) -> Value {
        json!({ "name": self.name, "target": self.target.url(), "paths": self.paths, "hosts": self.hosts, "at": self.at })
    }

    fn from_json(v: &Value) -> Option<Published> {
        let list = |k: &str| v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
        Published::new(v.get("name")?.as_str()?, v.get("target")?.as_str()?, &list("paths"), &list("hosts"), v.get("at").and_then(Value::as_i64).unwrap_or(0)).ok()
    }

    /// Whether `host` may call it.
    pub fn allows(&self, host: &str) -> bool {
        self.hosts.is_empty() || self.hosts.iter().any(|h| h == host)
    }

    /// Whether a decoded path is under one of its paths.
    pub fn reaches(&self, decoded: &str) -> bool {
        self.paths.iter().any(|p| decoded.starts_with(p.as_str()))
    }
}

// ---------- rates ----------

/// Each caller's requests to each service, 300 a minute.
pub struct Limiter {
    per_minute: f64,
    /// Each caller's bucket for each service, and when it was last used.
    buckets: Mutex<HashMap<(String, String), (Bucket, f64)>>,
}

impl Limiter {
    pub fn new(per_minute: f64) -> Limiter {
        Limiter { per_minute, buckets: Mutex::new(HashMap::new()) }
    }

    /// Takes one for `caller` and `name`, or says how many ms until there's
    /// one.
    pub fn take(&self, caller: &str, name: &str, now: f64) -> Result<(), f64> {
        let mut all = self.buckets.locked();
        if all.len() > 1024 {
            // A bucket left alone for a minute is full again: as good as none.
            all.retain(|_, (_, last)| now - *last < 60e3);
        }
        let (bucket, last) = all.entry((caller.to_string(), name.to_string())).or_insert_with(|| (Bucket::full(self.per_minute, now), now));
        *last = now;
        bucket.take(self.per_minute, now)
    }
}

// ---------- the services of this host ----------

/// This host's services: what it published, and the requests it makes and
/// serves.
pub struct Services {
    persist: Arc<dyn Persist>,
    /// The network the services are published to, while there's one.
    network: RwLock<Option<String>>,
    published: RwLock<Vec<Published>>,
    limiter: Limiter,
    /// Requests being forwarded, by caller host.
    in_flight: Mutex<HashMap<String, usize>>,
    /// Requests this host waits on, by the host asked.
    waiting: Mutex<HashMap<String, usize>>,
    /// The request ids each caller sent lately, and when.
    seen: Mutex<HashMap<(String, String), f64>>,
    client: OnceLock<Result<reqwest::Client, String>>,
    /// When each refusal was last logged, so a busy caller can't fill the
    /// log.
    said: Mutex<HashMap<String, f64>>,
}

/// One request counted while it lasts, by host: one being forwarded for a
/// caller, or one this host waits on.
pub struct Flight<'a> {
    counts: &'a Mutex<HashMap<String, usize>>,
    host: String,
}

impl Drop for Flight<'_> {
    fn drop(&mut self) {
        let mut all = self.counts.locked();
        if let Some(n) = all.get_mut(&self.host) {
            *n -= 1;
            if *n == 0 {
                all.remove(&self.host);
            }
        }
    }
}

/// A slot in `counts` for `host`, while there are fewer than `all` in all
/// and `each` for it; the refusal says which (`words(true)`: all of them).
fn slot<'a>(counts: &'a Mutex<HashMap<String, usize>>, host: &str, all: usize, each: usize, words: impl Fn(bool) -> String) -> Result<Flight<'a>, Answer> {
    let mut c = counts.locked();
    if c.values().sum::<usize>() >= all {
        return Err(Answer::refused(503, "busy", words(true)));
    }
    let n = c.entry(host.to_string()).or_insert(0);
    if *n >= each {
        return Err(Answer::refused(503, "busy", words(false)));
    }
    *n += 1;
    Ok(Flight { counts, host: host.to_string() })
}

impl Services {
    /// The services kept for `network`, if it's in one.
    pub fn open(persist: Arc<dyn Persist>, network: Option<String>) -> Services {
        let s = Services {
            persist,
            network: RwLock::new(None),
            published: RwLock::new(Vec::new()),
            limiter: Limiter::new(A_MINUTE),
            in_flight: Mutex::new(HashMap::new()),
            waiting: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            client: OnceLock::new(),
            said: Mutex::new(HashMap::new()),
        };
        s.load(network);
        s
    }

    /// What's kept for `network`, in place of what was here.
    pub fn load(&self, network: Option<String>) {
        let list = network.as_deref().and_then(|n| self.persist.services(n).ok()).map(|l| l.iter().filter_map(Published::from_json).collect()).unwrap_or_default();
        *self.network.writing() = network;
        *self.published.writing() = list;
    }

    /// The network was left: its services go with it (what was kept of it
    /// went).
    pub fn forget(&self) {
        *self.network.writing() = None;
        self.published.writing().clear();
    }

    fn save(&self, list: &[Published]) -> R<()> {
        let network = self.network.reading().clone().ok_or_else(|| NetError::new(409, "no-network", "this machine isn't in a XenoNet network"))?;
        self.persist.put_services(&network, &list.iter().map(Published::to_json).collect::<Vec<_>>())
    }

    pub fn list(&self) -> Vec<Published> {
        self.published.reading().clone()
    }

    pub fn get(&self, name: &str) -> Option<Published> {
        self.published.reading().iter().find(|p| p.name == name).cloned()
    }

    /// Publishes `p`, in place of one by its name.
    pub fn publish(&self, p: Published) -> R<()> {
        let mut list = self.published.writing();
        let mut next = list.clone();
        next.retain(|x| x.name != p.name);
        if next.len() >= MAX_SERVICES {
            return Err(NetError::bad(format!("a host publishes at most {MAX_SERVICES} services")));
        }
        next.push(p);
        next.sort_by(|a, b| a.name.cmp(&b.name));
        self.save(&next)?;
        *list = next;
        Ok(())
    }

    /// false when nothing was published by that name.
    pub fn unpublish(&self, name: &str) -> R<bool> {
        let mut list = self.published.writing();
        if !list.iter().any(|p| p.name == name) {
            return Ok(false);
        }
        let next: Vec<Published> = list.iter().filter(|p| p.name != name).cloned().collect();
        self.save(&next)?;
        *list = next;
        Ok(true)
    }

    // ---------- the caller's side ----------

    /// A slot to wait on an answer from `host` in, while there's one.
    pub fn wait(&self, host: &str) -> Result<Flight<'_>, Answer> {
        slot(&self.waiting, host, WAITING, WAITING_EACH, |all| {
            if all {
                format!("this machine waits on {WAITING} services' answers already")
            } else {
                format!("this machine waits on {WAITING_EACH} answers from {host} already")
            }
        })
    }

    pub fn waiting(&self) -> usize {
        self.waiting.locked().values().sum()
    }

    // ---------- the publishing host's side ----------

    /// Takes one of `caller`'s requests to `name` this minute, or says when
    /// there's one (the window's reads count here too, as `window`).
    pub fn rate(&self, caller: &str, name: &str, now: f64) -> Result<(), Answer> {
        self.limiter.take(caller, name, now).map_err(|wait| {
            Answer::refused(429, "rate", format!("at most {} requests a minute from {caller} to {name}: try again in {} s", A_MINUTE as u32, (wait / 1000.0).ceil()))
        })
    }

    /// Whether `caller` sent request `id` in the last 15 minutes: it's
    /// taken as sent now when it didn't.
    fn replayed(&self, caller: &str, id: &str, now: f64) -> bool {
        let mut seen = self.seen.locked();
        if seen.len() >= SEEN_MAX {
            seen.retain(|_, at| now - *at < SEEN_MS);
        }
        let key = (caller.to_string(), id.to_string());
        if seen.get(&key).is_some_and(|at| now - *at < SEEN_MS) {
            return true;
        }
        seen.insert(key, now);
        false
    }

    /// Whether to log a refusal of `key` now: once a minute at most.
    pub fn say(&self, key: &str, now: f64) -> bool {
        let mut said = self.said.locked();
        if said.len() > 256 {
            said.retain(|_, t| now - *t < 60e3);
        }
        if said.get(key).is_some_and(|t| now - *t < 60e3) {
            return false;
        }
        said.insert(key.to_string(), now);
        true
    }

    /// What may be forwarded of a request from `caller`, checked in order:
    /// the service, the caller, its rate, the method, the path and the
    /// body. The service and the path to send, or the refusal.
    pub fn check(&self, caller: &str, req: &Request, now: f64) -> Result<(Published, Normal), Answer> {
        let Some(p) = self.get(&req.name) else { return Err(Answer::refused(404, "not-published", format!("this host publishes no service named {}", req.name))) };
        if !p.allows(caller) {
            return Err(Answer::refused(403, "not-allowed", format!("{caller} may not reach {} on this host", p.name)));
        }
        if self.replayed(caller, &req.id, now) {
            return Err(Answer::refused(409, "replayed", format!("{caller} sent request {} before: it goes to {} once", req.id, p.name)));
        }
        self.rate(caller, &p.name, now)?;
        if !METHODS.contains(&req.method.as_str()) {
            return Err(Answer::refused(405, "method", format!("{} isn't passed on: a service is reached with {}", req.method, METHODS.join(", "))));
        }
        let n = normalize(&req.path).map_err(|why| Answer::refused(400, "bad-path", why))?;
        if !p.reaches(&n.decoded) {
            return Err(Answer::refused(403, "path-refused", format!("{} is reached at {} only", p.name, p.paths.join(", "))));
        }
        if req.body.len() > REQUEST_BODY {
            return Err(Answer::refused(413, "too-big", "a request's body is at most 8 MiB"));
        }
        Ok((p, n))
    }

    fn client(&self) -> Result<&reqwest::Client, String> {
        self.client
            .get_or_init(|| {
                let _ = rustls::crypto::ring::default_provider().install_default();
                reqwest::Client::builder()
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .connect_timeout(Duration::from_secs(5))
                    .pool_idle_timeout(Duration::from_secs(30))
                    .build()
                    .map_err(|e| format!("an HTTP client: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// A request from `caller` (the member whose endpoint id the
    /// connection authenticated), checked and passed to its target: the
    /// target's answer, or why there's none.
    pub async fn serve(&self, caller: &str, req: Request, now: f64) -> Answer {
        let (p, n) = match self.check(caller, &req, now) {
            Ok(x) => x,
            Err(a) => return a,
        };
        let _flight = match self.fly(caller) {
            Ok(f) => f,
            Err(a) => return a,
        };
        self.forward(&p, caller, &req, &n).await
    }

    /// A slot to forward a request from `caller` in, while there's one.
    fn fly(&self, caller: &str) -> Result<Flight<'_>, Answer> {
        slot(&self.in_flight, caller, IN_FLIGHT, IN_FLIGHT_EACH, |all| {
            if all {
                format!("this host passes {IN_FLIGHT} requests to its services at once, at most")
            } else {
                format!("this host passes {IN_FLIGHT_EACH} requests from {caller} to its services at once, at most")
            }
        })
    }

    async fn forward(&self, p: &Published, caller: &str, req: &Request, n: &Normal) -> Answer {
        let client = match self.client() {
            Ok(c) => c,
            Err(why) => return Answer::refused(500, "internal", why),
        };
        let Ok(method) = reqwest::Method::from_bytes(req.method.as_bytes()) else { return Answer::refused(405, "method", "that method isn't passed on") };
        let authority = p.target.authority();
        let url = format!("http://{authority}{}", n.target(&p.target.prefix));
        let mut r = client.request(method, url).timeout(Duration::from_millis(TARGET_MS));
        for (k, v) in request_headers(&req.headers, &authority, caller) {
            r = r.header(k, v);
        }
        if !req.body.is_empty() {
            r = r.body(req.body.clone());
        }
        let down = |e: &reqwest::Error| {
            if e.is_timeout() {
                Answer::refused(504, "timeout", format!("{} on this host didn't answer within {} s", p.name, TARGET_MS / 1000))
            } else {
                Answer::refused(502, "unreachable", format!("{} on this host didn't answer: is it running at {}?", p.name, p.target.url()))
            }
        };
        let mut res = match r.send().await {
            Ok(res) => res,
            Err(e) => return down(&e),
        };
        let status = res.status().as_u16();
        let headers: Vec<(String, String)> = res.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str().to_string(), v.to_string()))).collect();
        let mut body = Vec::new();
        loop {
            match res.chunk().await {
                Ok(Some(c)) => {
                    if body.len() + c.len() > ANSWER_BODY {
                        return Answer::refused(413, "answer-too-big", format!("{}'s answer is over 8 MiB", p.name));
                    }
                    body.extend_from_slice(&c);
                }
                Ok(None) => break,
                Err(e) => return down(&e),
            }
        }
        if !(200..=599).contains(&status) {
            return Answer::refused(502, "bad-answer", format!("{} answered with status {status}", p.name));
        }
        Answer { status, headers: answer_headers(&headers), body, code: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn paths_are_decoded_checked_and_encoded_one_way() {
        let n = normalize("/api/v1/spaces/project/atlas/pages?since=2&x=%2F").unwrap();
        assert_eq!((n.decoded.as_str(), n.path.as_str(), n.query.as_deref()), ("/api/v1/spaces/project/atlas/pages", "/api/v1/spaces/project/atlas/pages", Some("since=2&x=%2F")));
        // An encoded slash is a slash once decoded, and is sent as one: the
        // service is sent the path the allowlist matched.
        let n = normalize("/api%2Fv1/x").unwrap();
        assert_eq!((n.decoded.as_str(), n.path.as_str()), ("/api/v1/x", "/api/v1/x"));
        assert_eq!(normalize("/a%20b/%C3%A9").unwrap().path, "/a%20b/%C3%A9");
        assert_eq!(normalize("/a%3Fb").unwrap().path, "/a%3Fb", "a ? that was encoded stays in the path");
        assert_eq!(normalize("/x?").unwrap().target("/base"), "/base/x", "an empty query goes");
        assert_eq!(normalize("/x?a=1").unwrap().target(""), "/x?a=1");
        for bad in [
            "/api/v1/../admin",
            "/api/v1/%2e%2e/admin",
            "/api/v1/%2E%2e/admin",
            "/api/v1%2F..%2Fadmin",
            "/api/v1/./x",
            "/api//v1/",
            "/api/%2F/v1",
            "/api/v1;a=b",
            "/api/v1/%3Bx",
            "/api\\v1",
            "/api/v1/%5Cx",
            "/api/v1/%00",
            "/api/v1/%0a",
            "/api/v1/%zz",
            "/api/v1/%4",
            "/api/v1/%ff",
            "/api/v1/a b",
            "/api/v1/é",
            "/api/v1/#x",
            "api/v1/",
            "",
        ] {
            assert!(normalize(bad).is_err(), "{bad:?}");
        }
        assert!(normalize(&format!("/{}", "a".repeat(4096))).is_err());
        // Short as it came, but over 4 KiB once encoded again.
        let quoted = format!("/{}", "\"".repeat(2000));
        assert_eq!(normalize(&quoted), Err("the path is over 4 KiB once encoded"));
        assert_eq!(normalize(&format!("/{}", "\"".repeat(1000))).unwrap().path, format!("/{}", "%22".repeat(1000)));
        // The query isn't decoded: what's in it isn't a path.
        assert_eq!(normalize("/x?a=..%2F..").unwrap().query.as_deref(), Some("a=..%2F.."));
    }

    #[test]
    fn the_allowlist_matches_decoded_paths_by_prefix() {
        let p = Published::new("files", "http://127.0.0.1:4790", &["/api/v1/".into()], &[], 0).unwrap();
        for (path, ok) in [("/api/v1/spaces", true), ("/api/v1/", true), ("/api/v1", false), ("/api/v2/x", false), ("/mcp", false), ("/api/held", false), ("/api/v1x/", false)] {
            assert_eq!(p.reaches(&normalize(path).unwrap().decoded), ok, "{path}");
        }
        assert!(p.reaches(&normalize("/api%2Fv1/x").unwrap().decoded));
        assert!(!p.reaches(&normalize("/%73ync/v2/").unwrap().decoded));
        for bad in ["api/", "/api", "/api/../", "/a//", "/a;b/", "/%2e/", "/a b/"] {
            assert!(Published::new("x1", "http://127.0.0.1:1", &[bad.into()], &[], 0).is_err(), "{bad}");
        }
        assert!(Published::new("x1", "http://127.0.0.1:1", &[], &[], 0).is_err(), "say the paths");
        assert!(Published::new("x1", "http://127.0.0.1:1", &["/".into()], &[], 0).is_ok(), "all of it, when the owner says so");
    }

    #[test]
    fn targets_are_loopback_literals_over_http() {
        let t = Target::parse("http://127.0.0.1:4790").unwrap();
        assert_eq!((t.authority(), t.prefix.as_str(), t.url()), ("127.0.0.1:4790".into(), "", "http://127.0.0.1:4790".into()));
        let t = Target::parse("http://[::1]:8080/base/").unwrap();
        assert_eq!((t.authority(), t.prefix.as_str(), t.url()), ("[::1]:8080".into(), "/base", "http://[::1]:8080/base".into()));
        for bad in [
            "https://127.0.0.1:4790",
            "http://localhost:4790",
            "http://127.0.0.2:4790",
            "http://192.168.1.5:4790",
            "http://example.com:80",
            "http://127.0.0.1",
            "http://127.0.0.1:",
            "http://127.0.0.1:0",
            "http://127.0.0.1:65536",
            "http://127.0.0.1:+80",
            "http://user:pw@127.0.0.1:80",
            "http://127.0.0.1:80/a?b",
            "http://127.0.0.1:80/a#b",
            "http://127.0.0.1:80/../x",
            "http://127.0.0.1:80/a%2F",
            "http://[::2]:80",
            "http://[::1]80",
            "ftp://127.0.0.1:21",
            "127.0.0.1:80",
        ] {
            assert!(Target::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_service_is_sent_four_headers_its_host_and_who_called() {
        let given = h(&[
            ("Authorization", "Bearer files_0123"),
            ("content-type", "application/json"),
            ("accept", "application/json"),
            ("user-agent", "files/1.0"),
            ("forwarded", "for=evil"),
            ("x-forwarded-for", "10.0.0.1"),
            ("x-forwarded-host", "evil.example"),
            ("via", "1.1 evil"),
            ("cookie", "a=b"),
            ("host", "evil.example"),
            ("x-xenonet-window", "the-window-key"),
            ("authorization", "Bearer second"),
        ]);
        assert_eq!(
            request_headers(&given, "127.0.0.1:4790", "laptop"),
            h(&[
                ("authorization", "Bearer files_0123"),
                ("content-type", "application/json"),
                ("accept", "application/json"),
                ("user-agent", "files/1.0"),
                ("host", "127.0.0.1:4790"),
                ("forwarded", "by=xenonet;for=laptop"),
            ])
        );
        // A value that would break the request line or hyper is left out.
        assert_eq!(passed(&h(&[("accept", "a\r\nx-evil: 1"), ("user-agent", "ü")])), Vec::new());
    }

    #[test]
    fn answers_go_back_without_cookies_or_hop_by_hop_headers() {
        let given = h(&[
            ("Content-Type", "application/json"),
            ("Set-Cookie", "session=1"),
            ("set-cookie", "other=2"),
            ("Connection", "close, X-Private"),
            ("x-private", "p"),
            ("Keep-Alive", "timeout=5"),
            ("Transfer-Encoding", "chunked"),
            ("Upgrade", "h2c"),
            ("Content-Length", "12"),
            ("WWW-Authenticate", "Bearer realm=\"x\""),
            ("x-bad name", "1"),
            ("x-ok", "a\u{7f}"),
        ]);
        assert_eq!(answer_headers(&given), h(&[("content-type", "application/json"), ("www-authenticate", "Bearer realm=\"x\"")]));
    }

    #[test]
    fn heads_cross_and_are_checked() {
        let req = Request {
            id: request_id().unwrap(),
            name: "files".into(),
            method: "POST".into(),
            path: "/api/v1/x?y=1".into(),
            headers: h(&[("authorization", "Bearer t")]),
            body: b"{\"a\":1}".to_vec(),
        };
        let head = req.head("net_00112233445566778899aabbccddeeff");
        assert_eq!(network_of(&head), Some("net_00112233445566778899aabbccddeeff"));
        let (back, length) = Request::parse_head(&head).unwrap();
        assert_eq!((Request { body: req.body.clone(), ..back }, length), (req.clone(), 7));
        for (k, v) in [("id", json!("svq_x")), ("name", json!("Bad")), ("method", json!("get")), ("headers", json!([["a"]])), ("length", json!(-1)), ("op", json!("answer"))] {
            let mut bad = head.clone();
            bad[k] = v;
            assert!(Request::parse_head(&bad).is_err(), "{k}");
        }
        let a = Answer { status: 201, headers: h(&[("content-type", "text/plain"), ("set-cookie", "x=1")]), body: b"made".to_vec(), code: None };
        let (back, length) = Answer::parse_head(&a.head()).unwrap();
        assert_eq!((back.status, back.headers, length), (201, h(&[("content-type", "text/plain")]), 4), "cleaned again on the way in");
        let mut bad = a.head();
        bad["status"] = json!(101);
        assert!(Answer::parse_head(&bad).is_err(), "no switching protocols");
        let r = Answer::refused(503, "offline", "studio is offline");
        let (back, _) = Answer::parse_head(&r.head()).unwrap();
        assert_eq!(back.code.as_deref(), Some("offline"));
        assert_eq!(r.message(), "studio is offline");
        assert!(back.headers.contains(&(REFUSED_HEADER.to_string(), "offline".to_string())));
    }

    #[test]
    fn a_caller_makes_300_requests_a_minute_to_each_service() {
        let l = Limiter::new(A_MINUTE);
        let t = 1_000_000.0;
        for i in 0..300 {
            assert!(l.take("laptop", "files", t).is_ok(), "{i}");
        }
        let wait = l.take("laptop", "files", t).unwrap_err();
        assert!((wait - 200.0).abs() < 1.0, "one more in 0.2 s: {wait}");
        assert!(l.take("desk", "files", t).is_ok(), "each caller has its own");
        assert!(l.take("laptop", "other", t).is_ok(), "and each service");
        assert!(l.take("laptop", "files", t + 200.0).is_ok());
        assert!(l.take("laptop", "files", t + 200.0).is_err());
        assert!(l.take("laptop", "files", t + 60e3).is_ok());
    }

    // What a machine in a network keeps, in memory.
    fn in_a_network() -> Arc<dyn Persist> {
        let kept = crate::persist::Memory::default();
        let gateway = crate::files::Pin { host: "studio".into(), key_id: "k".into(), sign: "s".into(), seal: "x".into(), address: None };
        let net = crate::files::NetFile {
            id: "net_0".into(),
            name: "home".into(),
            host: "studio".into(),
            founder: "studio".into(),
            gateway,
            lan: None,
            titles_only: false,
            share_load: true,
            joined: 0,
        };
        kept.put_network(&net).unwrap();
        Arc::new(kept)
    }

    #[test]
    fn requests_are_checked_in_order_before_they_go() {
        let kept = in_a_network();
        let s = Services::open(kept.clone(), Some("net_0".into()));
        s.publish(Published::new("files", "http://127.0.0.1:4790", &["/api/v1/".into()], &["laptop".into()], 0).unwrap()).unwrap();
        let req = |name: &str, method: &str, path: &str, body: usize| Request {
            id: request_id().unwrap(),
            name: name.into(),
            method: method.into(),
            path: path.into(),
            headers: Vec::new(),
            body: vec![b'x'; body],
        };
        let code = |r: Result<(Published, Normal), Answer>| r.err().map(|a| (a.status, a.code.unwrap_or_default()));
        assert_eq!(code(s.check("laptop", &req("other", "GET", "/api/v1/", 0), 0.0)), Some((404, "not-published".into())));
        assert_eq!(code(s.check("desk", &req("files", "GET", "/api/v1/", 0), 0.0)), Some((403, "not-allowed".into())));
        assert_eq!(code(s.check("laptop", &req("files", "TRACE", "/api/v1/", 0), 0.0)), Some((405, "method".into())));
        assert_eq!(code(s.check("laptop", &req("files", "GET", "/api/v1/../../mcp", 0), 0.0)), Some((400, "bad-path".into())));
        assert_eq!(code(s.check("laptop", &req("files", "GET", "/mcp", 0), 0.0)), Some((403, "path-refused".into())));
        assert_eq!(code(s.check("laptop", &req("files", "POST", "/api/v1/x", REQUEST_BODY + 1), 0.0)), Some((413, "too-big".into())));
        let (p, n) = s.check("laptop", &req("files", "POST", "/api/v1/x?a=1", REQUEST_BODY), 0.0).unwrap();
        assert_eq!((p.name.as_str(), n.target(&p.target.prefix)), ("files", "/api/v1/x?a=1".to_string()));
        // Kept, and read again.
        assert_eq!(Services::open(kept.clone(), Some("net_0".into())).list(), s.list());
        assert!(s.unpublish("files").unwrap());
        assert!(!s.unpublish("files").unwrap());
        assert_eq!(Services::open(kept, Some("net_0".into())).list(), Vec::new());
    }

    #[test]
    fn a_request_id_goes_to_the_service_once_from_each_caller() {
        let s = Services::open(in_a_network(), Some("net_0".into()));
        s.publish(Published::new("files", "http://127.0.0.1:4790", &["/api/v1/".into()], &[], 0).unwrap()).unwrap();
        let req = Request { id: request_id().unwrap(), name: "files".into(), method: "GET".into(), path: "/api/v1/x".into(), headers: Vec::new(), body: Vec::new() };
        assert!(s.check("laptop", &req, 0.0).is_ok());
        let again = s.check("laptop", &req, 1000.0).err().unwrap();
        assert_eq!((again.status, again.code.as_deref()), (409, Some("replayed")));
        assert!(s.check("desk", &req, 1000.0).is_ok(), "another caller's id is its own");
        assert_eq!(s.check("laptop", &req, SEEN_MS + 1.0).err().map(|a| a.status), None, "forgotten after 15 minutes");
    }

    #[test]
    fn a_wait_ends_when_its_call_goes() {
        let s = Services::open(in_a_network(), Some("net_0".into()));
        // The program that asked went away: its call, and the wait in it,
        // are dropped before any answer.
        let w = s.wait("studio").unwrap();
        assert_eq!(s.waiting(), 1);
        drop(w);
        assert_eq!(s.waiting(), 0);
        // One host that never answers holds a quarter of the waits at most.
        let held: Vec<Flight<'_>> = (0..WAITING_EACH).map(|_| s.wait("studio").ok().unwrap()).collect();
        let refused = s.wait("studio").err().unwrap();
        assert_eq!((refused.status, refused.code.as_deref()), (503, Some("busy")));
        assert!(s.wait("desk").is_ok(), "another host still answers");
        drop(held);
        assert_eq!(s.waiting(), 0);
    }

    #[test]
    fn no_one_caller_holds_every_forwarding_slot() {
        let s = Services::open(in_a_network(), Some("net_0".into()));
        let laptop: Vec<Flight<'_>> = (0..IN_FLIGHT_EACH).map(|_| s.fly("laptop").ok().unwrap()).collect();
        assert_eq!(s.fly("laptop").err().map(|a| a.status), Some(503));
        let others: Vec<Flight<'_>> = (0..IN_FLIGHT - IN_FLIGHT_EACH).map(|i| s.fly(&format!("host{}", i / IN_FLIGHT_EACH)).ok().unwrap()).collect();
        assert_eq!(s.fly("desk").err().map(|a| a.status), Some(503), "and all of them hold {IN_FLIGHT} at most");
        drop(laptop);
        assert!(s.fly("laptop").is_ok(), "a slot comes back when its request ends");
        drop(others);
        assert!(s.in_flight.locked().is_empty());
    }
}
