//! xenonet hosts (cli/commands/hosts.js): other machines' XenoNets on this
//! one's map (docs/design/hosts.md), through a running XenoNet's API: this
//! XenoNet's own host name, and the hosts it shows, which it reaches over
//! SSH with an ssh config of its own. A prototype: the console shows hosts
//! only with XENONET_HOSTS=1, and only reads from them.

use crate::args::Flags;
use crate::clock;
use crate::js::{encode, have, list, num, on, str, t};
use crate::out::{ago, c, json, now, print, table, Done, Fail, Layout};
use crate::prompt::ask;
use crate::reach::need;
use crate::sys;
use serde_json::{json, Map, Value};
use xenonet_base::client::{name_of, Server};
use xenonet_base::json::number_string;
use xenonet_base::units::{js_round, to_fixed};

pub const HELP: &str = include_str!("help/hosts.txt");

fn dot(state: &str) -> String {
    match state {
        "live" => c::green("●"),
        "connecting" => c::cyan("◌"),
        "stale" => c::yellow("◐"),
        "offline" => c::dim("○"),
        "error" => c::red("✗"),
        _ => "?".into(),
    }
}

// A word a shell takes as it is, or quoted (core/hosts/link.js).
fn quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|ch| ch.is_ascii_alphanumeric() || "_@%+=:,./-".contains(ch)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// shellPath (core/hosts/link.js): a path as a shell takes it, ~/ left
/// for the shell to read.
pub fn shell_path(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(p),
    }
}

/// trustCommand (core/hosts/link.js): the command that checks a host's key,
/// and accepts it once, in a terminal, with XenoNet's own ssh config.
pub fn trust_command(id: &str, ssh_config: Option<&str>) -> String {
    match ssh_config.filter(|c| !c.is_empty()) {
        Some(config) => format!("ssh -F {} {id}", shell_path(&sys::tilde(config))),
        None => format!("ssh {id}"),
    }
}

/// How far another machine's clock is from this one's, when it's 2 s or
/// more.
pub fn offset(ms: Option<f64>) -> String {
    let Some(ms) = ms.filter(|m| m.abs() >= 2000.0 || m.is_nan()) else { return String::new() };
    let a = ms.abs();
    let size = if a < 90e3 { format!("{} s", number_string(js_round(a / 1000.0))) } else { format!("{} min", number_string(js_round(a / 60e3))) };
    format!("clock {size} {}", if ms > 0.0 { "ahead" } else { "behind" })
}

// How a host is doing, in a line, and what to do about it, if anything.
fn describe(h: &Value, now: f64) -> (String, Option<String>) {
    let mut what = Vec::new();
    if on(h, "version") {
        what.push(format!("XenoNet {}{}", t(h, "version"), if on(h, "platform") { format!(" on {}", t(h, "platform")) } else { String::new() }));
    }
    if on(h, "demo") {
        what.push("made-up data".into());
    }
    if on(h, "calls") {
        what.push(format!("calls itself {}", t(h, "calls")));
    }
    let off = offset(have(h, "offsetMs").map(|v| xenonet_base::text::js_number(Some(v))));
    if !off.is_empty() {
        what.push(off);
    }
    match str(h, "state") {
        Some("live") => return (if what.is_empty() { "live".into() } else { what.join(" · ") }, None),
        Some("connecting") => return ("connecting".into(), None),
        Some("stale") => return (format!("no word since {}", clock::hour_minute(have(h, "lastHeard").map_or(now, |v| xenonet_base::text::js_number(Some(v))))), None),
        _ => {}
    }
    let since = if on(h, "lastHeard") { format!("last heard {}", ago(num(h, "lastHeard"), now)) } else { "never reached".into() };
    let retry = if on(h, "nextTryAt") { format!(", trying again in {} s", number_string(js_round((num(h, "nextTryAt") - now) / 1000.0).max(1.0))) } else { String::new() };
    let error = h.get("error").filter(|e| !e.is_null());
    let said = error.and_then(|e| have(e, "message")).map_or_else(|| t(h, "state"), |m| crate::js::text(Some(m)));
    let cached = if on(h, "cached") { " · its last snapshot is on the map" } else { "" };
    (format!("{said} · {since}{retry}{cached}"), error.and_then(|e| have(e, "fix")).map(|f| crate::js::text(Some(f))))
}

// The hosts list, or None when hosts are off in that XenoNet.
fn hosts_of(s: &Server) -> Result<Option<Value>, Fail> {
    match s.get("/api/hosts") {
        Ok(v) => Ok(Some(v)),
        Err(e) if e.status == Some(404) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn off(s: &Server) -> String {
    format!("Hosts are off in {}: start it with XENONET_HOSTS=1 to show other machines' XenoNets (a prototype).", name_of(s))
}

// key.slice(0, 8)…
fn key_of(v: &Value) -> String {
    let key = t(v, "key");
    format!("key {}…", xenonet_base::log::js_slice(&key, 8))
}

fn this_self(s: &Server, rest: &[String], flags: &Flags) -> Done {
    let name = rest.first();
    let change = name.is_some() || flags.is_true("clear");
    let me = if change {
        let mut body = Map::new();
        if flags.on("clear") {
            body.insert("id".into(), Value::Null);
        } else if let Some(n) = name {
            body.insert("id".into(), json!(n));
        }
        s.post("/api/hosts/self", &Value::Object(body), &[])?
    } else {
        s.get("/api/hosts/self")?
    };
    if flags.on("json") {
        json(&me);
        return Ok(0);
    }
    let key = c::dim(format!("({})", key_of(&me)));
    let n = name_of(s);
    if change {
        print(if on(&me, "id") {
            format!("{} {n} calls itself {} as a host now {key}", c::green("✓"), c::bold(t(&me, "id")))
        } else {
            format!("{} {n} has no host name now {key}", c::green("✓"))
        });
    } else {
        print(if on(&me, "id") {
            format!("{n} calls itself {} as a host {key}", c::bold(t(&me, "id")))
        } else {
            format!("{n} hasn't named itself as a host {key}. xenonet hosts self <name> names it: a name you choose, such as studio or vm1.")
        });
    }
    Ok(0)
}

fn list_hosts(s: &Server, flags: &Flags) -> Done {
    let Some(all) = hosts_of(s)? else {
        if flags.on("json") {
            json(&json!({ "on": false, "self": s.get("/api/hosts/self")? }));
            return Ok(0);
        }
        print(off(s));
        let me = s.get("/api/hosts/self")?;
        print(c::dim(if on(&me, "id") { format!("As a host, it calls itself {}.", t(&me, "id")) } else { "As a host, it hasn't named itself: xenonet hosts self <name>.".into() }));
        return Ok(0);
    };
    if flags.on("json") {
        let mut m = Map::new();
        m.insert("on".into(), json!(true));
        m.extend(all.as_object().cloned().unwrap_or_default());
        json(&Value::Object(m));
        return Ok(0);
    }
    let me = all.get("self").cloned().unwrap_or(Value::Null);
    print(format!(
        "{} {} {}",
        c::green("●"),
        c::bold(have(&me, "id").map_or_else(|| "this XenoNet".into(), |i| crate::js::text(Some(i)))),
        c::dim(format!("· {}, here · {}", name_of(s), key_of(&me)))
    ));
    let hosts = list(&all, "hosts");
    if hosts.is_empty() {
        print(c::dim("  No hosts yet. xenonet hosts add <id> [user@host] adds one."));
        return Ok(0);
    }
    let mut rows = Vec::new();
    for h in hosts {
        let (line, fix) = describe(h, now());
        rows.push(vec![
            t(h, "id"),
            c::dim(have(h, "target").map_or(String::new(), |x| crate::js::text(Some(x)))),
            format!("{} {}", dot(str(h, "state").unwrap_or("")), t(h, "state")),
            line,
        ]);
        if let Some(fix) = fix {
            rows.push(vec![String::new(), String::new(), String::new(), c::dim(format!("↳ {fix}"))]);
        }
    }
    print(table(&rows, Layout::wrapped(2)));
    let config = have(&all, "sshConfig").map_or_else(|| "ssh_config".into(), |x| crate::js::text(Some(x)));
    print(c::dim(format!("\nXenoNet reaches them with its own ssh config, {}.", sys::tilde(&config))));
    Ok(0)
}

fn add(s: &Server, rest: &[String], flags: &Flags) -> Done {
    let Some(id) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new("say which host: xenonet hosts add <id> [user@host]", 2)) };
    let mut body = Map::new();
    body.insert("id".into(), json!(id));
    body.insert("replace".into(), json!(flags.is_true("replace")));
    if flags.on("local") {
        body.insert("transport".into(), json!("local"));
    } else {
        body.insert("target".into(), json!(rest.get(1).unwrap_or(id)));
        for k in ["sshPort", "identity", "knownHosts", "hostKeyAlias", "command"] {
            if let Some(v) = flags.get(k) {
                body.insert(k.into(), v.clone());
            }
        }
    }
    if let Some(v) = flags.get("xenonetPort") {
        body.insert("port".into(), v.clone());
    }
    if let Some(v) = flags.get("since") {
        body.insert("since".into(), v.clone());
    }
    if flags.on("detail") {
        body.insert("detail".into(), json!(true));
    }
    if flags.on("start") {
        body.insert("start".into(), json!(true));
    }
    let mut h = match s.post("/api/hosts", &Value::Object(body), &[]) {
        Ok(h) => h,
        Err(e) if e.status == Some(404) => return Err(Fail::new(off(s), 1)),
        Err(e) => return Err(e.into()),
    };
    // A moment for the first connection, so what it found can be said.
    for _ in 0..25 {
        if str(&h, "state") != Some("connecting") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        if let Some(found) = hosts_of(s)?.and_then(|l| list(&l, "hosts").iter().find(|x| str(x, "id") == Some(id)).cloned()) {
            h = found;
        }
    }
    if flags.on("json") {
        json(&h);
        return Ok(0);
    }
    let all = hosts_of(s)?;
    print(format!("{} Added {} {}", c::green("✓"), c::bold(id), c::dim(format!("({})", t(&h, "target")))));
    let (line, fix) = describe(&h, now());
    print(format!("  {} {} · {line}", dot(str(&h, "state").unwrap_or("")), t(&h, "state")));
    if let Some(fix) = fix {
        print(c::dim(format!("  ↳ {fix}")));
    }
    if let Some(config) = all.as_ref().and_then(|l| crate::js::some(l, "sshConfig")).filter(|_| str(&h, "transport") == Some("ssh")) {
        print(c::dim(format!(
            "  XenoNet connects with its own ssh config. The first time, check {id}'s host key in a terminal: {}",
            trust_command(id, Some(&crate::js::text(Some(config))))
        )));
    }
    Ok(0)
}

fn remove(s: &Server, rest: &[String], flags: &Flags) -> Done {
    let Some(id) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new("say which host: xenonet hosts remove <id>", 2)) };
    let forget = flags.is_true("forget") || (!flags.is_false("forget") && !flags.on("json") && ask(&format!("Also forget where you placed {id}'s nodes on the map?"), flags));
    let r = match s.del(&format!("/api/hosts/{}{}", encode(id), if forget { "?forget=1" } else { "" })) {
        Ok(r) => r,
        Err(e) if e.status == Some(404) && e.message.contains("XENONET_HOSTS") => return Err(Fail::new(off(s), 1)),
        Err(e) => return Err(e.into()),
    };
    if flags.on("json") {
        json(&r);
        return Ok(0);
    }
    let forgot = if on(&r, "forgot") { format!(", and where you placed {} of its nodes", t(&r, "forgot")) } else { String::new() };
    print(format!("{} Removed {}{forgot}. Its last snapshot is gone from the cache.", c::green("✓"), c::bold(id)));
    Ok(0)
}

fn test(s: &Server, rest: &[String], flags: &Flags) -> Done {
    let Some(id) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new("say which host: xenonet hosts test <id>", 2)) };
    let tried = s.post(&format!("/api/hosts/{}/test", encode(id)), &json!({}), &[])?;
    if flags.on("json") {
        json(&tried);
        return Ok(if on(&tried, "ok") { 0 } else { 1 });
    }
    let error = tried.get("error").filter(|e| !e.is_null());
    if !on(&tried, "ok") {
        let said =
            error.and_then(|e| have(e, "message")).map_or_else(|| format!("no answer in {} s", number_string(js_round(num(&tried, "ms") / 1000.0))), |m| crate::js::text(Some(m)));
        print(format!("{} {id}: {said}", c::red("✗")));
        if let Some(fix) = error.and_then(|e| crate::js::some(e, "fix")) {
            print(c::dim(format!("  ↳ {}", crate::js::text(Some(fix)))));
        }
        return Ok(1);
    }
    let h = tried.get("hello").filter(|x| !x.is_null()).cloned().unwrap_or(json!({}));
    let calling = if on(&h, "id") && str(&h, "id") != Some(id) { format!(", calling itself {}", t(&h, "id")) } else { String::new() };
    print(format!(
        "{} {id} answered in {} s: XenoNet {} on {}{}{calling}",
        c::green("✓"),
        to_fixed(num(&tried, "ms") / 1000.0, 1),
        t(&h, "version"),
        t(&h, "platform"),
        if on(&h, "demo") { ", made-up data" } else { "" }
    ));
    let days = js_round(num(&h, "sinceMs") / 86400e3);
    let window = if on(&h, "sinceMs") { format!(" for sessions of the last {} d", number_string(if days == 0.0 || days.is_nan() { 1.0 } else { days })) } else { String::new() };
    print(format!(
        "{} its relay speaks protocol {}, and sends {}{window}",
        c::green("✓"),
        t(&h, "protocol"),
        if on(&h, "detail") { "live detail too" } else { "titles, states and counts" }
    ));
    let snap = tried.get("snapshot").cloned().unwrap_or(Value::Null);
    print(format!("{} a map of {} places and sessions, {} of them sessions, {} live", c::green("✓"), t(&snap, "nodes"), t(&snap, "sessions"), t(&snap, "live")));
    let skew = have(&tried, "offsetMs").map(|v| xenonet_base::text::js_number(Some(v)));
    let off = offset(skew);
    let off = off.strip_prefix("clock ").unwrap_or(&off);
    print(match skew {
        None => format!("{} its clock: no answer yet", c::dim("·")),
        Some(ms) if ms.abs() > 120e3 => format!("{} its clock is {off}: have it set its clock by itself", c::yellow("!")),
        Some(_) => format!("{} its clock: {}", c::green("✓"), if off.is_empty() { "within 2 s of this one's" } else { off }),
    });
    Ok(0)
}

pub fn run(args: &[String], flags: &Flags) -> Done {
    let s = need(flags)?;
    let sub = args.first().map_or("list", String::as_str);
    let rest = args.get(1..).unwrap_or(&[]);
    match sub {
        "self" => this_self(&s, rest, flags),
        "list" => list_hosts(&s, flags),
        "add" => add(&s, rest, flags),
        "remove" => remove(&s, rest, flags),
        "test" => test(&s, rest, flags),
        "reconnect" => {
            let Some(id) = rest.first().filter(|x| !x.is_empty()) else { return Err(Fail::new("say which host: xenonet hosts reconnect <id>", 2)) };
            let h = s.post(&format!("/api/hosts/{}/reconnect", encode(id)), &json!({}), &[])?;
            if flags.on("json") {
                json(&h);
            } else {
                print(format!("{} Connecting to {id} now.", c::green("✓")));
            }
            Ok(0)
        }
        _ => Err(Fail::new(format!("hosts takes self, add, remove, test or reconnect, not \"{sub}\". xenonet hosts --help says more."), 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_clocks_as_a_shell_and_a_person_read_them() {
        assert_eq!(shell_path("~/Library/Application Support/XenoNet Dev/ssh_config"), "~/'Library/Application Support/XenoNet Dev/ssh_config'");
        assert_eq!(shell_path("/etc/ssh_config"), "/etc/ssh_config");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(trust_command("vm1", None), "ssh vm1");
        assert_eq!(offset(Some(300.0)), "");
        assert_eq!(offset(Some(5000.0)), "clock 5 s ahead");
        assert_eq!(offset(Some(-180e3)), "clock 3 min behind");
        assert_eq!(offset(None), "");
    }
}
