//! What the doctor says about the network (docs/design/network.md, "The
//! wire"), from the files in `<state>/network/` alone: the network's files
//! and key, the link (up, or down since when and why), the gateway's key
//! against the pinned one, the clock, the outbox and the queues, a roster
//! mismatch, being removed, and what the gateway's address means.
//!
//! The link writes how it stands to `link.json`; these checks read it, so
//! they work whether the service is running or not.

use crate::files::{Dir, KEY};
use crate::keys::HostKey;
use serde_json::{json, Map, Value};
use std::path::Path;

/// One check: its id, title and verdict ({ status, detail, fix? }).
pub type Check = (&'static str, &'static str, Value);

fn verdict(status: &str, detail: impl Into<String>, fix: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("status".into(), json!(status));
    m.insert("detail".into(), json!(detail.into()));
    if let Some(f) = fix {
        m.insert("fix".into(), json!(f));
    }
    Value::Object(m)
}

fn span(ms: f64) -> String {
    let s = (ms / 1000.0).abs();
    if s < 90.0 {
        format!("{} s", s.round())
    } else if s < 5400.0 {
        format!("{} min", (s / 60.0).round())
    } else {
        format!("{} h", (s / 3600.0).round())
    }
}

/// The time of day of `ms`, in UTC as HH:MM.
fn clock_time(ms: f64) -> String {
    let iso = xenonet_base::time::iso(ms);
    iso.get(11..16).unwrap_or(&iso).to_string()
}

/// The checks, or none when this machine isn't in a network.
pub fn checks(dir: &Path, now: f64) -> Vec<Check> {
    let d = Dir::new(dir);
    let net = match d.read_network() {
        Ok(Some(n)) => n,
        Ok(None) => return Vec::new(),
        Err(e) => return vec![("network.files", "The network's files are in order", verdict("fail", e.message, Some("xenonet net leave, then join the network again")))],
    };
    let mut out = Vec::new();
    out.push(("network.files", "The network's files are in order", files(&d, &net.host)));
    let link = d.read_link().unwrap_or(Value::Null);
    let s = |k: &str| link.get(k).and_then(Value::as_str);
    let gateway_here = link.get("gateway").and_then(Value::as_bool) == Some(true);

    // Removed.
    let removed = s("state") == Some("removed");
    out.push((
        "network.removed",
        "This machine is in its network",
        if removed {
            verdict("fail", s("reason").unwrap_or("this machine was removed from the network"), Some("xenonet net leave"))
        } else {
            verdict("ok", format!("{} in {}", net.host, net.name), None)
        },
    ));

    // The link.
    let since = link.get("since").and_then(Value::as_f64);
    let reason = s("reason").map(str::to_string);
    let link_check = match s("state") {
        _ if removed => verdict("skip", "removed from the network", None),
        Some("live") if gateway_here => verdict("ok", "this machine is the network's gateway", None),
        Some("live") => verdict(
            "ok",
            format!("live{}", if link.get("longPoll").and_then(Value::as_bool) == Some(true) { ", a request at a time (a proxy holds streams back)" } else { "" }),
            None,
        ),
        Some("connecting") => verdict("info", "connecting to the gateway", None),
        Some(state @ ("offline" | "refused")) => {
            let at = since.map_or_else(String::new, |t| format!(" since {} UTC ({} ago)", clock_time(t), span(now - t)));
            let why = reason.unwrap_or_else(|| if state == "refused" { "refused".into() } else { "no answer".into() });
            verdict(
                "warn",
                format!("The network's gateway hasn't answered{at}: {why}"),
                Some("Messages for other machines wait in the outbox, and go when the gateway is back. XenoNet tries again by itself."),
            )
        }
        _ => verdict("info", "the link isn't running: XenoNet's service runs it", Some("xenonet service start")),
    };
    out.push(("network.link", "This machine reaches the network's gateway", link_check));

    // The gateway's key.
    let pinned = s("pinnedKeyId").map_or(net.gateway.key_id.clone(), str::to_string);
    out.push((
        "network.key",
        "The gateway proves the key this machine pinned",
        match s("gatewayKeyId") {
            Some(seen) if seen != pinned => verdict(
                "fail",
                format!("the gateway answered with the key {seen}, not {pinned}"),
                Some("Check the gateway's address; if the gateway moved, xenonet net gateway set <address>"),
            ),
            Some(_) => verdict("ok", format!("{} ({pinned})", net.gateway.host), None),
            None => verdict("info", format!("pinned {pinned}; not checked since XenoNet started"), None),
        },
    ));

    // The clock.
    let skew = link.get("skewMs").and_then(Value::as_f64);
    out.push((
        "network.clock",
        "This computer's clock is close to the gateway's",
        match skew {
            Some(x) if x.abs() > 60e3 => verdict(
                "warn",
                format!("this clock is {} {} the gateway's; XenoNet signs with the gateway's time, so the link works", span(x), if x > 0.0 { "behind" } else { "ahead of" }),
                Some("Have this computer set its clock by itself: on Linux, timedatectl set-ntp true; on a Mac, System Settings › General › Date & Time."),
            ),
            Some(x) if x.abs() >= 2000.0 => verdict("info", format!("{} off the gateway's", span(x)), None),
            Some(_) => verdict("ok", "within two seconds of the gateway's", None),
            None => verdict("skip", "not linked yet", None),
        },
    ));

    // The outbox and the queues.
    let o = link.get("outbox").cloned().unwrap_or(Value::Null);
    let frames = o.get("frames").and_then(Value::as_u64).unwrap_or(0);
    let dropped = o.get("dropped").and_then(Value::as_u64).unwrap_or(0);
    let queues: Vec<String> = link
        .get("queues")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|q| q.get("frames").and_then(Value::as_u64).unwrap_or(0) > 0)
        .map(|q| format!("{} waits for {}", q.get("frames").and_then(Value::as_u64).unwrap_or(0), q.get("host").and_then(Value::as_str).unwrap_or("?")))
        .collect();
    let at_gateway = if queues.is_empty() { String::new() } else { format!("; at the gateway, {}", queues.join(", ")) };
    let detail = format!("{frames} frame{} in the outbox{at_gateway}", if frames == 1 { "" } else { "s" });
    out.push((
        "network.queues",
        "Messages for other machines aren't piling up",
        if dropped > 0 {
            verdict(
                "warn",
                format!("{detail}; {dropped} went unsent, oldest first, after a week or past 10,000 frames or 50 MB"),
                Some("Those messages' senders can send them again once the gateway is back."),
            )
        } else if frames > 1000 {
            verdict("warn", detail, Some("The gateway hasn't taken them yet; they go when it's back."))
        } else {
            verdict("ok", detail, None)
        },
    ));

    // The join route, on the gateway's host.
    if gateway_here {
        let closed = link.get("joinClosedUntil").and_then(Value::as_f64).filter(|t| *t > now);
        out.push((
            "network.join",
            "New machines can join through the gateway",
            match closed {
                Some(t) => verdict(
                    "warn",
                    format!("200 bad tickets came in an hour, so the join route is shut until {} UTC ({} from now)", clock_time(t), span(t - now)),
                    Some("Someone may be guessing tickets. A machine joins once it opens again, with a new ticket."),
                ),
                None => verdict("ok", "open: each source gets 20 bad tickets an hour, and a ticket five wrong tries", None),
            },
        ));
    }

    // The roster.
    let mismatch: Vec<&str> = link.get("rosterMismatch").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    out.push((
        "network.roster",
        "Every host sees the same roster",
        if mismatch.is_empty() {
            verdict("ok", "no host shows another roster", None)
        } else {
            verdict(
                "warn",
                format!("{} shows another roster than this machine's", mismatch.join(", ")),
                Some("A gateway may be holding entries back; each host fetches what it's missing when it links."),
            )
        },
    ));

    // The address.
    let address = net.gateway.address.clone().unwrap_or_default();
    let kind = if address.is_empty() { "" } else { crate::files::address_kind(&address) };
    out.push((
        "network.address",
        "The gateway's address suits a network",
        match kind {
            "" => verdict(
                "warn",
                "the gateway has no address yet, so no other machine can join",
                Some("Turn the gateway on with a tunnel (xenonet gateway on --tunnel <kind>), or give it a LAN address (xenonet net set --lan <ip:port>)."),
            ),
            "loopback" => verdict("info", format!("{address} is this computer alone: only XenoNets on it can join, which is for tests and same-machine use"), None),
            "lan" => verdict(
                "info",
                format!("LAN only, at {address}: plain HTTP carrying signed, sealed frames, so the network sees who talks to whom, never what; actor tokens never cross it"),
                None,
            ),
            _ if address.ends_with(".trycloudflare.com") => verdict(
                "warn",
                format!("{address} is a quick Cloudflare tunnel, whose address changes every time it starts"),
                Some("Use a named Cloudflare tunnel, Tailscale Funnel or ngrok's own domain for a network."),
            ),
            _ => verdict("ok", format!("through a tunnel, at {address}"), None),
        },
    ));
    out
}

fn files(d: &Dir, host: &str) -> Value {
    let roster = match d.read_roster() {
        Ok(r) => r,
        Err(e) => return verdict("fail", format!("roster.json doesn't check: {}", e.message), Some("xenonet net leave, then join the network again")),
    };
    let key = match HostKey::load(&d.file(KEY)) {
        Ok(Some(k)) => k,
        Ok(None) => return verdict("fail", "host.key is missing", Some("xenonet net leave, then join the network again")),
        Err(e) => return verdict("fail", e.message, Some("xenonet net leave, then join the network again")),
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(d.file(KEY)) {
            if m.permissions().mode() & 0o077 != 0 {
                return verdict("fail", "host.key can be read by other accounts on this computer", Some("chmod 600 on host.key in XenoNet's state folder, network/"));
            }
        }
    }
    let view = roster.view();
    match view.member(host) {
        Some(m) if m.keys == *key.public() => verdict("ok", format!("{} hosts in the roster; this machine's key is {}", view.members.len(), key.key_id()), None),
        Some(_) => verdict("fail", "host.key isn't the key the roster has for this machine", Some("xenonet net leave, then join the network again")),
        None if view.removal(host).is_some() => verdict("ok", "the roster says this machine was removed", None),
        None => verdict("warn", "this machine isn't in its copy of the roster yet", None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_network_no_checks() {
        let dir = std::env::temp_dir().join(format!("xenonet-net-doctor-none-{}", std::process::id()));
        assert_eq!(checks(&dir, 0.0).len(), 0);
    }

    #[test]
    fn spans_and_times() {
        assert_eq!(span(300_000.0), "5 min");
        assert_eq!(span(-3000.0), "3 s");
        assert_eq!(clock_time(0.0), "00:00");
    }
}
