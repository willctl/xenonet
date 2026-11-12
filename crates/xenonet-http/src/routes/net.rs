//! The network on this host's own API, `/api/net/…` (docs/design/
//! network.md, "The wire" and section 8), for its window and the `xenonet
//! net` command. Every route is the owner's: from XenoNet's window or the
//! person's own terminal, never from under an agent or with an actor's
//! token. What asks the person (their presence, Touch ID on a Mac) is done
//! only for a request that came with the window's key: making the first
//! network (it sets up their identity), **Add a machine** (`POST invite`,
//! whose join then asks to sign the machine in; it answers the invite and
//! its code once, and nothing keeps the code), **Replace**, and approving
//! or making an entry that admits or widens. Anything else is refused
//! there, never prompted for.
//!
//! A machine may be in several networks. What acts on one takes
//! `network` (its id or name) in the body, or `?network=` on a GET; with
//! none, the one network this machine is in, and a 409 `which-network`
//! when it's in several.
//!
//! - `GET state`: `network/state/v1`, every network in `networks`.
//! - `POST create`, `POST join`, `POST leave`: on this machine only. Each
//!   create and join adds a network beside the ones there are; `create`
//!   takes `relay`, the person's own iroh-relay's URL, or n0. A
//!   join takes `invite` and `code` and goes over join/2 to the machine
//!   that made the invite (`at`, its address, or `lan`, mDNS, when the
//!   invite names no relay).
//! - `POST invite`: an invite and its code, for join/2: the window's Add a
//!   machine.
//! - `POST remove`, `POST replace`.
//! - `POST approve`: signs an entry that waits for the founder or a second
//!   window host (a window host's add), by the id the state lists it
//!   under.
//! - `PUT settings`: titles only, whether this machine shares its load,
//!   and who may start and wake sessions here (`start`, `wake`). With
//!   `host`, another machine's `start` and `wake`. Start and wake are settings entries in the network's
//!   chain, signed here (with presence where they widen); `approve` signs
//!   one that waits, by its id.
//! - `GET forwards`: the actors whose grants forward an ALPN over this
//!   machine's endpoint (`xenonet_net::forward`), and their connections.
//! - `GET svc`, `POST svc/publish`, `POST svc/unpublish`: the services this
//!   host publishes to the network. Other hosts reach them at
//!   `/api/svc/…` (routes/svc.rs). Publishing widens what the network
//!   reaches, so it's the window's, with the person's presence; taking a
//!   service off is yours from your terminal too (`xenonet svc`).
//! - `PUT sharing`: what the person shares into a network, `sharing`
//!   "all" or a list of project ids. Sharing more is the window's.
//! - `POST relays`: a relay (`url`, or n0) put into a network's chain
//!   (`add`, which widens: the window's, with presence) or taken out; with
//!   `mine`, into or out of the relays this machine adds of its own, for
//!   its own dials.
//! - `POST mailboxes`: a member (`host`) named one of a network's
//!   mailboxes (`add`, which widens: the window's, with presence) or taken
//!   off them, by a window host or by the mailbox itself.
//! - `POST devices/revoke`: revokes another of the person's devices (a
//!   lost machine), with their presence: the window's.
//! - `POST feeds/unfreeze`: takes a machine's feed again after a fork froze
//!   it here (`frozen` in the state), by its author's key: the window's,
//!   since only the person can say a machine that signed two things at one
//!   place is to be trusted again.
//!
//! The person's devices are in `GET state`'s `person.devices`.

use crate::cores::CoreError;
use crate::ctx::{Answer, Ctx};
use crate::http::{Reply, Req};
use crate::parts::{NetChange, NetResult, Network};
use serde_json::{json, Value};
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
        ("GET", "state" | "svc" | "forwards")
            | (
                "POST",
                "forwards/approve"
                    | "forwards/sign"
                    | "forwards/deny"
                    | "forwards/revoke"
                    | "create"
                    | "join"
                    | "leave"
                    | "invite"
                    | "remove"
                    | "replace"
                    | "approve"
                    | "svc/publish"
                    | "svc/unpublish"
                    | "devices/revoke"
                    | "feeds/unfreeze"
                    | "relays"
                    | "mailboxes"
            )
            | ("PUT", "settings" | "sharing")
    );
    if !known {
        return Reply::error(404, "not found");
    }
    // Who may is rules.rs's: yours, and an invite the window's.
    if rest == "forwards" {
        return match ctx.cores.forwards.list() {
            Ok(v) => Reply::json(200, v),
            Err(e) => refused(&e),
        };
    }
    if let Some(what) = rest.strip_prefix("forwards/") {
        return match forwards(ctx, req, what).await {
            Ok(v) => Reply::json(200, v),
            Err(e) => refused(&e),
        };
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
    let net = &*svc;
    // The network it acts on, when it names one.
    let named = if method == "GET" { req.param("network").map(str::to_string) } else { body.get("network").and_then(Value::as_str).map(str::to_string) };
    let network = named.as_deref().map(str::trim).filter(|n| !n.is_empty());
    // Only a request from the window may ask the person.
    let window = req.by.window;
    let result = match rest {
        "state" => Ok(net.state()),
        "create" => {
            let relay = text("relay");
            net.create(&text("name"), &text("host"), Some(relay.as_str()).filter(|r| !r.trim().is_empty()), window).await
        }
        "join" => {
            let at = text("at");
            net.join_invite(&text("invite"), &text("code"), Some(at.as_str()).filter(|a| !a.is_empty()), on("lan")).await
        }
        "leave" => net.leave(network, on("force")).await,
        "invite" => net.invite(network, &text("host"), on("window"), on("lan"), window).await,
        "remove" => net.remove(network, &text("host")).await,
        "replace" => net.replace(network, &text("host"), window).await,
        "settings" => settings(net, network, &body, window).await,
        "sharing" => {
            let r = net.share(network, body.get("sharing").unwrap_or(&Value::Null), window);
            // Xenolith holds a project's space back from a network that
            // doesn't see the project any more.
            if r.is_ok() {
                ctx.cores.forwards.changed();
            }
            r
        }
        "relays" => net.relay(network, &text("url"), body.get("add") != Some(&Value::Bool(false)), on("mine"), window).await,
        "mailboxes" => net.mailbox(network, &text("host"), body.get("add") != Some(&Value::Bool(false)), window).await,
        "approve" => net.approve(network, &text("id"), window).await,
        "svc" => net.svc_list(network),
        "svc/publish" => publish(ctx, net, network, &body, window),
        "devices/revoke" => net.revoke_device(&text("device"), window),
        "feeds/unfreeze" => net.unfreeze(network, &text("author"), window),
        _ => net.svc_unpublish(network, &text("name")),
    };
    match result {
        Ok(v) => Reply::json(200, v),
        // The invite's maker hasn't signed this machine in yet, or a second
        // window host hasn't approved it: the join is taken, and waits.
        // The window and the CLI say why and ask again.
        Err(e) if e.code.as_deref() == Some("waiting") => Reply::json(202, json!({ "waiting": true, "message": e.message })),
        Err(e) => refused(&e),
    }
}

/// An actor's forward grant (Xenolith's, for its shared mode), and the
/// spaces it curates: `sign` one of their entries as the person (`space`,
/// `op`, `network`, `level`), the window's, since it asks them; `approve`
/// the ask for one (`alpn`, `key`, and the `networks` it reaches, each one
/// this machine is in), which asks the person, so it's the window's;
/// `deny` it; `revoke` the grant on `alpn`, which ends its forward. The
/// grant itself never comes back here: the actor collects it over the
/// forward socket.
async fn forwards(ctx: &Ctx, req: &mut Req, what: &str) -> Result<Value, CoreError> {
    let body = req.json(16 << 10).await.map_err(|e| net_error(e.status, "bad-request", e.message))?;
    let text = |k: &str| body.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let f = &ctx.cores.forwards;
    match what {
        "approve" => {
            let Some(svc) = ctx.net.get().cloned() else { return Err(net_error(503, "no-network", "the network isn't available in this XenoNet")) };
            let mine: Vec<String> = svc.state()["networks"].as_array().into_iter().flatten().filter_map(|n| n["id"].as_str().map(str::to_string)).collect();
            let networks: Vec<String> = body.get("networks").and_then(Value::as_array).into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)).collect();
            if networks.is_empty() {
                return Err(bad("name the networks it may reach (networks: [net_…])"));
            }
            if let Some(n) = networks.iter().find(|n| !mine.contains(n)) {
                return Err(bad(format!("this machine isn't in {n}")));
            }
            f.approve(&text("alpn"), &text("key"), &networks, req.by.window)
        }
        "sign" => f.sign(&body, req.by.window).await,
        "deny" => f.deny(&text("alpn"), &text("key")),
        _ => f.revoke(&text("alpn")),
    }
}

/// **Publish a service**: a service on this machine for the network's
/// other hosts, from the window, with the person's presence. Never
/// XenoNet's own API or its doors, which would hand what's this machine's
/// owner's to every allowed host.
fn publish(ctx: &Ctx, svc: &dyn Network, network: Option<&str>, body: &Value, window: bool) -> NetResult {
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
        // Its API, and the actor gateway's port as it runs (one it saved,
        // or the old default on an install set up before ports were saved)
        // and the default it would take.
        let mut own = vec![ctx.config.port, ctx.config.gateway_port.unwrap_or_else(|| crate::config::gateway_port_of(ctx.config.port))];
        own.extend(ctx.gateway.get().map(|g| g.port()));
        if own.contains(&port) {
            return Err(bad(format!("port {port} is XenoNet's own: publishing it would let other hosts act as this machine's owner")));
        }
    }
    svc.svc_publish(network, &text("name"), &target, &list("paths")?, &list("hosts")?, window)
}

async fn settings(svc: &dyn Network, network: Option<&str>, body: &Value, window: bool) -> NetResult {
    let text = |k: &str| body.get(k).and_then(Value::as_str).map(str::to_string);
    // An entry that waits for a second admin, approved by its id.
    if let Some(id) = text("approve") {
        return svc.approve(network, &id, window).await;
    }
    // Another machine's: who may start and wake sessions there, an entry
    // this machine signs.
    if let Some(host) = body.get("host").and_then(Value::as_str).filter(|h| svc.host(network).is_some_and(|me| me != *h)) {
        if text("start").is_none() && text("wake").is_none() {
            return Err(bad("say what to set: start or wake, or approve a change that waits"));
        }
        return svc.set_reach(network, host, text("start").as_deref(), text("wake").as_deref(), window).await;
    }
    let titles = body.get("titlesOnly").and_then(Value::as_bool);
    let load = body.get("shareLoad").and_then(Value::as_bool);
    let change = NetChange { titles_only: titles, share_load: load, start: text("start"), wake: text("wake") };
    svc.settings(network, change, window).await
}
