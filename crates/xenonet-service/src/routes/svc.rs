//! Services other hosts of the network published, on this host's own API:
//! `/api/svc/<host>/<name>/<path>` (docs/design/network.md, "Services
//! through the network"). Any program on this machine may call it, with
//! any method a service takes, as it would call the service on its own
//! machine: the request goes to `<host>` sealed in a `svc` frame, and the
//! service's answer comes back as this route's answer.
//!
//! It's on this local API only, never under `/net/v1/`, which the front
//! door hands to the network's gateway on its LAN and tunnel doors. The
//! API's own checks apply (`Host` is this machine's), and a request from a
//! web page is refused: it has an `Origin`, or a browser's `Sec-Fetch-Site`.
//! `Authorization` is the service's own credential: it passes untouched,
//! and is never read here as an actor's token.

use crate::ctx::{Answer, Ctx};
use crate::http::{Reply, ReplyBody, Req, TOO_BIG};
use std::sync::Arc;
use xenonet_net::svc::{self, Call};

/// `/api/svc/…`, or None for any other path.
pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Option<Answer> {
    let rest = req.path.strip_prefix("/api/svc/")?.to_string();
    Some(Ok(answer(ctx, req, &rest).await))
}

fn reply(a: svc::Answer) -> Reply {
    Reply { status: a.status, headers: a.headers, body: ReplyBody::Full(a.body.into()) }
}

async fn answer(ctx: &Arc<Ctx>, req: &mut Req, rest: &str) -> Reply {
    if req.header("origin").is_some() || req.header("sec-fetch-site").is_some_and(|s| s != "none") {
        return reply(svc::Answer::refused(403, "web-page", "a web page can't reach other hosts' services through XenoNet"));
    }
    let mut parts = rest.splitn(3, '/');
    let (host, name) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = format!("/{}", parts.next().unwrap_or(""));
    let path = match &req.raw_query {
        Some(q) => format!("{path}?{q}"),
        None => path,
    };
    let Some(net) = ctx.net.get().cloned() else { return reply(svc::Answer::refused(503, "no-network", "the network isn't available in this XenoNet")) };
    let body = match req.bytes(svc::REQUEST_BODY).await {
        Ok(b) => b,
        Err(e) if e == TOO_BIG => return reply(svc::Answer::refused(413, "too-big", "a request's body is at most 1 MiB")),
        Err(_) => return reply(svc::Answer::refused(400, "bad-request", "the request's body didn't all come")),
    };
    let call = Call { method: req.method.clone(), path, headers: req.headers.clone(), body };
    reply(net.node().shared().svc_call(host, name, call).await)
}
