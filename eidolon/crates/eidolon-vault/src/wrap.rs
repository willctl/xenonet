//! An eidolon key wrapped to another machine's X25519 key, for a move.
//!
//! The sender makes a one-time X25519 key, agrees a secret with the
//! receiver's public key, and derives a wrapping key from it with
//! HKDF-SHA256 over both public keys and a context string (so a wrapped key
//! cannot be replayed for another purpose). The wrapping key is used once,
//! so its nonce is fixed at zero.

use crate::keys::{derive_with, getrandom_fill, EidolonKey};
use crate::VaultError;
use ring::aead;
use serde::{Deserialize, Serialize};
use x25519_dalek::{PublicKey, StaticSecret};
use xenonet_id::codec::{b64, unb64};
use zeroize::Zeroize;

const WRAP_INFO: &[u8] = b"eidolon/wrap/1";

/// A wrapped eidolon key: the sender's one-time public key and the sealed
/// key. Safe to send over any channel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrappedKey {
    #[serde(with = "b64_array")]
    pub ephemeral: [u8; 32],
    #[serde(with = "b64_vec")]
    pub sealed: Vec<u8>,
}

mod b64_array {
    use super::{b64, unb64};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&b64(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let bytes = unb64(&String::deserialize(d)?).ok_or_else(|| de::Error::custom("not base64url"))?;
        bytes.try_into().map_err(|_| de::Error::custom("not 32 bytes"))
    }
}

mod b64_vec {
    use super::{b64, unb64};
    use serde::{de, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&b64(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        unb64(&String::deserialize(d)?).ok_or_else(|| de::Error::custom("not base64url"))
    }
}

// The wrapping key for one (ephemeral, receiver, context) triple.
fn wrapping_key(shared: &[u8; 32], ephemeral: &[u8; 32], receiver: &[u8; 32], context: &[u8]) -> Result<aead::LessSafeKey, VaultError> {
    let mut info = Vec::with_capacity(WRAP_INFO.len() + 64 + context.len());
    info.extend_from_slice(WRAP_INFO);
    info.extend_from_slice(ephemeral);
    info.extend_from_slice(receiver);
    info.extend_from_slice(context);
    let mut bytes = derive_with(shared, &info)?;
    let key = aead::UnboundKey::new(&aead::CHACHA20_POLY1305, &bytes).map(aead::LessSafeKey::new);
    bytes.zeroize();
    key.map_err(|_| VaultError::Crypto("the wrapping key was refused"))
}

const ZERO_NONCE: [u8; 12] = [0; 12];

/// Wraps `key` so that only the holder of the secret for
/// `receiver_x25519_public` can open it, and only with the same `context`.
pub fn wrap_key(key: &EidolonKey, receiver_x25519_public: [u8; 32], context: &[u8]) -> Result<WrappedKey, VaultError> {
    let mut secret_bytes = [0u8; 32];
    getrandom_fill(&mut secret_bytes)?;
    let secret = StaticSecret::from(secret_bytes);
    secret_bytes.zeroize();
    let ephemeral = PublicKey::from(&secret).to_bytes();
    let shared = secret.diffie_hellman(&PublicKey::from(receiver_x25519_public));
    // A receiver key that forces a known secret is no receiver.
    if !shared.was_contributory() {
        return Err(VaultError::Crypto("the receiver's key is not usable"));
    }
    let mut shared = shared.to_bytes();
    let wrapping = wrapping_key(&shared, &ephemeral, &receiver_x25519_public, context);
    shared.zeroize();
    let mut sealed = key.as_bytes().to_vec();
    wrapping?
        .seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(ZERO_NONCE), aead::Aad::from(context), &mut sealed)
        .map_err(|_| VaultError::Crypto("sealing the key failed"))?;
    Ok(WrappedKey { ephemeral, sealed })
}

/// The eidolon key in `wrapped`, if it was wrapped to this secret's public
/// key with this `context` and has not been changed.
pub fn unwrap_key(wrapped: &WrappedKey, receiver_secret: &StaticSecret, context: &[u8]) -> Result<EidolonKey, VaultError> {
    let receiver_public = PublicKey::from(receiver_secret).to_bytes();
    let shared = receiver_secret.diffie_hellman(&PublicKey::from(wrapped.ephemeral));
    if !shared.was_contributory() {
        return Err(VaultError::CannotUnwrap);
    }
    let mut shared = shared.to_bytes();
    let wrapping = wrapping_key(&shared, &wrapped.ephemeral, &receiver_public, context);
    shared.zeroize();
    let mut buf = wrapped.sealed.clone();
    let opened = wrapping
        .map_err(|_| VaultError::CannotUnwrap)?
        .open_in_place(aead::Nonce::assume_unique_for_key(ZERO_NONCE), aead::Aad::from(context), &mut buf)
        .map_err(|_| VaultError::CannotUnwrap);
    let key = opened.and_then(|plain| <[u8; 32]>::try_from(&*plain).map_err(|_| VaultError::CannotUnwrap));
    buf.zeroize();
    key.map(EidolonKey::from_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receiver() -> (StaticSecret, [u8; 32]) {
        let mut b = [0u8; 32];
        getrandom_fill(&mut b).unwrap();
        let secret = StaticSecret::from(b);
        let public = PublicKey::from(&secret).to_bytes();
        (secret, public)
    }

    #[test]
    fn a_wrapped_key_opens_at_the_receiver() {
        let (secret, public) = receiver();
        let key = EidolonKey::generate().unwrap();
        let wrapped = wrap_key(&key, public, b"move 7").unwrap();
        assert_eq!(unwrap_key(&wrapped, &secret, b"move 7").unwrap(), key);
    }

    #[test]
    fn the_wrong_receiver_cannot_open_it() {
        let (_, public) = receiver();
        let (other, _) = receiver();
        let wrapped = wrap_key(&EidolonKey::generate().unwrap(), public, b"c").unwrap();
        assert!(matches!(unwrap_key(&wrapped, &other, b"c"), Err(VaultError::CannotUnwrap)));
    }

    #[test]
    fn the_wrong_context_cannot_open_it() {
        let (secret, public) = receiver();
        let wrapped = wrap_key(&EidolonKey::generate().unwrap(), public, b"move 7").unwrap();
        assert!(matches!(unwrap_key(&wrapped, &secret, b"move 8"), Err(VaultError::CannotUnwrap)));
    }

    #[test]
    fn any_changed_byte_fails() {
        let (secret, public) = receiver();
        let wrapped = wrap_key(&EidolonKey::generate().unwrap(), public, b"c").unwrap();
        for i in 0..wrapped.sealed.len() {
            let mut bad = wrapped.clone();
            bad.sealed[i] ^= 1;
            assert!(unwrap_key(&bad, &secret, b"c").is_err(), "sealed byte {i}");
        }
        for i in 0..32 {
            let mut bad = wrapped.clone();
            bad.ephemeral[i] ^= 1;
            assert!(unwrap_key(&bad, &secret, b"c").is_err(), "ephemeral byte {i}");
        }
        let mut short = wrapped;
        short.sealed.pop();
        assert!(unwrap_key(&short, &secret, b"c").is_err());
    }

    #[test]
    fn a_receiver_key_of_all_zeros_is_refused() {
        assert!(wrap_key(&EidolonKey::generate().unwrap(), [0; 32], b"c").is_err());
    }

    #[test]
    fn it_travels_as_json_with_base64url_fields() {
        let (secret, public) = receiver();
        let key = EidolonKey::generate().unwrap();
        let wrapped = wrap_key(&key, public, b"c").unwrap();
        let json = serde_json::to_string(&wrapped).unwrap();
        assert!(json.contains("\"ephemeral\":\""));
        let back: WrappedKey = serde_json::from_str(&json).unwrap();
        assert_eq!(unwrap_key(&back, &secret, b"c").unwrap(), key);
    }
}
