//! xenonet gateway (cli/commands/gateway.js): the actor gateway
//! (server/actor-gateway.js), from your own terminal: whether it's on, the
//! tunnel that puts it on the internet and whether its address reaches it,
//! each app's connection, picking a tunnel, checking and repairing it,
//! asking for an actor's gateway token, and taking one away. A gateway
//! token is made in XenoNet's window only (Actors, then Connect… on the
//! actor, or Allow on an ask), where it's shown once, masked. This command
//! only asks, and never holds one.

use crate::args::Flags;
use crate::js::{encode, have, list, num, on, str, t};
use crate::out::{ago, c, json, json_at, now, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Map, Value};
use xenonet_actors::handoff::handoff;
use xenonet_base::client::{name_of, Server};

pub const HELP: &str = include_str!("help/gateway.txt");

/// `new URL(url).host`: the host, lowercased, and its port unless it's the
/// scheme's own; "" for what isn't a URL.
fn host_of(url: &str) -> String {
    let Some((scheme, rest)) = url.trim().split_once("://") else { return String::new() };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) || !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return String::new();
    }
    let authority = rest.split(['/', '?', '#', '\\']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h).to_ascii_lowercase();
    if host.is_empty() {
        return String::new();
    }
    let default = match scheme.to_ascii_lowercase().as_str() {
        "http" | "ws" => Some("80"),
        "https" | "wss" => Some("443"),
        "ftp" => Some("21"),
        _ => None,
    };
    match host.rsplit_once(':') {
        Some((h, port)) if !h.ends_with(']') || h.starts_with('[') => {
            if port.is_empty() || Some(port) == default {
                h.to_string()
            } else {
                host.clone()
            }
        }
        _ => host,
    }
}

/// What to give the app at `product` when the gateway's address changed and
/// its token still works (core/actors/handoff.js): { app, steps: [{ text,
/// copy }], message }.
fn new_address(product: Option<&str>, url: &str) -> Value {
    let h = handoff(product, url, &host_of(url), None);
    json!({
        "app": h.app,
        "steps": h.steps.iter().map(|(text, copy)| json!({ "text": text, "copy": copy })).collect::<Vec<_>>(),
        "message": h.message,
    })
}

fn reason_words(reason: &str) -> &'static str {
    match reason {
        "rotated" => "replaced",
        "removed" => "taken away",
        "expired" => "expired",
        _ => "refused",
    }
}

fn connection(a: &Value) -> String {
    let mut bits = Vec::new();
    if let Some(refused) = crate::js::some(a, "refused") {
        bits.push(c::yellow(format!(
            "! calls with a gateway token that was {}: Connect… in XenoNet's window gives it a new one",
            reason_words(str(refused, "reason").unwrap_or(""))
        )));
    }
    if on(a, "oldAddress") {
        let id = t(a, "id");
        bits.push(c::yellow(format!("! last called {}, an older address: xenonet gateway address {}", t(a, "lastHost"), id.strip_prefix("actor:").unwrap_or(&id))));
    }
    if on(a, "token") && on(a, "expiring") {
        let at = xenonet_base::time::iso(num(a, "expiresAt"));
        bits.push(c::yellow(format!("! its token stops working on {} unless it calls", &at[..at.len().min(10)])));
    }
    if bits.is_empty() {
        bits.push(if on(a, "lastAt") { c::dim(format!("last call {}", ago(num(a, "lastAt"), now()))) } else { c::dim("hasn't called yet") });
    }
    bits.join("  ")
}

fn reach_of(st: &Value) -> String {
    let r = st.get("reach").filter(|r| !r.is_null()).cloned().unwrap_or(json!({}));
    if !on(st, "on") {
        return String::new();
    }
    match str(&r, "state") {
        Some("ok") => c::green(format!(" ✓ reachable{}, checked {}", if on(&r, "local") { " (checked on this computer alone)" } else { "" }, ago(num(&r, "at"), now()))),
        Some("checking") => c::dim(" checking…"),
        Some("failing") => c::yellow(" ✗ not reachable"),
        _ => c::dim(" not checked yet"),
    }
}

fn show(st: &Value, s: &Server) {
    let state = if on(st, "on") {
        if on(st, "listening") {
            c::green("on")
        } else {
            c::yellow("on, but not listening")
        }
    } else {
        c::dim("off")
    };
    print(format!("{} {} the actor gateway is {state}", c::bold(name_of(s)), c::dim("·")));
    let tunnel = st.get("tunnel").filter(|x| !x.is_null()).cloned().unwrap_or(json!({}));
    let label = if on(&tunnel, "label") {
        let state = if on(&tunnel, "state") && str(&tunnel, "state") != Some("yours") { c::dim(format!(" ({})", t(&tunnel, "state"))) } else { String::new() };
        format!("{}{state}", t(&tunnel, "label"))
    } else {
        c::yellow("none yet: xenonet gateway tunnels")
    };
    print(format!("  {}           {label}", c::dim("tunnel")));
    print(format!("  {}          {}{}", c::dim("address"), have(st, "url").map_or_else(|| c::dim("none yet"), |u| crate::js::text(Some(u))), reach_of(st)));
    print(format!("  {}  {}", c::dim("on this machine"), t(st, "local")));
    if on(st, "problem") {
        print(c::yellow(format!("  ! {}", t(st, "problem"))));
    }
    if tunnel.get("stable") == Some(&Value::Bool(false)) {
        print(c::dim(format!("  {}'s address changes every time it starts.", t(&tunnel, "label"))));
    }
    let actors = list(st, "actors");
    if !actors.is_empty() {
        print(format!("  {}", c::dim("apps")));
        let wide = actors.iter().map(|a| crate::js::length(a.get("name"))).max().unwrap_or(0);
        for a in actors {
            print(format!("    {}  {}", pad(&t(a, "name"), wide), connection(a)));
        }
    }
    if on(st, "calls") {
        print(c::dim(format!(
            "  {} calls since XenoNet started{}",
            t(st, "calls"),
            if on(st, "lastActor") { format!(", last from {}", t(st, "lastActor")) } else { String::new() }
        )));
    }
}

fn show_kinds(st: &Value) {
    print("Tunnels for the actor gateway. XenoNet runs and checks the one you pick; its sign-in stays in the tool's own settings.");
    let in_use = st.get("tunnel").and_then(|x| x.get("kind"));
    for k in list(st, "kinds") {
        let here = if on(k, "tool") {
            if on(k, "found") {
                c::green(format!(
                    "found{}{}",
                    if on(k, "path") { format!(" at {}", t(k, "path")) } else { String::new() },
                    if on(k, "version") { format!(" ({})", t(k, "version")) } else { String::new() }
                ))
            } else {
                c::dim(format!("{} isn't on this computer", t(k, "tool")))
            }
        } else {
            c::dim("nothing to install")
        };
        let used = if in_use == k.get("kind") { c::green("  ← in use") } else { String::new() };
        print(format!("\n  {}  {}  {here}{used}", c::bold(t(k, "kind")), t(k, "label")));
        print(format!("    {}", t(k, "needs")));
        if on(k, "note") {
            print(c::dim(format!("    {}", t(k, "note"))));
        }
        if on(k, "docs") {
            print(c::dim(format!("    {}", t(k, "docs"))));
        }
    }
}

#[expect(clippy::unnecessary_wraps, reason = "it ends a command, whose answer is a Done like every other command's")]
fn out(st: &Value, s: &Server, flags: &Flags) -> Done {
    if flags.on("json") {
        json(st);
    } else {
        show(st, s);
    }
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let what = args.first().map(String::as_str).filter(|w| !w.is_empty());
    let slug = args.get(1).filter(|x| !x.is_empty()).map(|x| x.strip_prefix("actor:").unwrap_or(x).to_string());
    match what {
        None | Some("tunnels") => {
            let st = s.get("/api/actors/gateway")?;
            if flags.on("json") {
                if what.is_some() {
                    json_at(st.get("kinds"));
                } else {
                    json(&st);
                }
            } else if what.is_some() {
                show_kinds(&st);
            } else {
                show(&st, &s);
            }
            Ok(0)
        }
        Some(w @ ("on" | "off")) => {
            let mut body = Map::new();
            body.insert("on".into(), json!(w == "on"));
            if w == "on" && (flags.on("tunnel") || flags.on("host") || flags.on("name")) {
                let now = s.get("/api/actors/gateway")?;
                let current = now.get("tunnel").and_then(|x| x.get("kind"));
                let kind = match flags.str("tunnel") {
                    Some(k) => json!(k),
                    None => {
                        let asks_host =
                            list(&now, "kinds").iter().find(|k| k.get("kind") == current && current.is_some()).and_then(|k| k.get("asks")).is_some_and(|a| on(a, "host"));
                        if asks_host {
                            current.cloned().unwrap_or(Value::Null)
                        } else {
                            json!("own")
                        }
                    }
                };
                let mut tunnel = Map::new();
                tunnel.insert("kind".into(), kind);
                for k in ["host", "name"] {
                    if let Some(v) = flags.str(k) {
                        tunnel.insert(k.into(), json!(v));
                    }
                }
                body.insert("tunnel".into(), Value::Object(tunnel));
            }
            let st = s.request("POST", "/api/actors/gateway", Some(&Value::Object(body)), &[])?;
            out(&st, &s, flags)
        }
        Some(w @ ("check" | "repair")) => {
            let mut body = Map::new();
            body.insert(w.into(), json!(true));
            let st = s.request("POST", "/api/actors/gateway", Some(&Value::Object(body)), &[])?;
            out(&st, &s, flags)
        }
        Some("address") => {
            let Some(slug) = slug else { return Err(Fail::new("say whose: xenonet gateway address <slug>", 2)) };
            let st = s.get("/api/actors/gateway")?;
            let id = format!("actor:{slug}");
            let Some(a) = list(&st, "actors").iter().find(|x| str(x, "id") == Some(&id)) else {
                return Err(Fail::new(format!("{id} has no gateway token; connect it in XenoNet's window (Actors, then Connect… on it)"), 1));
            };
            let Some(url) = crate::js::some(&st, "url").map(|u| crate::js::text(Some(u))) else {
                return Err(Fail::new("the gateway has no address yet: xenonet gateway on --tunnel <kind>", 1));
            };
            let h = new_address(str(a, "product"), &url);
            if flags.on("json") {
                let mut m = Map::new();
                m.insert("actor".into(), json!(id));
                m.insert("url".into(), st["url"].clone());
                m.extend(h.as_object().cloned().unwrap_or_default());
                json(&Value::Object(m));
                return Ok(0);
            }
            print(format!("{}: the gateway's address is {url}, and its token still works", c::bold(t(a, "name"))));
            for (i, step) in list(&h, "steps").iter().enumerate() {
                print(format!("  {}. {}", i + 1, t(step, "text")));
            }
            if on(&h, "message") {
                print(format!("\n{}", t(&h, "message")));
            }
            Ok(0)
        }
        Some("token") => {
            let Some(slug) = slug else { return Err(Fail::new("say whose: xenonet gateway token <slug>", 2)) };
            let r = s.request("POST", &format!("/api/actors/{}/gateway/ask", encode(&slug)), Some(&json!({})), &[])?;
            if flags.on("json") {
                json(&r);
                return Ok(0);
            }
            print(format!(
                "{} asked: {}'s gateway token waits for you to click Allow in XenoNet's window{}",
                c::green("✓"),
                t(&r, "id"),
                if on(&r, "replaces") { ", and replaces the one its app has" } else { "" }
            ));
            print(c::dim("  The token is made and shown there, masked; this command never sees it."));
            Ok(0)
        }
        Some("revoke") => {
            let Some(slug) = slug else { return Err(Fail::new("say whose: xenonet gateway revoke <slug>", 2)) };
            let r = s.request("DELETE", &format!("/api/actors/{}/gateway", encode(&slug)), None, &[])?;
            if flags.on("json") {
                json(&r);
            } else {
                print(format!("{} {}'s gateway token stopped working", c::green("✓"), t(&r, "actor")));
            }
            Ok(0)
        }
        Some(w) => Err(Fail::new(format!("xenonet gateway doesn't know {w}: tunnels, on, off, check, repair, address, token or revoke"), 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_as_url_reads_them() {
        assert_eq!(host_of("https://xenonet-demo.ngrok-free.app/mcp"), "xenonet-demo.ngrok-free.app");
        assert_eq!(host_of("https://User@Example.COM:443/x"), "example.com");
        assert_eq!(host_of("http://127.0.0.1:47850/mcp"), "127.0.0.1:47850");
        assert_eq!(host_of("xenonet.example.com"), "");
    }

    #[test]
    fn a_new_address_alone() {
        let h = new_address(Some("muse"), "https://xenonet-demo.ngrok-free.app/mcp");
        assert_eq!(h["app"], "Muse");
        assert!(h["message"].as_str().unwrap().contains("Change your \"XenoNet\" connector's host to xenonet-demo.ngrok-free.app and keep its key"));
        assert_eq!(new_address(None, "https://x.example/mcp")["steps"][0]["copy"], "url");
        assert_eq!(new_address(None, "https://x.example/mcp")["message"], Value::Null);
        let a2a = new_address(Some("a2a"), "https://x.example/.well-known/agent-card.json");
        assert_eq!(a2a["app"], "A2A agent");
        assert_eq!(a2a["steps"][0]["copy"], "url");
        assert_eq!(a2a["message"], Value::Null);
    }
}
