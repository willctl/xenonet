//! The page's event stream, GET /api/events (server/index.js's broadcast):
//! `retry: 3000` first, a ping every 25 seconds, and each event as
//! `event: <name>\ndata: <JSON>\n\n` to every page.

use bytes::Bytes;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_os::sync::Locked;
use xenonet_text::json;

pub const PING: Duration = Duration::from_secs(25);

#[derive(Default)]
pub struct Events {
    clients: Mutex<Vec<mpsc::UnboundedSender<Bytes>>>,
}

pub fn frame(event: &str, data: &Value) -> Bytes {
    Bytes::from(format!("event: {event}\ndata: {}\n\n", json::stringify(data, 0)))
}

impl Events {
    pub fn new() -> Events {
        Events::default()
    }

    /// A page's stream: the receiver its answer reads from. It's dropped
    /// from the list when the page goes (its receiver is dropped).
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<Bytes> {
        let (tx, rx) = mpsc::unbounded_channel();
        let _ = tx.send(Bytes::from_static(b"retry: 3000\n\n"));
        let ping = tx.clone();
        tokio::spawn(async move {
            let mut every = tokio::time::interval_at(tokio::time::Instant::now() + PING, PING);
            loop {
                every.tick().await;
                if ping.send(Bytes::from_static(b": ping\n\n")).is_err() {
                    return;
                }
            }
        });
        self.clients.locked().push(tx);
        rx
    }

    /// How many pages are reading (Spotify looks more often while one is).
    pub fn count(&self) -> usize {
        let mut clients = self.clients.locked();
        clients.retain(|c| !c.is_closed());
        clients.len()
    }

    /// The same event to every page.
    pub fn send(&self, event: &str, data: &Value) {
        let msg = frame(event, data);
        self.clients.locked().retain(|c| c.send(msg.clone()).is_ok());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn every_page_hears_each_event() {
        let e = Events::new();
        let mut page = e.subscribe();
        let mut other = e.subscribe();
        assert_eq!(page.recv().await.unwrap(), Bytes::from_static(b"retry: 3000\n\n"));
        other.recv().await.unwrap();
        e.send("graph", &json!({ "n": 2 }));
        assert_eq!(page.recv().await.unwrap(), Bytes::from("event: graph\ndata: {\"n\":2}\n\n"));
        assert_eq!(other.recv().await.unwrap(), Bytes::from("event: graph\ndata: {\"n\":2}\n\n"));
        assert_eq!(e.count(), 2);
        drop(page);
        e.send("x", &json!(null));
        assert_eq!(e.count(), 1);
    }
}
