//! The roots this machine mints tokens with (docs/design/direction.md,
//! "Authorization"). Biscuit signs only with a key in memory, and the
//! machine's signing key may be one that never leaves a secure element, so
//! a root is made in memory the first time this service start mints for a
//! machine id, and the machine's signing key certifies it for a year. The
//! certificates are public: they're kept (`Persist`) so the tokens they
//! root outlive a restart, while a root's private half never touches the
//! disk. A root is made again once it has less than
//! `RENEW_MS` left, so a token minted from it lasts at least that long.

use crate::persist::Persist;
use crate::R;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use xenonet_authz::{MachineId, RootCertificate, TokenRoot};
use xenonet_id::sign::Signer;
use xenonet_os::sync::{Locked, ReadWrite};

/// How long a root's certificate lasts.
pub const ROOT_MS: i64 = 365 * 24 * 3600 * 1000;
/// A root with less than this left is replaced at the next mint.
pub const RENEW_MS: i64 = 330 * 24 * 3600 * 1000;

pub struct TokenRoots {
    persist: Arc<dyn Persist>,
    /// Every certificate still in its time, this start's and earlier ones'.
    certs: RwLock<Vec<RootCertificate>>,
    /// The roots made since this start, by machine id.
    live: Mutex<HashMap<String, Arc<TokenRoot>>>,
}

impl TokenRoots {
    /// The certificates kept; one that doesn't read is dropped.
    pub fn open(persist: Arc<dyn Persist>) -> TokenRoots {
        let certs = persist.token_roots().map(|list| list.iter().filter_map(RootCertificate::parse).collect()).unwrap_or_default();
        TokenRoots { persist, certs: RwLock::new(certs), live: Mutex::new(HashMap::new()) }
    }

    /// The root to mint with as `machine` now, made and certified by
    /// `signer` (the machine's signing key) when this start has none for it
    /// yet, or the one it has is near its end.
    pub fn root(&self, machine: &MachineId, signer: &dyn Signer, now: i64) -> R<Arc<TokenRoot>> {
        let mut live = self.live.locked();
        if let Some(r) = live.get(machine.as_str()).filter(|r| r.certificate().until() - now >= RENEW_MS) {
            return Ok(r.clone());
        }
        let root = Arc::new(TokenRoot::new(machine.clone(), signer, now, now + ROOT_MS)?);
        let mut certs = self.certs.writing();
        certs.retain(|c| c.until() >= now);
        certs.push(root.certificate().clone());
        self.persist.put_token_roots(&certs.iter().map(RootCertificate::to_json).collect::<Vec<_>>())?;
        live.insert(machine.as_str().to_string(), root.clone());
        Ok(root)
    }

    /// Every certificate kept, unverified: what this machine's actors are
    /// given of its roots (`forward`'s chains).
    pub fn all(&self) -> Vec<RootCertificate> {
        self.certs.reading().clone()
    }

    /// The certificates kept whose key id is `key_id`, unverified.
    pub fn certificates(&self, key_id: u32) -> Vec<RootCertificate> {
        self.certs.reading().iter().filter(|c| c.key_id() == key_id).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn a_root_is_made_once_a_start_and_its_certificate_outlives_it() {
        let dir: Arc<dyn Persist> = Arc::new(crate::persist::Memory::default());
        let signer = SeedSigner::from_seed(&[4; 32], KeyHome::File).unwrap();
        let m = MachineId::new("laptop").unwrap();
        let roots = TokenRoots::open(dir.clone());
        let a = roots.root(&m, &signer, NOW).unwrap();
        assert!(Arc::ptr_eq(&a, &roots.root(&m, &signer, NOW + 1000).unwrap()), "one root for the start");
        let cert = a.certificate().clone();
        cert.verify(&signer.public(), NOW).unwrap();
        // Another start: a new root, and the old certificate still held.
        let again = TokenRoots::open(dir.clone());
        assert_eq!(again.certificates(cert.key_id()), vec![cert.clone()]);
        let b = again.root(&m, &signer, NOW).unwrap();
        assert_ne!(b.certificate(), &cert);
        assert_eq!(TokenRoots::open(dir.clone()).certificates(cert.key_id()), vec![cert.clone()]);
        // Near its end, a root is made again; a certificate past its time
        // is dropped.
        let late = again.root(&m, &signer, NOW + ROOT_MS - RENEW_MS + 1).unwrap();
        assert_ne!(late.certificate(), b.certificate());
        again.root(&MachineId::new("other").unwrap(), &signer, NOW + ROOT_MS + 1).unwrap();
        assert_eq!(TokenRoots::open(dir.clone()).certificates(cert.key_id()), Vec::new());
    }
}
