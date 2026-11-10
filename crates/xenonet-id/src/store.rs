//! Where secrets are kept. A `KeyStore` puts, gets and deletes a secret by
//! name: the OS keychain on each platform (`xenonet-os`'s `keystore`), or
//! memory for tests. A `StoredSigner` is an Ed25519 key whose seed is in
//! one, made there the first time it's asked for.

use crate::codec::random;
use crate::sign::{KeyHome, Purpose, SeedSigner, Signer, VerifyingKey};
use crate::{IdError, R};
use std::collections::BTreeMap;
use std::sync::Mutex;

pub trait KeyStore: Send + Sync {
    fn get(&self, name: &str) -> R<Option<Vec<u8>>>;
    /// Keeps `secret` under `name`, replacing what was there.
    fn put(&self, name: &str, secret: &[u8]) -> R<()>;
    fn delete(&self, name: &str) -> R<()>;
    /// Where what it keeps lives.
    fn home(&self) -> KeyHome;
}

/// Secrets in memory, gone when it is: for tests, and for a machine with
/// no key store, where nothing is kept.
#[derive(Default)]
pub struct MemoryKeyStore {
    secrets: Mutex<BTreeMap<String, Vec<u8>>>,
    home: Option<KeyHome>,
}

impl MemoryKeyStore {
    pub fn new() -> MemoryKeyStore {
        MemoryKeyStore::default()
    }

    /// One that says it keeps its secrets at `home`, to stand in for the
    /// OS keychain in tests.
    pub fn as_home(home: KeyHome) -> MemoryKeyStore {
        MemoryKeyStore { secrets: Mutex::default(), home: Some(home) }
    }

    fn secrets(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        // The map is whole after every call, so a panic elsewhere leaves it
        // usable.
        self.secrets.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl KeyStore for MemoryKeyStore {
    fn get(&self, name: &str) -> R<Option<Vec<u8>>> {
        Ok(self.secrets().get(name).cloned())
    }

    fn put(&self, name: &str, secret: &[u8]) -> R<()> {
        self.secrets().insert(name.into(), secret.to_vec());
        Ok(())
    }

    fn delete(&self, name: &str) -> R<()> {
        self.secrets().remove(name);
        Ok(())
    }

    fn home(&self) -> KeyHome {
        self.home.unwrap_or(KeyHome::File)
    }
}

/// An Ed25519 key whose seed lives in a key store.
pub struct StoredSigner {
    inner: SeedSigner,
}

impl StoredSigner {
    /// The key named `name` in `store`, made and kept there when there's
    /// none.
    pub fn load_or_create(store: &dyn KeyStore, name: &str) -> R<StoredSigner> {
        let seed = match store.get(name)? {
            Some(s) => s.try_into().map_err(|_| IdError::internal(format!("the key {name} in the key store isn't one")))?,
            None => {
                let seed = random::<32>()?;
                store.put(name, &seed)?;
                seed
            }
        };
        Ok(StoredSigner { inner: SeedSigner::from_seed(&seed, store.home())? })
    }

    /// The key named `name`, or None when the store has none.
    pub fn load(store: &dyn KeyStore, name: &str) -> R<Option<StoredSigner>> {
        let Some(seed) = store.get(name)? else { return Ok(None) };
        let seed: [u8; 32] = seed.try_into().map_err(|_| IdError::internal(format!("the key {name} in the key store isn't one")))?;
        Ok(Some(StoredSigner { inner: SeedSigner::from_seed(&seed, store.home())? }))
    }
}

impl Signer for StoredSigner {
    fn public(&self) -> VerifyingKey {
        self.inner.public()
    }

    fn home(&self) -> KeyHome {
        self.inner.home()
    }

    fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
        self.inner.sign(purpose, msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_key_is_made_once_and_kept_where_the_store_keeps_it() {
        let store = MemoryKeyStore::as_home(KeyHome::Keychain);
        assert!(StoredSigner::load(&store, "device").unwrap().is_none());
        let a = StoredSigner::load_or_create(&store, "device").unwrap();
        let b = StoredSigner::load_or_create(&store, "device").unwrap();
        assert_eq!(a.public(), b.public());
        assert_eq!(a.home(), KeyHome::Keychain);
        store.put("device", b"short").unwrap();
        assert!(StoredSigner::load(&store, "device").is_err(), "a secret that isn't a seed");
        store.delete("device").unwrap();
        assert!(store.get("device").unwrap().is_none());
    }
}
