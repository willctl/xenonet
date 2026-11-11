//! Where XenoNet may connect when it calls an outside agent (revision 26):
//! https only; a name resolved by XenoNet's own resolver, so the addresses
//! it checks are the ones it connects to; no loopback, private, link-local
//! or otherwise non-public address unless the person allowed that one host
//! at the window; no system proxy, which would resolve the name itself;
//! no redirect to another host.

use crate::A2aError;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use url::{Host, Url};

/// How long a connection may take to open.
pub const CONNECT: Duration = Duration::from_secs(10);

/// Whether `ip` is an address anyone on the internet could reach: not
/// loopback, private, link-local, shared (CGNAT, which Tailscale uses),
/// unspecified, broadcast, multicast, documentation, benchmarking or
/// reserved. An IPv6 address that carries an IPv4 one (mapped, NAT64) is
/// judged by that one.
pub fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v6.segments();
            // NAT64's well-known prefix, 64:ff9b::/96.
            if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
                return public_v4(Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8));
            }
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local, fc00::/7.
                || (s[0] & 0xfe00) == 0xfc00
                // Link-local, fe80::/10.
                || (s[0] & 0xffc0) == 0xfe80
                // Site-local (deprecated), fec0::/10.
                || (s[0] & 0xffc0) == 0xfec0
                // Documentation, 2001:db8::/32.
                || (s[0] == 0x2001 && s[1] == 0x0db8)
                // IPv4-compatible (::a.b.c.d, deprecated) and the rest of ::/8.
                || s[0] == 0)
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || a == 0
        // Shared address space, 100.64.0.0/10 (carrier NAT, Tailscale).
        || (a == 100 && (b & 0xc0) == 64)
        // IETF protocol assignments, 192.0.0.0/24.
        || (a == 192 && b == 0 && c == 0)
        // Benchmarking, 198.18.0.0/15.
        || (a == 198 && (b & 0xfe) == 18)
        // Reserved, 240.0.0.0/4.
        || a >= 240)
}

/// The host of an https URL, lowercased, with its port when it isn't 443:
/// what a card's and an interface's hosts are compared by, and what the
/// person allows.
pub fn host_of(url: &Url) -> String {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    match url.port() {
        Some(p) if p != 443 => format!("{host}:{p}"),
        _ => host,
    }
}

/// `text` as an address XenoNet may call: https, a host, no user or
/// password in it.
pub fn https(text: &str) -> Result<Url, A2aError> {
    let url = Url::parse(text.trim()).map_err(|e| A2aError::new("address", format!("that isn't an address: {e}")))?;
    match url.scheme() {
        "https" => {}
        "http" => return Err(A2aError::new("http", "XenoNet calls an agent over https only, and this address is plain http")),
        other => return Err(A2aError::new("address", format!("XenoNet calls an agent over https, not {other}"))),
    }
    if url.host().is_none() {
        return Err(A2aError::new("address", "that address names no host"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(A2aError::new("address", "an agent's address carries no user name or password; its token goes in XenoNet's secure field"));
    }
    Ok(url)
}

/// Where the person let XenoNet reach a host that isn't public: the one
/// host (as [`host_of`] writes it) they allowed at the window, or none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Private {
    pub host: Option<String>,
}

impl Private {
    pub fn none() -> Private {
        Private { host: None }
    }

    pub fn allows(&self, url: &Url) -> bool {
        self.host.as_deref() == Some(host_of(url).as_str())
    }

    /// What a client that calls `url` alone may reach: the allowance when
    /// it's for that very host and port, else none. The resolver is given
    /// a name without its port, so each client is made for one host.
    #[must_use]
    pub fn for_url(&self, url: &Url) -> Private {
        if self.allows(url) {
            self.clone()
        } else {
            Private::none()
        }
    }

    /// The name alone, as the resolver sees it (it's given no port).
    fn allows_name(&self, name: &str) -> bool {
        self.host.as_deref().is_some_and(|h| h.split(':').next() == Some(name.to_ascii_lowercase().as_str()))
    }
}

/// Checks `url`'s host before a call: an address written as an IP is
/// judged here, since the resolver never sees one.
pub fn check(url: &Url, private: &Private) -> Result<(), A2aError> {
    let ip = match url.host() {
        Some(Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(Host::Ipv6(v6)) => IpAddr::V6(v6),
        Some(Host::Domain(_)) => return Ok(()),
        None => return Err(A2aError::new("address", "that address names no host")),
    };
    if public(ip) || private.allows(url) {
        Ok(())
    } else {
        Err(refused(&host_of(url)))
    }
}

fn refused(host: &str) -> A2aError {
    A2aError::new("private", format!("{host} is a loopback, private or link-local address; XenoNet calls one only when you allow that host for this agent in its window"))
}

/// The resolver: the system's lookup, then every address checked. A name
/// with any address that isn't public is refused whole, unless it's the
/// host the person allowed.
#[derive(Clone, Debug)]
struct Checked {
    private: Private,
}

impl Resolve for Checked {
    fn resolve(&self, name: Name) -> Resolving {
        let private = self.private.clone();
        Box::pin(async move {
            let host = name.as_str().to_string();
            let found: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if found.is_empty() {
                return Err(format!("{host} has no address").into());
            }
            if !private.allows_name(&host) && found.iter().any(|a| !public(a.ip())) {
                return Err(Box::new(refused(&host)) as Box<dyn std::error::Error + Send + Sync>);
            }
            let addrs: Addrs = Box::new(found.into_iter());
            Ok(addrs)
        })
    }
}

/// A client that holds to the rules above. `roots`, when given, are the
/// only certificates it trusts (the tests' own); otherwise it trusts the
/// system's, as the service's client does. Redirects are followed only
/// within the same https host, three at most.
pub fn client(private: &Private, roots: Option<&[reqwest::Certificate]>, redirects: bool) -> Result<reqwest::Client, A2aError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let policy = if redirects {
        reqwest::redirect::Policy::custom(|attempt| {
            let first = attempt.previous().first().map(host_of);
            let next = attempt.url();
            if attempt.previous().len() > 3 {
                attempt.error(A2aError::new("redirect", "the card's address redirects more than three times"))
            } else if next.scheme() != "https" || first.as_deref() != Some(host_of(next).as_str()) {
                let to = host_of(next);
                attempt.error(A2aError::new("redirect", format!("the card's address redirects to {to}, another host or plain http, which XenoNet doesn't follow")))
            } else {
                attempt.follow()
            }
        })
    } else {
        reqwest::redirect::Policy::none()
    };
    let mut b = reqwest::Client::builder()
        .user_agent(concat!("xenonet/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .no_proxy()
        .redirect(policy)
        .connect_timeout(CONNECT)
        .dns_resolver(Arc::new(Checked { private: private.clone() }));
    if let Some(roots) = roots {
        b = b.tls_certs_only(roots.iter().cloned());
    }
    b.build().map_err(|e| A2aError::new("transport", format!("XenoNet couldn't make its HTTPS client: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn only_public_addresses() {
        for s in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111", "151.101.1.69"] {
            assert!(public(ip(s)), "{s}");
        }
        for s in [
            "127.0.0.1",
            "127.8.9.1",
            "10.0.0.1",
            "172.16.4.4",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "100.100.1.1",
            "255.255.255.255",
            "224.0.0.1",
            "198.18.0.1",
            "192.0.0.8",
            "240.0.0.1",
            "::1",
            "::",
            "fd7a:115c:a1e0::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "::ffff:10.1.1.1",
            "64:ff9b::7f00:1",
            "2001:db8::1",
            "::127.0.0.1",
        ] {
            assert!(!public(ip(s)), "{s}");
        }
        assert!(public(ip("64:ff9b::808:808")), "NAT64 of a public address");
    }

    #[test]
    fn https_only_without_credentials() {
        assert!(https("https://agent.example.com/.well-known/agent-card.json").is_ok());
        assert_eq!(https("http://agent.example.com/").unwrap_err().code, "http");
        assert_eq!(https("ftp://agent.example.com/").unwrap_err().code, "address");
        assert_eq!(https("https://me:pw@agent.example.com/").unwrap_err().code, "address");
        assert_eq!(https("not a url").unwrap_err().code, "address");
    }

    #[test]
    fn an_address_by_ip_is_checked_before_the_call() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(check(&url("https://127.0.0.1/a2a"), &Private::none()).unwrap_err().code, "private");
        assert_eq!(check(&url("https://[::1]:8443/a2a"), &Private::none()).unwrap_err().code, "private");
        assert_eq!(check(&url("https://169.254.169.254/latest"), &Private::none()).unwrap_err().code, "private");
        assert!(check(&url("https://8.8.8.8/a2a"), &Private::none()).is_ok());
        let allowed = Private { host: Some("127.0.0.1:8443".into()) };
        assert!(check(&url("https://127.0.0.1:8443/a2a"), &allowed).is_ok());
        assert!(check(&url("https://127.0.0.1:9000/a2a"), &allowed).is_err(), "another port is another host");
        assert!(check(&url("https://agent.example.com/a2a"), &Private::none()).is_ok(), "a name is the resolver's to check");
    }

    #[test]
    fn hosts_compare_with_their_ports() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert_eq!(host_of(&url("https://Agent.Example.com/x")), "agent.example.com");
        assert_eq!(host_of(&url("https://agent.example.com:443/x")), "agent.example.com");
        assert_eq!(host_of(&url("https://agent.example.com:8443/x")), "agent.example.com:8443");
    }
}
