//! Whether the actor gateway's address reaches it from the internet
//! (server/gateway-reach.js), as a cloud agent would call it: the name
//! looked up in the internet's DNS (public resolvers, not this computer's
//! own), then a request to https://<name>/mcp at the address it gave,
//! carrying a one-time word the gateway answers only while XenoNet waits
//! for it. So a yes means this XenoNet's gateway answered, through whatever
//! tunnel is in between, and a no says where it stopped.
//!
//! The public lookup and the HTTPS request need a DNS resolver and TLS,
//! which the Rust service doesn't have yet (a dependency to decide on:
//! hickory-resolver and rustls, say); the check on this computer alone
//! (XENONET_GATEWAY_PROBE=local) is plain HTTP and is here.

use async_trait::async_trait;
use serde_json::{json, Value};
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

/// This computer: the local probe, and no public check yet.
pub struct SystemNetwork;

#[async_trait]
impl Network for SystemNetwork {
    async fn lookup(&self, _host: &str) -> Result<(String, u8), String> {
        Err(NO_PUBLIC_CHECK.into())
    }

    async fn request(&self, host: &str, _address: Option<&str>, _family: u8, nonce: &str, local: Option<u16>) -> Result<(u16, String), String> {
        let Some(port) = local else { return Err(NO_PUBLIC_CHECK.into()) };
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
