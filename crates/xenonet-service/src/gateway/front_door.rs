//! The actor gateway's front door (docs/design/network.md, section 2): one
//! listener, on 127.0.0.1 behind the tunnel the owner picked, that sends
//! each request to its handler by path before anything else is checked.
//! It's a surface onto this machine for actors outside the network, and
//! nothing of the network between machines passes through it: machines
//! reach each other on link/2.
//!
//! - `/a2a` and `/.well-known/agent-card.json`: A2A's binding (a2a/),
//!   while the owner has it on, and a 404 otherwise. The card is public;
//!   `/a2a` takes a gateway token, checked as `/mcp` checks one, and an
//!   answer that streams keeps its connection past the usual cut.
//! - `/mcp`, and anything else: the actor gateway, with every check it had
//!   (its host, no web pages, the bearer token first, 64 KiB).

use super::actor_gateway::{ActorGateway, CONNECTION_SECS};
use crate::a2a::{A2aActor, A2aRequest};
use crate::http::{node_headers, parse_target, Reply, ReplyBody};
use crate::server::{to_response, OutBody};
use http_body_util::BodyExt;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use xenonet_os::sync::Locked;

pub const A2A_PATH: &str = "/a2a";
pub const AGENT_CARD_PATH: &str = "/.well-known/agent-card.json";
/// What an A2A stream's connection gets past the stream's own end.
const A2A_STREAM_SLACK: Duration = Duration::from_secs(15);

/// The actor gateway's door, on its port.
#[derive(Clone)]
pub struct Door {
    pub port: u16,
    pub actor: ActorGateway,
}

/// A JSON answer, as the actor gateway writes its own.
fn reply(status: u16, body: &Value, extra: &[(&str, &str)]) -> Reply {
    let mut r = Reply::raw(status, body.to_string(), extra);
    r.headers.retain(|(k, _)| k != "content-type");
    r.headers.insert(0, ("content-type".into(), "application/json".into()));
    r
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
/// answer it carries streams (an A2A stream), so the
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

    async fn route(&self, req: hyper::Request<Incoming>, keep: &Keep) -> hyper::Response<OutBody> {
        let path = parse_target(&req.uri().to_string()).0;
        let actor = &self.actor;
        if path == AGENT_CARD_PATH {
            return to_response(agent_card(actor, &req));
        }
        if path == A2A_PATH {
            return to_response(a2a(actor, req, keep).await);
        }
        actor.serve_hyper(req).await
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
/// web pages, no token in the URL, unknown tokens held off); the gateway
/// token, refused as `/mcp` refuses one; a 404 while A2A is off, so only a
/// token's holder learns whether it's on; at most 64 KiB, counted as it
/// comes; the call counted for the window; then A2A's handler, as the
/// actor the token is for. A streamed answer keeps its connection until
/// the stream has had its time.
async fn a2a(actor: &ActorGateway, req: hyper::Request<Incoming>, keep: &Keep) -> Reply {
    let headers = node_headers(req.headers());
    let target = req.uri().path_and_query().map_or_else(|| req.uri().path().to_string(), |p| p.as_str().to_string());
    if let Some(refused) = actor.door_open(&headers, &target) {
        return refused;
    }
    let auth = match actor.door_token(&headers) {
        Ok(a) => a,
        Err(refused) => return refused,
    };
    let Some(a2a) = actor.a2a().filter(|a| a.enabled()) else { return reply(404, &json!({ "error": "not found" }), &[]) };
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
