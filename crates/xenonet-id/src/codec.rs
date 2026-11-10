//! The small encodings keys and signatures use: base64url without padding
//! (keys, signatures, bodies), base32 as RFC 4648 writes it in lowercase
//! (key ids), base58btc (did:key), SHA-256 and random bytes from the
//! system.

use crate::{IdError, R};
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

const BASE58: &[u8; 58] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

/// base58btc, Bitcoin's alphabet, as multibase's `z` prefix and did:key
/// use it.
pub fn base58(bytes: &[u8]) -> String {
    let zeros = bytes.iter().take_while(|b| **b == 0).count();
    // Base 58 digits, least significant first.
    let mut digits: Vec<u8> = Vec::with_capacity(bytes.len() * 138 / 100 + 1);
    for &b in &bytes[zeros..] {
        let mut carry = u32::from(b);
        for d in &mut digits {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let mut out = String::with_capacity(zeros + digits.len());
    out.extend(std::iter::repeat_n('1', zeros));
    out.extend(digits.iter().rev().map(|d| char::from(BASE58[usize::from(*d)])));
    out
}

/// `N` random bytes from the system.
pub fn random<const N: usize>() -> R<[u8; N]> {
    let mut out = [0u8; N];
    SystemRandom::new().fill(&mut out).map_err(|_| IdError::internal("the system gave no random numbers"))?;
    Ok(out)
}

pub fn hex(bytes: &[u8]) -> String {
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

/// A random id for nonces and epochs: 16 bytes of base64url.
pub fn nonce() -> R<String> {
    Ok(b64(&random::<16>()?))
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
    fn base64url_and_uuids() {
        assert_ne!(uuid().unwrap(), uuid().unwrap());
        assert_eq!(unb64(&b64(b"\xff\x00hi")).unwrap(), b"\xff\x00hi");
        assert!(unb64("a=").is_none());
        assert!(is_b64(&b64(&[0xfb, 0xff])) && !is_b64("a+b/"));
    }

    #[test]
    fn base58btc_as_did_key_writes_it() {
        // The draft's own vectors.
        assert_eq!(base58(b""), "");
        assert_eq!(base58(&[0, 0, 1]), "112");
        assert_eq!(base58(b"Hello World!"), "2NEpo7TZRRrLZSi2U");
    }
}
