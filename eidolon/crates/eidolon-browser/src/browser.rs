//! The live handle on a cell's browser.
//!
//! The automation port is on the guest's loopback and is never forwarded.
//! The host reaches it as a byte stream: `Driver::stream` runs
//! `socat - TCP:127.0.0.1:<port>` in the cell and its stdin and stdout are
//! the WebSocket. Ready means the protocol answered `session.status`.

use crate::error::{BrowserError, Result};
use crate::plan::stream_argv;
use adytum_driver::{Driver, DriverError};
use adytum_model::CellName;
use eidolon_bidi::{Error as BidiError, Event, Session};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};

/// How hard `connect` tries while the browser is starting.
#[derive(Debug, Clone, Copy)]
pub struct ConnectOptions {
    /// The most attempts at the stream and the first answer.
    pub attempts: u32,
    /// The wait between two attempts. Only a failed attempt waits.
    pub gap: Duration,
    /// The longest the browser may take to answer `session.status`.
    pub ready_within: Duration,
}

impl Default for ConnectOptions {
    fn default() -> Self {
        Self { attempts: 60, gap: Duration::from_millis(500), ready_within: Duration::from_secs(10) }
    }
}

/// Whether an error means "not up yet", so another attempt may work.
/// Everything else (a refusal, a missing tool, a broken protocol) will not
/// change by trying again.
fn not_up_yet(e: &BrowserError) -> bool {
    match e {
        // `socat` ran and ended: nothing listens on the port yet.
        BrowserError::Driver(DriverError::Tool { .. }) => true,
        BrowserError::Bidi(b) => matches!(b, BidiError::Io(_) | BidiError::Closed(_) | BidiError::Handshake(_) | BidiError::Timeout(_) | BidiError::NotReady(_)),
        _ => false,
    }
}

/// Whether a command's failure means the connection itself is gone.
fn connection_lost(e: &BidiError) -> bool {
    matches!(e, BidiError::Io(_) | BidiError::Closed(_) | BidiError::Handshake(_) | BidiError::Protocol(_) | BidiError::MessageTooLarge { .. })
}

async fn connect_once(driver: &dyn Driver, cell: &CellName, port: u16, ready_within: Duration) -> Result<Session> {
    let stream = driver.stream(cell, &stream_argv(port)).await?;
    let session = Session::connect(stream, &format!("127.0.0.1:{port}")).await?;
    session.wait_ready(ready_within).await?;
    Ok(session)
}

struct Subscription {
    events: Vec<String>,
    tx: mpsc::UnboundedSender<Event>,
}

#[derive(Default)]
struct Inner {
    session: Option<Arc<Session>>,
    /// Counts connections, so a late "it ended" about an old one is ignored.
    generation: u64,
    subscriptions: Vec<Subscription>,
}

/// A browser in a cell that reconnects when it restarts.
///
/// A command that fails because the connection broke returns that error
/// (it may or may not have run, so it is not repeated for you). The next
/// call connects again and subscribes again to what was subscribed.
pub struct Browser {
    driver: Arc<dyn Driver>,
    cell: CellName,
    port: u16,
    options: ConnectOptions,
    inner: Arc<Mutex<Inner>>,
}

impl Browser {
    /// Connects once, retrying the stream while the browser starts, and
    /// returns when the protocol has answered. For a connection that
    /// survives restarts use [`Browser::supervise`].
    pub async fn connect(driver: &dyn Driver, cell: &CellName, port: u16) -> Result<Session> {
        Self::connect_with(driver, cell, port, ConnectOptions::default()).await
    }

    /// Like [`Browser::connect`] with your own limits.
    pub async fn connect_with(driver: &dyn Driver, cell: &CellName, port: u16, options: ConnectOptions) -> Result<Session> {
        let mut last = None;
        for attempt in 1..=options.attempts {
            match connect_once(driver, cell, port, options.ready_within).await {
                Ok(session) => return Ok(session),
                Err(e) if not_up_yet(&e) => {
                    last = Some(e.to_string());
                    if attempt < options.attempts && !options.gap.is_zero() {
                        tokio::time::sleep(options.gap).await;
                    }
                }
                Err(e) => return Err(e),
            }
        }
        Err(BrowserError::GaveUp { attempts: options.attempts, last: last.unwrap_or_else(|| "no attempt was made".to_owned()) })
    }

    /// A handle that connects when first used and again after the
    /// connection breaks.
    pub fn supervise(driver: Arc<dyn Driver>, cell: CellName, port: u16, options: ConnectOptions) -> Browser {
        Browser { driver, cell, port, options, inner: Arc::new(Mutex::new(Inner::default())) }
    }

    /// The current session, connecting first if there is none. Commands
    /// sent on it directly are not watched: if one fails with a closed
    /// connection, call [`Browser::lost`].
    pub async fn session(&self) -> Result<Arc<Session>> {
        let mut inner = self.inner.lock().await;
        if let Some(session) = &inner.session {
            return Ok(Arc::clone(session));
        }
        let session = Arc::new(Self::connect_with(self.driver.as_ref(), &self.cell, self.port, self.options).await?);
        inner.generation += 1;
        let generation = inner.generation;
        // Subscribe again to everything a caller asked for before.
        inner.subscriptions.retain(|s| !s.tx.is_closed());
        let mut forwards = Vec::new();
        for sub in &inner.subscriptions {
            let names: Vec<&str> = sub.events.iter().map(String::as_str).collect();
            forwards.push((session.subscribe(&names).await?, sub.tx.clone()));
        }
        for (rx, tx) in forwards {
            tokio::spawn(forward(Arc::clone(&self.inner), generation, rx, tx));
        }
        inner.session = Some(Arc::clone(&session));
        Ok(session)
    }

    /// Says the connection of `session` is gone, so the next call connects
    /// again. Does nothing if a newer connection already replaced it.
    pub async fn lost(&self, session: &Arc<Session>) {
        let mut inner = self.inner.lock().await;
        if inner.session.as_ref().is_some_and(|s| Arc::ptr_eq(s, session)) {
            inner.session = None;
        }
    }

    /// Sends one command on the current session.
    pub async fn send(&self, method: &str, params: Value) -> std::result::Result<Value, BrowserError> {
        let session = self.session().await?;
        let answer = session.send(method, params).await;
        if let Err(e) = &answer {
            if connection_lost(e) {
                self.lost(&session).await;
            }
        }
        Ok(answer?)
    }

    /// Asks for `events`. The returned channel keeps working across
    /// reconnects: the events are subscribed to again on each new session.
    pub async fn subscribe(&self, events: &[&str]) -> Result<mpsc::UnboundedReceiver<Event>> {
        let session = self.session().await?;
        let rx = session.subscribe(events).await?;
        let (tx, out) = mpsc::unbounded_channel();
        let mut inner = self.inner.lock().await;
        inner.subscriptions.push(Subscription { events: events.iter().map(|e| (*e).to_owned()).collect(), tx: tx.clone() });
        let generation = inner.generation;
        tokio::spawn(forward(Arc::clone(&self.inner), generation, rx, tx));
        Ok(out)
    }
}

/// Passes events on until the connection ends, then marks it lost so the
/// next call reconnects.
async fn forward(inner: Arc<Mutex<Inner>>, generation: u64, mut rx: mpsc::UnboundedReceiver<Event>, tx: mpsc::UnboundedSender<Event>) {
    while let Some(event) = rx.recv().await {
        if tx.send(event).is_err() {
            return;
        }
    }
    let mut inner = inner.lock().await;
    if inner.generation == generation {
        inner.session = None;
    }
}
