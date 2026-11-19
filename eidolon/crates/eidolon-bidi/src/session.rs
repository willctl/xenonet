//! The BiDi connection: ids, answers matched to commands, events routed to
//! subscribers. One reader task owns the read half; any number of tasks
//! may send commands at once.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::error::Error;
use crate::frame::MAX_PAYLOAD;
use crate::handshake;
use crate::ws::{Message, Reader, Role, Writer};

/// How the connection starts.
#[derive(Clone, Debug)]
pub struct Options {
    /// The longest message accepted, in bytes.
    pub max_message: usize,
    /// The request path of the BiDi endpoint.
    pub path: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { max_message: MAX_PAYLOAD, path: "/session".into() }
    }
}

/// An event the browser pushed.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// For example `browsingContext.load`.
    pub method: String,
    pub params: Value,
}

struct Sub {
    events: Vec<String>,
    tx: mpsc::UnboundedSender<Event>,
}

impl Sub {
    fn wants(&self, method: &str) -> bool {
        self.events.iter().any(|e| method == e || method.strip_prefix(e.as_str()).is_some_and(|rest| rest.starts_with('.')))
    }
}

#[derive(Default)]
struct State {
    pending: HashMap<u64, oneshot::Sender<Result<Value, Error>>>,
    subs: Vec<Sub>,
    /// Set once, when the connection ends; later commands fail with it.
    closed: Option<Error>,
}

struct Shared {
    state: Mutex<State>,
    writer: tokio::sync::Mutex<Writer>,
    next_id: AtomicU64,
    closing: AtomicBool,
}

impl Shared {
    fn state(&self) -> MutexGuard<'_, State> {
        // Nothing here panics while holding the lock, so a poisoned lock
        // still holds consistent data.
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Ends the connection for everyone waiting on it.
    fn fail(&self, error: Error) {
        let (pending, why) = {
            let mut state = self.state();
            let why = state.closed.get_or_insert(error).clone();
            state.subs.clear();
            (std::mem::take(&mut state.pending), why)
        };
        for (_, tx) in pending {
            // A caller that gave up has dropped its receiver.
            let _ = tx.send(Err(why.clone()));
        }
    }

    fn dispatch(&self, text: &str) -> Result<(), Error> {
        let value: Value = serde_json::from_str(text).map_err(|e| Error::Protocol(format!("a message is not JSON: {e}")))?;
        match value.get("type").and_then(Value::as_str) {
            Some("success") => {
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let result = value.get("result").cloned().unwrap_or(Value::Null);
                    self.resolve(id, Ok(result));
                }
            }
            Some("error") => {
                if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_owned();
                    self.resolve(id, Err(Error::Remote { code: text("error"), message: text("message") }));
                }
            }
            Some("event") => {
                let method = value.get("method").and_then(Value::as_str).unwrap_or_default();
                let params = value.get("params").cloned().unwrap_or(Value::Null);
                let mut state = self.state();
                state.subs.retain(|s| !s.wants(method) || s.tx.send(Event { method: method.to_owned(), params: params.clone() }).is_ok());
            }
            // Anything else is not for us.
            _ => {}
        }
        Ok(())
    }

    fn resolve(&self, id: u64, result: Result<Value, Error>) {
        let tx = self.state().pending.remove(&id);
        if let Some(tx) = tx {
            let _ = tx.send(result);
        }
    }
}

/// A connection to a browser's BiDi endpoint.
///
/// Dropping it stops the reader task and closes the stream without a close
/// message; `close` ends it politely.
pub struct Session {
    shared: Arc<Shared>,
    reader: JoinHandle<()>,
}

impl Session {
    /// Upgrades `stream` to a WebSocket and starts reading. `host_header`
    /// is what the browser expects in `Host`, for example `127.0.0.1:9222`.
    pub async fn connect<S>(stream: S, host_header: &str) -> Result<Self, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_with(stream, host_header, Options::default()).await
    }

    pub async fn connect_with<S>(stream: S, host_header: &str, options: Options) -> Result<Self, Error>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        use tokio::io::AsyncWriteExt as _;
        let (mut read, mut write) = tokio::io::split(stream);
        let key = handshake::new_key()?;
        write.write_all(handshake::request(&options.path, host_header, &key).as_bytes()).await?;
        write.flush().await?;
        let (head, leftover) = handshake::read_head(&mut read).await?;
        handshake::check_response(&head, &key)?;

        let reader = Reader::new(Box::new(read), Role::Client, leftover, options.max_message);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            writer: tokio::sync::Mutex::new(Writer::new(Box::new(write), Role::Client)),
            next_id: AtomicU64::new(1),
            closing: AtomicBool::new(false),
        });
        let task = tokio::spawn(read_loop(Arc::clone(&shared), reader));
        Ok(Self { shared, reader: task })
    }

    /// Sends one command and waits for its answer: the `result` object, or
    /// the error the browser gave. Safe to call from many tasks at once.
    pub async fn send(&self, method: &str, params: Value) -> Result<Value, Error> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.shared.state();
            if let Some(why) = &state.closed {
                return Err(why.clone());
            }
            state.pending.insert(id, tx);
        }
        let text = json!({ "id": id, "method": method, "params": params }).to_string();
        let sent = self.shared.writer.lock().await.send_text(&text).await;
        if let Err(e) = sent {
            self.shared.fail(e.clone());
            return Err(e);
        }
        match rx.await {
            Ok(answer) => answer,
            // The sender is dropped only when the connection ended.
            Err(_) => Err(self.shared.state().closed.clone().unwrap_or_else(|| Error::Closed("the connection ended".into()))),
        }
    }

    /// Like `send`, giving up after `limit`.
    pub async fn send_within(&self, method: &str, params: Value, limit: Duration) -> Result<Value, Error> {
        tokio::time::timeout(limit, self.send(method, params)).await.map_err(|_| Error::Timeout(limit))?
    }

    /// Asks for `events` (names like `browsingContext.load`, or a whole
    /// module like `browsingContext`) and returns where they arrive. The
    /// channel ends when the connection does.
    pub async fn subscribe(&self, events: &[&str]) -> Result<mpsc::UnboundedReceiver<Event>, Error> {
        let (tx, rx) = mpsc::unbounded_channel();
        let names: Vec<String> = events.iter().map(|e| (*e).to_owned()).collect();
        // Registered first, so an event right behind the answer is not lost.
        {
            let mut state = self.shared.state();
            if let Some(why) = &state.closed {
                return Err(why.clone());
            }
            state.subs.push(Sub { events: names.clone(), tx });
        }
        self.send("session.subscribe", json!({ "events": names })).await?;
        Ok(rx)
    }

    /// Ends the connection with a close message and waits for the browser's.
    pub async fn close(mut self) -> Result<(), Error> {
        self.shared.closing.store(true, Ordering::Relaxed);
        let sent = self.shared.writer.lock().await.send_close(1000, "").await;
        if sent.is_ok() && tokio::time::timeout(Duration::from_secs(5), &mut self.reader).await.is_err() {
            self.reader.abort();
        }
        self.shared.fail(Error::Closed("closed by us".into()));
        self.shared.writer.lock().await.shutdown().await;
        sent
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
        self.shared.fail(Error::Closed("the session was dropped".into()));
    }
}

async fn read_loop(shared: Arc<Shared>, mut reader: Reader) {
    let end = loop {
        match reader.next().await {
            Ok(Message::Text(text)) => {
                if let Err(e) = shared.dispatch(&text) {
                    break e;
                }
            }
            Ok(Message::Binary(_)) => break Error::Protocol("the browser sent a binary message".into()),
            Ok(Message::Ping(payload)) => {
                if let Err(e) = shared.writer.lock().await.send_pong(&payload).await {
                    break e;
                }
            }
            Ok(Message::Pong(_)) => {}
            Ok(Message::Close { code, reason }) => {
                if !shared.closing.load(Ordering::Relaxed) {
                    // Answer a close with a close, as the standard asks.
                    let _ = shared.writer.lock().await.send_close(code.unwrap_or(1000), "").await;
                }
                break Error::Closed(match (code, reason.is_empty()) {
                    (Some(c), false) => format!("the browser closed it with code {c}: {reason}"),
                    (Some(c), true) => format!("the browser closed it with code {c}"),
                    (None, _) => "the browser closed it".into(),
                });
            }
            Err(e) => break e,
        }
    };
    shared.fail(end);
}
