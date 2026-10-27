//! xenonet net (docs/design/network.md, "The wire"): one network across
//! the person's machines, through the local service's /api/net routes.
//! Creating, joining and leaving are the person's: the service refuses
//! them from under an agent, as it does `xenonet service install`. A
//! ticket from a terminal is macOS's alone, where process checks can tell
//! the person from an agent; elsewhere it's made in XenoNet's window.

use crate::args::Flags;
use crate::js::{list, num, on, str, t};
use crate::out::{ago, c, json, now, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Map, Value};
use xenonet_base::client::{Refused, Server};

pub const HELP: &str = include_str!("help/net.txt");

fn refused(r: Refused) -> Fail {
    Fail::new(r.message, 1)
}

fn call(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    s.request(method, route, body, &[]).map_err(refused)
}

fn link_words(g: &Value) -> String {
    let since = (num(g, "since") > 0.0).then(|| ago(num(g, "since"), now()));
    let reason = str(g, "reason").map(|r| format!(": {r}")).unwrap_or_default();
    match str(g, "link") {
        Some("live") => c::green("live"),
        Some("connecting") => c::dim("connecting…"),
        Some("removed") => c::red(format!("removed{reason}")),
        Some(state @ ("offline" | "refused")) => c::yellow(format!("{state}{}{reason}", since.map(|s| format!(" since {s}")).unwrap_or_default())),
        _ => c::dim("off"),
    }
}

fn show_status(st: &Value) {
    let Some(net) = st.get("network").filter(|n| !n.is_null()) else {
        print("This machine isn't in a network. xenonet net create makes one; xenonet net join joins one.");
        return;
    };
    let me = st.get("self").cloned().unwrap_or(Value::Null);
    let roles: Vec<&str> = [("founder", "founder"), ("gateway", "gateway"), ("window", "window host")].iter().filter(|(k, _)| on(&me, k)).map(|(_, w)| *w).collect();
    print(format!(
        "{} {} this machine is {}{}",
        c::bold(t(net, "name")),
        c::dim("·"),
        c::bold(t(&me, "id")),
        if roles.is_empty() { String::new() } else { format!(" ({})", roles.join(", ")) }
    ));
    print(format!("  {}  {}", c::dim("key     "), t(&me, "keyId")));
    let g = st.get("gateway").cloned().unwrap_or(Value::Null);
    let address = str(&g, "address").filter(|a| !a.is_empty()).map_or_else(|| c::yellow("no address yet"), str::to_string);
    print(format!("  {}  {address} ({})", c::dim("gateway "), t(&g, "kind")));
    print(format!("  {}  {}", c::dim("link    "), link_words(&g)));
    if let Some(skew) = g.get("skewMs").and_then(Value::as_f64).filter(|s| s.abs() >= 2000.0) {
        print(format!("  {}  this clock is {} s {} the gateway's", c::dim("clock   "), (skew.abs() / 1000.0).round(), if skew > 0.0 { "behind" } else { "ahead of" }));
    }
    let o = st.get("outbox").cloned().unwrap_or(Value::Null);
    print(format!("  {}  {} waiting to go", c::dim("outbox  "), num(&o, "frames")));
    for q in list(st, "queues").iter().filter(|q| num(q, "frames") > 0.0) {
        print(format!("  {}  {} waiting for {}", c::dim("queue   "), num(q, "frames"), t(q, "host")));
    }
    print(format!("  {}  start: {}, wake: {}", c::dim("sessions"), t(st, "start"), t(st, "wake")));
    for p in list(st, "proposals") {
        print(c::yellow(format!("  ! {} waits for approval: xenonet net approve {}", proposal_words(p), t(p, "host"))));
    }
    // Widenings of a machine's start or wake that wait for a second window
    // host.
    for h in list(st, "hosts") {
        for w in h.get("reach").map(|r| list(r, "pending")).unwrap_or_default() {
            let what = format!("{} asked to let the network {} sessions on {}", t(w, "by"), t(w, "setting"), t(h, "id"));
            let how = if on(h, "self") { "another window host approves it".to_string() } else { format!("xenonet net set --on {} --approve {}", t(h, "id"), t(w, "id")) };
            print(c::yellow(format!("  ! {what}: {how}")));
        }
    }
    let mismatch: Vec<String> = list(st, "rosterMismatch").iter().filter_map(Value::as_str).map(str::to_string).collect();
    if !mismatch.is_empty() {
        print(c::yellow(format!("  ! {} shows another roster than this machine's", mismatch.join(", "))));
    }
    print(format!("  {}  {}", c::dim("hosts   "), list(st, "hosts").len()));
}

/// What an entry waiting for approval says, as the person approves it:
/// its op and host, the address for a move of the gateway, a window host's
/// add, and who signed it so far.
fn proposal_words(p: &Value) -> String {
    let by: Vec<&str> = list(p, "signers").iter().filter_map(Value::as_str).collect();
    let what = match (str(p, "op"), str(p, "gateway")) {
        (Some("gateway"), Some(a)) => format!("{} to be the gateway, at {a}", t(p, "host")),
        (Some("add"), _) if on(p, "window") => format!("add {}, as a window host", t(p, "host")),
        _ => format!("{} {}", t(p, "op"), t(p, "host")),
    };
    format!("{what} (signed by {})", by.join(", "))
}

fn show_hosts(st: &Value) {
    let hosts = list(st, "hosts");
    if hosts.is_empty() {
        print("This machine isn't in a network.");
        return;
    }
    let wide = hosts.iter().map(|h| t(h, "id").len()).max().unwrap_or(0);
    for h in hosts {
        let mut bits = Vec::new();
        for (k, w) in [("self", "this machine"), ("founder", "founder"), ("gateway", "gateway"), ("window", "window")] {
            if on(h, k) {
                bits.push(w.to_string());
            }
        }
        let state = if on(h, "online") {
            c::green("online")
        } else if num(h, "seen") > 0.0 {
            c::dim(format!("offline, seen {}", ago(num(h, "seen"), now())))
        } else {
            c::dim("offline")
        };
        let agents = num(h, "agents");
        let waits = num(h, "waitsOnYou");
        let extra = if on(h, "self") {
            String::new()
        } else {
            format!("  {agents} agent{}{}", if agents == 1.0 { "" } else { "s" }, if waits > 0.0 { c::yellow(format!(", {waits} waiting on you")) } else { String::new() })
        };
        print(format!("  {}  {state}{extra}  {}", pad(&t(h, "id"), wide), c::dim(bits.join(", "))));
    }
}

fn arg(args: &[String], i: usize, what: &str) -> Result<String, Fail> {
    args.get(i).filter(|a| !a.is_empty()).cloned().ok_or_else(|| Fail::new(format!("say {what}: xenonet net --help"), 2))
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let what = args.first().map_or("status", String::as_str);
    // Add a machine from a terminal is macOS's alone: refused here before
    // anything is asked.
    if what == "add-machine" && !cfg!(target_os = "macos") {
        return Err(Fail::new("Add a machine is XenoNet's window's on this system: choose Add a machine in the palette", 1));
    }
    let s = need(flags)?;
    let out = |v: &Value, show: &dyn Fn(&Value)| {
        if flags.on("json") {
            json(v);
        } else {
            show(v);
        }
        Ok(0)
    };
    match what {
        "status" => out(&call(&s, "GET", "/api/net/state", None)?, &show_status),
        "hosts" => {
            let st = call(&s, "GET", "/api/net/state", None)?;
            if flags.on("json") {
                json(st.get("hosts").unwrap_or(&Value::Null));
                return Ok(0);
            }
            show_hosts(&st);
            Ok(0)
        }
        "create" => {
            let name = flags.text("name").ok_or_else(|| Fail::new("say the network's name: xenonet net create --name <name> --host <id>", 2))?;
            let host = flags.text("host").ok_or_else(|| Fail::new("say this machine's id in the network: --host <id>", 2))?;
            let mut body = Map::new();
            body.insert("name".into(), json!(name));
            body.insert("host".into(), json!(host));
            if let Some(lan) = flags.text("lan") {
                body.insert("lan".into(), json!(lan));
            }
            let st = call(&s, "POST", "/api/net/create", Some(&Value::Object(body)))?;
            out(&st, &|st| {
                print(format!("{} made the network {}, with this machine as {}", c::green("✓"), c::bold(t(&st["network"], "name")), t(&st["self"], "id")));
                if str(&st["gateway"], "address").is_none_or(str::is_empty) {
                    print(c::yellow("  The gateway has no address yet: xenonet net set --lan <ip:port>, with this machine's tailnet or LAN address."));
                }
                print(c::dim("  Add a machine: xenonet net add-machine <id> on macOS, or Add a machine in XenoNet's window."));
            })
        }
        "join" => {
            let address = arg(args, 1, "the gateway's address and the ticket, as Add a machine showed them")?;
            let ticket = arg(args, 2, "the ticket, as Add a machine showed it")?;
            // While the ticket's maker hasn't signed this machine in, or a
            // second window host hasn't approved it, the join waits: it says
            // why once, and asks again until the ticket lapses.
            let started = std::time::Instant::now();
            let mut said = false;
            let st = loop {
                let st = call(&s, "POST", "/api/net/join", Some(&json!({ "address": address, "ticket": ticket })))?;
                if !on(&st, "waiting") {
                    break st;
                }
                if started.elapsed() > std::time::Duration::from_secs(11 * 60) {
                    return Err(Fail::new(t(&st, "message"), 1));
                }
                if !said && !flags.on("json") {
                    print(c::yellow(t(&st, "message")));
                    said = true;
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            };
            out(&st, &|st| {
                print(format!("{} joined {} as {}", c::green("✓"), c::bold(t(&st["network"], "name")), t(&st["self"], "id")));
                print(format!("  {}  {}", c::dim("gateway key"), t(&st["gateway"], "keyId")));
            })
        }
        "add-machine" => {
            let host = arg(args, 1, "the new machine's id: xenonet net add-machine <id>")?;
            let r = call(&s, "POST", "/api/net/ticket", Some(&json!({ "host": host, "window": flags.on("window") })))?;
            out(&r, &|r| {
                print(format!("Run this on {} within ten minutes. It works once.", c::bold(t(r, "host"))));
                print(format!("\n  {}\n", t(r, "line")));
            })
        }
        "remove" | "replace" => {
            let host = arg(args, 1, "which machine: its id in the network")?;
            let r = call(&s, "POST", &format!("/api/net/{what}"), Some(&json!({ "host": host })))?;
            out(&r, &|r| {
                if what == "replace" {
                    print(format!("{} {}'s old keys are out. Run this on it within ten minutes:", c::green("✓"), host));
                    print(format!("\n  {}\n", t(r, "line")));
                } else {
                    print(format!("{} {host} is out of the network", c::green("✓")));
                }
            })
        }
        "leave" => {
            let r = call(&s, "POST", "/api/net/leave", Some(&json!({ "force": flags.on("force") })))?;
            out(&r, &|r| {
                // The founder never asks to be removed: its key is the root, so the
                // network has no founder after it.
                let asked = if on(r, "founder") {
                    " (it founded the network, which has no founder now)"
                } else if on(r, "removed") {
                    ""
                } else {
                    " (the gateway didn't hear it ask: a window host can remove it)"
                };
                print(format!("{} left {}{asked}; this XenoNet carries on alone", c::green("✓"), t(r, "left")));
            })
        }
        "set" => {
            let mut body = Map::new();
            if flags.has("titlesOnly") {
                body.insert("titlesOnly".into(), json!(flags.is_true("titlesOnly")));
            }
            if flags.has("shareLoad") {
                body.insert("shareLoad".into(), json!(flags.is_true("shareLoad")));
            }
            if flags.is_false("lan") {
                body.insert("lan".into(), Value::Null);
            } else if let Some(lan) = flags.str("lan") {
                body.insert("lan".into(), json!(lan));
            }
            for k in ["start", "wake", "approve"] {
                if let Some(v) = flags.text(k) {
                    body.insert(k.into(), json!(v));
                }
            }
            if body.is_empty() {
                return Err(Fail::new(
                    "say what to set: --titles-only, --no-titles-only, --share-load, --no-share-load, --start <off|you|network>, --wake <consent|network>, --lan <ip:port> or --no-lan",
                    2,
                ));
            }
            // Another machine's start and wake, from this window host.
            let on = flags.text("on");
            if let Some(host) = &on {
                if body.keys().any(|k| !matches!(k.as_str(), "start" | "wake" | "approve")) {
                    return Err(Fail::new("--on sets another machine's --start and --wake, or approves a change that waits there (--approve <id>)", 2));
                }
                body.insert("host".into(), json!(host));
            }
            let st = call(&s, "PUT", "/api/net/settings", Some(&Value::Object(body)))?;
            if let Some(host) = on {
                if !flags.on("json") {
                    print(format!("{} sent to {host}, which checks it: a change to network waits for the founder or another window host", c::green("✓")));
                    return Ok(0);
                }
            }
            out(&st, &show_status)
        }
        "approve" => {
            // What's approved is an entry, by its id: the one the person
            // names, by its host (and op) or its id, shown before it's signed.
            let which = arg(args, 1, "which machine the waiting entry is about, or the entry's id")?;
            let st = call(&s, "GET", "/api/net/state", None)?;
            let op = flags.text("op");
            let found: Vec<Value> = list(&st, "proposals")
                .iter()
                .filter(|p| str(p, "id") == Some(which.as_str()) || (str(p, "host") == Some(which.as_str()) && op.as_deref().is_none_or(|o| str(p, "op") == Some(o))))
                .cloned()
                .collect();
            let p = match found.as_slice() {
                [one] => one.clone(),
                [] => return Err(Fail::new(format!("nothing waits for approval about {which}: xenonet net status lists what does"), 1)),
                many => {
                    let lines: Vec<String> = many.iter().map(|p| format!("  {}  {}", t(p, "id"), proposal_words(p))).collect();
                    return Err(Fail::new(format!("more than one entry waits about {which}; approve one by its id:\n{}", lines.join("\n")), 2));
                }
            };
            if !flags.on("json") {
                print(format!("Approving: {}", proposal_words(&p)));
            }
            out(&call(&s, "POST", "/api/net/approve", Some(&json!({ "id": t(&p, "id") })))?, &show_status)
        }
        "gateway" => match args.get(1).map(String::as_str) {
            Some("take") => {
                let mut body = Map::new();
                if let Some(lan) = flags.text("lan") {
                    body.insert("lan".into(), json!(lan));
                }
                let st = call(&s, "POST", "/api/net/gateway/take", Some(&Value::Object(body)))?;
                if on(&st, "proposed") && !flags.on("json") {
                    print(format!("{} asked: the founder or another window host approves it with xenonet net approve {}", c::green("✓"), t(&st["self"], "id")));
                    return Ok(0);
                }
                out(&st, &show_status)
            }
            Some("set") => {
                let address = arg(args, 2, "the gateway's new address")?;
                out(&call(&s, "POST", "/api/net/gateway/set", Some(&json!({ "address": address })))?, &show_status)
            }
            _ => Err(Fail::new("xenonet net gateway take, or xenonet net gateway set <address>", 2)),
        },
        w => Err(Fail::new(format!("xenonet net doesn't know {w}: create, join, status, hosts, add-machine, remove, replace, leave, set, approve or gateway"), 2)),
    }
}
