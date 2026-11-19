//! Where an eidolon key lives: the machine's key store, under
//! `eidolon/<id>`. Never a file beside the vault.

use crate::keys::EidolonKey;
use crate::VaultError;
use xenonet_id::store::KeyStore;
use zeroize::Zeroizing;

pub struct KeyRing<'a> {
    store: &'a dyn KeyStore,
}

impl<'a> KeyRing<'a> {
    pub fn new(store: &'a dyn KeyStore) -> KeyRing<'a> {
        KeyRing { store }
    }

    fn name(eidolon_id: &str) -> String {
        format!("eidolon/{eidolon_id}")
    }

    /// Keeps `key` for `eidolon_id`, replacing any key there.
    pub fn put(&self, eidolon_id: &str, key: &EidolonKey) -> Result<(), VaultError> {
        self.store.put(&Self::name(eidolon_id), key.as_bytes()).map_err(|e| VaultError::KeyStore(e.to_string()))
    }

    /// The key for `eidolon_id`, or None when this machine has none.
    pub fn get(&self, eidolon_id: &str) -> Result<Option<EidolonKey>, VaultError> {
        let Some(bytes) = self.store.get(&Self::name(eidolon_id)).map_err(|e| VaultError::KeyStore(e.to_string()))? else {
            return Ok(None);
        };
        let bytes = Zeroizing::new(bytes);
        let array: [u8; 32] = bytes.as_slice().try_into().map_err(|_| VaultError::BadStoredKey)?;
        Ok(Some(EidolonKey::from_bytes(array)))
    }

    /// Forgets the key for `eidolon_id`. Without it the vault's chunks can no
    /// longer be opened here.
    pub fn delete(&self, eidolon_id: &str) -> Result<(), VaultError> {
        self.store.delete(&Self::name(eidolon_id)).map_err(|e| VaultError::KeyStore(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xenonet_id::store::MemoryKeyStore;

    #[test]
    fn a_key_goes_in_comes_out_and_can_be_deleted() {
        let store = MemoryKeyStore::new();
        let ring = KeyRing::new(&store);
        let key = EidolonKey::generate().unwrap();
        assert!(ring.get("e1").unwrap().is_none());
        ring.put("e1", &key).unwrap();
        assert_eq!(ring.get("e1").unwrap().unwrap(), key);
        assert!(ring.get("e2").unwrap().is_none());
        ring.delete("e1").unwrap();
        assert!(ring.get("e1").unwrap().is_none());
    }

    #[test]
    fn it_is_kept_under_the_eidolon_name() {
        let store = MemoryKeyStore::new();
        KeyRing::new(&store).put("e1", &EidolonKey::from_bytes([3; 32])).unwrap();
        assert_eq!(store.get("eidolon/e1").unwrap().unwrap(), vec![3; 32]);
    }

    #[test]
    fn something_that_is_not_a_key_is_refused() {
        let store = MemoryKeyStore::new();
        store.put("eidolon/e1", b"short").unwrap();
        assert!(matches!(KeyRing::new(&store).get("e1"), Err(VaultError::BadStoredKey)));
    }
}
