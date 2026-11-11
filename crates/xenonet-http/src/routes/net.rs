//! The network on this host's own API, `/api/net/…` (docs/design/
//! network.md, "The wire" and section 8), for its window and the `xenonet
//! net` command. Every route is the owner's: from XenoNet's window or the
//! person's own terminal, never from under an agent or with an actor's
//! token. What asks the person (their presence, Touch ID on a Mac) is done
//! only for a request that came with the window's key: making the first
//! network (it sets up their identity), **Add a machine** (`POST ticket`,
//! whose join then asks to sign the machine in; it answers the join line
//! once, and nothing keeps it), **Replace**, approving or making an entry
//! that admits or widens, and moving the gateway. Anything else is refused
//! there, never prompted for.
//!
//! - `GET state`: `network/state/v1`.
//! - `POST create`, `POST join`, `POST leave`: on this machine only.
//! - `POST ticket`, `POST remove`, `POST replace`.
//! - `POST approve`: signs an entry that waits for the founder or a second
//!   window host (a move of the gateway, or a window host's add), by the
//!   id the state lists it under.
//! - `POST gateway/take`, `POST gateway/set`: moving the gateway by hand.
//! - `PUT settings`: titles only, whether this machine shares its load,
//!   who may start and wake sessions here (`start`, `wake`), and a
//!   LAN-only gateway's address. With `host`, another machine's `start`
//!   and `wake`. Start and wake are settings entries in the network's
//!   chain, signed here (with presence where they widen); `approve` signs
//!   one that waits, by its id.
//! - `GET svc`, `POST svc/publish`, `POST svc/unpublish`: the services this
//!   host publishes to the network (`xenonet svc`). Other hosts reach them
//!   at `/api/svc/…` (routes/svc.rs).

use crate::cores::CoreError;
use crate::ctx::{Answer, Ctx};
use crate::http::{Reply, Req};
use crate::parts::{NetChange, NetResult, Network};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::Arc;

/// The network's state as a development instance's `simulate` set it, or
/// None (net_sim.rs). The console's `simulate` fills this in, so its
/// network states show without real machines; `GET state` answers it first
/// when there's one.
pub fn simulated_net_state(ctx: &Ctx) -> Option<Value> {
    super::net_sim::simulated_net_state(ctx)
}

/// A refusal as the network words it: its status and code.
fn net_error(status: u16, code: &str, message: impl Into<String>) -> CoreError {
    CoreError { status: Some(status), code: Some(code.to_string()), ..CoreError::new(message) }
}

/// A request that isn't right.
fn bad(message: impl Into<String>) -> CoreError {
    net_error(400, "bad-request", message)
}

fn refused(e: &CoreError) -> Reply {
    Reply::json(e.status.unwrap_or(500), json!({ "error": e.message, "code": e.code }))
}

/// A LAN address as the person gives it: an IP address this machine has
/// and a port, never 0.0.0.0. The words of why it isn't one otherwise.
pub fn parse_lan(text: &str) -> Result<SocketAddr, String> {
    let addr: SocketAddr = text.trim().parse().map_err(|_| "a LAN address is an IP address and a port, such as 192.168.1.5:47850".to_string())?;
    if addr.ip().is_unspecified() || addr.port() == 0 {
        return Err("a LAN address is one this machine has, with a port: never 0.0.0.0".into());
    }
    Ok(addr)
}

fn lan_of(text: &str) -> Result<SocketAddr, CoreError> {
    parse_lan(text).map_err(bad)
}

/// The actor gateway's public host, which a network through its tunnel
/// is reached at.
/// The actor gateway's public host, when its tunnel carries the network's
/// routes too. Tailscale Funnel publishes `/mcp` alone, so a network on
/// that gateway takes a LAN or tailnet address instead.
fn public_host(ctx: &Ctx) -> Option<String> {
    let g = ctx.gateway.get()?;
    if g.tunnel_view()["kind"] == "tailscale" {
        return None;
    }
    g.public_host()
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
        ("GET", "state" | "svc")
            | ("POST", "create" | "join" | "leave" | "ticket" | "remove" | "replace" | "approve" | "gateway/take" | "gateway/set" | "svc/publish" | "svc/unpublish")
            | ("PUT", "settings")
    );
    if !known {
        return Reply::error(404, "not found");
    }
    // Who may is rules.rs's: yours, and a ticket the window's.
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
    let net = &*svc;
    // Only a request from the window may ask the person.
    let window = req.by.window;
    let result = match rest {
        "state" => Ok(net.state()),
        "create" => create(ctx, net, &body, window).await,
        "join" => net.join(&text("address"), &text("ticket")).await,
        "leave" => {
            let r = net.leave(on("force")).await;
            if r.is_ok() {
                net.unlisten();
            }
            r
        }
        "ticket" => {
            net.tunnel_address(public_host(ctx).as_deref());
            net.ticket(&text("host"), on("window"), window).await
        }
        "remove" => net.remove(&text("host")).await,
        "replace" => net.replace(&text("host"), window).await,
        "settings" => settings(net, &body, window).await,
        "approve" => net.approve(&text("id"), window).await,
        "gateway/take" => take(ctx, net, &body, window).await,
        "svc" => Ok(net.svc_list()),
        "svc/publish" => publish(ctx, net, &body),
        "svc/unpublish" => net.svc_unpublish(&text("name")),
        _ => net.set_gateway(&text("address")).await,
    };
    match result {
        Ok(v) => Reply::json(200, v),
        // The ticket's maker hasn't signed this machine in yet, or a second
        // window host hasn't approved it: the join is taken, and waits.
        // The window and the CLI say why and ask again.
        Err(e) if e.code.as_deref() == Some("waiting") => Reply::json(202, json!({ "waiting": true, "message": e.message })),
        Err(e) => refused(&e),
    }
}

async fn create(ctx: &Ctx, svc: &dyn Network, body: &Value, window: bool) -> NetResult {
    if let Some(name) = svc.network_name() {
        return Err(net_error(409, "in-network", format!("this machine is in the network {name} already: xenonet net leave first")));
    }
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let lan = match body.get("lan").and_then(Value::as_str).filter(|l| !l.trim().is_empty()) {
        Some(l) => Some(lan_of(l)?),
        None => None,
    };
    // The LAN listener first: an address this machine can't listen on is
    // refused before the network is made.
    if let Some(addr) = lan {
        svc.listen(addr).await?;
    }
    let lan_text = lan.map(|a| a.to_string());
    let made = svc.create(&text("name"), &text("host"), public_host(ctx).as_deref(), lan_text.as_deref(), window);
    if made.is_err() && lan.is_some() {
        svc.unlisten();
    }
    made
}

/// **Make this the network's gateway**: at the LAN address given, which it
/// listens on first, or at the actor gateway's tunnel.
async fn take(ctx: &Ctx, svc: &dyn Network, body: &Value, window: bool) -> NetResult {
    let lan = match body.get("lan").and_then(Value::as_str).filter(|l| !l.trim().is_empty()) {
        Some(l) => Some(lan_of(l)?),
        None => None,
    };
    if let Some(addr) = lan {
        svc.listen(addr).await?;
    }
    let lan_text = lan.map(|a| a.to_string());
    let took = svc.take_gateway(public_host(ctx).as_deref(), lan_text.as_deref(), window).await;
    if took.is_err() && lan.is_some() && !svc.is_gateway() {
        svc.unlisten();
    }
    took
}

/// `xenonet svc publish`: a service on this machine for the network's
/// other hosts. Never XenoNet's own API or its doors, which would hand
/// what's this machine's owner's to every allowed host.
fn publish(ctx: &Ctx, svc: &dyn Network, body: &Value) -> NetResult {
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let list = |k: &str| -> Result<Vec<String>, CoreError> {
        match body.get(k) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(a)) => a.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(format!("{k} is a list of words")))).collect(),
            Some(_) => Err(bad(format!("{k} is a list of words"))),
        }
    };
    let target = text("target");
    if let Some(port) = svc.svc_port(&target) {
        // Its API, the actor gateway's port as it runs (one it saved, or
        // the old default on an install set up before ports were saved)
        // and the default it would take, and the network's LAN door.
        let mut own = vec![ctx.config.port, ctx.config.gateway_port.unwrap_or_else(|| crate::config::gateway_port_of(ctx.config.port))];
        own.extend(ctx.gateway.get().map(|g| g.port()));
        own.extend(svc.lan().map(|a| a.port()));
        if own.contains(&port) {
            return Err(bad(format!("port {port} is XenoNet's own: publishing it would let other hosts act as this machine's owner")));
        }
    }
    svc.svc_publish(&text("name"), &target, &list("paths")?, &list("hosts")?)
}

async fn settings(svc: &dyn Network, body: &Value, window: bool) -> NetResult {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    // An entry that waits for a second admin, approved by its id.
    if let Some(id) = text("approve") {
        return svc.approve(&id, window).await;
    }
    // Another machine's: who may start and wake sessions there, an entry
    // this machine signs.
    if let Some(host) = body.get("host").and_then(Value::as_str).filter(|h| svc.host().is_some_and(|me| me != *h)) {
        if text("start").is_none() && text("wake").is_none() {
            return Err(bad("say what to set: start or wake, or approve a change that waits"));
        }
        return svc.set_reach(host, text("start").as_deref(), text("wake").as_deref(), window).await;
    }
    let titles = body.get("titlesOnly").and_then(Value::as_bool);
    let load = body.get("shareLoad").and_then(Value::as_bool);
    let lan = match body.get("lan") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s.trim().is_empty() => Some(None),
        Some(Value::String(s)) => Some(Some(lan_of(s)?)),
        Some(_) => return Err(bad("lan is an address, or null")),
    };
    if let Some(l) = lan {
        if !svc.is_gateway() {
            return Err(net_error(409, "not-gateway", "the LAN address is the gateway's: set it on its host"));
        }
        match l {
            Some(addr) => svc.listen(addr).await?,
            None => svc.unlisten(),
        }
    }
    let change = NetChange { titles_only: titles, share_load: load, lan: lan.map(|l| l.map(|a| a.to_string())), start: text("start"), wake: text("wake") };
    svc.settings(change, window).await
}
