//! Windows: the machine signing key in the TPM, through NCrypt's Microsoft
//! Platform Crypto Provider (docs/design/direction.md, "The machine key").
//!
//! The key is a P-256 key the TPM makes and keeps, persisted by name in the
//! key store of the account XenoNet runs as (no machine-store flag), with
//! an export policy of none. The private half never leaves the TPM, so a
//! program that reads every file of the account can't copy it off this
//! machine; it can still open the key by name and ask the TPM to sign, as
//! with the Secure Enclave, so the property is "can't be copied", not
//! "can't sign as the machine". The key's file (`crate::machine`) holds its
//! name, the account and the public key, never a secret.
//!
//! Every call is silent (`NCRYPT_SILENT_FLAG` where NCrypt takes it), and
//! the key has no UI policy, so nothing here shows a window.

#![expect(unsafe_code, reason = "NCrypt's calls, which the windows crate marks unsafe, and its handles' Send and Sync")]

use crate::machine::NamedKeys;
use std::sync::Mutex;
use windows::core::HSTRING;
use windows::Win32::Foundation::{NTE_BAD_KEYSET, NTE_NOT_FOUND};
use windows::Win32::Security::Cryptography::{
    NCryptCreatePersistedKey, NCryptDeleteKey, NCryptExportKey, NCryptFinalizeKey, NCryptFreeObject, NCryptGetProperty, NCryptOpenKey, NCryptOpenStorageProvider,
    NCryptSetProperty, NCryptSignHash, BCRYPT_ECCPUBLIC_BLOB, CERT_KEY_SPEC, MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_ECDSA_P256_ALGORITHM, NCRYPT_EXPORT_POLICY_PROPERTY, NCRYPT_FLAGS,
    NCRYPT_HANDLE, NCRYPT_KEY_HANDLE, NCRYPT_PCP_PLATFORM_TYPE_PROPERTY, NCRYPT_PROV_HANDLE, NCRYPT_SILENT_FLAG,
};
use windows::Win32::Security::OBJECT_SECURITY_INFORMATION;
use xenonet_id::sign::{KeyHome, Purpose, Signer, VerifyingKey};
use xenonet_id::{IdError, R};

/// This machine's TPM, as the Platform Crypto Provider gives it.
pub struct Tpm;

impl NamedKeys for Tpm {
    /// The provider opens and names its TPM. Without a TPM (or with one
    /// switched off) the provider has no platform to name.
    fn present(&self) -> bool {
        Provider::open().is_ok_and(|p| p.platform().is_some())
    }

    fn account(&self) -> String {
        let user = std::env::var("USERNAME").unwrap_or_default();
        match std::env::var("USERDOMAIN") {
            Ok(domain) if !domain.is_empty() => format!("{domain}\\{user}"),
            _ => user,
        }
    }

    fn open(&self, name: &str) -> R<Option<Box<dyn Signer>>> {
        let provider = Provider::open().map_err(|e| tpm_error("opening the TPM's key store", &e))?;
        let mut handle = NCRYPT_KEY_HANDLE::default();
        // SAFETY: the provider handle is open, and `handle` is written only
        // when the call succeeds.
        let opened = unsafe { NCryptOpenKey(provider.0, &raw mut handle, &HSTRING::from(name), CERT_KEY_SPEC(0), NCRYPT_SILENT_FLAG) };
        match opened {
            Ok(()) => Ok(Some(Box::new(TpmSigner::from_key(Key(handle), provider)?))),
            Err(e) if e.code() == NTE_BAD_KEYSET || e.code() == NTE_NOT_FOUND => Ok(None),
            Err(e) => Err(tpm_error("opening the machine signing key", &e)),
        }
    }

    fn create(&self, name: &str) -> R<Box<dyn Signer>> {
        let provider = Provider::open().map_err(|e| tpm_error("opening the TPM's key store", &e))?;
        let mut handle = NCRYPT_KEY_HANDLE::default();
        // SAFETY: as in `open`. No overwrite flag: a key by this name stays.
        unsafe { NCryptCreatePersistedKey(provider.0, &raw mut handle, NCRYPT_ECDSA_P256_ALGORITHM, &HSTRING::from(name), CERT_KEY_SPEC(0), NCRYPT_FLAGS(0)) }
            .map_err(|e| tpm_error("making the machine signing key", &e))?;
        let key = Key(handle);
        // Never exportable, set before the key exists in the TPM.
        // SAFETY: the key handle is open and not finalized yet; the policy
        // is a DWORD.
        let made = unsafe { NCryptSetProperty(NCRYPT_HANDLE::from(key.0), NCRYPT_EXPORT_POLICY_PROPERTY, &0u32.to_le_bytes(), NCRYPT_FLAGS(0)) }
            // SAFETY: the same handle, finalized once.
            .and_then(|()| unsafe { NCryptFinalizeKey(key.0, NCRYPT_SILENT_FLAG) });
        if let Err(e) = made {
            key.delete();
            return Err(tpm_error("making the machine signing key", &e));
        }
        Ok(Box::new(TpmSigner::from_key(key, provider)?))
    }
}

/// A P-256 key in the TPM, open for signing.
pub struct TpmSigner {
    // Dropped before the provider it was opened from.
    key: Mutex<Key>,
    public: VerifyingKey,
    _provider: Provider,
}

impl TpmSigner {
    fn from_key(key: Key, provider: Provider) -> R<TpmSigner> {
        let blob = export_public(&key).map_err(|e| tpm_error("reading the machine signing key's public half", &e))?;
        let public = crate::ecdsa::sec1_from_ecc_blob(&blob).ok_or_else(|| IdError::internal("the TPM's key isn't a P-256 key"))?;
        Ok(TpmSigner { key: Mutex::new(key), public: VerifyingKey::P256(public), _provider: provider })
    }
}

impl Signer for TpmSigner {
    fn public(&self) -> VerifyingKey {
        self.public.clone()
    }

    fn home(&self) -> KeyHome {
        KeyHome::Hardware
    }

    /// ECDSA over the SHA-256 of what's signed: the TPM signs the digest
    /// and gives r and s, which go out as DER for `VerifyingKey::P256`.
    fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
        let digest = ring::digest::digest(&ring::digest::SHA256, &purpose.bytes(msg));
        let key = self.key.lock().map_err(|_| IdError::internal("the TPM key's lock was poisoned"))?;
        let mut sig = [0u8; 64];
        let mut len = 0u32;
        // SAFETY: the key handle is open while the lock is held; the digest
        // and the buffer outlive the call, which writes at most 64 bytes.
        unsafe { NCryptSignHash(key.0, None, digest.as_ref(), Some(&mut sig), &raw mut len, NCRYPT_SILENT_FLAG) }.map_err(|e| tpm_error("signing in the TPM", &e))?;
        let raw = sig.get(..len as usize).ok_or_else(|| IdError::internal("the TPM's signature was too long"))?;
        crate::ecdsa::der_from_raw(raw).ok_or_else(|| IdError::internal("the TPM's signature isn't a P-256 one"))
    }
}

/// The key's public half as a BCRYPT_ECCPUBLIC_BLOB.
fn export_public(key: &Key) -> windows::core::Result<Vec<u8>> {
    let mut len = 0u32;
    // SAFETY: the key handle is open; the first call asks the size, the
    // second writes at most that into a buffer of that size.
    unsafe { NCryptExportKey(key.0, None, BCRYPT_ECCPUBLIC_BLOB, None, None, &raw mut len, NCRYPT_SILENT_FLAG) }?;
    let mut blob = vec![0u8; len as usize];
    // SAFETY: as above.
    unsafe { NCryptExportKey(key.0, None, BCRYPT_ECCPUBLIC_BLOB, None, Some(&mut blob), &raw mut len, NCRYPT_SILENT_FLAG) }?;
    blob.truncate(len as usize);
    Ok(blob)
}

fn tpm_error(doing: &str, e: &windows::core::Error) -> IdError {
    IdError::internal(format!("{doing}: {} ({:#010x})", e.message(), e.code().0))
}

/// The Platform Crypto Provider, opened.
struct Provider(NCRYPT_PROV_HANDLE);

impl Provider {
    fn open() -> windows::core::Result<Provider> {
        let mut handle = NCRYPT_PROV_HANDLE::default();
        // SAFETY: `handle` is written only when the call succeeds.
        unsafe { NCryptOpenStorageProvider(&raw mut handle, MS_PLATFORM_CRYPTO_PROVIDER, 0) }?;
        Ok(Provider(handle))
    }

    /// The TPM the provider runs on ("TPM-Version:2.0 …"), or None when it
    /// has none.
    fn platform(&self) -> Option<String> {
        let mut len = 0u32;
        let property = NCRYPT_PCP_PLATFORM_TYPE_PROPERTY;
        // SAFETY: the provider handle is open; the size first, then at most
        // that many bytes into a buffer of that size.
        unsafe { NCryptGetProperty(NCRYPT_HANDLE(self.0 .0), property, None, &raw mut len, OBJECT_SECURITY_INFORMATION(0)) }.ok()?;
        let mut out = vec![0u8; len as usize];
        // SAFETY: as above.
        unsafe { NCryptGetProperty(NCRYPT_HANDLE(self.0 .0), property, Some(&mut out), &raw mut len, OBJECT_SECURITY_INFORMATION(0)) }.ok()?;
        out.truncate(len as usize);
        let wide: Vec<u16> = out.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).take_while(|&c| c != 0).collect();
        let name = String::from_utf16_lossy(&wide);
        (!name.is_empty()).then_some(name)
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        // SAFETY: the handle was opened by `open` and is freed once.
        let _ = unsafe { NCryptFreeObject(NCRYPT_HANDLE(self.0 .0)) };
    }
}

/// A key handle, freed when dropped.
struct Key(NCRYPT_KEY_HANDLE);

impl Key {
    /// Deletes the key from the store; NCrypt frees the handle with it.
    fn delete(self) {
        let handle = self.0;
        std::mem::forget(self);
        // SAFETY: the handle is open, and isn't used or freed after this.
        let _ = unsafe { NCryptDeleteKey(handle, NCRYPT_SILENT_FLAG.0) };
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: the handle is open and freed once.
        let _ = unsafe { NCryptFreeObject(NCRYPT_HANDLE::from(self.0)) };
    }
}

// SAFETY: NCrypt's handles are plain values, usable from any thread; a
// key's is used under TpmSigner's lock alone.
unsafe impl Send for Key {}
// SAFETY: as above.
unsafe impl Send for Provider {}
// SAFETY: a provider handle is only kept open for its keys and freed once,
// on drop.
unsafe impl Sync for Provider {}

#[cfg(test)]
mod tests {
    //! On Windows hardware with a TPM: a key is made, signs, verifies with
    //! its public key, and opens again by name. Skipped where the TPM isn't
    //! there (GitHub's runners), unless XENONET_REQUIRE_TPM is set, which
    //! makes a missing TPM a failure for a check on a real machine.
    use super::*;
    use xenonet_id::codec::{hex, random};

    #[test]
    fn a_tpm_key_is_made_signs_and_opens_again_by_name() {
        if !Tpm.present() {
            assert!(std::env::var_os("XENONET_REQUIRE_TPM").is_none(), "XENONET_REQUIRE_TPM is set and this machine has no TPM XenoNet can use");
            eprintln!("skipped: no TPM here");
            return;
        }
        let name = format!("XenoNet test key {}", hex(&random::<8>().unwrap()));
        assert!(Tpm.open(&name).unwrap().is_none(), "no key by a new name");
        let made = Tpm.create(&name).unwrap();
        assert_eq!(made.home(), KeyHome::Hardware);
        let started = std::time::Instant::now();
        let sig = made.sign(Purpose::FEED, b"an entry").unwrap();
        eprintln!("one TPM signature took {} ms", started.elapsed().as_millis());
        assert!(made.public().verify(Purpose::FEED, b"an entry", &sig), "the public key checks the TPM's signature");
        assert!(!made.public().verify(Purpose::FRAME, b"an entry", &sig), "for its purpose alone");
        assert!(Tpm.create(&name).is_err(), "a key by that name isn't replaced");
        let opened = Tpm.open(&name).unwrap().expect("the key persists by name");
        assert_eq!(opened.public(), made.public());
        let again = opened.sign(Purpose::FEED, b"another").unwrap();
        assert!(made.public().verify(Purpose::FEED, b"another", &again));
        drop((made, opened));
        // Clean up the test's key.
        let provider = Provider::open().unwrap();
        let mut handle = NCRYPT_KEY_HANDLE::default();
        // SAFETY: as in `Tpm::open`.
        unsafe { NCryptOpenKey(provider.0, &raw mut handle, &HSTRING::from(name.as_str()), CERT_KEY_SPEC(0), NCRYPT_SILENT_FLAG) }.unwrap();
        Key(handle).delete();
        assert!(Tpm.open(&name).unwrap().is_none(), "deleted");
    }

    #[test]
    fn the_machine_key_lives_in_the_tpm_when_there_is_one() {
        let dir = std::env::temp_dir().join(format!("xenonet-keys-tpm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let key = crate::machine_signer(&dir).unwrap();
        let expected = if Tpm.present() { KeyHome::Hardware } else { KeyHome::File };
        // A TPM that's there but won't make keys falls back to a file key,
        // which the file explains; only a required TPM must hold it.
        if std::env::var_os("XENONET_REQUIRE_TPM").is_some() {
            assert_eq!(key.home(), KeyHome::Hardware);
        } else if key.home() != expected {
            eprintln!("the TPM didn't make the key: {}", std::fs::read_to_string(dir.join(crate::MACHINE_FILE)).unwrap_or_default());
        }
        let again = crate::machine_signer(&dir).unwrap();
        assert_eq!(again.public(), key.public(), "the same key after a reopen");
        let row = crate::machine_standing(&dir).unwrap();
        eprintln!("doctor: {row}");
        if key.home() == KeyHome::Hardware {
            assert_eq!(row["status"], "ok", "{row}");
            let name = crate::machine::key_name(&dir);
            let provider = Provider::open().unwrap();
            let mut handle = NCRYPT_KEY_HANDLE::default();
            // SAFETY: as in `Tpm::open`.
            unsafe { NCryptOpenKey(provider.0, &raw mut handle, &HSTRING::from(name.as_str()), CERT_KEY_SPEC(0), NCRYPT_SILENT_FLAG) }.unwrap();
            Key(handle).delete();
            // With the key gone from the TPM, the file still names it: a
            // refusal, never a new key.
            drop((key, again));
            assert!(crate::machine_signer(&dir).is_err());
            assert_eq!(crate::machine_standing(&dir).unwrap()["status"], "fail");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
