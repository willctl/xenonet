//! Sealing for one machine (docs/design/network.md, section 3): what an
//! envelope's entry, and an act's authority beside it, are sealed with.
//!
//! An ephemeral X25519 key agrees a secret with the recipient's transport
//! key; HKDF-SHA256 (no salt) makes a key from it, with the ephemeral
//! public key, the recipient's key id and a head (what the sealing binds)
//! as its info; the plaintext is ChaCha20-Poly1305 with a zero nonce (each
//! key is used once) and the head as associated data, with the ephemeral
//! public key in front of the ciphertext. A change to a line of the head
//! makes it fail to open.

use crate::keys::{HostKey, PublicKeys};
use crate::{NetError, R};
use ring::{aead, hkdf};
use x25519_dalek::{PublicKey as SealPublic, StaticSecret};

const NONCE: [u8; 12] = [0u8; 12];

fn sealing_key(shared: &[u8; 32], eph_pub: &[u8; 32], key_id: &str, head: &str) -> R<aead::LessSafeKey> {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[]).extract(shared);
    let info: [&[u8]; 3] = [eph_pub, key_id.as_bytes(), head.as_bytes()];
    let okm = prk.expand(&info, &aead::CHACHA20_POLY1305).map_err(|_| NetError::internal("HKDF refused"))?;
    Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
}

/// Seals `plain` for `to` with the ephemeral key `eph`: the ephemeral
/// public key, then the ciphertext and its tag. A fixed `eph` is for
/// known-answer tests only; `seal` draws a new one each time.
pub fn seal_with(eph: [u8; 32], to: &PublicKeys, head: &str, plain: &[u8]) -> R<Vec<u8>> {
    let secret = StaticSecret::from(eph);
    let eph_pub = SealPublic::from(&secret).to_bytes();
    let shared = secret.diffie_hellman(&SealPublic::from(to.seal));
    if !shared.was_contributory() {
        return Err(NetError::bad("that machine's sealing key can't be sealed to"));
    }
    let key = sealing_key(shared.as_bytes(), &eph_pub, &to.key_id(), head)?;
    let mut buf = plain.to_vec();
    key.seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(NONCE), aead::Aad::from(head.as_bytes()), &mut buf).map_err(|_| NetError::internal("sealing failed"))?;
    let mut out = Vec::with_capacity(32 + buf.len());
    out.extend_from_slice(&eph_pub);
    out.extend_from_slice(&buf);
    Ok(out)
}

/// Seals `plain` for `to`, binding `head`.
pub fn seal(to: &PublicKeys, head: &str, plain: &[u8]) -> R<Vec<u8>> {
    seal_with(crate::codec::random::<32>()?, to, head, plain)
}

/// Opens what was sealed for this machine's key, binding `head`.
pub fn open(key: &HostKey, head: &str, sealed: &[u8]) -> R<Vec<u8>> {
    let refused = || NetError::new(400, "unsealed", "that wasn't sealed for this machine, or was changed");
    if sealed.len() < 32 + 16 {
        return Err(refused());
    }
    let mut eph_pub = [0u8; 32];
    eph_pub.copy_from_slice(&sealed[..32]);
    let shared = key.agree(&eph_pub).ok_or_else(refused)?;
    let k = sealing_key(&shared, &eph_pub, &key.key_id(), head)?;
    let mut buf = sealed[32..].to_vec();
    let plain = k.open_in_place(aead::Nonce::assume_unique_for_key(NONCE), aead::Aad::from(head.as_bytes()), &mut buf).map_err(|_| refused())?;
    Ok(plain.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_sealed_opens_for_its_recipient_under_its_head_alone() {
        let to = HostKey::generate().unwrap();
        let other = HostKey::generate().unwrap();
        let sealed = seal(to.public(), "head", b"plain").unwrap();
        assert_eq!(open(&to, "head", &sealed).unwrap(), b"plain");
        assert_eq!(open(&to, "another head", &sealed).unwrap_err().code, "unsealed");
        assert_eq!(open(&other, "head", &sealed).unwrap_err().code, "unsealed");
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(open(&to, "head", &changed).is_err());
        assert!(open(&to, "head", &sealed[..20]).is_err());
        // A fixed ephemeral key seals the same bytes each time.
        assert_eq!(seal_with([7; 32], to.public(), "h", b"p").unwrap(), seal_with([7; 32], to.public(), "h", b"p").unwrap());
    }
}
