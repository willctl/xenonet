//! The API's requests as hyper hands them over: the Host and Origin
//! checks, the page's files, XenoNet Dev passing reads on to another
//! XenoNet, the routes and what their results answer; the answer as hyper
//! sends it; and Spotify's poll, which the routes poke.

use crate::borrowed::now_ms;
use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{node_headers, parse_target, raw_query, Body, By, Peer, Reply, ReplyBody, Req};
use crate::routes;
use bytes::Bytes;
use hyper::body::{Frame, Incoming};
use serde_json::Value;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_os::log::ErrInfo;
use xenonet_os::sync::Locked;

/// What an answer's body is to hyper: all of it, or a stream.
pub enum OutBody {
    Full(Option<Bytes>),
    Stream(mpsc::UnboundedReceiver<Bytes>),
}

impl hyper::body::Body for OutBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        match self.get_mut() {
            OutBody::Full(b) => Poll::Ready(b.take().filter(|b| !b.is_empty()).map(|b| Ok(Frame::data(b)))),
            OutBody::Stream(rx) => rx.poll_recv(cx).map(|b| b.map(|b| Ok(Frame::data(b)))),
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self, OutBody::Full(None))
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        match self {
            OutBody::Full(Some(b)) => hyper::body::SizeHint::with_exact(b.len() as u64),
            OutBody::Full(None) => hyper::body::SizeHint::with_exact(0),
            OutBody::Stream(_) => hyper::body::SizeHint::default(),
        }
    }
}

pub fn to_response(reply: Reply) -> hyper::Response<OutBody> {
    let mut b = hyper::Response::builder().status(reply.status);
    for (k, v) in &reply.headers {
        b = b.header(k.as_str(), v.as_str());
    }
    let body = match reply.body {
        ReplyBody::Full(bytes) => OutBody::Full(Some(bytes)),
        ReplyBody::Stream(rx) => OutBody::Stream(rx),
    };
    b.body(body).unwrap_or_else(|_| hyper::Response::new(OutBody::Full(None)))
}

/// One request, as server.on("request") handles it: transcripts are
/// private, so DNS-rebinding hosts and cross-site posts are refused before
/// anything else.
pub async fn handle(ctx: &Arc<Ctx>, req: hyper::Request<Incoming>, peer: Peer) -> Reply {
    let headers = node_headers(req.headers());
    let get = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let host = get("host").unwrap_or_default();
    let port = ctx.config.port;
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Reply::error(421, "bad host");
    }
    let method = req.method().as_str().to_string();
    if method != "GET" {
        if let Some(origin) = get("origin") {
            if origin != format!("http://127.0.0.1:{port}") && origin != format!("http://localhost:{port}") {
                return Reply::error(403, "bad origin");
            }
        }
    }
    let target = req.uri().to_string();
    let (path, query) = parse_target(&target);
    if !path.starts_with("/api/") {
        return crate::static_files::serve(&ctx.config.root.join("dist"), &path);
    }
    let incoming = req.into_body();
    let mut request = Req { method: method.clone(), path: path.clone(), query, raw_query: raw_query(&target), headers, peer, body: Body::Incoming(incoming), by: By::default() };
    // XenoNet Dev reading another XenoNet passes on only what this one's
    // rules let in.
    if let Some(up) = ctx.upstream.as_ref().filter(|u| u.handles(&path)) {
        if let Err(refused) = routes::rules::admit(ctx, &mut request).await {
            return refused;
        }
        return up.pass(&method, &target, &request.headers).await;
    }
    let answer = routes::api(ctx, request).await;
    answered(ctx, &method, &path, answer)
}

/// What the service answers for a route's result: what a core threw goes
/// out with its status, a core not in Rust yet says so, and anything else
/// is logged and an internal error.
pub fn answered(ctx: &Ctx, method: &str, path: &str, answer: Result<Reply, CoreError>) -> Reply {
    match answer {
        Ok(r) => r,
        Err(e) if e.is_pending() => Reply::coded(501, "pending", e.message),
        Err(CoreError { status: Some(status), message, .. }) => Reply::error(status, message),
        Err(e) => {
            eprintln!("{}", e.message);
            let short = shorten(path);
            ctx.log("error", "service.route", &format!("{method} {short}: {}", e.message), Some(&ErrInfo { message: Some(e.message), stack: None, code: None }), None);
            Reply::error(500, "internal error")
        }
    }
}

/// A path for the log, with long segments (ids, tokens) cut: [^/]{24,} → …
fn shorten(path: &str) -> String {
    path.split('/').map(|s| if s.chars().count() >= 24 { "…" } else { s }).collect::<Vec<_>>().join("/")
}

/// Pokes Spotify's poll to look again after `ms`.
pub fn poll_spotify_soon(ctx: &Ctx, ms: u64) {
    if let Some(tx) = &ctx.music.locked().again {
        let _ = tx.send(ms);
    }
}

// What's playing goes to the page, a few seconds apart while a window is
// open; with the dip on, your volume dips while a session waits on you.
pub async fn poll_spotify(ctx: Arc<Ctx>, mut again: mpsc::UnboundedReceiver<u64>) {
    let mut wait = 0u64;
    loop {
        tokio::select! {
            () = tokio::time::sleep(Duration::from_millis(wait)) => {}
            got = again.recv() => match got {
                Some(ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                }
                None => return,
            },
        }
        let s = &ctx.cores.spotify;
        wait = if ctx.events.count() > 0 { 5000 } else { 30000 };
        let looked: Result<(), CoreError> = async {
            let st = s.status().await?;
            if xenonet_text::text::truthy(st.get("connected")) {
                let now = s.player().await?;
                let text = xenonet_text::json::stringify(&now, 0);
                let changed = {
                    let mut m = ctx.music.locked();
                    let changed = m.last != text;
                    m.last = text;
                    changed
                };
                if changed {
                    ctx.broadcast("spotify", &now);
                }
                if xenonet_text::text::truthy(st.get("duck")) {
                    let waiting = ctx.cores.state.someone_waits(now_ms()).unwrap_or(false);
                    let held = ctx.music.locked().held.clone();
                    let plan = s.duck_plan(waiting, &now, st.get("device").unwrap_or(&Value::Null), &held)?;
                    ctx.music.locked().held = plan.get("held").cloned().unwrap_or(Value::Null);
                    if plan["op"] == "volume" {
                        s.command("volume", &plan["volume"]).await?;
                    }
                }
            }
            Ok(())
        }
        .await;
        match looked {
            Ok(()) => {}
            // Spotify isn't in the Rust service yet: no poll, and nothing to log.
            Err(e) if e.is_pending() => return,
            Err(e) => {
                // Spotify's asked XenoNet to wait, needs Premium, or isn't
                // reachable: say so once in a while, and look again later.
                let t = now_ms();
                let quiet = ctx.music.locked().quiet_until;
                if t > quiet {
                    ctx.log("warn", &format!("spotify.{}", e.code.as_deref().unwrap_or("error")), &e.message, None, None);
                    ctx.music.locked().quiet_until = t + 10.0 * 60e3;
                }
                wait = 60000;
            }
        }
    }
}
