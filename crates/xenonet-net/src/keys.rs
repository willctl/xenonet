//! A host's keys (docs/design/network.md, section 3): an Ed25519 key that
//! signs and an independent X25519 key that frames are sealed to, both in
//! one owner-only file, `<state>/network/host.key`. The private keys never
//! leave this file and this process: no route, log or frame carries them.
//!
//! A host's key id is `xnk1_` and the base32 of the first 20 bytes of the
//! SHA-256 of both public keys, Ed25519 first.

use crate::codec::{b64, base32, random, sha256, unb64_n};
use crate::{NetError, R};
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use serde_json::{json, Value};
use std::fmt;
use std::io::Write as _;
use std::path::Path;
use x25519_dalek::{PublicKey as SealPublic, StaticSecret};

/// A host's two public keys.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKeys {
    pub sign: [u8; 32],
    pub seal: [u8; 32],
}

impl fmt::Debug for PublicKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKeys({})", self.key_id())
    }
}

impl PublicKeys {
    /// What people compare: `xnk1_` and 32 base32 characters.
    pub fn key_id(&self) -> String {
        let mut both = [0u8; 64];
        both[..32].copy_from_slice(&self.sign);
        both[32..].copy_from_slice(&self.seal);
        format!("xnk1_{}", base32(&sha256(&both)[..20]))
    }

    /// Whether `sig` is this host's Ed25519 signature over `msg`.
    pub fn verify(&self, msg: &[u8], sig: &[u8]) -> bool {
        UnparsedPublicKey::new(&ED25519, &self.sign).verify(msg, sig).is_ok()
    }

    pub fn sign_b64(&self) -> String {
        b64(&self.sign)
    }

    pub fn seal_b64(&self) -> String {
        b64(&self.seal)
    }

    /// From their base64url, as the roster and the join request carry them.
    pub fn from_b64(sign: &str, seal: &str) -> Option<PublicKeys> {
        Some(PublicKeys { sign: unb64_n(sign)?, seal: unb64_n(seal)? })
    }
}

/// This host's private keys.
pub struct HostKey {
    sign: Ed25519KeyPair,
    seal: StaticSecret,
    public: PublicKeys,
}

// Logged by mistake, it says whose it is and nothing more.
impl fmt::Debug for HostKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[the host key {}]", self.public.key_id())
    }
}

impl HostKey {
    /// The keys from their secrets: the Ed25519 seed and the X25519 scalar.
    pub fn from_secrets(seed: &[u8; 32], seal: [u8; 32]) -> R<HostKey> {
        let sign = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| NetError::internal("that isn't an Ed25519 key"))?;
        let seal = StaticSecret::from(seal);
        let mut sign_pub = [0u8; 32];
        sign_pub.copy_from_slice(sign.public_key().as_ref());
        let public = PublicKeys { sign: sign_pub, seal: SealPublic::from(&seal).to_bytes() };
        Ok(HostKey { sign, seal, public })
    }

    /// New keys, from the system's random numbers.
    pub fn generate() -> R<HostKey> {
        HostKey::from_secrets(&random::<32>()?, random::<32>()?)
    }

    pub fn public(&self) -> &PublicKeys {
        &self.public
    }

    pub fn key_id(&self) -> String {
        self.public.key_id()
    }

    /// An Ed25519 signature over `msg`.
    pub fn sign(&self, msg: &[u8]) -> Vec<u8> {
        self.sign.sign(msg).as_ref().to_vec()
    }

    /// The X25519 secret this key agrees with another's public key, or None
    /// when that key is one of the few that force a known secret.
    pub fn agree(&self, their: &[u8; 32]) -> Option<[u8; 32]> {
        let shared = self.seal.diffie_hellman(&SealPublic::from(*their));
        shared.was_contributory().then(|| shared.to_bytes())
    }

    /// The key in its file, or None when there's no file.
    pub fn load(file: &Path) -> R<Option<HostKey>> {
        let text = match std::fs::read_to_string(file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let v: Value = serde_json::from_str(&text).map_err(|_| NetError::internal("the host key's file isn't readable"))?;
        let seed = v["sign"].as_str().and_then(unb64_n::<32>);
        let seal = v["seal"].as_str().and_then(unb64_n::<32>);
        match (seed, seal) {
            (Some(seed), Some(seal)) => HostKey::from_secrets(&seed, seal).map(Some),
            _ => Err(NetError::internal("the host key's file isn't readable")),
        }
    }

    /// The key in its file, made and written owner-only first when there's
    /// none.
    pub fn load_or_create(file: &Path) -> R<HostKey> {
        if let Some(k) = HostKey::load(file)? {
            return Ok(k);
        }
        let (seed, seal) = (random::<32>()?, random::<32>()?);
        let key = HostKey::from_secrets(&seed, seal)?;
        let text = format!("{}\n", json!({ "v": 1, "sign": b64(&seed), "seal": b64(&seal), "keyId": key.key_id() }));
        write_private(file, text.as_bytes())?;
        Ok(key)
    }
}

/// Writes a new file only this account can read: mode 0600 in a 0700
/// folder, or an ACL with this account alone on Windows. It never replaces
/// a file that's there.
pub fn write_private(file: &Path, data: &[u8]) -> R<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(file)?;
    #[cfg(not(unix))]
    {
        let name = file.to_string_lossy().into_owned();
        let protected = xenonet_base::private_file::protect_file(&name, true, &mut |cmd: &str, args: &[&str]| xenonet_base::private_file::run_private(cmd, args));
        if let Err(e) = protected {
            drop(f);
            let _ = std::fs::remove_file(file);
            return Err(NetError::internal(format!("XenoNet couldn't make {name} yours alone: {}", e.message())));
        }
    }
    f.write_all(data)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sign_agree_and_have_an_id() {
        let a = HostKey::generate().unwrap();
        let b = HostKey::generate().unwrap();
        let id = a.key_id();
        assert!(crate::codec::is_key_id(&id), "{id}");
        assert_ne!(id, b.key_id());
        let sig = a.sign(b"hello");
        assert!(a.public().verify(b"hello", &sig));
        assert!(!a.public().verify(b"hullo", &sig));
        assert!(!b.public().verify(b"hello", &sig));
        assert_eq!(a.agree(&b.public().seal), b.agree(&a.public().seal));
        // A low-order point would force the secret: refused.
        assert_eq!(a.agree(&[0u8; 32]), None);
        assert_eq!(format!("{a:?}"), format!("[the host key {id}]"));
        let back = PublicKeys::from_b64(&a.public().sign_b64(), &a.public().seal_b64()).unwrap();
        assert_eq!(&back, a.public());
    }

    #[test]
    fn the_key_file_is_made_once_and_owner_only() {
        let dir = std::env::temp_dir().join(format!("xenonet-net-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("network").join("host.key");
        let made = HostKey::load_or_create(&file).unwrap();
        let again = HostKey::load_or_create(&file).unwrap();
        assert_eq!(made.public(), again.public());
        assert!(write_private(&file, b"x").is_err(), "never replaced");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
