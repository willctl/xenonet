//! The page's event stream, GET /api/events (server/index.js's broadcast):
//! `retry: 3000` first, a ping every 25 seconds, and each event as
//! `event: <name>\ndata: <JSON>\n\n` to every page. With hosts on, the
//! stream a relay follows (?hosts=0) gets this machine's own view of an
//! event, which is nothing for what came from another host.

use bytes::Bytes;
use serde_json::Value;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_base::json;
use xenonet_base::sync::Locked;

pub const PING: Duration = Duration::from_secs(25);

struct Client {
    tx: mpsc::UnboundedSender<Bytes>,
    own_only: bool,
}

/// What a relay's stream gets of an event.
pub enum Local<'a> {
    /// The same as every page.
    Same,
    /// Nothing: it came from another host.
    None,
    /// This machine's own view.
    Own(&'a Value),
}

#[derive(Default)]
pub struct Events {
    clients: Mutex<Vec<Client>>,
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
    pub fn subscribe(&self, own_only: bool) -> mpsc::UnboundedReceiver<Bytes> {
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
        self.clients.locked().push(Client { tx, own_only });
        rx
    }

    /// How many pages are reading (Spotify looks more often while one is).
    pub fn count(&self) -> usize {
        let mut clients = self.clients.locked();
        clients.retain(|c| !c.tx.is_closed());
        clients.len()
    }

    pub fn broadcast(&self, event: &str, data: &Value, local: Local<'_>) {
        let msg = frame(event, data);
        let own = match local {
            Local::Same => Some(msg.clone()),
            Local::None => None,
            Local::Own(v) => Some(frame(event, v)),
        };
        let mut clients = self.clients.locked();
        clients.retain(|c| {
            let m = if c.own_only { own.clone() } else { Some(msg.clone()) };
            match m {
                Some(m) => c.tx.send(m).is_ok(),
                None => !c.tx.is_closed(),
            }
        });
    }

    /// broadcast(event, data): the same to every page.
    pub fn send(&self, event: &str, data: &Value) {
        self.broadcast(event, data, Local::Same);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn every_page_hears_each_event_and_a_relay_its_own() {
        let e = Events::new();
        let mut page = e.subscribe(false);
        let mut relay = e.subscribe(true);
        assert_eq!(page.recv().await.unwrap(), Bytes::from_static(b"retry: 3000\n\n"));
        relay.recv().await.unwrap();
        e.broadcast("graph", &json!({ "n": 2 }), Local::Own(&json!({ "n": 1 })));
        assert_eq!(page.recv().await.unwrap(), Bytes::from("event: graph\ndata: {\"n\":2}\n\n"));
        assert_eq!(relay.recv().await.unwrap(), Bytes::from("event: graph\ndata: {\"n\":1}\n\n"));
        e.broadcast("hosts", &json!([]), Local::None);
        e.send("live", &json!(1));
        assert_eq!(page.recv().await.unwrap(), Bytes::from("event: hosts\ndata: []\n\n"));
        assert_eq!(relay.recv().await.unwrap(), Bytes::from("event: live\ndata: 1\n\n"));
        drop(page);
        e.send("x", &json!(null));
        assert_eq!(e.count(), 1);
    }
}
