//! The gateway's front door (docs/design/network.md, section 2): one
//! listener that sends each request to its handler by path before anything
//! else is checked.
//!
//! - `/net/v1/…`: the network's routes, while this host is the network's
//!   gateway, each with its own body limit (1 MiB for an up batch, 16 MiB
//!   for a batch of relay frames, 64 KiB for the rest), counted while the
//!   body streams in. A link's down stream isn't cut after the ~100 s every
//!   other connection gets: it ends itself after ten minutes.
//! - `/a2a` and `/.well-known/agent-card.json`: A2A's binding (a2a/),
//!   while the owner has it on, and a 404 otherwise. The card is public;
//!   `/a2a` takes a gateway token, checked as `/mcp` checks one, and an
//!   answer that streams keeps its connection past the usual cut.
//! - `/mcp`, and anything else: the actor gateway, with every check it had
//!   (its host, no web pages, the bearer token first, 64 KiB).
//!
//! Two listeners use it. The actor gateway's, on 127.0.0.1 behind the
//! tunnel the owner picked, serves all of the above. A LAN-only gateway's,
//! on the address the person picked from the machine's interfaces (never
//! 0.0.0.0), serves `/net/v1/…` alone, for that `ip:port` in `Host`, so
//! actor tokens never cross a LAN in the clear.

use super::actor_gateway::{ActorGateway, CONNECTION_SECS};
use crate::a2a::{A2aActor, A2aRequest};
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
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use xenonet_base::sync::Locked;
use xenonet_net::gateway::{NetBody, NetReply, NetRequest, RELAY_LIMIT, SMALL_LIMIT, UP_LIMIT};
use xenonet_net::node::Node;

pub const NET_PREFIX: &str = "/net/v1/";
pub const A2A_PATH: &str = "/a2a";
pub const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
/// A link's stream ends itself after ten minutes; its connection gets a
/// little more.
const STREAM_SECS: u64 = 11 * 60;
/// What an A2A stream's connection gets past the stream's own end.
const A2A_STREAM_SLACK: Duration = Duration::from_secs(15);

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

/// A JSON answer, as the actor gateway writes its own.
fn reply(status: u16, body: &Value, extra: &[(&str, &str)]) -> Reply {
    let mut r = Reply::raw(status, body.to_string(), extra);
    r.headers.retain(|(k, _)| k != "content-type");
    r.headers.insert(0, ("content-type".into(), "application/json".into()));
    r
}

fn plain(status: u16, body: &Value) -> hyper::Response<OutBody> {
    to_response(reply(status, body, &[]))
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

/// When a connection is cut: ~100 s after it opened, or later while an
/// answer it carries streams (a link's down stream, an A2A stream), so the
/// cut never ends a stream before the stream ends itself.
#[derive(Clone)]
struct Keep(Arc<Mutex<Instant>>);

impl Keep {
    fn new() -> Keep {
        Keep(Arc::new(Mutex::new(Instant::now() + Duration::from_secs(CONNECTION_SECS))))
    }

    /// The connection lives at least `d` from now.
    fn at_least(&self, d: Duration) {
        let mut until = self.0.locked();
        *until = (*until).max(Instant::now() + d);
    }

    fn until(&self) -> Instant {
        *self.0.locked()
    }
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
    /// after ~100 s, or once the streams it carried have had their time.
    pub async fn serve(self, stream: TcpStream) {
        let keep = Keep::new();
        let held = keep.clone();
        let svc = service_fn(move |req: hyper::Request<Incoming>| {
            let (door, keep) = (self.clone(), held.clone());
            async move { Ok::<_, Infallible>(door.route(req, &keep).await) }
        });
        let conn = http1::Builder::new().serve_connection(TokioIo::new(stream), svc);
        tokio::pin!(conn);
        loop {
            let until = keep.until();
            tokio::select! {
                _ = &mut conn => return,
                () = tokio::time::sleep_until(until) => {}
            }
            if keep.until() <= Instant::now() {
                return;
            }
        }
    }

    fn node(&self) -> Option<Arc<Node>> {
        self.node.clone().or_else(|| self.actor.as_ref().and_then(ActorGateway::net))
    }

    async fn route(&self, req: hyper::Request<Incoming>, keep: &Keep) -> hyper::Response<OutBody> {
        let path = parse_target(&req.uri().to_string()).0;
        if path.starts_with(NET_PREFIX) {
            return self.network(req, keep).await;
        }
        // A LAN-only door serves the network alone: no actor token, and
        // nothing of A2A, crosses a LAN in the clear.
        if let DoorKind::Lan(_) = self.kind {
            return not_found();
        }
        let Some(actor) = &self.actor else { return not_found() };
        if path == AGENT_CARD_PATH {
            return to_response(agent_card(actor, &req));
        }
        if path == A2A_PATH {
            return to_response(a2a(actor, req, keep).await);
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

    async fn network(&self, req: hyper::Request<Incoming>, keep: &Keep) -> hyper::Response<OutBody> {
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
        // Read as the gateway's route reads it, so the two can't differ.
        let relay = NetRequest { target: target.clone(), ..NetRequest::default() }.query("relay").as_deref() == Some("1");
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
            keep.at_least(Duration::from_secs(STREAM_SECS));
        }
        to_response(net_reply(reply))
    }
}

/// A2A's Agent Card at `/.well-known/agent-card.json`: anyone may read it,
/// with `/mcp`'s checks of the host and web pages; a 404 while A2A is off.
/// Its address is the public host's, or this machine's when it's read
/// from here.
pub fn agent_card(actor: &ActorGateway, req: &hyper::Request<Incoming>) -> Reply {
    let headers = node_headers(req.headers());
    if let Some(refused) = actor.door_host(&headers) {
        return refused;
    }
    if req.method() != hyper::Method::GET {
        return reply(405, &json!({ "error": "the Agent Card is read with GET" }), &[("allow", "GET")]);
    }
    let Some(a2a) = actor.a2a() else { return reply(404, &json!({ "error": "not found" }), &[]) };
    let host = headers.iter().find(|(k, _)| k == "host").map_or("", |(_, v)| v.as_str()).to_ascii_lowercase();
    let address = match actor.public_host() {
        Some(public) if public == host => format!("https://{public}"),
        _ => format!("http://{host}"),
    };
    match a2a.card(&address) {
        Some(card) => reply(200, &card, &[("cache-control", "no-cache")]),
        None => reply(404, &json!({ "error": "not found" }), &[]),
    }
}

/// A2A's JSON-RPC at `/a2a`, in this order: `/mcp`'s checks (the host, no
/// web pages, no token in the URL, unknown tokens held off); a 404 while
/// A2A is off; the gateway token, refused as `/mcp` refuses one; at most
/// 64 KiB, counted as it comes; the call counted for the window; then
/// A2A's handler, as the actor the token is for. A streamed answer keeps
/// its connection until the stream has had its time.
async fn a2a(actor: &ActorGateway, req: hyper::Request<Incoming>, keep: &Keep) -> Reply {
    let headers = node_headers(req.headers());
    let target = req.uri().path_and_query().map_or_else(|| req.uri().path().to_string(), |p| p.as_str().to_string());
    if let Some(refused) = actor.door_open(&headers, &target) {
        return refused;
    }
    let Some(a2a) = actor.a2a().filter(|a| a.enabled()) else { return reply(404, &json!({ "error": "not found" }), &[]) };
    let auth = match actor.door_token(&headers) {
        Ok(a) => a,
        Err(refused) => return refused,
    };
    let method = req.method().as_str().to_string();
    let Some(body) = read_body(req.into_body(), crate::a2a::BODY_MAX).await else {
        return reply(413, &json!({ "error": "that body is over 64 KiB" }), &[]);
    };
    let host = headers.iter().find(|(k, _)| k == "host").map_or("", |(_, v)| v.as_str()).to_ascii_lowercase();
    actor.door_called(&auth, &host);
    let Some(who) = A2aActor::from_auth(&auth) else {
        return reply(403, &json!({ "error": "A2A is for actors, by their gateway tokens" }), &[]);
    };
    let reply = a2a.handle(&who, A2aRequest { method, target, headers, body }).await;
    if matches!(reply.body, ReplyBody::Stream(_)) {
        keep.at_least(a2a.stream_limit() + A2A_STREAM_SLACK);
    }
    reply
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn a_stream_keeps_its_connection_past_the_usual_cut_and_never_shortens_it() {
        let keep = Keep::new();
        let opened = Instant::now();
        assert_eq!(keep.until() - opened, Duration::from_secs(CONNECTION_SECS));
        // A short stream early on doesn't bring the cut forward.
        keep.at_least(Duration::from_secs(5));
        assert_eq!(keep.until() - opened, Duration::from_secs(CONNECTION_SECS));
        // An A2A stream that starts at 90 s has its 85 s and the slack.
        tokio::time::advance(Duration::from_secs(90)).await;
        keep.at_least(Duration::from_secs(85) + A2A_STREAM_SLACK);
        assert_eq!(keep.until() - opened, Duration::from_secs(90 + 85) + A2A_STREAM_SLACK);
    }
}
