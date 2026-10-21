//! The gateway's front door (docs/design/network.md, section 2): one
//! listener that sends each request to its handler by path before anything
//! else is checked.
//!
//! - `/net/v1/…`: the network's routes, while this host is the network's
//!   gateway, each with its own body limit (1 MiB for an up batch, 16 MiB
//!   for a batch of relay frames, 64 KiB for the rest), counted while the
//!   body streams in. A link's down stream isn't cut after the ~100 s every
//!   other connection gets: it ends itself after ten minutes.
//! - `/a2a` and `/.well-known/agent-card.json`: A2A's binding, a hook the
//!   A2A module fills (`a2a` and `agent_card` below).
//! - `/mcp`, and anything else: the actor gateway, with every check it had
//!   (its host, no web pages, the bearer token first, 64 KiB).
//!
//! Two listeners use it. The actor gateway's, on 127.0.0.1 behind the
//! tunnel the owner picked, serves all of the above. A LAN-only gateway's,
//! on the address the person picked from the machine's interfaces (never
//! 0.0.0.0), serves `/net/v1/…` alone, for that `ip:port` in `Host`, so
//! actor tokens never cross a LAN in the clear.

use super::actor_gateway::{ActorGateway, CONNECTION_SECS};
use crate::http::{node_headers, parse_target, Reply, ReplyBody};
use crate::serve::{to_response, OutBody};
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use xenonet_net::gateway::{NetBody, NetReply, NetRequest, RELAY_LIMIT, SMALL_LIMIT, UP_LIMIT};
use xenonet_net::node::Node;

pub const NET_PREFIX: &str = "/net/v1/";
pub const A2A_PATH: &str = "/a2a";
pub const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
/// A link's stream ends itself after ten minutes; its connection gets a
/// little more.
const STREAM_SECS: u64 = 11 * 60;
const A2A_LIMIT: usize = 1 << 20;

/// Which listener a request came in on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorKind {
    /// The actor gateway's, on 127.0.0.1 behind a tunnel.
    Tunnel { port: u16 },
    /// A LAN-only gateway's, on this address.
    Lan(SocketAddr),
}

#[derive(Clone)]
pub struct Door {
    pub kind: DoorKind,
    pub actor: Option<ActorGateway>,
    /// The network's node; on the tunnel's door it's the actor gateway's.
    pub node: Option<Arc<Node>>,
}

/// A request for the A2A binding, read whole.
pub struct A2aRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The A2A binding's JSON-RPC at `/a2a`, for the actor whose gateway token
/// the front door checked (`auth`, as the registration answers it). The A2A
/// module fills this in; until then it isn't there.
#[expect(clippy::unused_async, reason = "the A2A module that fills it in awaits the engine")]
pub async fn a2a(_auth: &Value, _req: A2aRequest) -> Reply {
    Reply::error(404, "not found")
}

/// A2A's Agent Card, at `/.well-known/agent-card.json`, which anyone may
/// read. The A2A module fills this in; until then it isn't there.
#[expect(clippy::unused_async, reason = "the A2A module that fills it in reads the actors' settings")]
pub async fn agent_card(_req: A2aRequest) -> Reply {
    Reply::error(404, "not found")
}

fn plain(status: u16, body: &Value) -> hyper::Response<OutBody> {
    let mut r = Reply::raw(status, body.to_string(), &[]);
    r.headers.retain(|(k, _)| k != "content-type");
    r.headers.insert(0, ("content-type".into(), "application/json".into()));
    to_response(r)
}

fn not_found() -> hyper::Response<OutBody> {
    plain(404, &json!({ "error": "not found" }))
}

/// A body read as it streams, up to `limit`: the bytes, or None past it.
async fn read_body(body: Incoming, limit: usize) -> Option<Vec<u8>> {
    let mut body = body;
    let mut out = Vec::new();
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else { return Some(out) };
        if let Ok(d) = frame.into_data() {
            if out.len() + d.len() > limit {
                return None;
            }
            out.extend_from_slice(&d);
        }
    }
    Some(out)
}

impl Door {
    /// Accepts connections until the task is stopped; stopping it closes
    /// the connections it holds.
    pub async fn listen(self, listener: TcpListener) {
        let mut conns = tokio::task::JoinSet::new();
        loop {
            let Ok((stream, _)) = listener.accept().await else { continue };
            while conns.try_join_next().is_some() {}
            conns.spawn(self.clone().serve(stream));
        }
    }

    /// One connection: each request to its handler, and the connection cut
    /// after ~100 s unless it carried a link's stream.
    pub async fn serve(self, stream: TcpStream) {
        let streaming = Arc::new(AtomicBool::new(false));
        let flag = streaming.clone();
        let svc = service_fn(move |req: hyper::Request<Incoming>| {
            let (door, flag) = (self.clone(), flag.clone());
            async move { Ok::<_, Infallible>(door.route(req, &flag).await) }
        });
        let conn = http1::Builder::new().serve_connection(TokioIo::new(stream), svc);
        tokio::pin!(conn);
        tokio::select! {
            _ = &mut conn => return,
            () = tokio::time::sleep(Duration::from_secs(CONNECTION_SECS)) => {}
        }
        if streaming.load(Ordering::SeqCst) {
            let _ = tokio::time::timeout(Duration::from_secs(STREAM_SECS - CONNECTION_SECS), conn).await;
        }
    }

    fn node(&self) -> Option<Arc<Node>> {
        self.node.clone().or_else(|| self.actor.as_ref().and_then(ActorGateway::net))
    }

    async fn route(&self, req: hyper::Request<Incoming>, streaming: &AtomicBool) -> hyper::Response<OutBody> {
        let path = parse_target(&req.uri().to_string()).0;
        if path.starts_with(NET_PREFIX) {
            return self.network(req, streaming).await;
        }
        if let DoorKind::Lan(_) = self.kind {
            return not_found();
        }
        let Some(actor) = &self.actor else { return not_found() };
        if path == A2A_PATH || path == AGENT_CARD_PATH {
            let headers = node_headers(req.headers());
            let method = req.method().as_str().to_string();
            let target = req.uri().to_string();
            let auth = if path == A2A_PATH {
                match actor.door_auth(&headers) {
                    Ok(a) => Some(a),
                    Err(refused) => return to_response(refused),
                }
            } else {
                None
            };
            let Some(body) = read_body(req.into_body(), A2A_LIMIT).await else { return plain(413, &json!({ "error": "that body is over 1 MiB" })) };
            let r = A2aRequest { method, target, headers, body };
            let reply = match auth {
                Some(a) => a2a(&a, r).await,
                None => agent_card(r).await,
            };
            return to_response(reply);
        }
        actor.serve_hyper(req).await
    }

    /// Whether `Host` names this door: the LAN's own address, or the
    /// tunnel's public host or this machine on the gateway's port.
    fn host_ok(&self, host: &str) -> bool {
        match self.kind {
            DoorKind::Lan(addr) => host == addr.to_string(),
            DoorKind::Tunnel { port } => {
                let public = self.actor.as_ref().and_then(ActorGateway::public_host);
                public.as_deref() == Some(host) || host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
            }
        }
    }

    async fn network(&self, req: hyper::Request<Incoming>, streaming: &AtomicBool) -> hyper::Response<OutBody> {
        let headers = node_headers(req.headers());
        let header = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let host = header("host").unwrap_or("").to_ascii_lowercase();
        if !self.host_ok(&host) {
            return plain(421, &json!({ "error": "this gateway doesn't answer for that host" }));
        }
        if header("origin").is_some() {
            return plain(403, &json!({ "error": "the network's routes take no requests from web pages" }));
        }
        // Only while this host is the network's gateway.
        let Some(gateway) = self.node().and_then(|n| n.gateway()) else { return not_found() };
        let target = req.uri().path_and_query().map_or_else(|| req.uri().path().to_string(), |p| p.as_str().to_string());
        let path = target.split('?').next().unwrap_or("").to_string();
        let relay = target.split_once('?').is_some_and(|(_, q)| q.split('&').any(|p| p == "relay=1"));
        let limit = match path.as_str() {
            "/net/v1/frames" if relay => RELAY_LIMIT,
            "/net/v1/frames" => UP_LIMIT,
            _ => SMALL_LIMIT,
        };
        let method = req.method().as_str().to_string();
        let Some(body) = read_body(req.into_body(), limit).await else {
            return plain(413, &json!({ "error": format!("that body is over {} KiB", limit >> 10) }));
        };
        let reply = gateway.handle(NetRequest { method, target, headers, body }).await;
        if matches!(reply.body, NetBody::Stream(_)) {
            streaming.store(true, Ordering::SeqCst);
        }
        to_response(net_reply(reply))
    }
}

/// The network's answer as the service writes one.
pub fn net_reply(r: NetReply) -> Reply {
    let body = match r.body {
        NetBody::Full(b) => ReplyBody::Full(Bytes::from(b)),
        NetBody::Stream(rx) => ReplyBody::Stream(rx),
    };
    Reply { status: r.status, headers: r.headers, body }
}

/// A LAN-only gateway's listener: `/net/v1/…` alone, for `addr` in `Host`.
/// Never 0.0.0.0: the person picks an address this machine has.
pub async fn listen_lan(addr: SocketAddr, node: Arc<Node>) -> std::io::Result<tokio::task::JoinHandle<()>> {
    if addr.ip().is_unspecified() || addr.port() == 0 {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "a LAN address is one this machine has, with a port: never 0.0.0.0"));
    }
    let listener = TcpListener::bind(addr).await?;
    let door = Door { kind: DoorKind::Lan(addr), actor: None, node: Some(node) };
    Ok(tokio::spawn(door.listen(listener)))
}
