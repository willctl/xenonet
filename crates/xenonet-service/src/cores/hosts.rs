//! Hosts (core/hosts/): this XenoNet as a host (config.js), and the hub
//! that shows other machines' XenoNets: the hosts in hosts.json over SSH
//! with XENONET_HOSTS=1 (hub.js), and the hosts of this machine's network
//! through its gateway (docs/design/network.md, section 7).

use super::{pending, Pending, R};
use crate::events::Events;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use xenonet_base::network::Net;
use xenonet_net::sink::SinkSlot;

const CONFIG: &str = "core/hosts/config.js";
const HUB: &str = "core/hosts/hub.js";

pub trait HostsConfig: Send + Sync {
    /// `hosts.self()`: its name, its key, the folder ~ stands for.
    fn self_info(&self) -> R<Value> {
        pending(CONFIG)
    }
    fn set_self(&self, _id: &Value) -> R<Value> {
        pending(CONFIG)
    }
    fn ssh_config(&self) -> R<Value> {
        pending(CONFIG)
    }
}

/// A host's answer through its relay: `{ status, body }`.
pub struct HubAnswer {
    pub status: u16,
    pub body: Value,
}

/// This machine's own API, read as any local program reads it: a GET with
/// no key, so its routes' own checks and the relay's allowlist both apply.
/// The relay that serves this host's map to another host reads through it.
#[async_trait]
pub trait SelfApi: Send + Sync {
    /// The status and JSON of `path`, or why there was none and the status
    /// to answer with.
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)>;
}

/// The API over HTTP on 127.0.0.1, as `xenonet relay` reads it. It sends
/// no window key and no token: whatever it reads, any program here could.
pub struct HttpApi {
    pub port: u16,
    /// A read's longest wait, and its biggest answer.
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl HttpApi {
    /// The headers a read sends: nothing but what it accepts.
    pub const HEADERS: [(&'static str, &'static str); 1] = [("accept", "application/json")];
}

#[async_trait]
impl SelfApi for HttpApi {
    async fn get(&self, path: &str) -> Result<(u16, Value), (String, u16)> {
        let (port, route, timeout, max) = (self.port, path.to_string(), self.timeout, self.max_bytes);
        let headers: Vec<(String, String)> = Self::HEADERS.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
        let r = tokio::task::spawn_blocking(move || xenonet_base::http::request(port, "GET", &route, &headers, None, Some(timeout))).await.map_err(|e| (e.to_string(), 502))?;
        let a = r.map_err(|e| match e.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => (format!("no answer in {} s", timeout.as_secs()), 504),
            _ => (e.to_string(), 502),
        })?;
        let body = a.body.map_err(|e| (e.to_string(), 502))?;
        if body.len() > max {
            return Err(("too big to relay".into(), 413));
        }
        let text = String::from_utf8_lossy(&body);
        if text.is_empty() {
            return Ok((a.status, Value::Null));
        }
        Ok(match xenonet_base::json::parse(&text) {
            Ok(v) => (a.status, v),
            Err(_) => (502, serde_json::json!({ "error": "XenoNet answered with something that isn't JSON" })),
        })
    }
}

/// What the hub needs of the network once the service has it: the network
/// itself, where its relay sinks go, and, for the relay that serves this
/// host's map to a console on another host, this machine's event stream
/// and its own API.
pub struct HubNet {
    pub net: Arc<dyn Net>,
    pub sinks: Arc<SinkSlot>,
    pub events: Arc<Events>,
    pub api: Arc<dyn SelfApi>,
    pub home: String,
    pub version: String,
    pub platform: String,
    pub demo: bool,
}

#[async_trait]
pub trait Hub: Send + Sync {
    fn has(&self, _host: &str) -> bool {
        false
    }
    async fn get(&self, _host: &str, _path: &str) -> R<HubAnswer> {
        pending(HUB)
    }
    fn list(&self) -> R<Value> {
        pending(HUB)
    }
    fn add(&self, _body: &Value, _replace: bool) -> R<Value> {
        pending(HUB)
    }
    fn detail(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    fn remove(&self, _id: &str, _forget: bool) -> R<Value> {
        pending(HUB)
    }
    fn reconnect(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    async fn test(&self, _id: &str) -> R<Value> {
        pending(HUB)
    }
    /// This machine's map with other hosts' in it.
    fn graph(&self, g: Value) -> Value {
        g
    }
    fn live(&self, s: Value) -> Value {
        s
    }
    fn summary(&self, s: Value) -> Value {
        s
    }
    /// The latest load each other host sent (system/v1, under `host`), for
    /// GET /api/system?hosts=1: a network host's relay sends one every few
    /// seconds while this console watches it, unless it doesn't share it.
    fn systems(&self) -> Value {
        Value::Array(Vec::new())
    }
    /// The host-… simulations (dev only).
    fn simulate(&self, _kind: &str, _args: &Value) -> R<Option<Value>> {
        pending(HUB)
    }
    /// Whether hosts over SSH (hosts.json, XENONET_HOSTS=1) are on: their
    /// routes answer only then.
    fn ssh(&self) -> bool {
        true
    }
    /// The network, once the service has it: the hub's relay sinks go in,
    /// and its network hosts come from the roster.
    fn plug_net(&self, _net: HubNet) {}
    /// Whether a console is open here: Desktop's window (shown, hidden or
    /// closed, as it last said), and how many pages read the event stream.
    /// The hub subscribes to network hosts' maps only while one is.
    fn console(&self, _window: Option<&str>, _pages: usize) {}
    fn start(&self) {}
    fn stop(&self) {}
}

impl HostsConfig for Pending {}
impl Hub for Pending {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    // The relay's reads of this machine's own API carry no key and no
    // token: whatever another host asks for, any program here could read.
    #[tokio::test]
    async fn the_relays_own_reads_carry_no_key() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).unwrap();
            let body = "{\"nodes\":[]}";
            write!(s, "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).unwrap();
            String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase()
        });
        let api = HttpApi { port, timeout: Duration::from_secs(5), max_bytes: 1 << 20 };
        assert_eq!(api.get("/api/graph?hosts=0").await.unwrap(), (200, serde_json::json!({ "nodes": [] })));
        let request = seen.join().unwrap();
        assert!(request.starts_with("get /api/graph?hosts=0 http/1.1"), "{request}");
        assert!(!request.contains("x-xenonet-window") && !request.contains("authorization") && !request.contains("x-xenonet-agent"), "{request}");
    }
}
