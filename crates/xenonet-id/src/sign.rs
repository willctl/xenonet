//! Signatures and what they're for. Every signature XenoNet makes covers
//! a first line naming its purpose (`xenonet-net/1` for a frame's head,
//! `xenonet-roster/2` for a network chain entry, and so on), so a
//! signature made for one thing can never be taken for another.
//!
//! A `VerifyingKey` checks a signature: Ed25519, which every software key
//! is, or ECDSA P-256, which is what a secure element signs with. A
//! `Signer` holds the private half wherever it lives and says where
//! (`KeyHome`), so policy can tell a key in hardware from one in a file.

use crate::codec::{b64, base32, sha256, unb64};
use crate::keys::write_private;
use crate::{IdError, R};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_ASN1, ED25519};
use serde_json::{json, Value};
use std::fmt;
use std::path::Path;

/// The first line of what a signature covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Purpose(&'static str);

impl Purpose {
    /// A frame's head (`xenonet-net`'s frames).
    pub const FRAME: Purpose = Purpose("xenonet-net/1");
    /// An entry of a network's chain, the roster.
    pub const ROSTER: Purpose = Purpose("xenonet-roster/2");
    /// An entry of a person's chain.
    pub const PERSON: Purpose = Purpose("xenonet-person/1");
    /// A presence block.
    pub const PRESENCE: Purpose = Purpose("xenonet-presence/1");
    /// A token root's certificate.
    pub const TOKEN_ROOT: Purpose = Purpose("xenonet-token-root/1");
    /// An act done as a person, signed by their device.
    pub const ACT: Purpose = Purpose("xenonet-act/1");
    /// An entry of a machine's feed in a network (`xenonet-log`'s feeds).
    pub const FEED: Purpose = Purpose("xenonet-feed/1");

    pub const fn tag(self) -> &'static str {
        self.0
    }

    /// What's signed: the tag, a newline, the message.
    pub fn bytes(self, msg: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.0.len() + 1 + msg.len());
        out.extend_from_slice(self.0.as_bytes());
        out.push(b'\n');
        out.extend_from_slice(msg);
        out
    }
}

/// Where a private key lives, as policy weighs it. None of these keeps a
/// key from another program of the same account: a file it can read, a
/// keychain item it can reach through the binary the item trusts, a secure
/// element's key it can ask to sign. What does is a key that asks the
/// person before each signature (`Signer::asks_person`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyHome {
    /// In a file this account can read.
    File,
    /// In the OS's credential store.
    Keychain,
    /// In a secure element (Secure Enclave, TPM): it signs there and never
    /// leaves.
    Hardware,
}

impl KeyHome {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyHome::File => "file",
            KeyHome::Keychain => "keychain",
            KeyHome::Hardware => "hardware",
        }
    }

    pub fn parse(s: &str) -> Option<KeyHome> {
        [KeyHome::File, KeyHome::Keychain, KeyHome::Hardware].into_iter().find(|h| h.as_str() == s)
    }
}

/// A public key that checks signatures.
#[derive(Clone, PartialEq, Eq, Hash)]
pub enum VerifyingKey {
    Ed25519([u8; 32]),
    /// SEC1 uncompressed, 65 bytes.
    P256(Vec<u8>),
}

impl fmt::Debug for VerifyingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VerifyingKey({})", self.id())
    }
}

impl VerifyingKey {
    fn alg(&self) -> &'static str {
        match self {
            VerifyingKey::Ed25519(_) => "ed25519",
            VerifyingKey::P256(_) => "p256",
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            VerifyingKey::Ed25519(k) => k,
            VerifyingKey::P256(k) => k,
        }
    }

    /// `xns1_` and the base32 of the first 20 bytes of the SHA-256 of the
    /// algorithm's name, a newline and the key.
    pub fn id(&self) -> String {
        let mut b = format!("{}\n", self.alg()).into_bytes();
        b.extend_from_slice(self.bytes());
        format!("xns1_{}", base32(&sha256(&b)[..20]))
    }

    /// Whether `sig` is this key's signature over `msg` for `purpose`.
    pub fn verify(&self, purpose: Purpose, msg: &[u8], sig: &[u8]) -> bool {
        self.verify_raw(&purpose.bytes(msg), sig)
    }

    /// Whether `sig` is this key's signature over exactly `bytes`, for the
    /// formats that write their tag themselves.
    pub fn verify_raw(&self, bytes: &[u8], sig: &[u8]) -> bool {
        match self {
            VerifyingKey::Ed25519(k) => UnparsedPublicKey::new(&ED25519, k).verify(bytes, sig).is_ok(),
            VerifyingKey::P256(k) => UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, k).verify(bytes, sig).is_ok(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "alg": self.alg(), "key": b64(self.bytes()) })
    }

    pub fn parse(v: &Value) -> Option<VerifyingKey> {
        let key = unb64(v.get("key")?.as_str()?)?;
        match v.get("alg")?.as_str()? {
            "ed25519" => Some(VerifyingKey::Ed25519(key.try_into().ok()?)),
            "p256" if key.len() == 65 && key[0] == 4 => Some(VerifyingKey::P256(key)),
            _ => None,
        }
    }
}

/// Holds a private key wherever it lives and signs with it.
pub trait Signer: Send + Sync {
    fn public(&self) -> VerifyingKey;
    fn home(&self) -> KeyHome;
    /// A signature over `msg` for `purpose`.
    fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>>;
    /// Whether the key itself asks the person before each signature, as a
    /// Secure Enclave key made with biometry does: then nobody signs with
    /// it unless the person answers Touch ID, whoever holds it.
    fn asks_person(&self) -> bool {
        false
    }
}

/// An Ed25519 key held in this process, loaded from wherever its seed was
/// kept.
pub struct SeedSigner {
    pair: Ed25519KeyPair,
    home: KeyHome,
}

impl fmt::Debug for SeedSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[the {} key {}]", self.home.as_str(), self.public().id())
    }
}

impl SeedSigner {
    pub fn from_seed(seed: &[u8; 32], home: KeyHome) -> R<SeedSigner> {
        let pair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| IdError::internal("that isn't an Ed25519 key"))?;
        Ok(SeedSigner { pair, home })
    }

    /// A key in a file of its own (`{v, seed}`), made owner-only when
    /// there's none.
    pub fn load_or_create(file: &Path) -> R<SeedSigner> {
        match std::fs::read_to_string(file) {
            Ok(text) => {
                let v: Value = serde_json::from_str(&text).map_err(|_| IdError::internal("a signing key's file isn't readable"))?;
                let seed = v["seed"].as_str().and_then(crate::codec::unb64_n::<32>).ok_or_else(|| IdError::internal("a signing key's file isn't readable"))?;
                SeedSigner::from_seed(&seed, KeyHome::File)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let seed = crate::codec::random::<32>()?;
                let key = SeedSigner::from_seed(&seed, KeyHome::File)?;
                let text = format!("{}\n", json!({ "v": 1, "seed": b64(&seed), "keyId": key.public().id() }));
                write_private(file, text.as_bytes())?;
                Ok(key)
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl Signer for SeedSigner {
    fn public(&self) -> VerifyingKey {
        let mut k = [0u8; 32];
        k.copy_from_slice(self.pair.public_key().as_ref());
        VerifyingKey::Ed25519(k)
    }

    fn home(&self) -> KeyHome {
        self.home
    }

    fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
        Ok(self.pair.sign(&purpose.bytes(msg)).as_ref().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_signature_checks_only_for_its_purpose_and_message() {
        let k = SeedSigner::from_seed(&[7u8; 32], KeyHome::File).unwrap();
        let sig = k.sign(Purpose::ACT, b"answer allow").unwrap();
        let public = k.public();
        assert!(public.verify(Purpose::ACT, b"answer allow", &sig));
        assert!(!public.verify(Purpose::PRESENCE, b"answer allow", &sig), "another purpose");
        assert!(!public.verify(Purpose::ACT, b"answer deny", &sig), "another message");
        assert!(public.verify_raw(&Purpose::ACT.bytes(b"answer allow"), &sig));
        let other = SeedSigner::from_seed(&[8u8; 32], KeyHome::File).unwrap();
        assert!(!other.public().verify(Purpose::ACT, b"answer allow", &sig));
        assert_eq!(VerifyingKey::parse(&public.to_json()), Some(public.clone()));
        assert!(public.id().starts_with("xns1_") && public.id().len() == 37);
        assert_eq!(format!("{k:?}"), format!("[the file key {}]", public.id()));
    }

    #[test]
    fn a_p256_key_verifies_what_a_secure_element_signs() {
        // A P-256 key pair made here, standing in for a secure element.
        let rng = ring::rand::SystemRandom::new();
        let doc = ring::signature::EcdsaKeyPair::generate_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let pair = ring::signature::EcdsaKeyPair::from_pkcs8(&ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING, doc.as_ref(), &rng).unwrap();
        let public = VerifyingKey::P256(pair.public_key().as_ref().to_vec());
        let sig = pair.sign(&rng, &Purpose::TOKEN_ROOT.bytes(b"root")).unwrap();
        assert!(public.verify(Purpose::TOKEN_ROOT, b"root", sig.as_ref()));
        assert!(!public.verify(Purpose::TOKEN_ROOT, b"rout", sig.as_ref()));
        assert_eq!(VerifyingKey::parse(&public.to_json()), Some(public));
    }

    #[test]
    fn a_key_file_is_made_once() {
        let dir = std::env::temp_dir().join(format!("xenonet-id-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("machine.key");
        let a = SeedSigner::load_or_create(&file).unwrap();
        let b = SeedSigner::load_or_create(&file).unwrap();
        assert_eq!(a.public(), b.public());
        assert_eq!(a.home(), KeyHome::File);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
