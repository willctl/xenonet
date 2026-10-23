//! A host's requests to the gateway (docs/design/network.md, section 2),
//! signed by its key over the method and target, a time, a nonce, its host
//! id and the body's SHA-256. The gateway keeps nonces for 2 minutes and
//! refuses a time more than 60 s off with its own time, so a host whose
//! clock drifted signs again with that offset.

use crate::codec::{b64, hash_b64, unb64};
use crate::keys::{HostKey, PublicKeys};

pub const HOST: &str = "x-xenonet-net-host";
pub const TIME: &str = "x-xenonet-net-time";
pub const NONCE: &str = "x-xenonet-net-nonce";
pub const SIG: &str = "x-xenonet-net-sig";

pub const SKEW_MS: f64 = 60e3;
pub const NONCE_MS: f64 = 120e3;

/// What a request's signature covers.
pub fn signing_string(network: &str, method: &str, target: &str, time: i64, nonce: &str, host: &str, body: &[u8]) -> String {
    format!("xenonet-req/1\n{network}\n{method}\n{target}\n{time}\n{nonce}\n{host}\n{}", hash_b64(body))
}

/// The headers that sign a request.
#[expect(clippy::too_many_arguments, reason = "each is a line of what's signed, named where it's called")]
pub fn sign(key: &HostKey, network: &str, host: &str, method: &str, target: &str, time: i64, nonce: &str, body: &[u8]) -> Vec<(String, String)> {
    let sig = key.sign(signing_string(network, method, target, time, nonce, host, body).as_bytes());
    vec![(HOST.into(), host.into()), (TIME.into(), time.to_string()), (NONCE.into(), nonce.into()), (SIG.into(), b64(&sig))]
}

/// A request's signature, as its headers say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    pub host: String,
    pub time: i64,
    pub nonce: String,
    pub sig: Vec<u8>,
}

impl Signed {
    /// From the headers, or None when one is missing or malformed.
    pub fn from_headers(get: impl Fn(&str) -> Option<String>) -> Option<Signed> {
        let host = get(HOST).filter(|h| xenonet_base::ids::is_host_id(h))?;
        let time = get(TIME)?.parse::<i64>().ok()?;
        let nonce = get(NONCE).filter(|n| (8..=64).contains(&n.len()) && crate::codec::is_b64(n))?;
        let sig = unb64(&get(SIG)?).filter(|s| s.len() == 64)?;
        Some(Signed { host, time, nonce, sig })
    }

    pub fn verify(&self, keys: &PublicKeys, network: &str, method: &str, target: &str, body: &[u8]) -> bool {
        keys.verify(signing_string(network, method, target, self.time, &self.nonce, &self.host, body).as_bytes(), &self.sig)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signed_request_checks_against_its_key_and_its_parts() {
        let key = HostKey::generate().unwrap();
        let net = "net_0123456789abcdef0123456789abcdef";
        let headers = sign(&key, net, "laptop", "POST", "/net/v1/frames", 1000, "bm9uY2Vub25jZQ", b"{}");
        let get = |k: &str| headers.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let s = Signed::from_headers(get).unwrap();
        assert!(s.verify(key.public(), net, "POST", "/net/v1/frames", b"{}"));
        assert!(!s.verify(key.public(), net, "POST", "/net/v1/frames", b"{ }"), "the body");
        assert!(!s.verify(key.public(), net, "GET", "/net/v1/frames", b"{}"), "the method");
        assert!(!s.verify(key.public(), net, "POST", "/net/v1/link?after=1", b"{}"), "the target");
        assert!(!s.verify(HostKey::generate().unwrap().public(), net, "POST", "/net/v1/frames", b"{}"), "the key");
        let no_sig = |k: &str| if k == SIG { None } else { get(k) };
        assert!(Signed::from_headers(no_sig).is_none());
    }
}
