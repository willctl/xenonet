//! A host's link to its gateway (docs/design/network.md, section 2): the
//! down stream (`GET /net/v1/link`), read a chunk at a time with an idle
//! timer of 45 s and no overall timeout, and the up batches
//! (`POST /net/v1/frames`) that empty the outbox.
//!
//! - Live means heard: each down request's answer, its headers and its
//!   hello, must come within 35 s (a long-poll's 25 s wait and 10 s to
//!   spare), and a stream that then brings nothing for 45 s is dropped.
//!   Either puts the link offline, with the reason, so a gateway that
//!   takes connections and says nothing isn't shown as live.
//! - Backoff: 2 s, doubling to 60 s, with jitter, as the hosts link has.
//! - The clock: a time the gateway refuses comes back with its own; the
//!   link keeps the offset and signs again with it.
//! - Epochs: `hello` names the gateway's; a new one means resume from 0,
//!   dropping repeats by frame id, and a `gap`, since what the old store
//!   held may be gone.
//! - Long-poll: after two streams that went quiet past a due ping, the
//!   link asks with `&wait=25`, for proxies that hold streams back.
//! - `connecting` is the first try's state alone: a try after that keeps
//!   live, offline or refused, with its reason and since, until the hello
//!   or another failure.
//!
//! The gateway's own host links the same way, in-process: its requests go
//! straight to its own gateway's routes, signed as any host's are.

use crate::codec::random;
use crate::files::address_kind;
use crate::frame::Frame;
use crate::gateway::{Gateway, NetBody, NetReply, NetRequest};
use crate::node::Shared;
use crate::roster::Entry;
use crate::{NetError, R};
use bytes::Bytes;
use serde_json::Value;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The longest line a stream may carry: a relay frame and its JSON.
const LINE_MAX: usize = 24 << 20;
const POST_MS: u64 = 30_000;

/// How a host reaches its gateway.
#[derive(Clone)]
pub enum Transport {
    Http {
        client: reqwest::Client,
        base: String,
    },
    /// The gateway's own host, in-process.
    Local(Weak<Gateway>),
}

/// A client for a gateway's address: through the system's proxy for a
/// tunnel's name, never for a LAN or loopback address.
pub fn http(address: &str) -> R<Transport> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut b = reqwest::Client::builder().user_agent(concat!("xenonet/", env!("CARGO_PKG_VERSION"))).connect_timeout(Duration::from_secs(10));
    if address_kind(address) != "tunnel" {
        b = b.no_proxy();
    }
    let client = b.build().map_err(|e| NetError::internal(format!("an HTTP client: {e}")))?;
    Ok(Transport::Http { client, base: crate::files::base_url(address) })
}

/// What went wrong reaching the gateway, in a few words.
fn why(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "timed out".into();
    }
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        text.push_str(": ");
        text.push_str(&s.to_string());
        source = s.source();
    }
    let lower = text.to_ascii_lowercase();
    if lower.contains("dns") || lower.contains("lookup") || lower.contains("resolve") {
        "no DNS for the gateway's address".into()
    } else if e.is_connect() {
        "the gateway refused the connection".into()
    } else {
        text
    }
}

impl Transport {
    /// One request: its answer, or why there was none. `timeout` covers
    /// the whole of it; a stream has none, and the down link sets its own
    /// deadline for the answer's headers and hello.
    pub async fn request(&self, req: NetRequest, timeout: Option<Duration>) -> Result<NetReply, String> {
        match self {
            Transport::Local(gw) => match gw.upgrade() {
                Some(g) => Ok(g.handle(req).await),
                None => Err("the gateway stopped".into()),
            },
            Transport::Http { client, base } => {
                let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|_| "a bad method".to_string())?;
                let mut r = client.request(method, format!("{base}{}", req.target));
                for (k, v) in &req.headers {
                    r = r.header(k.as_str(), v.as_str());
                }
                if !req.body.is_empty() {
                    r = r.header("content-type", "application/json").body(req.body);
                }
                if let Some(t) = timeout {
                    r = r.timeout(t);
                }
                let mut res = r.send().await.map_err(|e| why(&e))?;
                let status = res.status().as_u16();
                let headers: Vec<(String, String)> = res.headers().iter().map(|(k, v)| (k.as_str().to_string(), String::from_utf8_lossy(v.as_bytes()).into_owned())).collect();
                let ndjson = headers.iter().any(|(k, v)| k == "content-type" && v.starts_with("application/x-ndjson"));
                if status == 200 && ndjson {
                    let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
                    tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                chunk = res.chunk() => match chunk {
                                    Ok(Some(b)) => {
                                        if tx.send(b).is_err() {
                                            break;
                                        }
                                    }
                                    _ => break,
                                },
                                () = tx.closed() => break,
                            }
                        }
                    });
                    return Ok(NetReply { status, headers, body: NetBody::Stream(rx) });
                }
                let body = tokio::time::timeout(Duration::from_millis(POST_MS), res.bytes()).await.map_err(|_| "timed out".to_string())?.map_err(|e| why(&e))?;
                Ok(NetReply { status, headers, body: NetBody::Full(body.to_vec()) })
            }
        }
    }
}

/// The first line of an answer, whole or streamed, within `wait`.
pub async fn first_line(body: NetBody, wait: Duration) -> Option<String> {
    let mut lines = Lines::default();
    match body {
        NetBody::Full(b) => String::from_utf8_lossy(&b).lines().next().map(str::to_string),
        NetBody::Stream(mut rx) => {
            let end = tokio::time::Instant::now() + wait;
            loop {
                let chunk = tokio::time::timeout_at(end, rx.recv()).await.ok()??;
                if let Some(line) = lines.push(&chunk).ok()?.into_iter().next() {
                    return Some(line);
                }
            }
        }
    }
}

/// Splits a stream into lines.
#[derive(Default)]
struct Lines {
    buf: Vec<u8>,
}

impl Lines {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<String>, String> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1]).trim().to_string();
            if !text.is_empty() {
                out.push(text);
            }
        }
        if self.buf.len() > LINE_MAX {
            return Err("a line from the gateway was too long".into());
        }
        Ok(out)
    }
}

/// Why the gateway refused a request.
pub enum Refusal {
    /// Our time was off: the gateway's own.
    Clock(f64),
    /// This host was removed.
    Removed,
    Other(String),
}

/// What a refusal of the request made with `nonce` says. A time is taken
/// only in a frame the pinned gateway signed over that nonce.
pub fn refusal(n: &Shared, reply: &NetReply, nonce: &str) -> Refusal {
    if reply.status == 410 {
        if let NetBody::Full(b) = &reply.body {
            let line = String::from_utf8_lossy(b);
            if let Ok(f) = Frame::parse_line(line.trim()) {
                if n.removed_frame(&f) {
                    return Refusal::Removed;
                }
            }
        }
        return Refusal::Other("the gateway says this host was removed, but its answer didn't check".into());
    }
    let v = reply.body_json();
    if reply.status == 401 && v.get("code").and_then(Value::as_str) == Some("clock") {
        return match n.clock_answer(&v, nonce) {
            Ok(now) => Refusal::Clock(now),
            Err(why) => Refusal::Other(why),
        };
    }
    let error = v.get("error").and_then(Value::as_str).map_or_else(|| format!("the gateway answered {}", reply.status), str::to_string);
    Refusal::Other(error)
}

/// The wait before the next try: 2 s doubling to 60 s, with ±25% jitter.
pub fn backoff(attempt: u32, min_ms: f64, max_ms: f64) -> Duration {
    let jitter = random::<2>().map_or(0.5, |b| f64::from(u16::from_le_bytes(b)) / 65535.0);
    let ms = (min_ms * 2f64.powi(attempt.min(16) as i32)).min(max_ms) * (0.75 + jitter * 0.5);
    Duration::from_millis(ms as u64)
}

/// Starts the link's two halves.
pub fn spawn(n: Arc<Shared>, t: Transport) -> Vec<JoinHandle<()>> {
    vec![tokio::spawn(down(n.clone(), t.clone())), tokio::spawn(up(n, t))]
}

enum Read {
    /// The stream ended; `quiet` when no ping came when one was due.
    Done {
        quiet: bool,
    },
    NoHello(String),
    /// The pinned gateway isn't the roster's any more: refused, and said so.
    NotOurs,
}

async fn down(n: Arc<Shared>, t: Transport) {
    let mut attempt = 0u32;
    let mut quiet = 0u32;
    let mut long_poll = false;
    let mut clock_tries = 0u32;
    loop {
        if n.is_removed() || n.network().is_none() {
            return;
        }
        let timing = n.timing();
        let (epoch, seq) = n.cursor();
        let wait = if long_poll { format!("&wait={}", (timing.poll_ms / 1000).max(1)) } else { String::new() };
        let target = format!("/net/v1/link?epoch={}&after={seq}{wait}", epoch.unwrap_or_default());
        let Ok((req, nonce)) = n.signed_request("GET", &target, Vec::new()) else { return };
        n.connecting();
        // The answer, its headers and its hello, comes by then or not at
        // all: a gateway whose process a firewall blocks still has its
        // connections taken by the kernel, and would hold a request with
        // no deadline, and the link with it, for good. Dropping the
        // request drops its connection.
        let by = tokio::time::Instant::now() + Duration::from_millis(timing.answer_ms);
        let failed = match tokio::time::timeout_at(by, t.request(req, None)).await {
            Err(_) => Some(format!("the gateway didn't answer in {}", secs(timing.answer_ms))),
            Ok(Err(why)) => Some(why),
            Ok(Ok(reply)) if reply.status == 200 => match read(&n, &t, reply.body, &nonce, by).await {
                Read::Done { quiet: q } => {
                    attempt = 0;
                    clock_tries = 0;
                    if q {
                        // Nothing heard for the idle time: not live, whatever
                        // the next request finds.
                        n.offline(&format!("nothing came from the gateway in {}", secs(timing.idle_ms)));
                    }
                    if !long_poll {
                        quiet = if q { quiet + 1 } else { 0 };
                        if quiet >= 2 {
                            long_poll = true;
                            n.set_long_poll();
                        }
                    }
                    None
                }
                Read::NoHello(why) => Some(why),
                Read::NotOurs => {
                    tokio::time::sleep(backoff(attempt, n.timing().backoff_min_ms, n.timing().backoff_max_ms)).await;
                    attempt += 1;
                    continue;
                }
            },
            Ok(Ok(reply)) => match refusal(&n, &reply, &nonce) {
                Refusal::Clock(now) => {
                    n.learn_offset(now);
                    clock_tries += 1;
                    if clock_tries <= 2 {
                        continue;
                    }
                    Some("the gateway keeps refusing this host's time".into())
                }
                Refusal::Removed => return,
                Refusal::Other(why) => {
                    n.refused(&why);
                    tokio::time::sleep(backoff(attempt, n.timing().backoff_min_ms, n.timing().backoff_max_ms)).await;
                    attempt += 1;
                    continue;
                }
            },
        };
        if let Some(why) = failed {
            n.offline(&why);
            tokio::time::sleep(backoff(attempt, n.timing().backoff_min_ms, n.timing().backoff_max_ms)).await;
            attempt += 1;
        }
    }
}

/// Seconds, as a reason says them.
fn secs(ms: u64) -> String {
    format!("{} s", (ms as f64 / 1000.0).round())
}

/// The down answer: its hello by `hello_by`, then a line at least every
/// idle time.
async fn read(n: &Arc<Shared>, t: &Transport, body: NetBody, nonce: &str, hello_by: tokio::time::Instant) -> Read {
    let mut lines = Lines::default();
    let mut hello = false;
    let idle = Duration::from_millis(n.timing().idle_ms);
    let mut rx = match body {
        NetBody::Full(b) => {
            let (tx, rx) = mpsc::unbounded_channel();
            let _ = tx.send(Bytes::from(b));
            drop(tx);
            rx
        }
        NetBody::Stream(rx) => rx,
    };
    loop {
        let until = if hello { tokio::time::Instant::now() + idle } else { hello_by };
        let chunk = match tokio::time::timeout_at(until, rx.recv()).await {
            Ok(Some(c)) => c,
            Ok(None) => return if hello { Read::Done { quiet: false } } else { Read::NoHello("the gateway's answer ended before its hello".into()) },
            Err(_) => return if hello { Read::Done { quiet: true } } else { Read::NoHello(format!("the gateway didn't say hello in {}", secs(n.timing().answer_ms))) },
        };
        let got = match lines.push(&chunk) {
            Ok(g) => g,
            Err(why) => return if hello { Read::Done { quiet: false } } else { Read::NoHello(why) },
        };
        for line in got {
            let Ok(f) = Frame::parse_line(&line) else { continue };
            if !hello {
                if let Some(net) = n.network() {
                    if let Err(why) = n.pinned_gateway(&net) {
                        n.refused(&why);
                        return Read::NotOurs;
                    }
                }
                match n.hello(&f, nonce) {
                    Ok(behind) => {
                        hello = true;
                        if behind {
                            fetch_roster(n, t).await;
                        }
                    }
                    Err(why) => return Read::NoHello(why),
                }
                continue;
            }
            if n.frame(&f) {
                fetch_roster(n, t).await;
            }
            // Hosts this one signed in for its tickets go to the gateway.
            let signed = n.take_signed();
            if !signed.is_empty() {
                tokio::spawn(submit_signed(n.clone(), t.clone(), signed));
            }
        }
    }
}

/// `add` entries this host signed as its tickets' maker, to the gateway:
/// appended there, or kept waiting for a second approval when the new host
/// is a window host and this one isn't the founder.
async fn submit_signed(n: Arc<Shared>, t: Transport, entries: Vec<Entry>) {
    for e in entries {
        let body = serde_json::json!({ "entry": e.to_json(), "propose": true });
        if let Err(err) = call(&n, &t, "POST", "/net/v1/roster", body).await {
            n.log("net.ticket", &format!("{} was signed in here, and the gateway didn't take it: {}", e.host.id, err.message));
        }
    }
}

/// The roster's entries this host hasn't got.
pub async fn fetch_roster(n: &Arc<Shared>, t: &Transport) {
    let from = n.roster().len();
    let Ok((req, _)) = n.signed_request("GET", &format!("/net/v1/roster?from={from}"), Vec::new()) else { return };
    let Ok(reply) = t.request(req, Some(Duration::from_millis(POST_MS))).await else { return };
    if reply.status != 200 {
        return;
    }
    let v = reply.body_json();
    for e in v.get("entries").and_then(Value::as_array).into_iter().flatten() {
        match Entry::parse(e).and_then(|e| n.append(e)) {
            Ok(_) => {}
            Err(e) => {
                n.log("net.roster", &format!("an entry from the gateway didn't extend this host's roster: {}", e.message));
                break;
            }
        }
    }
}

async fn up(n: Arc<Shared>, t: Transport) {
    let mut attempt = 0u32;
    loop {
        tokio::select! {
            () = n.woken() => {}
            () = tokio::time::sleep(Duration::from_secs(5)) => {}
        }
        loop {
            if n.is_removed() || n.network().is_none() {
                return;
            }
            if !n.is_live() {
                break;
            }
            let Some(batch) = n.next_batch() else { break };
            let target = if batch.relay { "/net/v1/frames?relay=1" } else { "/net/v1/frames" };
            let Ok((req, nonce)) = n.signed_request("POST", target, batch.body.clone().into_bytes()) else { break };
            match t.request(req, Some(Duration::from_millis(POST_MS))).await {
                Ok(reply) if reply.status == 200 => {
                    attempt = 0;
                    let v = reply.body_json();
                    n.batch_done(&batch, &v);
                    if let Some(ms) = v.get("retryAfterMs").and_then(Value::as_u64) {
                        tokio::time::sleep(Duration::from_millis(ms.min(60_000))).await;
                    }
                }
                Ok(reply) => match refusal(&n, &reply, &nonce) {
                    Refusal::Clock(now) => {
                        n.learn_offset(now);
                        n.batch_failed(&batch, "", true);
                    }
                    Refusal::Removed => return,
                    Refusal::Other(why) => {
                        n.log("net.up", &format!("the gateway refused a batch: {why}"));
                        n.batch_failed(&batch, &why, false);
                        tokio::time::sleep(backoff(attempt, n.timing().backoff_min_ms, n.timing().backoff_max_ms)).await;
                        attempt += 1;
                        break;
                    }
                },
                Err(why) => {
                    n.log("net.up", &format!("a batch didn't reach the gateway: {why}"));
                    n.batch_failed(&batch, &why, false);
                    tokio::time::sleep(backoff(attempt, n.timing().backoff_min_ms, n.timing().backoff_max_ms)).await;
                    attempt += 1;
                    break;
                }
            }
        }
    }
}

/// A one-off request to the gateway: its answer as JSON, or why not.
pub async fn call(n: &Arc<Shared>, t: &Transport, method: &str, target: &str, body: Value) -> R<Value> {
    let body = if body.is_null() { Vec::new() } else { body.to_string().into_bytes() };
    for _ in 0..3 {
        let (req, nonce) = n.signed_request(method, target, body.clone())?;
        let reply = t.request(req, Some(Duration::from_millis(POST_MS))).await.map_err(|why| NetError::new(502, "unreachable", format!("the gateway didn't answer: {why}")))?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body_json());
        }
        match refusal(n, &reply, &nonce) {
            Refusal::Clock(now) => n.learn_offset(now),
            Refusal::Removed => return Err(NetError::new(410, "removed", "this host was removed from the network")),
            Refusal::Other(why) => {
                let code = reply.body_json().get("code").and_then(Value::as_str).map_or("refused", |c| if c == "roster-refused" { "roster-refused" } else { "refused" });
                return Err(NetError::new(reply.status, code, why));
            }
        }
    }
    Err(NetError::new(502, "clock", "the gateway keeps refusing this host's time"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_split_across_chunks() {
        let mut l = Lines::default();
        assert_eq!(l.push(b"{\"a\"").unwrap(), Vec::<String>::new());
        assert_eq!(l.push(b":1}\n{\"b\":2}\n\n{\"c\"").unwrap(), ["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(l.push(b":3}\n").unwrap(), ["{\"c\":3}"]);
    }

    #[test]
    fn backoff_doubles_to_its_most_with_jitter() {
        for (attempt, base) in [(0u32, 2000.0), (1, 4000.0), (4, 32000.0), (5, 60000.0), (30, 60000.0)] {
            let d = backoff(attempt, 2000.0, 60000.0).as_millis() as f64;
            assert!(d >= base * 0.74 && d <= base * 1.26, "{attempt}: {d}");
        }
    }
}
