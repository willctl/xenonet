//! A private, bounded local connection (core/components/rpc.js): JSON lines
//! over a socket, requests (`{ id, method, params }`) answered with
//! `{ reply, result }` or `{ reply, error }`, and events (`{ event, value }`).
//! Requests interrupted after sending are never replayed: an action may
//! already have taken effect. A line that isn't a JSON object, or a buffer
//! over `max_line`, ends the connection.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, watch};
use xenonet_base::json;
use xenonet_base::sync::Locked;

pub const MAX_LINE: usize = 16 << 20;
const BUSY: usize = 16 << 20;
pub const DISCONNECTED: &str = "XenoNet disconnected; the action was not retried.";

pub type Handled = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
/// What answers a request: (method, params) → result or error message.
pub type Handler = Arc<dyn Fn(String, Value) -> Handled + Send + Sync>;

struct Shared {
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    sequence: AtomicI64,
    closed: AtomicBool,
    queued: AtomicUsize,
    out: mpsc::UnboundedSender<Vec<u8>>,
    gone: watch::Sender<bool>,
    max_line: usize,
}

/// One end of a connection.
#[derive(Clone)]
pub struct Rpc {
    shared: Arc<Shared>,
}

impl Rpc {
    /// Serves `socket` with `handle`; events named in `incoming` come out of
    /// the returned receiver (a server takes none).
    pub fn new(socket: TcpStream, handle: Handler, incoming: &'static [&'static str], max_line: usize) -> (Rpc, mpsc::UnboundedReceiver<(String, Value)>) {
        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            sequence: AtomicI64::new(0),
            closed: AtomicBool::new(false),
            queued: AtomicUsize::new(0),
            out: out_tx,
            gone: watch::channel(false).0,
            max_line,
        });
        let rpc = Rpc { shared };
        let (mut read, mut write) = socket.into_split();

        let w = rpc.clone();
        let mut gone = rpc.shared.gone.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    line = out_rx.recv() => match line {
                        Some(line) => {
                            let n = line.len();
                            let wrote = write.write_all(&line).await;
                            w.shared.queued.fetch_sub(n, Ordering::Relaxed);
                            if wrote.is_err() {
                                break;
                            }
                        }
                        None => break,
                    },
                    _ = gone.changed() => break,
                }
            }
            let _ = write.shutdown().await;
            w.disconnect();
        });

        let r = rpc.clone();
        let mut gone = rpc.shared.gone.subscribe();
        tokio::spawn(async move {
            let mut buffer: Vec<u8> = Vec::new();
            let mut chunk = vec![0u8; 64 << 10];
            loop {
                let n = tokio::select! {
                    n = read.read(&mut chunk) => match n {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    },
                    _ = gone.changed() => break,
                };
                buffer.extend_from_slice(&chunk[..n]);
                if buffer.len() > r.shared.max_line {
                    break;
                }
                let mut bad = false;
                while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=end).collect();
                    let text = String::from_utf8_lossy(&line[..line.len() - 1]);
                    let Ok(message) = json::parse(&text) else {
                        bad = true;
                        break;
                    };
                    if !message.is_object() {
                        bad = true;
                        break;
                    }
                    r.take(message, &handle, incoming, &ev_tx);
                }
                if bad {
                    break;
                }
            }
            r.close();
        });
        (rpc, ev_rx)
    }

    fn take(&self, message: Value, handle: &Handler, incoming: &[&str], events: &mpsc::UnboundedSender<(String, Value)>) {
        if xenonet_base::text::truthy(message.get("reply")) {
            let id = message["reply"].as_i64().unwrap_or(-1);
            if let Some(p) = self.shared.pending.locked().remove(&id) {
                let _ = p.send(if xenonet_base::text::truthy(message.get("error")) {
                    Err(xenonet_base::text::js_string(&message["error"]))
                } else {
                    Ok(message.get("result").cloned().unwrap_or(Value::Null))
                });
            }
        } else if message.get("method").is_some_and(Value::is_string)
            && message.get("id").and_then(Value::as_f64).is_some_and(|n| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0)
        {
            let id = message["id"].clone();
            let method = message["method"].as_str().unwrap().to_string();
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let fut = handle(method, params);
            let me = self.clone();
            tokio::spawn(async move {
                let answer = match fut.await {
                    Ok(result) => json!({ "reply": id, "result": result }),
                    Err(error) => json!({ "reply": id, "error": error }),
                };
                if me.send(&answer).is_err() {
                    me.close();
                }
            });
        } else if let Some(event) = message.get("event").and_then(Value::as_str).filter(|e| incoming.contains(e)) {
            let _ = events.send((event.to_string(), message.get("value").cloned().unwrap_or(Value::Null)));
        }
    }

    fn send(&self, value: &Value) -> Result<(), String> {
        if self.shared.closed.load(Ordering::SeqCst) {
            return Err(DISCONNECTED.into());
        }
        let mut line = json::stringify(value, 0).into_bytes();
        line.push(b'\n');
        if line.len() > self.shared.max_line || self.shared.queued.load(Ordering::Relaxed) > BUSY {
            return Err("XenoNet's connection is too busy.".into());
        }
        self.shared.queued.fetch_add(line.len(), Ordering::Relaxed);
        self.shared.out.send(line).map_err(|_| DISCONNECTED.to_string())
    }

    /// A request to the other end, which it has `timeout` to answer.
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.shared.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = oneshot::channel();
        self.shared.pending.locked().insert(id, tx);
        if let Err(e) = self.send(&json!({ "id": id, "method": method, "params": params })) {
            self.shared.pending.locked().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err(DISCONNECTED.into()),
            Err(_) => {
                self.shared.pending.locked().remove(&id);
                Err(format!("{method} did not answer; the action was not retried."))
            }
        }
    }

    pub fn notify(&self, event: &str, value: Value) -> Result<(), String> {
        self.send(&json!({ "event": event, "value": value }))
    }

    pub fn close(&self) {
        self.disconnect();
    }

    pub fn is_closed(&self) -> bool {
        self.shared.closed.load(Ordering::SeqCst)
    }

    fn disconnect(&self) {
        if self.shared.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        for (_, p) in self.shared.pending.locked().drain() {
            let _ = p.send(Err(DISCONNECTED.into()));
        }
        let _ = self.shared.gone.send(true);
    }

    /// Waits until the connection has ended.
    pub async fn closed(&self) {
        let mut gone = self.shared.gone.subscribe();
        while !*gone.borrow_and_update() {
            if gone.changed().await.is_err() {
                return;
            }
        }
    }
}

/// negotiate (core/components/protocol.js): the version both ends speak.
pub fn negotiate(local: (u32, u32), remote: &Value) -> Result<u32, String> {
    let int = |v: &Value| v.as_f64().filter(|n| n.fract() == 0.0 && n.is_finite());
    let (Some(rmin), Some(rmax)) = (remote.get("min").and_then(int), remote.get("max").and_then(int)) else {
        return Err("XenoNet did not report a supported interface version. Update the service first.".into());
    };
    let (lmin, lmax) = (f64::from(local.0), f64::from(local.1));
    let version = lmax.min(rmax);
    if lmin > lmax || rmin > rmax || version < lmin.max(rmin) {
        return Err("These XenoNet components use incompatible interfaces. Open Updates or run xenonet update.".into());
    }
    Ok(version as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::net::TcpListener;

    #[test]
    fn versions_as_protocol_js_negotiates_them() {
        assert_eq!(negotiate((1, 1), &json!({ "min": 1, "max": 1 })), Ok(1));
        assert!(negotiate((1, 1), &json!({ "min": 2, "max": 3 })).unwrap_err().contains("incompatible"));
        assert!(negotiate((1, 1), &json!({ "min": "1" })).unwrap_err().contains("did not report"));
    }

    #[tokio::test]
    async fn requests_are_answered_and_bad_lines_end_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let handle: Handler = Arc::new(|method, params| {
                Box::pin(async move {
                    if method == "echo" {
                        Ok(params)
                    } else {
                        Err(format!("no {method}"))
                    }
                })
            });
            let (rpc, _) = Rpc::new(s, handle, &[], MAX_LINE);
            rpc.closed().await;
        });
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        c.write_all(b"{\"id\":1,\"method\":\"echo\",\"params\":{\"a\":1}}\n{\"id\":2,\"method\":\"nope\"}\n").await.unwrap();
        let mut lines = BufReader::new(c).lines();
        let mut got = vec![lines.next_line().await.unwrap().unwrap(), lines.next_line().await.unwrap().unwrap()];
        got.sort();
        assert_eq!(got, vec!["{\"reply\":1,\"result\":{\"a\":1}}".to_string(), "{\"reply\":2,\"error\":\"no nope\"}".to_string()]);
        let mut c = lines.into_inner().into_inner();
        c.write_all(b"[1]\n").await.unwrap();
        let mut rest = Vec::new();
        assert_eq!(c.read_to_end(&mut rest).await.unwrap(), 0, "the server hung up");
    }
}
