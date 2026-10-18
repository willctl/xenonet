//! Whether the actor gateway's address reaches it from the internet
//! (server/gateway-reach.js), as a cloud agent would call it: the name
//! looked up in the internet's DNS (public resolvers, not this computer's
//! own), then a request to https://<name>/mcp at the address it gave,
//! carrying a one-time word the gateway answers only while XenoNet waits
//! for it. So a yes means this XenoNet's gateway answered, through whatever
//! tunnel is in between, and a no says where it stopped.
//!
//! The public lookup asks 1.1.1.1, 8.8.8.8 and 9.9.9.9 over UDP itself (a
//! question for A, then AAAA, as Node's Resolver asks), and the probe goes
//! over the service's HTTPS client, pinned to the address DNS gave, with
//! the certificate checked for the name. The check on this computer alone
//! (XENONET_GATEWAY_PROBE=local) is plain HTTP.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

pub use super::actor_gateway::PROBE_HEADER;

/// What the check asks of the network: an address for a name, and one
/// probe of /mcp. Errors are codes (missing, ECONNREFUSED, a TLS one…).
#[async_trait]
pub trait Network: Send + Sync {
    async fn lookup(&self, host: &str) -> Result<(String, u8), String>;
    async fn request(&self, host: &str, address: Option<&str>, family: u8, nonce: &str, local: Option<u16>) -> Result<(u16, String), String>;
}

pub const NO_PUBLIC_CHECK: &str = "no-public-check";

const RESOLVERS: [&str; 3] = ["1.1.1.1", "8.8.8.8", "9.9.9.9"];

/// A DNS question for `host` of `kind` (1: A, 28: AAAA).
fn question(id: u16, host: &str, kind: u16) -> Vec<u8> {
    let mut q = Vec::with_capacity(32 + host.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    for label in host.trim_end_matches('.').split('.') {
        q.push(label.len().min(63) as u8);
        q.extend_from_slice(&label.as_bytes()[..label.len().min(63)]);
    }
    q.push(0);
    q.extend_from_slice(&kind.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    q
}

// A name in a DNS message, skipped: labels, or a pointer.
fn skip_name(m: &[u8], mut i: usize) -> Option<usize> {
    loop {
        let len = *m.get(i)? as usize;
        if len == 0 {
            return Some(i + 1);
        }
        if len & 0xc0 == 0xc0 {
            return Some(i + 2);
        }
        i += 1 + len;
    }
}

/// The first address of `kind` in an answer, or the code Node's resolver
/// says for none (ENODATA, ENOTFOUND, ESERVFAIL, EREFUSED…).
fn answer(m: &[u8], id: u16, kind: u16) -> Result<String, String> {
    if m.len() < 12 || u16::from_be_bytes([m[0], m[1]]) != id {
        return Err("EBADRESP".into());
    }
    match m[3] & 0x0f {
        0 => {}
        2 => return Err("ESERVFAIL".into()),
        3 => return Err("ENOTFOUND".into()),
        5 => return Err("EREFUSED".into()),
        _ => return Err("EBADRESP".into()),
    }
    let (qd, an) = (u16::from_be_bytes([m[4], m[5]]), u16::from_be_bytes([m[6], m[7]]));
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(m, i).ok_or("EBADRESP")? + 4;
    }
    for _ in 0..an {
        i = skip_name(m, i).ok_or("EBADRESP")?;
        let head = m.get(i..i + 10).ok_or("EBADRESP")?;
        let (t, len) = (u16::from_be_bytes([head[0], head[1]]), u16::from_be_bytes([head[8], head[9]]) as usize);
        let data = m.get(i + 10..i + 10 + len).ok_or("EBADRESP")?;
        if t == kind && kind == 1 && len == 4 {
            return Ok(std::net::Ipv4Addr::new(data[0], data[1], data[2], data[3]).to_string());
        }
        if t == kind && kind == 28 && len == 16 {
            let mut b = [0u8; 16];
            b.copy_from_slice(data);
            return Ok(std::net::Ipv6Addr::from(b).to_string());
        }
        i += 10 + len;
    }
    Err("ENODATA".into())
}

/// One question to the public resolvers, each tried twice within its time.
async fn resolve(host: &str, kind: u16) -> Result<String, String> {
    let mut last = "ETIMEOUT".to_string();
    for server in RESOLVERS {
        for _ in 0..2 {
            let mut idb = [0u8; 2];
            let _ = getrandom::fill(&mut idb);
            let id = u16::from_be_bytes(idb);
            let ask = async {
                let sock = tokio::net::UdpSocket::bind(if server.contains(':') { "[::]:0" } else { "0.0.0.0:0" }).await.map_err(|e| e.to_string())?;
                sock.send_to(&question(id, host, kind), (server, 53)).await.map_err(|_| "ECONNREFUSED".to_string())?;
                let mut buf = [0u8; 1500];
                let n = sock.recv(&mut buf).await.map_err(|_| "ECONNREFUSED".to_string())?;
                answer(&buf[..n], id, kind)
            };
            match tokio::time::timeout(Duration::from_secs(3), ask).await {
                Err(_) => last = "ETIMEOUT".into(),
                Ok(Ok(a)) => return Ok(a),
                // An answer, even "there's none", is the resolver's word.
                Ok(Err(e)) if ["ENODATA", "ENOTFOUND", "ESERVFAIL", "EREFUSED"].contains(&e.as_str()) => return Err(e),
                Ok(Err(e)) => last = e,
            }
        }
    }
    Err(last)
}

/// A TLS error's words, as Node's codes for them would say it.
fn https_code(e: &reqwest::Error) -> String {
    let mut words = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        let _ = write!(words, ": {s}");
        src = s.source();
    }
    let lower = words.to_lowercase();
    if e.is_timeout() {
        "ETIMEDOUT".into()
    } else if lower.contains("certificate") || lower.contains("tls") || lower.contains("handshake") {
        if lower.contains("name") || lower.contains("notvalidforname") {
            "ERR_TLS_CERT_ALTNAME_INVALID".into()
        } else {
            "CERT_INVALID".into()
        }
    } else if lower.contains("refused") {
        "ECONNREFUSED".into()
    } else {
        // A reset, or anything else the connection did.
        "ECONNRESET".into()
    }
}

/// This computer: the internet's DNS, and the probe through the service's
/// HTTPS client, or over plain HTTP to the gateway's own port.
pub struct SystemNetwork;

#[async_trait]
impl Network for SystemNetwork {
    async fn lookup(&self, host: &str) -> Result<(String, u8), String> {
        let mut failed: Option<String> = None;
        for (kind, family) in [(1u16, 4u8), (28, 6)] {
            match resolve(host, kind).await {
                Ok(a) => return Ok((a, family)),
                Err(e) if e == "ENODATA" || e == "ENOTFOUND" => {}
                Err(e) => failed = Some(e),
            }
        }
        Err(failed.unwrap_or_else(|| "missing".into()))
    }

    async fn request(&self, host: &str, address: Option<&str>, _family: u8, nonce: &str, local: Option<u16>) -> Result<(u16, String), String> {
        let Some(port) = local else {
            let Some(addr) = address.and_then(|a| a.parse::<std::net::IpAddr>().ok()) else { return Err("ENOTFOUND".into()) };
            let _ = crate::net::client();
            let client = reqwest::Client::builder()
                .resolve(host, std::net::SocketAddr::new(addr, 443))
                .timeout(Duration::from_secs(8))
                .user_agent("xenonet-gateway-check")
                .build()
                .map_err(|e| e.to_string())?;
            let res = client.get(format!("https://{host}/mcp")).header(PROBE_HEADER, nonce).header("accept", "application/json").send().await.map_err(|e| https_code(&e))?;
            let echoed = res.headers().get(PROBE_HEADER).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned()).unwrap_or_default();
            return Ok((res.status().as_u16(), echoed));
        };
        let go = async {
            let mut s = TcpStream::connect(("127.0.0.1", port)).await.map_err(|e| {
                if e.kind() == std::io::ErrorKind::ConnectionRefused {
                    "ECONNREFUSED".to_string()
                } else {
                    e.to_string()
                }
            })?;
            let head = format!(
                "GET /mcp HTTP/1.1\r\nhost: {host}\r\n{PROBE_HEADER}: {nonce}\r\nuser-agent: xenonet-gateway-check\r\naccept: application/json\r\nconnection: close\r\n\r\n"
            );
            s.write_all(head.as_bytes()).await.map_err(|e| e.to_string())?;
            let mut r = BufReader::new(s);
            let mut line = String::new();
            r.read_line(&mut line).await.map_err(|e| e.to_string())?;
            let status = line.split(' ').nth(1).and_then(|c| c.parse::<u16>().ok()).ok_or("ECONNRESET")?;
            let mut echoed = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).await.unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim().eq_ignore_ascii_case(PROBE_HEADER) {
                        echoed = v.trim().to_string();
                    }
                }
            }
            Ok((status, echoed))
        };
        tokio::time::timeout(Duration::from_secs(8), go).await.unwrap_or_else(|_| Err("ETIMEDOUT".into()))
    }
}

fn tls(code: &str) -> bool {
    ["CERT", "SSL", "TLS", "SELF_SIGNED", "ALTNAME"].iter().any(|w| code.contains(w))
}

/// The check, in words: `{ ok, stage, why }`. `expect(nonce)` tells the
/// gateway which word to answer; `local` is the gateway's port, to check the
/// loop on this computer alone.
pub async fn check_reach(host: Option<&str>, nonce: &str, expect: impl FnOnce(&str), local: Option<u16>, net: &dyn Network) -> Value {
    let no = |stage: &str, why: String| json!({ "ok": false, "stage": stage, "why": why });
    let Some(host) = host.filter(|h| !h.is_empty()) else { return no("address", "The gateway has no address yet.".into()) };
    let (mut address, mut family) = (None, 4u8);
    if local.is_none() {
        match net.lookup(host).await {
            Ok((a, f)) => (address, family) = (Some(a), f),
            Err(e) if e == "missing" => return no("dns", format!("{host} isn't in public DNS yet. A new tunnel's name can take a few minutes.")),
            Err(e) if e == NO_PUBLIC_CHECK => {
                return no("dns", "This XenoNet can't look its address up in public DNS yet: the Rust service's check of a public address is still to come.".into())
            }
            Err(e) => return no("dns", format!("Public DNS didn't answer ({e}). Is this computer online?")),
        }
    }
    expect(nonce);
    match net.request(host, address.as_deref(), family, nonce, local).await {
        Err(e) if tls(&e) => no("tls", format!("{host} answered with a certificate for another name ({e}).")),
        Err(e) => no("connect", format!("Nothing answered at {host} ({e}). Is the tunnel running?")),
        Ok((204, echoed)) if echoed == nonce => json!({ "ok": true, "stage": "ok", "why": null }),
        Ok((421, _)) => no("host", "The tunnel changes the Host header, which the gateway refuses. Have it pass the header on unchanged.".into()),
        Ok((s @ (502..=504), _)) => no("answer", format!("The tunnel answers, but can't reach the gateway on this computer (HTTP {s}).")),
        Ok((s, _)) => no("answer", format!("Something else answers at {host} (HTTP {s}).")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases of test/gateway-tunnel.test.js's check, in words.
    struct Says(Result<(String, u8), String>, Result<(u16, String), String>);

    #[async_trait]
    impl Network for Says {
        async fn lookup(&self, _h: &str) -> Result<(String, u8), String> {
            self.0.clone()
        }
        async fn request(&self, _h: &str, _a: Option<&str>, _f: u8, _n: &str, _l: Option<u16>) -> Result<(u16, String), String> {
            self.1.clone()
        }
    }

    #[test]
    fn dns_questions_and_answers() {
        let q = question(0x1234, "mac.example.ts.net", 1);
        assert_eq!(&q[..4], &[0x12, 0x34, 0x01, 0x00]);
        assert_eq!(&q[12..16], &[3, b'm', b'a', b'c']);
        // An answer with the question and one A record, by a pointer.
        let mut a = vec![0x12, 0x34, 0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0];
        a.extend_from_slice(&q[12..]);
        a.extend_from_slice(&[0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 203, 0, 113, 7]);
        assert_eq!(answer(&a, 0x1234, 1), Ok("203.0.113.7".into()));
        assert_eq!(answer(&a, 0x1234, 28), Err("ENODATA".into()));
        a[3] = 0x83;
        assert_eq!(answer(&a, 0x1234, 1), Err("ENOTFOUND".into()));
        assert_eq!(answer(&a, 0x9999, 1), Err("EBADRESP".into()));
    }

    #[tokio::test]
    async fn the_check_in_words() {
        let at = || Ok(("203.0.113.7".to_string(), 4));
        let check = |n: Says| async move { check_reach(Some("x.example.com"), "w", |_| {}, None, &n).await };
        assert_eq!(check(Says(Err("missing".into()), Ok((204, "w".into())))).await["stage"], "dns");
        assert!(check(Says(Err("ETIMEOUT".into()), Ok((204, "w".into())))).await["why"].as_str().unwrap().contains("Public DNS didn't answer (ETIMEOUT)"));
        assert_eq!(check(Says(at(), Err("ECONNREFUSED".into()))).await["stage"], "connect");
        assert_eq!(check(Says(at(), Err("ERR_TLS_CERT_ALTNAME_INVALID".into()))).await["stage"], "tls");
        assert_eq!(check(Says(at(), Ok((421, String::new())))).await["stage"], "host");
        assert!(check(Says(at(), Ok((502, String::new())))).await["why"].as_str().unwrap().contains("can't reach the gateway"));
        assert!(check(Says(at(), Ok((200, String::new())))).await["why"].as_str().unwrap().contains("Something else answers"));
        assert_eq!(check(Says(at(), Ok((204, "w".into())))).await, json!({ "ok": true, "stage": "ok", "why": null }));
        assert_eq!(check_reach(None, "w", |_| {}, None, &Says(at(), Ok((204, "w".into())))).await["stage"], "address");
    }
}

#[cfg(test)]
mod online {
    use super::*;

    #[tokio::test]
    #[ignore = "it asks the internet: run with --ignored on a computer that's online"]
    async fn a_public_name_through_public_dns_and_https() {
        let (addr, family) = SystemNetwork.lookup("example.com").await.unwrap();
        assert!(addr.parse::<std::net::IpAddr>().is_ok() && (family == 4 || family == 6));
        assert_eq!(SystemNetwork.lookup("no-such-name.invalid").await, Err("missing".into()));
        let (status, echoed) = SystemNetwork.request("example.com", Some(&addr), family, "w", None).await.unwrap();
        assert!(status >= 200 && echoed.is_empty(), "{status}");
    }
}
