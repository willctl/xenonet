//! The small encodings the network uses: base64url without padding (keys,
//! signatures, bodies), base32 as RFC 4648 writes it in lowercase (key
//! ids), SHA-256, random bytes from the system, and the ids of networks
//! and frames.

use crate::{NetError, R};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};

/// base64url, no padding.
pub fn b64(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// base64url without padding, back to bytes; None for anything else.
pub fn unb64(text: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(text).ok()
}

/// Exactly `N` bytes of base64url, or None.
pub fn unb64_n<const N: usize>(text: &str) -> Option<[u8; N]> {
    unb64(text)?.try_into().ok()
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// The base64url SHA-256 of some bytes, as heads and the roster carry it.
pub fn hash_b64(bytes: &[u8]) -> String {
    b64(&sha256(bytes))
}

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32 in lowercase, without padding.
pub fn base32(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(5) * 8);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// `N` random bytes from the system.
pub fn random<const N: usize>() -> R<[u8; N]> {
    let mut out = [0u8; N];
    SystemRandom::new().fill(&mut out).map_err(|_| NetError::internal("the system gave no random numbers"))?;
    Ok(out)
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|b| [char::from(HEX[usize::from(b >> 4)]), char::from(HEX[usize::from(b & 15)])]).collect()
}

/// A random (version 4) UUID, in lowercase.
pub fn uuid() -> R<String> {
    let mut b = random::<16>()?;
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex(&b);
    Ok(format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..]))
}

/// `frm_<uuid>`.
pub fn frame_id() -> R<String> {
    Ok(format!("frm_{}", uuid()?))
}

/// `net_<32 hex>`, made once by the founder.
pub fn network_id() -> R<String> {
    Ok(format!("net_{}", hex(&random::<16>()?)))
}

/// A random id for nonces and epochs: 16 bytes of base64url.
pub fn nonce() -> R<String> {
    Ok(b64(&random::<16>()?))
}

pub fn is_frame_id(id: &str) -> bool {
    id.strip_prefix("frm_").is_some_and(|u| u.len() == 36 && u.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-'))
}

pub fn is_network_id(id: &str) -> bool {
    id.strip_prefix("net_").is_some_and(|h| h.len() == 32 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}

pub fn is_key_id(id: &str) -> bool {
    id.strip_prefix("xnk1_").is_some_and(|k| k.len() == 32 && k.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b)))
}

pub fn is_b64(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base32_as_rfc_4648_writes_it() {
        // RFC 4648's own vectors, lowercased and without padding.
        for (input, want) in [("", ""), ("f", "my"), ("fo", "mzxq"), ("foo", "mzxw6"), ("foob", "mzxw6yq"), ("fooba", "mzxw6ytb"), ("foobar", "mzxw6ytboi")] {
            assert_eq!(base32(input.as_bytes()), want, "{input}");
        }
        assert_eq!(base32(&[0u8; 20]).len(), 32);
    }

    #[test]
    fn ids_have_their_shapes() {
        let id = frame_id().unwrap();
        assert!(is_frame_id(&id), "{id}");
        assert!(!is_frame_id("frm_x"));
        assert!(is_network_id(&network_id().unwrap()));
        assert!(!is_network_id("net_ABC"));
        assert_ne!(uuid().unwrap(), uuid().unwrap());
        assert_eq!(unb64(&b64(b"\xff\x00hi")).unwrap(), b"\xff\x00hi");
        assert!(unb64("a=").is_none());
        assert!(is_b64(&b64(&[0xfb, 0xff])) && !is_b64("a+b/"));
    }
}
