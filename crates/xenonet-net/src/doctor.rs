//! What the doctor says about the network (docs/design/network.md, "The
//! wire"), from what this machine keeps of it alone: the network, its
//! roster and key, the link (up, or down since when and why), the
//! gateway's key against the pinned one, the clock, the outbox and the
//! queues, a roster mismatch, being removed, and what the gateway's
//! address means.
//!
//! The link keeps how it stands; these checks read it, so they work
//! whether the link is running or not.

use crate::files::{Dir, KEY};
use crate::keys::HostKey;
use serde_json::{json, Map, Value};

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
    let iso = xenonet_text::time::iso(ms);
    iso.get(11..16).unwrap_or(&iso).to_string()
}

/// The checks, or none when this machine isn't in a network.
pub fn checks(d: &Dir, now: f64) -> Vec<Check> {
    let net = match d.read_network() {
        Ok(Some(n)) => n,
        Ok(None) => return Vec::new(),
        Err(e) => return vec![("network.files", "The network's files are in order", verdict("fail", e.message, Some("xenonet net leave, then join the network again")))],
    };
    let mut out = Vec::new();
    out.push(("network.files", "The network's files are in order", files(d, &net.id, &net.host)));
    let link = d.read_link(&net.id).unwrap_or(Value::Null);
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
                Some("Give the gateway its tailnet or LAN address (xenonet net set --lan <ip:port> on its host)."),
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
                Some("Give the gateway its tailnet or LAN address instead (xenonet net set --lan <ip:port> on its host)."),
            ),
            // Funnel publishes only the actor gateway's /mcp, so a network's
            // frames never get through a .ts.net name.
            _ if crate::files::host_of(&address).trim_end_matches('.').to_ascii_lowercase().ends_with(".ts.net") => {
                let whose = if gateway_here || net.gateway.host == net.host { "this machine's".to_string() } else { format!("{}'s", net.gateway.host) };
                let fix = format!("Use {whose} tailnet address (xenonet net set --lan <tailnet ip:port> on the gateway's host).");
                verdict("warn", format!("{address} is a Tailscale Funnel name: a network can't link through Funnel, which publishes only /mcp"), Some(fix.as_str()))
            }
            _ => verdict("ok", format!("through a tunnel, at {address}"), None),
        },
    ));
    out
}

/// How this machine asks you and keeps your identity's root key, from what
/// it keeps of you and the identity folder beside the network's, without
/// opening any key: none when no identity was set up here.
///
/// - `network.presence`: how this machine's device asks you before it acts
///   as you, as your chain recorded it when the device was paired:
///   biometric, weaker, or none (no active device of yours here).
/// - `network.root`: whether the root key is sealed to the device key, kept
///   in an owner-only file (a warning), or held by another machine.
pub fn identity_checks(d: &Dir) -> Vec<Check> {
    let Ok(identity) = crate::identity::Identity::open(d.identity_dir(), d.persist.clone(), None) else { return Vec::new() };
    let chain = identity.chain();
    let Some(person) = chain.person() else { return Vec::new() };
    // This machine's device: the one its roster entry names, or, with no
    // network, the one device the person has (it was set up here).
    let net = d.read_network().ok().flatten();
    let named = net.and_then(|n| read_roster(d, &n.id).ok()?.view().member(&n.host)?.device.clone());
    let device = match named {
        Some(id) => person.device(&id),
        None => {
            let mut active = person.active_devices();
            match (active.next(), active.next()) {
                (Some(one), None) => Some(one),
                _ => None,
            }
        }
    };
    let presence = match device.filter(|d| d.active()).map(|d| d.kind) {
        Some(xenonet_log::person::DeviceKind::Biometric) => verdict("ok", "biometric: Touch ID, for every signature as you", None),
        Some(xenonet_log::person::DeviceKind::Weaker) => {
            verdict("info", "weaker: Windows Hello, polkit or a passphrase, which a program that can type could pass. XenoNet notes it on every decision this device signs", None)
        }
        None => verdict("info", "none: this machine has no device of yours that can sign, so it does nothing as you", None),
    };
    let root = match identity.root_sealed() {
        Some(true) => verdict("ok", "sealed to this machine's device key: opening it asks you", None),
        Some(false) => verdict(
            "warn",
            "kept in an owner-only file (identity/root.json), since this machine can't seal it to its device key: a program running as you could read it and add devices to your identity",
            Some("Run only programs you trust under this account. A Mac with Touch ID seals the root key to its device key instead."),
        ),
        None => verdict("info", "not on this machine: the machine that set up your identity holds it", None),
    };
    vec![("network.presence", "This machine asks you before it acts as you", presence), ("network.root", "Your identity's root key is kept safe", root)]
}

/// What the doctor's firewall check (xenonet-doctor's firewall.rs) reads of
/// the network: null when this machine isn't in one, else whether it's the
/// network's gateway (as the roster says, or the pinned gateway when the
/// roster can't be read), the gateway's address and its kind
/// (address_kind).
pub fn standing(d: &Dir) -> Value {
    let Ok(Some(net)) = d.read_network() else { return Value::Null };
    let gateway = read_roster(d, &net.id).map_or(net.gateway.host == net.host, |r| r.view().gateway == net.host);
    let address = net.gateway.address.filter(|a| !a.is_empty());
    let kind = address.as_deref().map(crate::files::address_kind);
    json!({ "gateway": gateway, "address": address, "kind": kind })
}

/// The roster, checked against the person chains kept beside it, which
/// the doctor reads without opening any key.
fn read_roster(d: &Dir, network: &str) -> crate::R<crate::roster::Roster> {
    let identity = crate::identity::Identity::open(d.identity_dir(), d.persist.clone(), None)?;
    d.read_roster(network, crate::roster::NetworkPolicy::with(identity))
}

fn files(d: &Dir, network: &str, host: &str) -> Value {
    let roster = match read_roster(d, network) {
        Ok(r) => r,
        Err(e) => return verdict("fail", format!("the roster doesn't check: {}", e.message), Some("xenonet net leave, then join the network again")),
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

    use std::path::Path;

    fn kept(dir: &Path) -> Dir {
        Dir::new(dir, std::sync::Arc::new(crate::persist::Memory::default()))
    }

    #[test]
    fn no_network_no_checks() {
        let d = kept(&std::env::temp_dir().join(format!("xenonet-net-doctor-none-{}", std::process::id())));
        assert_eq!(checks(&d, 0.0).len(), 0);
        assert_eq!(standing(&d), Value::Null);
    }

    #[test]
    fn a_funnel_name_is_no_address_for_a_network() {
        let d = kept(&std::env::temp_dir().join(format!("xenonet-net-doctor-funnel-{}", std::process::id())));
        let pin = |address: &str| crate::files::Pin { host: "studio".into(), key_id: "k".into(), sign: "s".into(), seal: "x".into(), address: Some(address.into()) };
        let net = |host: &str, address: &str| crate::files::NetFile {
            id: "net_0".into(),
            name: "home".into(),
            host: host.into(),
            founder: "studio".into(),
            gateway: pin(address),
            lan: None,
            titles_only: false,
            share_load: true,
            joined: 0,
        };
        let address = |host: &str, a: &str| {
            d.write_network(&net(host, a)).unwrap();
            checks(&d, 0.0).into_iter().find(|c| c.0 == "network.address").unwrap().2
        };
        let here = address("studio", "studio.example-tailnet.ts.net");
        assert_eq!(here["status"], "warn");
        assert!(here["detail"].as_str().unwrap().contains("can't link through Funnel, which publishes only /mcp"), "{here}");
        assert!(here["fix"].as_str().unwrap().starts_with("Use this machine's tailnet address"), "{here}");
        let there = address("laptop", "https://Studio.Example-Tailnet.ts.net/");
        assert!(there["fix"].as_str().unwrap().starts_with("Use studio's tailnet address"), "{there}");
        assert_eq!(address("laptop", "100.64.0.7:48114")["status"], "info", "a tailnet address is a LAN one");
        assert_eq!(address("laptop", "gw.example.com")["status"], "ok");
    }

    #[test]
    fn the_identity_checks_say_how_this_machine_asks_and_keeps_the_root() {
        use std::sync::Arc;
        use xenonet_id::presence::FixedCheck;
        use xenonet_id::sign::{SeedSigner, Signer, VerifyingKey};
        use xenonet_id::vault::{DeviceKey, Vault};
        struct Fake(bool);
        impl Vault for Fake {
            fn machine_signer(&self, dir: &Path) -> xenonet_id::R<Box<dyn Signer>> {
                Ok(Box::new(SeedSigner::load_or_create(&dir.join("machine.key"))?))
            }
            fn device(&self, dir: &Path) -> xenonet_id::R<Option<DeviceKey>> {
                Ok(Some(DeviceKey { signer: Box::new(SeedSigner::load_or_create(&dir.join("device.key"))?), check: Some(Box::new(FixedCheck(true))), biometric: self.0 }))
            }
            fn seal(&self, _to: &VerifyingKey, secret: &[u8]) -> xenonet_id::R<Option<Vec<u8>>> {
                Ok(self.0.then(|| secret.to_vec()))
            }
            fn open(&self, _dir: &Path, sealed: &[u8], _reason: &str) -> xenonet_id::R<Vec<u8>> {
                Ok(sealed.to_vec())
            }
            fn asking(&self, _reason: &str, f: &mut dyn FnMut()) {
                f();
            }
        }
        let checks = |name: &str, biometric: bool| {
            let dir = std::env::temp_dir().join(format!("xenonet-net-doctor-identity-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            let network = kept(&dir.join("network"));
            assert!(identity_checks(&network).is_empty(), "no identity, no checks");
            let id = crate::identity::Identity::open(network.identity_dir(), network.persist.clone(), Some(Arc::new(Fake(biometric)))).unwrap();
            id.found(true, "laptop", 1).unwrap();
            let out: Vec<(String, Value)> = identity_checks(&network).into_iter().map(|(id, _, v)| (id.to_string(), v)).collect();
            let _ = std::fs::remove_dir_all(&dir);
            out
        };
        let mac = checks("mac", true);
        assert_eq!((mac[0].0.as_str(), mac[0].1["status"].clone()), ("network.presence", json!("ok")), "{mac:?}");
        assert!(mac[0].1["detail"].as_str().unwrap().starts_with("biometric"), "{mac:?}");
        assert_eq!((mac[1].0.as_str(), mac[1].1["status"].clone()), ("network.root", json!("ok")), "{mac:?}");
        let linux = checks("linux", false);
        assert!(linux[0].1["detail"].as_str().unwrap().starts_with("weaker"), "{linux:?}");
        assert_eq!(linux[1].1["status"], "warn", "a root in a file is a warning: {linux:?}");
        assert!(linux[1].1["fix"].as_str().is_some_and(|f| !f.is_empty()), "{linux:?}");
    }

    #[test]
    fn spans_and_times() {
        assert_eq!(span(300_000.0), "5 min");
        assert_eq!(span(-3000.0), "3 s");
        assert_eq!(clock_time(0.0), "00:00");
    }
}
