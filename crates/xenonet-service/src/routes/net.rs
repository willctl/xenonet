//! The network on this host's own API, `/api/net/…` (docs/design/
//! network.md, "The wire" and section 8), for its window and the `xenonet
//! net` command. Every route is the owner's: from XenoNet's window or the
//! person's own terminal, never from under an agent or with an actor's
//! token. `POST ticket` (**Add a machine**) is the window's, or a
//! terminal's on macOS alone, where process checks can tell the person
//! from an agent; it answers the join line once, and nothing keeps it.
//!
//! - `GET state`: `network/state/v1`.
//! - `POST create`, `POST join`, `POST leave`: on this machine only.
//! - `POST ticket`, `POST remove`, `POST replace`.
//! - `POST approve`: signs an entry that waits for the founder or a second
//!   window host (a move of the gateway, or a window host's add), by the
//!   id the state lists it under.
//! - `POST gateway/take`, `POST gateway/set`: moving the gateway by hand.
//! - `PUT settings`: titles only, and a LAN-only gateway's address.

use crate::ctx::{Answer, Ctx};
use crate::gateway::network::{parse_lan, NetService};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_net::{NetError, R};

/// The network's state as a development instance's `simulate` set it, or
/// None (net_sim.rs). The console's `simulate` fills this in, so its
/// network states show without real machines; `GET state` answers it first
/// when there's one.
pub fn simulated_net_state(ctx: &Ctx) -> Option<Value> {
    super::net_sim::simulated_net_state(ctx)
}

fn refused(e: &NetError) -> Reply {
    Reply::json(e.status, json!({ "error": e.message, "code": e.code }))
}

/// The actor gateway's public host, which a network through its tunnel
/// is reached at.
fn public_host(ctx: &Ctx) -> Option<String> {
    ctx.gateway.get().and_then(|g| g.gateway().public_host())
}

/// Whether a request may make a ticket: the window's, or on macOS the
/// person's own terminal. The reason it may not, or None.
async fn ticket_check(ctx: &Ctx, req: &Req) -> Option<String> {
    if req.header("x-xenonet-window").is_some() || req.header("authorization").is_some() {
        return ctx.from_window(req).await;
    }
    if !cfg!(target_os = "macos") {
        return Some("Add a machine is XenoNet's window's: choose Add a machine in the palette".into());
    }
    ctx.from_you(req).await
}

/// `/api/net/…`, or None for any other path.
pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Option<Answer> {
    let path = req.path.clone();
    let rest = path.strip_prefix("/api/net/")?.to_string();
    let method = req.method.clone();
    Some(Ok(answer(ctx, req, &method, &rest).await))
}

async fn answer(ctx: &Arc<Ctx>, req: &mut Req, method: &str, rest: &str) -> Reply {
    let known = matches!(
        (method, rest),
        ("GET", "state") | ("POST", "create" | "join" | "leave" | "ticket" | "remove" | "replace" | "approve" | "gateway/take" | "gateway/set") | ("PUT", "settings")
    );
    if !known {
        return Reply::error(404, "not found");
    }
    let why = if rest == "ticket" { ticket_check(ctx, req).await } else { ctx.from_you(req).await };
    if let Some(why) = why {
        return Reply::coded(403, "yours", why);
    }
    if rest == "state" {
        if let Some(s) = simulated_net_state(ctx) {
            return Reply::json(200, super::net_sim::with_own_agents(ctx, s).await);
        }
    }
    let Some(svc) = ctx.net.get().cloned() else { return Reply::coded(503, "no-network", "the network isn't available in this XenoNet") };
    let body = if method == "GET" {
        Value::Null
    } else {
        match req.json(64 << 10).await {
            Ok(b) => b,
            Err(e) => return Reply::error(e.status, e.message),
        }
    };
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let on = |k: &str| body.get(k) == Some(&Value::Bool(true));
    let node = svc.node();
    let result = match rest {
        "state" => Ok(svc.state()),
        "create" => create(ctx, &svc, &body).await,
        "join" => node.join(&text("address"), &text("ticket")).await,
        "leave" => {
            let r = node.leave(on("force")).await;
            if r.is_ok() {
                svc.unlisten();
            }
            r
        }
        "ticket" => {
            node.tunnel_address(public_host(ctx).as_deref());
            node.ticket(&text("host"), on("window")).await
        }
        "remove" => node.remove(&text("host")).await,
        "replace" => node.replace(&text("host")).await,
        "settings" => settings(&svc, &body).await,
        "approve" => node.approve(&text("id")).await,
        "gateway/take" => take(ctx, &svc, &body).await,
        _ => node.set_gateway(&text("address")).await,
    };
    match result {
        Ok(v) => Reply::json(200, v),
        // The ticket's maker hasn't signed this machine in yet, or a second
        // window host hasn't approved it: the join is taken, and waits.
        // The window and the CLI say why and ask again.
        Err(e) if e.code == "waiting" => Reply::json(202, json!({ "waiting": true, "message": e.message })),
        Err(e) => refused(&e),
    }
}

async fn create(ctx: &Ctx, svc: &NetService, body: &Value) -> R<Value> {
    let node = svc.node();
    if let Some(n) = node.shared().network() {
        return Err(NetError::new(409, "in-network", format!("this machine is in the network {} already: xenonet net leave first", n.name)));
    }
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let lan = match body.get("lan").and_then(Value::as_str).filter(|l| !l.trim().is_empty()) {
        Some(l) => Some(parse_lan(l)?),
        None => None,
    };
    // The LAN listener first: an address this machine can't listen on is
    // refused before the network is made.
    if let Some(addr) = lan {
        svc.listen(addr).await?;
    }
    let lan_text = lan.map(|a| a.to_string());
    let made = node.create(&text("name"), &text("host"), public_host(ctx).as_deref(), lan_text.as_deref());
    if made.is_err() && lan.is_some() {
        svc.unlisten();
    }
    made
}

/// **Make this the network's gateway**: at the LAN address given, which it
/// listens on first, or at the actor gateway's tunnel.
async fn take(ctx: &Ctx, svc: &NetService, body: &Value) -> R<Value> {
    let lan = match body.get("lan").and_then(Value::as_str).filter(|l| !l.trim().is_empty()) {
        Some(l) => Some(parse_lan(l)?),
        None => None,
    };
    if let Some(addr) = lan {
        svc.listen(addr).await?;
    }
    let lan_text = lan.map(|a| a.to_string());
    let took = svc.node().take_gateway(public_host(ctx).as_deref(), lan_text.as_deref()).await;
    if took.is_err() && lan.is_some() && !svc.node().shared().is_gateway() {
        svc.unlisten();
    }
    took
}

async fn settings(svc: &NetService, body: &Value) -> R<Value> {
    let titles = body.get("titlesOnly").and_then(Value::as_bool);
    let lan = match body.get("lan") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s.trim().is_empty() => Some(None),
        Some(Value::String(s)) => Some(Some(parse_lan(s)?)),
        Some(_) => return Err(NetError::bad("lan is an address, or null")),
    };
    if let Some(l) = lan {
        if !svc.node().shared().is_gateway() {
            return Err(NetError::new(409, "not-gateway", "the LAN address is the gateway's: set it on its host"));
        }
        match l {
            Some(addr) => svc.listen(addr).await?,
            None => svc.unlisten(),
        }
    }
    svc.node().settings(titles, lan.map(|l| l.map(|a| a.to_string())))
}
