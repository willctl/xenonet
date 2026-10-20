//! The adapter's way to the service (core/assistant/client.js, and
//! docs/design/cloud-assistants.md, section 4): `xenonet mcp assistant
//! --grant <grant>` finds the private endpoint (server/assistant-endpoint.js)
//! by the XenoNet it's for, reads its grant's credential from that
//! XenoNet's state folder, and says hello with it, over 127.0.0.1 only. One
//! connection is kept for its calls; when it fails, the next try waits two
//! seconds, then four, and so on up to a minute, with some jitter, and
//! calls meanwhile are answered at once as SERVICE_UNAVAILABLE, with when
//! to try again.
//!
//! Every answer is an envelope: a result, or an error with a fixed code,
//! which the adapter passes on as it is. A credential never leaves this
//! process except in hello.
//!
//! Only the adapter uses this, so it's here with the parts of the wire it
//! needs: the endpoint's descriptor (core/assistant/protocol.js), the
//! newline-delimited JSON of core/components/rpc.js, and the reading side
//! of the grant's files (core/assistant/credentials.js).

use super::{error_envelope, now, random};
use crate::process;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use xenonet_base::json as js;
use xenonet_base::paths::join_all;
use xenonet_base::text::{js_number, js_string, js_trim, truthy};

/// How long a read may take, the service's ten seconds and some.
pub const CALL_MS: u64 = 12_000;
const HELLO_MS: u64 = 5_000;
const BACKOFF_FIRST: f64 = 2000.0;
const BACKOFF_MAX: f64 = 60_000.0;
/// The longest message either way (core/assistant/protocol.js).
pub const MAX_LINE: usize = 320 << 10;
const DISCONNECTED: &str = "XenoNet disconnected; the action was not retried.";

// ---------- where the endpoint is, and the grant's files ----------

fn home() -> String {
    std::env::home_dir().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
}

fn platform() -> &'static str {
    if cfg!(windows) {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// The runtime folder (core/components/runtime.js).
fn runtime_dir() -> String {
    process::set("XENONET_RUNTIME_DIR").unwrap_or_else(|| {
        let state = xenonet_base::dirs::app_dir("state", process::set("XDG_STATE_HOME").as_deref(), process::var("LOCALAPPDATA").as_deref(), platform(), &home(), "XenoNet");
        join_all(&[&state, "runtime"])
    })
}

/// The endpoint's descriptor (core/assistant/protocol.js): in the runtime
/// folder, named for the XenoNet's port.
pub fn endpoint_file(runtime: &str, port: &str) -> String {
    join_all(&[runtime, &format!("assistant-{port}.json")])
}

fn is_grant(grant: &str) -> bool {
    Regex::new(super::schema::GRANT_ID).expect("a pattern").is_match(grant)
}

fn is_token(s: &str) -> bool {
    s.strip_prefix("xenonet_assistant_").is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

fn read_json(file: &str) -> Option<Value> {
    let raw = std::fs::read(file).ok()?;
    js::parse(&String::from_utf8_lossy(&raw)).ok()
}

/// A grant's credential, as credentials.js reads it.
pub struct Credential {
    pub token: String,
    pub expires_at: Option<f64>,
}

/// A grant's credential, or None when there's none (revoked, or never
/// written on this computer).
pub fn read_credential(dir: &str, grant: &str) -> Option<Credential> {
    if !is_grant(grant) {
        return None;
    }
    let c = read_json(&join_all(&[dir, "credentials", &format!("{grant}.json")]))?;
    let token = c.get("token").and_then(Value::as_str).filter(|t| is_token(t))?;
    if c.get("grant_id").and_then(Value::as_str) != Some(grant) {
        return None;
    }
    Some(Credential { token: token.to_string(), expires_at: c.get("expires_at").and_then(Value::as_f64) })
}

/// A grant's revision, as the service last moved it on, or None.
pub fn revision_of(dir: &str, grant: &str) -> Option<f64> {
    if !is_grant(grant) {
        return None;
    }
    let raw = std::fs::read(join_all(&[dir, "revisions", grant])).ok()?;
    let n = js_number(Some(&Value::String(js_trim(&String::from_utf8_lossy(&raw)).to_string())));
    (n.fract() == 0.0 && n > 0.0 && n.is_finite()).then_some(n)
}

fn is_marked_revoked(dir: &str, grant: &str) -> bool {
    is_grant(grant) && std::path::Path::new(&join_all(&[dir, "revoked", grant])).exists()
}

fn is_disabled(dir: &str) -> bool {
    std::path::Path::new(&join_all(&[dir, "disabled"])).exists()
}

// ---------- the connection (core/components/rpc.js) ----------

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Option<Value>, String>>>>>;

/// A private, bounded local connection. A request interrupted after it was
/// sent is never sent again: it may already have taken effect.
pub struct Conn {
    lines: mpsc::UnboundedSender<String>,
    pending: Pending,
    seq: AtomicU64,
    closed: Arc<AtomicBool>,
    stop: watch::Sender<bool>,
}

impl Conn {
    fn open(stream: TcpStream) -> Arc<Conn> {
        let (mut read, mut write) = stream.into_split();
        let (lines, mut queue) = mpsc::unbounded_channel::<String>();
        let pending: Pending = Arc::default();
        let closed = Arc::new(AtomicBool::new(false));
        let (stop, stopped) = watch::channel(false);
        let conn = Arc::new(Conn { lines: lines.clone(), pending: pending.clone(), seq: AtomicU64::new(0), closed: closed.clone(), stop });
        // The line to write each answer to a request the service makes.
        let replies = lines;
        let disconnect = {
            let (pending, closed) = (pending.clone(), closed.clone());
            move || {
                if closed.swap(true, Ordering::SeqCst) {
                    return;
                }
                for (_, p) in pending.lock().expect("pending").drain() {
                    let _ = p.send(Err(DISCONNECTED.into()));
                }
            }
        };
        let gone = disconnect.clone();
        let mut until = stopped.clone();
        tokio::spawn(async move {
            loop {
                let line = tokio::select! {
                    line = queue.recv() => line,
                    _ = until.changed() => None,
                };
                let Some(line) = line else { break };
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
            let _ = write.shutdown().await;
            gone();
        });
        let mut until = stopped;
        tokio::spawn(async move {
            let mut buffer: Vec<u8> = vec![];
            let mut chunk = vec![0u8; 64 << 10];
            loop {
                let got = tokio::select! {
                    got = read.read(&mut chunk) => got,
                    _ = until.changed() => break,
                };
                let n = match got {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.len() > MAX_LINE {
                    break;
                }
                let mut bad = false;
                while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=end).collect();
                    let Ok(message) = js::parse(&String::from_utf8_lossy(&line[..end])) else {
                        bad = true;
                        break;
                    };
                    match &message {
                        Value::Object(m) => {
                            if truthy(m.get("reply")) {
                                let waiting = m
                                    .get("reply")
                                    .and_then(Value::as_f64)
                                    .filter(|r| r.fract() == 0.0 && *r >= 0.0)
                                    .and_then(|r| pending.lock().expect("pending").remove(&(r as u64)));
                                if let Some(p) = waiting {
                                    let _ = p.send(match m.get("error").filter(|e| truthy(Some(e))) {
                                        Some(e) => Err(js_string(e)),
                                        None => Ok(m.get("result").cloned()),
                                    });
                                }
                            } else if let (Some(Value::String(_)), Some(id)) = (m.get("method"), m.get("id").and_then(Value::as_f64)) {
                                // The adapter serves nothing.
                                if id.fract() == 0.0 && id.abs() <= 9_007_199_254_740_991.0 {
                                    let _ = replies.send(format!("{}\n", js::stringify(&json!({ "reply": js::number(id), "error": "unsupported request" }), 0)));
                                }
                            }
                        }
                        Value::Array(_) => {}
                        _ => {
                            bad = true;
                            break;
                        }
                    }
                }
                if bad {
                    break;
                }
            }
            disconnect();
        });
        conn
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// One request, answered within `timeout`.
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Option<Value>, String> {
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending").insert(id, tx);
        let line = format!("{}\n", js::stringify(&json!({ "id": id, "method": method, "params": params }), 0));
        let refused = if self.is_closed() {
            Some(DISCONNECTED)
        } else if line.len() > MAX_LINE {
            Some("XenoNet's connection is too busy.")
        } else if self.lines.send(line).is_err() {
            Some(DISCONNECTED)
        } else {
            None
        };
        if let Some(why) = refused {
            self.pending.lock().expect("pending").remove(&id);
            return Err(why.into());
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(DISCONNECTED.into()),
            Err(_) => {
                self.pending.lock().expect("pending").remove(&id);
                Err(format!("{method} did not answer; the action was not retried."))
            }
        }
    }

    /// Ends the connection, as socket.destroy() does.
    pub fn close(&self) {
        let _ = self.stop.send(true);
        self.closed.store(true, Ordering::SeqCst);
        for (_, p) in self.pending.lock().expect("pending").drain() {
            let _ = p.send(Err(DISCONNECTED.into()));
        }
    }
}

// ---------- the client ----------

/// Why the adapter got no connection: a code of the assistant's errors,
/// and what happened, for its log.
#[derive(Clone, Debug)]
pub struct Refused {
    pub code: &'static str,
    pub detail: String,
}

fn refused(code: &'static str, detail: &str) -> Refused {
    Refused { code, detail: detail.to_string() }
}

#[derive(Default)]
struct State {
    client: Option<Arc<Conn>>,
    hello: Option<Value>,
    failures: u32,
    retry_at: f64,
    // The state folder the endpoint named last, for standing() when it's
    // gone.
    endpoint_dir: Option<String>,
    // Each open that finished, and how, so callers that waited on it share
    // its outcome, as they share the JS client's promise.
    opened: u64,
    last: Option<Result<Arc<Conn>, Refused>>,
}

/// What a read answered: a result envelope, or an error one, with what
/// happened when it was this side that failed.
pub struct Read {
    pub ok: bool,
    pub envelope: Value,
    pub detail: Option<String>,
}

/// A clock or a source of randomness, which tests stand in for.
pub type Source = Box<dyn Fn() -> f64 + Send + Sync>;

pub struct Client {
    grant: String,
    endpoint: String,
    now: Source,
    random: Source,
    state: Mutex<State>,
    opening: tokio::sync::Mutex<()>,
}

/// The grant's standing, as far as this computer's files say.
#[derive(Debug, PartialEq)]
pub enum Standing {
    Stands(Option<f64>),
    Not(&'static str),
}

impl Client {
    /// A client for `grant`, of the XenoNet on `port` (XENONET_PORT as
    /// given, or 47840).
    pub fn new(grant: &str, port: Option<String>) -> Result<Client, String> {
        if !is_grant(grant) {
            return Err("--grant takes a grant's id (gr_…), as XenoNet's window shows it".into());
        }
        let port = port.or_else(|| process::set("XENONET_PORT")).unwrap_or_else(|| "47840".into());
        Ok(Client::with(grant, endpoint_file(&runtime_dir(), &port), Box::new(now), Box::new(random)))
    }

    /// A client for `grant` that finds the endpoint described in
    /// `endpoint`, with this clock and randomness.
    pub fn with(grant: &str, endpoint: String, now: Source, random: Source) -> Client {
        Client { grant: grant.to_string(), endpoint, now, random, state: Mutex::default(), opening: tokio::sync::Mutex::new(()) }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the client's state")
    }

    async fn open(&self) -> Result<Arc<Conn>, Refused> {
        let Some(descriptor) = read_json(&self.endpoint) else {
            return Err(refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint for remote actors isn't listening: XenoNet isn't running, or no grant is active"));
        };
        let int = |k: &str| descriptor.get(k).and_then(Value::as_f64).filter(|n| n.fract() == 0.0);
        let (Some(port), Some(Value::String(state))) = (int("port"), descriptor.get("state")) else {
            return Err(refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is unreadable"));
        };
        self.state().endpoint_dir = Some(state.clone());
        if !int("pid").is_some_and(process::alive) {
            return Err(refused("SERVICE_UNAVAILABLE", "XenoNet's endpoint file is from a service that isn't running"));
        }
        let Some(credential) = read_credential(state, &self.grant) else {
            return Err(refused("NOT_AUTHORIZED", "this computer has no credential for that grant: it was revoked, or made by another XenoNet"));
        };
        let hello = json!({ "role": "assistant", "api": { "min": 1, "max": 1 }, "grant_id": self.grant, "token": credential.token });
        let said = tokio::time::timeout(Duration::from_millis(HELLO_MS), async {
            let p = u16::try_from(port as i64).map_err(|_| DISCONNECTED.to_string())?;
            let stream = TcpStream::connect(("127.0.0.1", p)).await.map_err(|_| DISCONNECTED.to_string())?;
            let c = Conn::open(stream);
            match c.request("hello", hello, Duration::from_millis(HELLO_MS)).await {
                Ok(info) => Ok((c, info)),
                Err(e) => {
                    c.close();
                    Err(e)
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err("hello did not answer; the action was not retried.".into()));
        match said {
            Ok((c, info)) => {
                self.state().hello = info;
                Ok(c)
            }
            Err(message) => {
                let code = super::schema::ERROR_CODES.iter().find(|c| **c == message).copied().unwrap_or("SERVICE_UNAVAILABLE");
                Err(Refused { code, detail: message })
            }
        }
    }

    async fn connection(&self) -> Result<Arc<Conn>, Refused> {
        let seen = {
            let s = self.state();
            if let Some(c) = s.client.as_ref().filter(|c| !c.is_closed()) {
                return Ok(c.clone());
            }
            if (self.now)() < s.retry_at {
                return Err(refused("SERVICE_UNAVAILABLE", "waiting before trying XenoNet again"));
            }
            s.opened
        };
        let _one = self.opening.lock().await;
        {
            // An open that finished while this waited for it is this
            // call's too.
            let s = self.state();
            if s.opened != seen {
                if let Some(last) = &s.last {
                    return last.clone();
                }
            }
        }
        let outcome = self.open().await;
        let mut s = self.state();
        match &outcome {
            Ok(c) => {
                s.client = Some(c.clone());
                s.failures = 0;
                s.retry_at = 0.0;
            }
            Err(_) => {
                // A refusal for good (revoked, not allowed) isn't retried
                // sooner than an outage would be.
                s.client = None;
                s.failures += 1;
                let base = BACKOFF_MAX.min(BACKOFF_FIRST * 2f64.powi(s.failures as i32 - 1));
                s.retry_at = (self.now)() + js_round(base * (0.8 + 0.4 * (self.random)()));
            }
        }
        s.opened += 1;
        s.last = Some(outcome.clone());
        outcome
    }

    /// One read: a result envelope, or an error one.
    pub async fn read(&self, operation: &str, args: &Value, request_id: &str) -> Read {
        let c = match self.connection().await {
            Ok(c) => c,
            Err(e) => {
                let retry = (e.code == "SERVICE_UNAVAILABLE").then(|| 1000f64.max(self.state().retry_at - (self.now)()));
                return Read { ok: false, envelope: error_envelope(request_id, e.code, retry), detail: Some(e.detail) };
            }
        };
        match c.request("assistant.read", json!({ "request_id": request_id, "operation": operation, "arguments": args }), Duration::from_millis(CALL_MS)).await {
            Ok(Some(Value::Object(r))) => match r.get("ok") {
                Some(Value::Bool(true)) => Read { ok: true, envelope: r.get("result").cloned().unwrap_or(Value::Null), detail: None },
                Some(Value::Bool(false)) => {
                    Read { ok: false, envelope: r.get("error").cloned().unwrap_or(Value::Null), detail: r.get("detail").filter(|d| truthy(Some(d))).map(js_string) }
                }
                _ => Read { ok: false, envelope: error_envelope(request_id, "INTERNAL_ERROR", None), detail: None },
            },
            Ok(_) => Read { ok: false, envelope: error_envelope(request_id, "INTERNAL_ERROR", None), detail: None },
            // Interrupted after it was sent: not sent again; a read can be
            // asked for again by the caller.
            Err(message) => {
                let code = if message.contains("did not answer") { "TIMEOUT" } else { "SERVICE_UNAVAILABLE" };
                Read { ok: false, envelope: error_envelope(request_id, code, None), detail: Some(message) }
            }
        }
    }

    /// Whether the grant still stands as far as this computer's files say,
    /// with no service to ask: not revoked (its credential and
    /// tombstone), not expired, cloud access not turned off, and at which
    /// revision (None when there's no record of one).
    pub fn standing(&self) -> Standing {
        let Some(dir) = self.state().endpoint_dir.clone() else { return Standing::Not("SERVICE_UNAVAILABLE") };
        if is_disabled(&dir) {
            return Standing::Not("DISABLED");
        }
        if is_marked_revoked(&dir, &self.grant) {
            return Standing::Not("GRANT_REVOKED");
        }
        let Some(credential) = read_credential(&dir, &self.grant) else { return Standing::Not("GRANT_REVOKED") };
        if credential.expires_at.is_some_and(|t| t <= (self.now)()) {
            return Standing::Not("GRANT_EXPIRED");
        }
        Standing::Stands(revision_of(&dir, &self.grant))
    }
}

// Math.round: halves go up.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    // A clock the test moves by hand.
    fn clock(start: f64) -> (Arc<AtomicU64>, Source) {
        let t = Arc::new(AtomicU64::new(start.to_bits()));
        let read = t.clone();
        (t, Box::new(move || f64::from_bits(read.load(Ordering::SeqCst))))
    }

    fn folder(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("xenonet-mcp-client-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().into_owned()
    }

    const GRANT: &str = "gr_0123456789abcdef01234567";

    // test/assistant-endpoint.test.js: a XenoNet that isn't there is
    // refused at once, then not tried again until the wait is over, which
    // doubles.
    #[tokio::test(flavor = "current_thread")]
    async fn waits_longer_after_each_failure() {
        let dir = folder("nowhere");
        let (t, now) = clock(1000.0);
        let set = |ms: f64| t.store(ms.to_bits(), Ordering::SeqCst);
        let client = Client::with(GRANT, endpoint_file(&dir, "1"), now, Box::new(|| 0.5));
        let id = "r_00000000-0000-4000-8000-000000000000";
        let first = client.read("status", &json!({}), id).await;
        assert_eq!([&first.envelope["code"], &first.envelope["retryable"], &first.envelope["retry_after_ms"]], [&json!("SERVICE_UNAVAILABLE"), &json!(true), &json!(2000)]);
        set(1500.0);
        assert_eq!(client.read("status", &json!({}), id).await.envelope["retry_after_ms"], json!(1500));
        set(3100.0);
        assert_eq!(client.read("status", &json!({}), id).await.envelope["retry_after_ms"], json!(4000), "doubled");
        assert_eq!(client.standing(), Standing::Not("SERVICE_UNAVAILABLE"), "no endpoint seen, so no folder to ask");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // A descriptor left by a service that isn't running: nothing is sent,
    // and the grant's files still say how it stands.
    #[tokio::test(flavor = "current_thread")]
    async fn sends_nothing_to_a_service_that_is_gone_and_reads_the_grant_from_its_files() {
        let dir = folder("gone");
        let state = join_all(&[&dir, "assistant"]);
        std::fs::create_dir_all(join_all(&[&state, "credentials"])).unwrap();
        let token = format!("xenonet_assistant_{}", "a".repeat(64));
        std::fs::write(
            join_all(&[&state, "credentials", &format!("{GRANT}.json")]),
            format!("{{\"schema\":\"assistant/credential/v1\",\"grant_id\":\"{GRANT}\",\"device_id\":\"dev_x\",\"expires_at\":5000,\"token\":\"{token}\"}}\n"),
        )
        .unwrap();
        std::fs::create_dir_all(join_all(&[&state, "revisions"])).unwrap();
        std::fs::write(join_all(&[&state, "revisions", GRANT]), "3\n").unwrap();
        // A port nothing listens on, and a pid nobody has.
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        std::fs::write(endpoint_file(&dir, "9"), json!({ "api": { "min": 1, "max": 1 }, "pid": 1u64 << 30, "port": free, "state": state }).to_string()).unwrap();
        let (t, now) = clock(1000.0);
        let client = Client::with(GRANT, endpoint_file(&dir, "9"), now, Box::new(|| 0.5));
        let r = client.read("status", &json!({}), "r_00000000-0000-4000-8000-000000000000").await;
        assert_eq!((r.envelope["code"].as_str(), r.detail.as_deref()), (Some("SERVICE_UNAVAILABLE"), Some("XenoNet's endpoint file is from a service that isn't running")));
        assert_eq!(client.standing(), Standing::Stands(Some(3.0)));
        t.store(5000f64.to_bits(), Ordering::SeqCst);
        assert_eq!(client.standing(), Standing::Not("GRANT_EXPIRED"));
        std::fs::create_dir_all(join_all(&[&state, "revoked"])).unwrap();
        std::fs::write(join_all(&[&state, "revoked", GRANT]), "{}\n").unwrap();
        assert_eq!(client.standing(), Standing::Not("GRANT_REVOKED"));
        std::fs::write(join_all(&[&state, "disabled"]), "{}\n").unwrap();
        assert_eq!(client.standing(), Standing::Not("DISABLED"));
        assert!(Client::new("gr_nope", None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
