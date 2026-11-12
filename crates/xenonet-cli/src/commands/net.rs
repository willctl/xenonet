//! xenonet net (docs/design/network.md, "The wire"): the networks across
//! the person's machines, through the local service's /api/net routes.
//! What acts on one network takes `--network <id|name>`, which a machine
//! in one network needs no more than its list does.
//! Creating, joining and leaving are the person's: the service refuses
//! them from under an agent, as it does `xenonet service install`. What
//! asks the person (an invite, a widening, setting up their identity with
//! the first network) is made in XenoNet's window, never here.

use crate::args::Flags;
use crate::js::{list, num, on, str, t};
use crate::out::{ago, c, json, now, pad, print, Done, Fail};
use crate::reach::need;
use serde_json::{json, Map, Value};
use xenonet_api::client::{Refused, Server};

pub const HELP: &str = include_str!("help/net.txt");

fn refused(r: Refused) -> Fail {
    Fail::new(r.message, 1)
}

fn call(s: &Server, method: &str, route: &str, body: Option<&Value>) -> Result<Value, Fail> {
    s.request(method, route, body, &[]).map_err(refused)
}

// How this machine stands in the network: live while its node runs,
// removed once a member took it out.
fn link_words(l: &Value) -> String {
    let since = if num(l, "since") > 0.0 { format!(" since {}", ago(num(l, "since"), now())) } else { String::new() };
    let reason = str(l, "reason").map(|r| format!(": {r}")).unwrap_or_default();
    match str(l, "state") {
        Some("live") => c::green(format!("live{since}")),
        Some("removed") => c::red(format!("removed{reason}")),
        _ => c::dim("off"),
    }
}

fn show_status(st: &Value) {
    let Some(net) = st.get("network").filter(|n| !n.is_null()) else {
        print("This machine isn't in a network. xenonet net create makes one; xenonet net join joins one.");
        return;
    };
    let me = st.get("self").cloned().unwrap_or(Value::Null);
    let roles: Vec<&str> = [("founder", "founder"), ("window", "window host")].iter().filter(|(k, _)| on(&me, k)).map(|(_, w)| *w).collect();
    print(format!(
        "{} {} this machine is {}{}",
        c::bold(t(net, "name")),
        c::dim("·"),
        c::bold(t(&me, "id")),
        if roles.is_empty() { String::new() } else { format!(" ({})", roles.join(", ")) }
    ));
    print(format!("  {}  {}", c::dim("key     "), t(&me, "keyId")));
    print(format!("  {}  {}", c::dim("link    "), link_words(st.get("link").unwrap_or(&Value::Null))));
    // What waits for a machine that's away, in its mailboxes.
    for h in list(st, "hosts").iter().filter(|h| num(h, "held") > 0.0) {
        print(format!("  {}  {} held for {}", c::dim("held    "), num(h, "held"), t(h, "id")));
    }
    print(format!("  {}  start: {}, wake: {}", c::dim("sessions"), t(st, "start"), t(st, "wake")));
    if let Some(d) = st.get("delegated").filter(|d| d.is_object()) {
        print(format!(
            "  {}  delegated by {} on {}: what its sessions and actors start here runs under your delegation",
            c::dim("        "),
            t(d, "by"),
            xenonet_text::time::iso(num(d, "at")).chars().take(10).collect::<String>()
        ));
    }
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
/// its op and host, a window host's add, and who signed it so far.
fn proposal_words(p: &Value) -> String {
    let by: Vec<&str> = list(p, "signers").iter().filter_map(Value::as_str).collect();
    let what = match str(p, "op") {
        Some("add") if on(p, "window") => format!("add {}, as a window host", t(p, "host")),
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
        for (k, w) in [("self", "this machine"), ("founder", "founder"), ("window", "window")] {
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

/// One network's part of the state, as `show_status` and `show_hosts`
/// read it: its entry in `networks`, with `network` beside it.
fn view_of(entry: &Value) -> Value {
    let mut v = entry.clone();
    v["network"] = json!({ "id": entry["id"], "name": entry["name"] });
    v
}

/// The networks the state lists, or the one `which` names (its id or
/// name).
fn views(st: &Value, which: Option<&str>) -> Result<Vec<Value>, Fail> {
    let all = list(st, "networks");
    match which {
        None => Ok(all.iter().map(view_of).collect()),
        Some(w) => {
            let found: Vec<&Value> = all.iter().filter(|n| str(n, "id") == Some(w) || str(n, "name") == Some(w)).collect();
            match found.as_slice() {
                [one] => Ok(vec![view_of(one)]),
                [] => Err(Fail::new(format!("this machine isn't in a network named {w}: xenonet net status lists the ones it's in"), 1)),
                _ => Err(Fail::new(format!("two networks here are named {w}: say which by its id"), 2)),
            }
        }
    }
}

/// The relays as they stand: each network's, and this machine's own and
/// home relay.
fn show_relays(st: &Value) {
    for n in list(st, "networks") {
        let relays: Vec<&str> = list(n, "relays").iter().filter_map(Value::as_str).collect();
        print(format!("{}  {}", c::bold(t(n, "name")), if relays.is_empty() { c::dim("no relay: the LAN and direct paths only") } else { relays.join(", ") }));
    }
    let here = st.get("relaysHere").cloned().unwrap_or(Value::Null);
    let own: Vec<&str> = list(&here, "own").iter().filter_map(Value::as_str).collect();
    if !own.is_empty() {
        print(format!("  {}  {}", c::dim("this machine's own"), own.join(", ")));
    }
    match str(&here, "url") {
        Some(u) => print(format!("  {}  {u}", c::dim("home relay        "))),
        None => print(format!("  {}  {}", c::dim("home relay        "), c::dim("none"))),
    }
    let direct: Vec<&str> = list(&here, "directOnly").iter().filter_map(Value::as_str).collect();
    if !direct.is_empty() && list(st, "networks").len() > 1 {
        print(c::yellow(format!(
            "  ! no relay every network lists: {} reach this machine by direct paths only, unless you add one of its own (xenonet net relay add <url> --mine)",
            direct.join(", ")
        )));
    }
}

/// An invite's answer (`invite`, `replace`): the invite, its code apart,
/// and how the new machine finds this one when the invite names no relay.
fn show_invite(r: &Value) {
    print(format!("\n  {}\n", t(r, "invite")));
    print(format!("and tell it the code some other way than the invite went: {}", c::bold(t(r, "code"))));
    let how = if str(r, "relay").is_some() {
        String::new()
    } else if on(r, "lan") {
        " --mdns".to_string()
    } else {
        let at = list(r, "at").iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>();
        format!(" --at <{}>", if at.is_empty() { "this machine's address".to_string() } else { at.join(" or ") })
    };
    print(format!("\nOn {}: xenonet net join <invite> <code>{how}", t(r, "host")));
}

fn arg(args: &[String], i: usize, what: &str) -> Result<String, Fail> {
    args.get(i).filter(|a| !a.is_empty()).cloned().ok_or_else(|| Fail::new(format!("say {what}: xenonet net --help"), 2))
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let what = args.first().map_or("status", String::as_str);
    // Add a machine is the window's, since signing the machine in asks
    // the person: refused here before anything is asked.
    if what == "add-machine" {
        return Err(Fail::new("Add a machine is XenoNet's window's, since it asks you: choose Add a machine in the palette", 1));
    }
    // So is revoking a device, which takes the person's presence.
    if what == "revoke-device" {
        return Err(Fail::new("Revoking a device is XenoNet's window's, since it asks you: Network settings › Your devices, then Revoke on the lost machine's device", 1));
    }
    let s = need(flags)?;
    let network = flags.text("network");
    // A body for the network `--network` names, when it names one.
    let within = |mut body: Value| {
        if let Some(n) = &network {
            body["network"] = json!(n);
        }
        body
    };
    let out = |v: &Value, show: &dyn Fn(&Value)| {
        if flags.on("json") {
            json(v);
        } else {
            show(v);
        }
        Ok(0)
    };
    match what {
        "status" => {
            let st = call(&s, "GET", "/api/net/state", None)?;
            if network.is_none() {
                return out(&st, &|st| {
                    let all = views(st, None).unwrap_or_default();
                    if all.is_empty() {
                        show_status(st);
                    }
                    for (i, v) in all.iter().enumerate() {
                        if i > 0 {
                            print("");
                        }
                        show_status(v);
                    }
                });
            }
            let v = views(&st, network.as_deref())?.remove(0);
            out(&v, &show_status)
        }
        "hosts" => {
            let st = call(&s, "GET", "/api/net/state", None)?;
            let all = views(&st, network.as_deref())?;
            if flags.on("json") {
                match all.as_slice() {
                    [one] => json(&one["hosts"]),
                    _ => json(&Value::Array(all.iter().map(|v| json!({ "network": v["network"], "host": v["host"], "hosts": v["hosts"] })).collect())),
                }
                return Ok(0);
            }
            if all.is_empty() {
                show_hosts(&st);
            }
            for v in &all {
                if all.len() > 1 {
                    print(format!("{} {} as {}", c::bold(t(&v["network"], "name")), c::dim("·"), t(v, "host")));
                }
                show_hosts(v);
            }
            Ok(0)
        }
        "share" => {
            let given = arg(args, 1, "what the network sees: all, none, or project ids (prj_…), comma-separated")?;
            let sharing = match given.as_str() {
                "all" => json!("all"),
                "none" => json!([]),
                ids => json!(ids.split(',').map(str::trim).filter(|p| !p.is_empty()).collect::<Vec<_>>()),
            };
            let st = call(&s, "PUT", "/api/net/sharing", Some(&within(json!({ "sharing": sharing }))))?;
            out(&st, &|st| {
                for v in views(st, network.as_deref()).unwrap_or_default() {
                    let words = match &v["sharing"] {
                        Value::String(a) if a == "all" => "every project's sessions".to_string(),
                        Value::Array(p) if p.is_empty() => "no sessions".to_string(),
                        Value::Array(p) => p.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                        _ => "?".to_string(),
                    };
                    print(format!("{} {} sees {words}", c::green("✓"), c::bold(t(&v["network"], "name"))));
                }
            })
        }
        "relay" => {
            let verb = args.get(1).map(String::as_str);
            if verb.is_none() {
                return out(&call(&s, "GET", "/api/net/state", None)?, &show_relays);
            }
            let add = match verb {
                Some("add") => true,
                Some("remove") => false,
                _ => return Err(Fail::new("xenonet net relay add <url|n0>, or xenonet net relay remove <url|n0>", 2)),
            };
            let url = arg(args, 2, "the relay: your own iroh-relay's URL, or n0 for n0's public relays")?;
            let st = call(&s, "POST", "/api/net/relays", Some(&within(json!({ "url": url, "add": add, "mine": flags.on("mine") }))))?;
            if on(&st, "proposed") && !flags.on("json") {
                print(format!("{} asked: the founder or another window host approves it", c::green("✓")));
                return Ok(0);
            }
            out(&st, &show_relays)
        }
        "create" => {
            let name = flags.text("name").ok_or_else(|| Fail::new("say the network's name: xenonet net create --name <name> --host <id>", 2))?;
            let host = flags.text("host").ok_or_else(|| Fail::new("say this machine's id in the network: --host <id>", 2))?;
            let mut body = Map::new();
            body.insert("name".into(), json!(name));
            body.insert("host".into(), json!(host));
            if let Some(relay) = flags.text("relay") {
                body.insert("relay".into(), json!(relay));
            }
            let st = call(&s, "POST", "/api/net/create", Some(&Value::Object(body)))?;
            out(&st, &|st| {
                let made = list(st, "networks").iter().find(|n| str(n, "name") == Some(name.as_str())).map_or_else(|| st.clone(), view_of);
                print(format!("{} made the network {}, with this machine as {}", c::green("✓"), c::bold(t(&made["network"], "name")), t(&made["self"], "id")));
                if let Some(why) = str(st, "relayRefused") {
                    print(c::yellow(format!("  The relay wasn't added: {why}. Add it from XenoNet's window, Network settings.")));
                }
                print(c::dim("  Add a machine: xenonet net invite <id> on macOS, or Add a machine in XenoNet's window."));
            })
        }
        "join" => {
            let invite = arg(args, 1, "the invite and its code: xenonet net join <invite> <code>")?;
            let code = arg(args, 2, "the code that came with the invite")?;
            if !invite.trim().starts_with("xni2_") {
                return Err(Fail::new("that isn't an invite: an invite starts with xni2_, as Add a machine or xenonet net invite shows it", 2));
            }
            // The invite goes over join/2 to the machine that made it.
            let mut body = json!({ "invite": invite, "code": code, "lan": flags.on("mdns") });
            if let Some(at) = flags.text("at") {
                body["at"] = json!(at);
            }
            // While the invite's maker hasn't signed this machine in, or a
            // second window host hasn't approved it, the join waits: it says
            // why once, and asks again until the invite lapses.
            let started = std::time::Instant::now();
            let mut said = false;
            let st = loop {
                let st = call(&s, "POST", "/api/net/join", Some(&body))?;
                if !on(&st, "waiting") {
                    break st;
                }
                if started.elapsed() > std::time::Duration::from_mins(11) {
                    return Err(Fail::new(t(&st, "message"), 1));
                }
                if !said && !flags.on("json") {
                    print(c::yellow(t(&st, "message")));
                    said = true;
                }
                std::thread::sleep(std::time::Duration::from_secs(3));
            };
            out(&st, &|st| {
                // The network joined is the last the state lists.
                let joined = list(st, "networks").last().map_or_else(|| st.clone(), view_of);
                print(format!("{} joined {} as {}", c::green("✓"), c::bold(t(&joined["network"], "name")), t(&joined["self"], "id")));
            })
        }
        "invite" => {
            let host = arg(args, 1, "the new machine's id: xenonet net invite <id>")?;
            let r = call(&s, "POST", "/api/net/invite", Some(&within(json!({ "host": host, "window": flags.on("window"), "lan": flags.on("mdns") }))))?;
            out(&r, &|r| {
                print(format!("Give {} this invite. It works once, for ten minutes:", c::bold(t(r, "host"))));
                show_invite(r);
            })
        }
        "remove" | "replace" => {
            let host = arg(args, 1, "which machine: its id in the network")?;
            let r = call(&s, "POST", &format!("/api/net/{what}"), Some(&within(json!({ "host": host }))))?;
            out(&r, &|r| {
                if what == "replace" {
                    print(format!("{} {}'s old keys are out. Give it this invite. It works once, for ten minutes:", c::green("✓"), host));
                    show_invite(r);
                } else {
                    print(format!("{} {host} is out of the network", c::green("✓")));
                }
            })
        }
        "leave" => {
            let r = call(&s, "POST", "/api/net/leave", Some(&within(json!({ "force": flags.on("force") }))))?;
            out(&r, &|r| {
                // The founder never asks to be removed: its key is the root, so the
                // network has no founder after it.
                let asked = if on(r, "founder") {
                    " (it founded the network, which has no founder now)"
                } else if on(r, "removed") {
                    ""
                } else {
                    " (no member heard it ask: a window host can remove it)"
                };
                print(format!("{} left {}{asked}; this machine's other networks carry on as they were", c::green("✓"), t(r, "left")));
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
            for k in ["start", "wake", "approve"] {
                if let Some(v) = flags.text(k) {
                    body.insert(k.into(), json!(v));
                }
            }
            if body.is_empty() {
                return Err(Fail::new(
                    "say what to set: --titles-only, --no-titles-only, --share-load, --no-share-load, --start <off|you|network|delegate> or --wake <consent|network>",
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
            let st = call(&s, "PUT", "/api/net/settings", Some(&within(Value::Object(body))))?;
            let st = match &network {
                Some(_) => views(&st, network.as_deref())?.remove(0),
                None => st,
            };
            if let Some(host) = on {
                if !flags.on("json") {
                    print(format!("{} set for {host}: a change to network or delegate waits for the founder or another window host", c::green("✓")));
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
            let st = match views(&st, network.as_deref())?.as_slice() {
                [one] => one.clone(),
                [] => st,
                _ => return Err(Fail::new("this machine is in several networks: say which with --network <id|name>", 2)),
            };
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
            out(&call(&s, "POST", "/api/net/approve", Some(&within(json!({ "id": t(&p, "id") }))))?, &show_status)
        }
        w => Err(Fail::new(format!("xenonet net doesn't know {w}: create, join, invite, status, hosts, remove, replace, leave, set, share, relay or approve"), 2)),
    }
}
