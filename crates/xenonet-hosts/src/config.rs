//! core/hosts/config.js: hosts as this XenoNet keeps them: hosts.json in
//! the state folder (host/list/v1: this XenoNet's own host id and key, and
//! the hosts it shows) and XenoNet's own ssh config beside it, written
//! from that list. Links run ssh -F with that file, so ~/.ssh/config is
//! never read or changed.
//!
//! Every value that goes into the ssh config is checked here first, so
//! nothing can become an ssh option, a shell command or a second line.
//! The schema (zod's HostEntry) is checked too, through `check`.

use crate::cache::host_key;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::sync::OnceLock;
use xenonet_app::jsv::truthy;
use xenonet_base::ids::is_host_id;
use xenonet_base::text::{js_number, js_string, js_trim};

pub const HOSTS_FILE: &str = "hosts.json";
pub const SSH_CONFIG_FILE: &str = "ssh_config";
pub const DEFAULT_PORT: f64 = 47840.0;
pub const DEFAULT_SINCE: &str = "7d";

/// An error with the HTTP status it answers with.
#[derive(Clone, Debug, PartialEq)]
pub struct Refused {
    pub message: String,
    pub status: u16,
}

fn fail(message: impl Into<String>) -> Refused {
    Refused { message: message.into(), status: 400 }
}

// JSON.stringify(String(v)).
fn say(v: Option<&Value>) -> String {
    xenonet_base::json::stringify(&Value::String(v.map_or_else(|| "undefined".into(), js_string)), 0)
}

// schema/host.js's patterns.
fn re(i: usize) -> &'static Regex {
    static R: OnceLock<[Regex; 5]> = OnceLock::new();
    let r = R.get_or_init(|| {
        let s = r"[\t\n\x0B\x0C\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";
        [
            Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$").unwrap(),
            Regex::new(r"^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$").unwrap(),
            Regex::new(r"^(?:xenonet|/[A-Za-z0-9._+/-]{1,1023})$").unwrap(),
            Regex::new(r"^[0-9]{1,4}[mhd]$").unwrap(),
            Regex::new(&format!("^(?:([^@{}]+)@)?([^@{}]+)$", &s[1..s.len() - 1], &s[1..s.len() - 1])).unwrap(),
        ]
    });
    &r[i]
}

pub fn ssh_hostname_ok(s: &str) -> bool {
    re(0).is_match(s)
}

pub fn ssh_user_ok(s: &str) -> bool {
    re(1).is_match(s)
}

pub fn command_ok(s: &str) -> bool {
    re(2).is_match(s)
}

pub fn since_ok(s: &str) -> bool {
    re(3).is_match(s)
}

/// SSH_FILE: a full path, or one under ~/, of 1 to 1022 UTF-16 units after
/// that, with no quote, backslash, % or control character.
pub fn ssh_file_ok(s: &str) -> bool {
    let rest = s.strip_prefix('/').or_else(|| s.strip_prefix("~/"));
    let Some(rest) = rest else { return false };
    let n = xenonet_app::jstext::utf16_len(rest);
    (1..=1022).contains(&n) && !rest.chars().any(|c| matches!(c, '"' | '\\' | '%' | '\u{0}'..='\u{1f}' | '\u{7f}'))
}

/// "lab@vm1.local" → (Some("lab"), "vm1.local"). Only what ssh takes as
/// it is: nothing that starts with a dash, no spaces, no second @.
pub fn parse_target(target: Option<&Value>) -> Result<(Option<String>, String), Refused> {
    let t = target.filter(|v| !v.is_null()).map_or_else(String::new, js_string);
    let Some(c) = re(4).captures(js_trim(&t)) else { return Err(fail(format!("{} isn't an ssh target: give [user@]host, such as lab@192.168.5.15 or vm1.local", say(target)))) };
    let user = c.get(1).map(|m| m.as_str().to_string());
    let hostname = c[2].to_string();
    if !ssh_hostname_ok(&hostname) {
        return Err(fail(format!("{} isn't a host name or address ssh can take as it is", say(Some(&json!(hostname))))));
    }
    if let Some(u) = &user {
        if !ssh_user_ok(u) {
            return Err(fail(format!("{} isn't a user name ssh can take as it is", say(Some(&json!(u))))));
        }
    }
    Ok((user, hostname))
}

fn ssh_file(value: Option<&Value>, what: &str) -> Result<Value, Refused> {
    match value {
        None | Some(Value::Null) => Ok(Value::Null),
        Some(Value::String(s)) if s.is_empty() => Ok(Value::Null),
        Some(v) => {
            let s = js_string(v);
            if !ssh_file_ok(&s) {
                return Err(fail(format!("{what} has to be a full path (or one under ~/) with no quotes, backslashes or % in it, not {}", say(Some(v)))));
            }
            Ok(Value::String(s))
        }
    }
}

fn blank(v: Option<&Value>) -> bool {
    matches!(v, None | Some(Value::Null)) || v == Some(&json!(""))
}

fn get<'a>(o: &'a Value, k: &str) -> Option<&'a Value> {
    o.get(k).filter(|v| !v.is_null())
}

/// A host as someone asked for it (xenonet hosts add, POST /api/hosts),
/// checked and filled in. `check` is the schema's check of the entry made,
/// with what it says is wrong.
pub fn host_entry(input: &Value, now: f64, check: &mut dyn FnMut(&Value) -> Result<(), String>) -> Result<Value, Refused> {
    let id = input.get("id");
    let id_s = id.and_then(Value::as_str).filter(|s| is_host_id(s));
    let Some(id_s) = id_s else {
        let not = if blank(id) { String::new() } else { format!(", not {}", say(id)) };
        return Err(fail(format!("a host's id is a name you choose: 2 to 32 lowercase letters, digits and dashes, starting with a letter, such as vm1 or studio{not}")));
    };
    let transport = get(input, "transport").cloned().unwrap_or_else(|| json!(if truthy(input.get("local")) { "local" } else { "ssh" }));
    let transport_s = transport.as_str().filter(|t| *t == "ssh" || *t == "local");
    let Some(transport_s) = transport_s else {
        return Err(fail(format!("a host is reached over ssh, or locally (another XenoNet on this machine), not {}", say(Some(&transport)))));
    };
    let port = if blank(input.get("port")) { DEFAULT_PORT } else { js_number(input.get("port")) };
    if port.fract() != 0.0 || !(1.0..=65535.0).contains(&port) {
        return Err(fail(format!("the port of the XenoNet to read there is a number from 1 to 65535, not {}", say(input.get("port")))));
    }
    let since = get(input, "since").map_or_else(|| DEFAULT_SINCE.to_string(), js_string);
    if !since_ok(&since) {
        return Err(fail(format!("how far back sessions go is a number and m, h or d, such as 7d, 12h or 90m, not {}", say(Some(&json!(since))))));
    }
    let (mut ssh, mut command) = (Value::Null, Value::Null);
    if transport_s == "ssh" {
        let empty = json!({});
        let given = get(input, "ssh").unwrap_or(&empty);
        let target = if truthy(given.get("hostname")) {
            let user = if truthy(given.get("user")) { format!("{}@", js_string(&given["user"])) } else { String::new() };
            json!(format!("{user}{}", js_string(&given["hostname"])))
        } else {
            get(input, "target").cloned().unwrap_or_else(|| json!(id_s))
        };
        let (user, hostname) = parse_target(Some(&target))?;
        let ssh_port = get(given, "port").or_else(|| input.get("sshPort"));
        let ssh_port_n = (!blank(ssh_port)).then(|| js_number(ssh_port));
        if let Some(p) = ssh_port_n {
            if !(p.fract() == 0.0 && (1.0..=65535.0).contains(&p)) {
                return Err(fail(format!("ssh's port is a number from 1 to 65535, not {}", say(ssh_port))));
            }
        }
        let alias = get(given, "hostKeyAlias").or_else(|| get(input, "hostKeyAlias"));
        if !blank(alias) && !ssh_hostname_ok(&js_string(alias.unwrap())) {
            return Err(fail(format!("{} isn't a name ssh can keep a host key under", say(alias))));
        }
        let identity = ssh_file(get(given, "identity").or_else(|| input.get("identity")), "the key file")?;
        let known = ssh_file(get(given, "knownHosts").or_else(|| input.get("knownHosts")), "the known_hosts file")?;
        ssh = json!({
            "hostname": hostname,
            "user": user,
            "port": ssh_port_n.map(xenonet_base::json::number),
            "identity": identity,
            "knownHosts": known,
            "hostKeyAlias": if truthy(alias) { alias.cloned().unwrap() } else { Value::Null },
        });
        let c = get(input, "command").map_or_else(|| "xenonet".to_string(), js_string);
        if !command_ok(&c) {
            return Err(fail(format!("the xenonet command on the host is xenonet, or its full path with no spaces or shell characters in it, not {}", say(Some(&json!(c))))));
        }
        command = json!(c);
    }
    let mut entry = Map::new();
    entry.insert("id".into(), json!(id_s));
    entry.insert("transport".into(), json!(transport_s));
    entry.insert("ssh".into(), ssh);
    entry.insert("port".into(), xenonet_base::json::number(port));
    entry.insert("command".into(), command);
    entry.insert("since".into(), json!(since));
    entry.insert("detail".into(), json!(input.get("detail") == Some(&json!(true))));
    if input.get("start") == Some(&json!(true)) {
        entry.insert("start".into(), json!(true));
    }
    entry.insert("key".into(), Value::Null);
    entry.insert("name".into(), Value::Null);
    entry.insert("addedAt".into(), xenonet_base::json::number(now));
    let entry = Value::Object(entry);
    // The schema again, so nothing written here can be what it refuses.
    check(&entry).map_err(|why| fail(format!("that host can't be kept: {why}")))?;
    Ok(entry)
}

// Nothing of an ssh block may hold a line break, whatever passed before.
fn one_line(h: &Value) -> bool {
    let ssh = &h["ssh"];
    [&h["id"], &ssh["hostname"], &ssh["user"], &ssh["port"], &ssh["identity"], &ssh["knownHosts"], &ssh["hostKeyAlias"]]
        .iter()
        .all(|v| !js_string(v).contains(['\n', '\r', '\u{2028}', '\u{2029}', '\0']))
}

/// XenoNet's own ssh config, a Host block per ssh host, named by its id.
/// An entry the schema refuses is left out, and so is any with a line
/// break in a value. Files go in quotes, since they may hold spaces.
pub fn ssh_config_text(hosts: &[Value], check: &mut dyn FnMut(&Value) -> bool) -> String {
    let mut lines: Vec<String> = vec![
        "# XenoNet's own ssh config, for the hosts added with xenonet hosts add.".into(),
        "# XenoNet writes it from hosts.json and runs ssh -F with it, so your".into(),
        "# ~/.ssh/config is neither read nor changed. Edits here are written over.".into(),
    ];
    for h in hosts {
        if h.get("transport").and_then(Value::as_str) != Some("ssh") || !truthy(h.get("ssh")) || !check(h) || !one_line(h) {
            continue;
        }
        let ssh = &h["ssh"];
        lines.push(String::new());
        lines.push(format!("Host {}", js_string(&h["id"])));
        lines.push(format!("  HostName {}", js_string(&ssh["hostname"])));
        if truthy(ssh.get("user")) {
            lines.push(format!("  User {}", js_string(&ssh["user"])));
        }
        if truthy(ssh.get("port")) {
            lines.push(format!("  Port {}", js_string(&ssh["port"])));
        }
        if truthy(ssh.get("identity")) {
            lines.push(format!("  IdentityFile \"{}\"", js_string(&ssh["identity"])));
            lines.push("  IdentitiesOnly yes".into());
        }
        if truthy(ssh.get("knownHosts")) {
            lines.push(format!("  UserKnownHostsFile \"{}\"", js_string(&ssh["knownHosts"])));
        }
        if truthy(ssh.get("hostKeyAlias")) {
            lines.push(format!("  HostKeyAlias {}", js_string(&ssh["hostKeyAlias"])));
        }
    }
    format!("{}\n", lines.join("\n"))
}

/// hosts.json's version from its schema field; none means the first.
pub fn version_of(doc: &Value) -> Option<f64> {
    match doc.get("schema") {
        None => Some(1.0),
        Some(s) => xenonet_base::ids::parse_schema_id(&js_string(s)).filter(|p| p.path == "host/list").map(|p| p.major as f64),
    }
}

/// hosts.json as XenoNet keeps it: its schema named, this XenoNet's own
/// entry when it holds a key (and a name only when it's a host id), and
/// the hosts that pass the schema once each, in order; the ids of the
/// rest, as they were dropped.
pub fn prepare(doc: &Value, now: f64, check: &mut dyn FnMut(&Value) -> bool) -> (Value, Value, Vec<String>) {
    let self_v = match doc.get("self") {
        Some(s @ (Value::Object(_) | Value::Array(_))) if host_key().is_match(&s.get("key").filter(|k| !k.is_null()).map_or_else(String::new, js_string)) => {
            let id = s.get("id").and_then(Value::as_str).filter(|i| is_host_id(i));
            let created = s.get("createdAt").and_then(Value::as_f64).unwrap_or(now);
            json!({ "id": id, "key": s["key"], "createdAt": xenonet_base::json::number(created) })
        }
        _ => Value::Null,
    };
    let mut seen: Vec<String> = Vec::new();
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    if let Some(Value::Array(hosts)) = doc.get("hosts") {
        for h in hosts {
            let id = h.get("id").and_then(Value::as_str);
            let ok = check(h) && id.is_some_and(|i| !seen.iter().any(|s| s == i));
            if ok {
                seen.push(id.unwrap().to_string());
                kept.push(h.clone());
            } else {
                dropped.push(id.filter(|i| is_host_id(i)).map_or_else(|| "(no id)".to_string(), String::from));
            }
        }
    }
    (self_v, Value::Array(kept), dropped)
}

/// This XenoNet's name as a host, if it's one (null takes it away).
pub fn check_self_id(id: Option<&Value>) -> Result<(), Refused> {
    match id {
        None | Some(Value::Null) => Ok(()),
        Some(Value::String(s)) if is_host_id(s) => Ok(()),
        Some(v) => {
            let not = if truthy(Some(v)) { format!(", not {}", say(Some(v))) } else { String::new() };
            Err(fail(format!("a host's id is a name you choose: 2 to 32 lowercase letters, digits and dashes, starting with a letter, such as studio or vm1{not}")))
        }
    }
}

/// The hosts with `entry` added, or put in place of the host of its id
/// with `replace`.
pub fn add(hosts: &[Value], entry: &Value, replace: bool) -> Result<Vec<Value>, Refused> {
    let id = entry.get("id");
    let at = hosts.iter().position(|h| h.get("id") == id);
    let mut out = hosts.to_vec();
    match at {
        Some(_) if !replace => {
            let id = id.map(js_string).unwrap_or_default();
            Err(Refused { message: format!("there's already a host called {id}: remove it first, or replace it (xenonet hosts add {id} … --replace)"), status: 409 })
        }
        Some(i) => {
            out[i] = entry.clone();
            Ok(out)
        }
        None => {
            out.push(entry.clone());
            Ok(out)
        }
    }
}

/// What a host said about itself the first time it answered: its key, and
/// its own name when that isn't the id here.
pub fn learn(entry: &Value, key: Option<&Value>, name: Option<&Value>) -> Value {
    let mut e = entry.clone();
    if let Some(k) = key {
        if k.is_null() || k.as_str().is_some_and(|s| host_key().is_match(s)) {
            e["key"] = k.clone();
        }
    }
    if let Some(n) = name {
        let id = entry.get("id");
        e["name"] = match n.as_str() {
            Some(s) if is_host_id(s) && Some(n) != id => n.clone(),
            _ => Value::Null,
        };
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(_: &Value) -> Result<(), String> {
        Ok(())
    }

    #[test]
    fn nothing_that_could_become_an_option_gets_in() {
        for (input, msg) in [
            (json!({ "id": "VM1" }), "name you choose"),
            (json!({ "id": "vm1", "target": "-oProxyCommand=open -a Calculator" }), "ssh target"),
            (json!({ "id": "vm1", "target": "lab@-oProxyCommand=x" }), "host name"),
            (json!({ "id": "vm1", "target": "la b@host" }), "ssh target"),
            (json!({ "id": "vm1", "target": "a@b@host" }), "ssh target"),
            (json!({ "id": "vm1", "command": "xenonet; rm -rf ~" }), "xenonet command"),
            (json!({ "id": "vm1", "identity": "/tmp/key\nProxyCommand evil" }), "key file"),
            (json!({ "id": "vm1", "identity": "/tmp/%h" }), "key file"),
            (json!({ "id": "vm1", "port": "47840; id" }), "port"),
            (json!({ "id": "vm1", "sshPort": 70000 }), "ssh's port"),
            (json!({ "id": "vm1", "since": "forever" }), "how far back"),
            (json!({ "id": "vm1", "transport": "telnet" }), "over ssh, or locally"),
        ] {
            let e = host_entry(&input, 1.0, &mut ok).unwrap_err();
            assert!(e.message.contains(msg), "{input}: {}", e.message);
            assert_eq!(e.status, 400);
        }
        let e = host_entry(&json!({ "id": "vm1", "target": "lab@127.0.0.1", "sshPort": 60851, "identity": "~/k" }), 1.0, &mut ok).unwrap();
        assert_eq!(e["ssh"], json!({ "hostname": "127.0.0.1", "user": "lab", "port": 60851, "identity": "~/k", "knownHosts": null, "hostKeyAlias": null }));
        let mut bad = e.clone();
        bad["ssh"]["hostname"] = json!("x\nProxyCommand evil");
        assert!(!ssh_config_text(&[bad], &mut |_| true).contains("ProxyCommand"));
        assert!(ssh_config_text(&[e], &mut |_| true).contains("\nHost vm1\n  HostName 127.0.0.1\n  User lab\n  Port 60851\n  IdentityFile \"~/k\"\n  IdentitiesOnly yes\n"));
    }
}
