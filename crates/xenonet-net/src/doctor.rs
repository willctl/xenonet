//! What the doctor says about the network (docs/design/network.md, "The
//! wire"), from what this machine keeps of it alone: the network, its
//! roster and key, the link (up, or down since when and why), the
//! gateway's key against the pinned one, the clock, the outbox and the
//! queues, a member's feed frozen by a fork, a roster mismatch, being
//! removed, and what the gateway's address means.
//!
//! The link keeps how it stands; these checks read it, so they work
//! whether the link is running or not.
//!
//! The relays are the exception: each one the machine has (every
//! network's list, and its own) is asked (`relays`), the way the endpoint
//! would reach it, so a relay that is down, or whose certificate doesn't
//! check, says so under its own URL.

use crate::endpoint::{added_roots, relay_tls, RELAY_CA};
use crate::files::{Dir, KEY};
use crate::keys::HostKey;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, SignatureScheme};
use serde_json::{json, Map, Value};
use std::io::{Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use xenonet_os::sync::Locked;

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
                Some("What's for other machines goes again when the gateway is back. XenoNet tries again by itself."),
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

    // The members' feeds.
    out.push(("network.feeds", "Every member's feed is taken", feeds(d, &net.id)));

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
    if let Some(ca) = relay_ca(d) {
        out.push(("network.relay-ca", "The CAs you added for your own relays are in order", ca));
    }
    out
}

/// What the CAs the person added for their relays (`relay-ca.pem`) come
/// to: none when there's no such file.
fn relay_ca(d: &Dir) -> Option<Value> {
    if !d.file(RELAY_CA).exists() {
        return None;
    }
    let fix = format!("Put your relay's CA certificate, as PEM, in {RELAY_CA} in XenoNet's state folder, network/, then xenonet service restart");
    Some(match added_roots(&d.path) {
        Ok(roots) => verdict(
            "ok",
            format!("{} CA{} in {RELAY_CA}, trusted for relays beside the public CAs, on this machine alone", roots.len(), if roots.len() == 1 { "" } else { "s" }),
            None,
        ),
        Err(e) => verdict("fail", format!("{}: relays with a certificate from your own CA can't be reached", e.message), Some(fix.as_str())),
    })
}

/// How long the doctor gives a relay to answer: to connect, and then to
/// shake hands and answer, each.
const RELAY_WAIT: Duration = Duration::from_secs(3);

/// A relay this machine has, for the doctor to ask: its URL, who lists
/// it, and the CAs the person added.
#[derive(Clone, Debug)]
pub struct RelayProbe {
    pub url: String,
    /// The networks that list it, by name, and "this machine" for one of
    /// its own.
    pub by: Vec<String>,
    roots: Vec<CertificateDer<'static>>,
}

/// The relays this machine has: every network's, and its own, each once,
/// in the order they're listed.
pub fn relays(d: &Dir) -> Vec<RelayProbe> {
    let mut out: Vec<RelayProbe> = Vec::new();
    let mut add = |url: &str, by: &str| match out.iter_mut().find(|p| p.url == url) {
        Some(p) => p.by.push(by.to_string()),
        None => out.push(RelayProbe { url: url.to_string(), by: vec![by.to_string()], roots: Vec::new() }),
    };
    for net in d.read_networks().unwrap_or_default() {
        let Ok(roster) = read_roster(d, &net.id) else { continue };
        for url in &roster.view().relays {
            add(url, &net.name);
        }
    }
    for url in d.persist.own_relays().unwrap_or_default() {
        add(&url, "this machine");
    }
    let roots = added_roots(&d.path).unwrap_or_default();
    for p in &mut out {
        p.roots.clone_from(&roots);
    }
    out
}

impl RelayProbe {
    pub fn new(url: impl Into<String>, by: Vec<String>, roots: Vec<CertificateDer<'static>>) -> RelayProbe {
        RelayProbe { url: url.into(), by, roots }
    }

    /// The check's id: `network.relay.` and the relay's host (and port,
    /// when it isn't the scheme's).
    pub fn id(&self) -> String {
        let at = self.url.parse::<iroh::RelayUrl>().ok().map_or_else(
            || self.url.clone(),
            |u| match (u.host_str(), u.port()) {
                (Some(h), Some(p)) => format!("{h}:{p}"),
                (Some(h), None) => h.to_string(),
                _ => self.url.clone(),
            },
        );
        format!("network.relay.{at}")
    }

    pub fn title(&self) -> String {
        format!("The relay {} answers", self.url.trim_end_matches('/'))
    }

    /// Asks the relay as the endpoint would: TCP, then TLS checked against
    /// the public CAs and the person's own, then iroh-relay's /healthz.
    pub async fn check(self) -> Value {
        tokio::task::spawn_blocking(move || self.ask()).await.unwrap_or_else(|e| verdict("warn", format!("the check failed: {e}"), None))
    }

    fn listed(&self) -> String {
        let (own, nets): (Vec<&String>, Vec<&String>) = self.by.iter().partition(|b| *b == "this machine");
        let mut words = Vec::new();
        if !nets.is_empty() {
            words.push(format!("listed by {}", nets.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }
        if !own.is_empty() {
            words.push("one of this machine's own".to_string());
        }
        words.join("; ")
    }

    fn ask(&self) -> Value {
        let name = self.url.trim_end_matches('/');
        let down_fix = "Check that the relay runs and its ports are open (docs/guide/own-relay.md). Meanwhile XenoNet goes through the network's other relays, or direct paths.";
        let Ok(url) = self.url.parse::<iroh::RelayUrl>() else {
            return verdict("fail", format!("{name} isn't a relay's URL"), Some("xenonet net relay remove <url>, then add it again as https://<host>"));
        };
        let host = url.host_str().unwrap_or_default().trim_start_matches('[').trim_end_matches(']').to_string();
        let tls = url.scheme() == "https";
        let port = url.port_or_known_default().unwrap_or(443);
        let Ok(Some(at)) = (host.as_str(), port).to_socket_addrs().map(|mut a| a.next()) else {
            return verdict("warn", format!("{name} doesn't resolve: no address for {host}"), Some(down_fix));
        };
        let t0 = Instant::now();
        let tcp = match TcpStream::connect_timeout(&at, RELAY_WAIT) {
            Ok(s) => s,
            Err(e) => return verdict("warn", format!("{name} doesn't answer at {at}: {e}"), Some(down_fix)),
        };
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let _ = tcp.set_read_timeout(Some(RELAY_WAIT));
        let _ = tcp.set_write_timeout(Some(RELAY_WAIT));
        let ask = format!("GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        let (answer, whose) = if tls {
            match https(tcp, &host, &self.roots, ask.as_bytes()) {
                Ok(a) => a,
                Err(Asked::Cert(e)) => {
                    let fix = format!("If it's your own relay with your own CA, put that CA's certificate in {RELAY_CA} in XenoNet's state folder, network/, then xenonet service restart. Otherwise the relay needs a valid certificate (ACME, say).");
                    return verdict("fail", format!("{name}'s certificate doesn't check: {e}"), Some(fix.as_str()));
                }
                Err(Asked::Other(e)) => return verdict("warn", format!("{name} took the connection but didn't answer over TLS: {e}"), Some(down_fix)),
            }
        } else {
            match plain(tcp, ask.as_bytes()) {
                Ok(a) => (a, ""),
                Err(e) => return verdict("warn", format!("{name} took the connection but didn't answer: {e}"), Some(down_fix)),
            }
        };
        let version = healthz(&answer);
        let mut words = vec![self.listed(), format!("{} ms away", ms.round())];
        words.push(match whose {
            "public" => "TLS valid, a public CA's certificate".to_string(),
            "added" => format!("TLS valid, a certificate from a CA you added ({RELAY_CA})"),
            _ => "plain HTTP".to_string(),
        });
        match &version {
            Some(v) => words.push(format!("iroh-relay {v}")),
            None => words.push("not answering /healthz as iroh-relay does".to_string()),
        }
        let detail = words.into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join("; ");
        if !tls {
            return verdict(
                "warn",
                detail,
                Some("Serve the relay over HTTPS: whoever is on the path to a plain HTTP relay sees which machines use it, though never what they send."),
            );
        }
        if version.is_none() {
            return verdict("warn", detail, Some("Check that the URL is the relay's own (https://<host>, no path)."));
        }
        verdict("ok", detail, None)
    }
}

enum Asked {
    Cert(String),
    Other(String),
}

/// The relay's answer over TLS, and whose CA vouched for it: "public" or
/// "added".
fn https(tcp: TcpStream, host: &str, roots: &[CertificateDer<'static>], ask: &[u8]) -> Result<(Vec<u8>, &'static str), Asked> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let other = |e: std::io::Error| Asked::Other(e.to_string());
    let public = relay_tls(&[]).server_cert_verifier(provider.clone()).map_err(other)?;
    let added = if roots.is_empty() { None } else { Some(iroh::tls::CaTlsConfig::custom_roots(roots.to_vec()).server_cert_verifier(provider.clone()).map_err(other)?) };
    let which = Arc::new(Which { public, added, by: Mutex::new(None) });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Asked::Other(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(which.clone())
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_string()).map_err(|e| Asked::Other(e.to_string()))?;
    let conn = rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| Asked::Other(e.to_string()))?;
    let mut tls = rustls::StreamOwned::new(conn, tcp);
    let sent = tls.write_all(ask).and_then(|()| tls.flush());
    let mut answer = Vec::new();
    let read = sent.and_then(|()| tls.read_to_end(&mut answer));
    let failed = |e: std::io::Error| match e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
        Some(rustls::Error::InvalidCertificate(c)) => Asked::Cert(cert_words(c)),
        _ => Asked::Other(e.to_string()),
    };
    // A server that closes without TLS's close_notify still answered.
    match read {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !answer.is_empty() => {}
        Err(e) => return Err(failed(e)),
    }
    let by = *which.by.locked();
    Ok((answer, by.unwrap_or("public")))
}

fn plain(mut tcp: TcpStream, ask: &[u8]) -> std::io::Result<Vec<u8>> {
    tcp.write_all(ask)?;
    let mut answer = Vec::new();
    tcp.read_to_end(&mut answer)?;
    Ok(answer)
}

fn cert_words(c: &CertificateError) -> String {
    match c {
        CertificateError::UnknownIssuer => "no CA this machine trusts issued it".into(),
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => "it has expired".into(),
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => "it isn't valid yet (or this computer's clock is behind)".into(),
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => "it's for another name than the relay's".into(),
        CertificateError::Revoked => "it was revoked".into(),
        other => format!("{other:?}"),
    }
}

/// iroh-relay's version, from its /healthz answer, when it gave one.
fn healthz(answer: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(answer);
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") && !head.starts_with("HTTP/1.0 200") {
        return None;
    }
    let v: Value = serde_json::from_str(body.trim()).ok()?;
    (v.get("status").and_then(Value::as_str) == Some("ok")).then(|| v.get("version").and_then(Value::as_str).unwrap_or("?").to_string())
}

/// Checks a relay's certificate against the public CAs, then the person's
/// own, and keeps which one vouched for it.
#[derive(Debug)]
struct Which {
    public: Arc<dyn ServerCertVerifier>,
    added: Option<Arc<dyn ServerCertVerifier>>,
    by: Mutex<Option<&'static str>>,
}

impl ServerCertVerifier for Which {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        middle: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let public = match self.public.verify_server_cert(end, middle, name, ocsp, now) {
            Ok(v) => {
                *self.by.locked() = Some("public");
                return Ok(v);
            }
            Err(e) => e,
        };
        let Some(added) = &self.added else { return Err(public) };
        match added.verify_server_cert(end, middle, name, ocsp, now) {
            Ok(v) => {
                *self.by.locked() = Some("added");
                Ok(v)
            }
            // What the person's CA says is the more telling, unless it
            // didn't issue the certificate either.
            Err(e) if matches!(public, rustls::Error::InvalidCertificate(CertificateError::UnknownIssuer)) => Err(e),
            Err(_) => Err(public),
        }
    }

    fn verify_tls12_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.public.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.public.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.public.supported_verify_schemes()
    }
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
/// Whether a fork froze a member's feed here: it signed two different
/// entries at one place, so nothing more from it is taken until the person
/// unfreezes it at the window.
fn feeds(d: &Dir, network: &str) -> Value {
    let frozen = d.persist.frozen_feeds(network).unwrap_or_default();
    if frozen.is_empty() {
        return verdict("ok", "no member signed two entries at one place in its feed", None);
    }
    let roster = read_roster(d, network).ok();
    let names: Vec<String> = frozen
        .iter()
        .map(|(author, fork)| {
            let host = roster.as_ref().and_then(|r| r.view().members.values().find(|m| m.machine.id() == *author).map(|m| m.id.clone()));
            format!("{} (at {}.{})", host.unwrap_or_else(|| author.clone()), fork.at.inc, fork.at.seq)
        })
        .collect();
    verdict(
        "fail",
        format!("{} signed two different entries at one place in its feed, so nothing more from it is taken here", names.join(", ")),
        Some("If you trust that machine again (it was restored from a backup, say), unfreeze its feed in XenoNet's window: Network, then Unfreeze beside it. Otherwise remove it from the network."),
    )
}

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
        Dir::new(dir, Arc::new(crate::persist::Memory::default()))
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
