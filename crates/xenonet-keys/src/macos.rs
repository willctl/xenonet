//! macOS: keys in the Secure Enclave, and sealing a secret to one.
//!
//! A key is made non-permanent, so it never enters a keychain and needs no
//! keychain entitlement or prompt: what the Secure Enclave hands back is a
//! blob (its `toid` attribute) that only this Mac's Secure Enclave can use,
//! and the blob and public key go in an owner-only file. `load` restores the
//! key from the blob with `SecKeyCreateWithData`. Any program of this account
//! that reads the file can do the same, so the key's access control is what
//! guards it:
//!
//! - the machine signing key (`biometric: false`): after first unlock, this
//!   device only, private-key use. Non-exportable, but any program of the
//!   account can ask it to sign, and policy is told so.
//! - the person's device key (`biometric: true`): the same plus the current
//!   set of enrolled fingers. The Secure Enclave asks Touch ID for every
//!   signature and every unwrap, whoever holds the blob, and offers no
//!   password instead. Enrolling a new finger or removing one invalidates
//!   the key, and the device has to be added again.
//!
//! Touch ID shows a reason: "`program` is trying to `reason`." Each call
//! that may ask makes its own `LAContext` with `localizedReason` set and
//! passes it as `kSecUseAuthenticationContext` when it restores the key,
//! which is how Apple's own CryptoKit passes one for a Secure Enclave key
//! (`kSecUseOperationPrompt` is deprecated for it). The reason comes from
//! `crate::with_reason` when the caller set one, else from what is signed
//! (`crate::default_reason`).

use crate::{current_reason, default_reason, UNWRAP_REASON};
use core_foundation::base::{CFType, TCFType};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::error::CFError;
use core_foundation::string::CFString;
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};
use security_framework_sys::access_control::{kSecAccessControlBiometryCurrentSet, kSecAccessControlPrivateKeyUsage};
use serde_json::{json, Value};
use std::fmt;
use std::path::Path;
use std::sync::OnceLock;
use xenonet_id::codec::{b64, unb64};
use xenonet_id::keys::write_private;
use xenonet_id::sign::{KeyHome, Purpose, Signer, VerifyingKey};
use xenonet_id::{IdError, R};

/// What a Secure Enclave key's file says it is (`kind`).
pub const KIND: &str = "secure-enclave";

/// The signature and the sealing XenoNet asks the Secure Enclave for.
/// X9.62 ECDSA over SHA-256 is DER, which `VerifyingKey::P256` checks;
/// ECIES with the cofactor, X9.63 KDF over SHA-256 and AES-GCM is what the
/// Secure Enclave decrypts.
const SIGN: Algorithm = Algorithm::ECDSASignatureMessageX962SHA256;
const SEAL: Algorithm = Algorithm::ECIESEncryptionCofactorX963SHA256AESGCM;

/// A P-256 key in this Mac's Secure Enclave.
pub struct SecureEnclaveSigner {
    toid: Vec<u8>,
    public: VerifyingKey,
    biometric: bool,
    /// The key that never asks, restored once. A biometric key is restored
    /// for each call instead, with that call's reason.
    key: Option<SecKey>,
}

impl fmt::Debug for SecureEnclaveSigner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[the Secure Enclave key {}{}]", self.public.id(), if self.biometric { ", with Touch ID" } else { "" })
    }
}

impl SecureEnclaveSigner {
    /// A new key, written to `file` (owner-only, never replacing one).
    /// Making it never prompts, biometric or not.
    pub fn create(file: &Path, biometric: bool) -> R<SecureEnclaveSigner> {
        let key = generate(biometric)?;
        let toid = blob(&key).ok_or_else(|| IdError::internal("the Secure Enclave gave no blob for its key"))?;
        let public = public_bytes(&key)?;
        let signer = SecureEnclaveSigner { toid, public: VerifyingKey::P256(public), biometric, key: (!biometric).then_some(key) };
        let text =
            format!("{}\n", json!({ "v": 1, "kind": KIND, "biometric": biometric, "toid": b64(&signer.toid), "public": b64(signer.public.bytes()), "keyId": signer.public.id() }));
        write_private(file, text.as_bytes())?;
        Ok(signer)
    }

    /// The key in `file`. The non-biometric key is restored at once, and its
    /// public key checked against the file's; a biometric one is restored
    /// at each use, since restoring it with a reason is how that use asks.
    pub fn load(file: &Path) -> R<SecureEnclaveSigner> {
        let text = std::fs::read_to_string(file)?;
        let bad = || IdError::internal("a Secure Enclave key's file isn't readable");
        let v: Value = serde_json::from_str(&text).map_err(|_| bad())?;
        if v["kind"].as_str() != Some(KIND) {
            return Err(bad());
        }
        let toid = v["toid"].as_str().and_then(unb64).filter(|t| !t.is_empty()).ok_or_else(bad)?;
        let public = v["public"].as_str().and_then(unb64).filter(|p| p.len() == 65 && p[0] == 4).ok_or_else(bad)?;
        let biometric = v["biometric"].as_bool().ok_or_else(bad)?;
        let key = if biometric {
            None
        } else {
            let key = restore(Restore::Enclave { toid: &toid, context: None })?;
            if public_bytes(&key)? != public {
                return Err(IdError::internal("the Secure Enclave's key isn't the one its file names"));
            }
            Some(key)
        };
        Ok(SecureEnclaveSigner { toid, public: VerifyingKey::P256(public), biometric, key })
    }

    pub fn is_biometric(&self) -> bool {
        self.biometric
    }

    /// A signature over `msg` for `purpose`; a biometric key asks Touch ID
    /// with `reason` (it completes "xenonet is trying to …").
    pub fn sign_with_reason(&self, purpose: Purpose, msg: &[u8], reason: &str) -> R<Vec<u8>> {
        self.with_key(reason, |key| key.create_signature(SIGN, &purpose.bytes(msg)).map_err(|e| cf_error("signing in the Secure Enclave", &e)))
    }

    /// What `wrap` sealed to this key; a biometric key asks Touch ID with
    /// `reason`.
    pub fn unwrap_with_reason(&self, ciphertext: &[u8], reason: &str) -> R<Vec<u8>> {
        self.with_key(reason, |key| open(key, ciphertext))
    }

    fn with_key<T>(&self, reason: &str, f: impl FnOnce(&SecKey) -> R<T>) -> R<T> {
        if let Some(key) = &self.key {
            return f(key);
        }
        let context = la::context(reason).ok_or_else(|| IdError::internal("LocalAuthentication isn't there to ask"))?;
        let key = restore(Restore::Enclave { toid: &self.toid, context: Some(&context) })?;
        if public_bytes(&key)?.as_slice() != self.public.bytes() {
            return Err(IdError::internal("the Secure Enclave's key isn't the one its file names"));
        }
        f(&key)
    }
}

impl Signer for SecureEnclaveSigner {
    fn public(&self) -> VerifyingKey {
        self.public.clone()
    }

    fn home(&self) -> KeyHome {
        KeyHome::Hardware
    }

    fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
        let reason = current_reason().unwrap_or_else(|| default_reason(purpose, msg));
        self.sign_with_reason(purpose, msg, &reason)
    }

    fn asks_person(&self) -> bool {
        self.biometric
    }
}

/// Seals `secret` to the P-256 key `to` (ECIES), from its public half
/// alone, so any Mac can seal to any device's key.
pub fn wrap(to: &VerifyingKey, secret: &[u8]) -> R<Vec<u8>> {
    let VerifyingKey::P256(public) = to else {
        return Err(IdError::internal("a secret is sealed to a P-256 key only"));
    };
    let key = restore(Restore::Public(public))?;
    key.encrypt_data(SEAL, secret).map_err(|e| cf_error("sealing a secret", &e))
}

/// Opens what `wrap` sealed to `device`'s key. A biometric key asks Touch
/// ID, with the reason `crate::with_reason` set or one that names
/// unlocking the identity key.
pub fn unwrap(device: &SecureEnclaveSigner, ciphertext: &[u8]) -> R<Vec<u8>> {
    let reason = current_reason().unwrap_or_else(|| UNWRAP_REASON.to_owned());
    device.unwrap_with_reason(ciphertext, &reason)
}

/// Whether this Mac has a Secure Enclave XenoNet can make keys in: it
/// makes a non-biometric, non-permanent key once and drops it, which never
/// prompts. Intel Macs without a T2 chip have none.
pub fn has_secure_enclave() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| generate(false).is_ok())
}

/// Whether Touch ID is set up and usable now (LAContext's
/// `canEvaluatePolicy`, which never prompts). False with the lid closed,
/// after too many failed tries, or with no finger enrolled.
pub fn has_touch_id() -> bool {
    la::biometry_available()
}

fn generate(biometric: bool) -> R<SecKey> {
    let mut flags = kSecAccessControlPrivateKeyUsage;
    if biometric {
        flags |= kSecAccessControlBiometryCurrentSet;
    }
    let access = SecAccessControl::create_with_protection(Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly), flags)
        .map_err(|e| IdError::internal(format!("a Secure Enclave key's access control: {e}")))?;
    let mut options = GenerateKeyOptions::default();
    // No location: the key isn't kept in any keychain.
    options.set_key_type(KeyType::ec_sec_prime_random()).set_size_in_bits(256).set_token(Token::SecureEnclave).set_access_control(access);
    SecKey::new(&options).map_err(|e| cf_error("making a Secure Enclave key", &e))
}

/// A key's public half as SEC1 uncompressed bytes.
fn public_bytes(key: &SecKey) -> R<Vec<u8>> {
    let data = key.public_key().and_then(|p| p.external_representation()).ok_or_else(|| IdError::internal("a key's public half can't be read"))?;
    let bytes = data.bytes().to_vec();
    if bytes.len() == 65 && bytes[0] == 4 {
        Ok(bytes)
    } else {
        Err(IdError::internal("a key's public half isn't P-256"))
    }
}

/// The Secure Enclave's blob for `key`: its `toid` attribute.
#[expect(unsafe_code, reason = "a value of SecKeyCopyAttributes's dictionary, which security-framework hands back untyped")]
fn blob(key: &SecKey) -> Option<Vec<u8>> {
    let attributes = key.attributes();
    let value = attributes.find(CFString::from_static_string("toid").as_CFTypeRef())?;
    // SAFETY: the dictionary's values are CF objects, and `attributes` keeps
    // this one alive while it's wrapped; the get rule retains it for the
    // wrapper and releases it when it drops.
    let value = unsafe { CFType::wrap_under_get_rule(*value) };
    value.downcast::<CFData>().map(|d| d.bytes().to_vec())
}

enum Restore<'a> {
    /// A public key from its SEC1 bytes.
    Public(&'a [u8]),
    /// A Secure Enclave key from its blob, with the context its next use
    /// asks with.
    Enclave { toid: &'a [u8], context: Option<&'a CFType> },
}

/// `SecKeyCreateWithData`. For a Secure Enclave key the data is empty and
/// the attributes name the token and its blob, as the Security framework
/// itself restores a key from a keychain item.
#[expect(unsafe_code, reason = "SecKeyCreateWithData and the Security framework's attribute names, which security-framework doesn't wrap")]
fn restore(what: Restore<'_>) -> R<SecKey> {
    use security_framework_sys::item::{
        kSecAttrKeyClass, kSecAttrKeyClassPrivate, kSecAttrKeyClassPublic, kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom, kSecAttrTokenID, kSecAttrTokenIDSecureEnclave,
        kSecUseAuthenticationContext,
    };
    use security_framework_sys::key::SecKeyCreateWithData;
    let name = |s| {
        // SAFETY: `s` is one of the Security framework's constant CFStrings,
        // which live as long as the process; the get rule retains it.
        unsafe { CFString::wrap_under_get_rule(s) }
    };
    // SAFETY: reading the framework's extern constants, set when it loads.
    let (key_type, ec, class, public, private, token, enclave, context_key) = unsafe {
        (
            kSecAttrKeyType,
            kSecAttrKeyTypeECSECPrimeRandom,
            kSecAttrKeyClass,
            kSecAttrKeyClassPublic,
            kSecAttrKeyClassPrivate,
            kSecAttrTokenID,
            kSecAttrTokenIDSecureEnclave,
            kSecUseAuthenticationContext,
        )
    };
    let mut pairs: Vec<(CFString, CFType)> = vec![(name(key_type), name(ec).as_CFType())];
    let data = match what {
        Restore::Public(bytes) => {
            pairs.push((name(class), name(public).as_CFType()));
            CFData::from_buffer(bytes)
        }
        Restore::Enclave { toid, context } => {
            pairs.push((name(class), name(private).as_CFType()));
            pairs.push((name(token), name(enclave).as_CFType()));
            pairs.push((CFString::from_static_string("toid"), CFData::from_buffer(toid).as_CFType()));
            if let Some(context) = context {
                pairs.push((name(context_key), context.clone()));
            }
            CFData::from_buffer(&[])
        }
    };
    let attributes = CFDictionary::from_CFType_pairs(&pairs);
    let mut error = std::ptr::null_mut();
    // SAFETY: `data` and `attributes` are live CF objects for the call, and
    // `error` is an out pointer it sets only when it fails.
    let key = unsafe { SecKeyCreateWithData(data.as_concrete_TypeRef(), attributes.as_concrete_TypeRef(), &raw mut error) };
    if key.is_null() {
        if error.is_null() {
            return Err(IdError::internal("the Security framework restored no key"));
        }
        // SAFETY: a failed Create call hands back an error we own (+1).
        let error = unsafe { CFError::wrap_under_create_rule(error) };
        return Err(cf_error("restoring a key", &error));
    }
    // SAFETY: a key from a Create call is ours to release (+1).
    Ok(unsafe { SecKey::wrap_under_create_rule(key) })
}

fn open(key: &SecKey, ciphertext: &[u8]) -> R<Vec<u8>> {
    key.decrypt_data(SEAL, ciphertext).map_err(|e| cf_error("opening a sealed secret", &e))
}

/// A Security or LocalAuthentication error, as a refusal when the person
/// didn't confirm: errSecUserCanceled (-128), errSecAuthFailed (-25293),
/// and LAError's authenticationFailed (-1), userCancel (-2), userFallback
/// (-3), systemCancel (-4) and appCancel (-9).
fn cf_error(what: &str, e: &CFError) -> IdError {
    let code = e.code();
    if matches!(code, -128 | -25293 | -1 | -2 | -3 | -4 | -9) {
        IdError::refused(format!("{what}: you didn't confirm it"))
    } else {
        IdError::internal(format!("{what}: {} ({code})", e.description()))
    }
}

/// LocalAuthentication's `LAContext`, through the Objective-C runtime:
/// one to ask with a reason, and whether biometry can be asked at all.
/// Neither prompts by itself.
mod la {
    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use std::ffi::{c_char, c_void};

    type Id = *mut c_void;
    type Sel = *const c_void;

    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    // LAContext lives in LocalAuthentication.framework: linking it loads the
    // class for objc_getClass.
    #[link(name = "LocalAuthentication", kind = "framework")]
    extern "C" {}

    /// LAPolicyDeviceOwnerAuthenticationWithBiometrics: biometry alone, no
    /// password.
    const BIOMETRICS: isize = 1;

    /// A new LAContext (`[[LAContext alloc] init]`), or None when the class
    /// isn't there.
    #[expect(unsafe_code, reason = "Objective-C messages to LAContext, which no crate in the workspace wraps")]
    fn new_context() -> Option<Id> {
        // SAFETY: objc_msgSend is called through the exact signature of each
        // method it sends: +alloc and -init take no arguments and return an
        // object. The class name and selectors are NUL-terminated literals.
        unsafe {
            let class = objc_getClass(c"LAContext".as_ptr());
            if class.is_null() {
                return None;
            }
            let send = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel) -> Id>(objc_msgSend);
            let object = send(send(class, sel_registerName(c"alloc".as_ptr())), sel_registerName(c"init".as_ptr()));
            (!object.is_null()).then_some(object)
        }
    }

    /// A context whose Touch ID prompt reads `reason`, owned as a CF object
    /// (CFRelease releases an Objective-C object), to pass as
    /// `kSecUseAuthenticationContext`.
    #[expect(unsafe_code, reason = "Objective-C messages to LAContext, which no crate in the workspace wraps")]
    pub fn context(reason: &str) -> Option<CFType> {
        let object = new_context()?;
        // SAFETY: `object` is the +1 LAContext that new_context made, which
        // the wrapper now owns.
        let context = unsafe { CFType::wrap_under_create_rule(object.cast_const()) };
        let reason = CFString::new(reason);
        // SAFETY: -setLocalizedReason: takes one NSString (CFString is
        // toll-free bridged to it) and returns nothing; the context copies
        // it, and both outlive the call.
        unsafe {
            let send = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel, CFTypeRef)>(objc_msgSend);
            send(object, sel_registerName(c"setLocalizedReason:".as_ptr()), reason.as_CFTypeRef());
        }
        Some(context)
    }

    /// `-canEvaluatePolicy:error:` for biometry alone: whether Touch ID is
    /// enrolled and usable now. It never shows anything.
    #[expect(unsafe_code, reason = "Objective-C messages to LAContext, which no crate in the workspace wraps")]
    pub fn biometry_available() -> bool {
        // SAFETY: the pool brackets whatever the call autoreleases on this
        // thread. -canEvaluatePolicy:error: takes an NSInteger and an
        // NSError** (nil: no error wanted) and returns a BOOL, read as one
        // byte; -release takes nothing and returns nothing, and balances
        // the +1 from new_context.
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let answer = match new_context() {
                Some(object) => {
                    let can = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel, isize, *mut Id) -> i8>(objc_msgSend);
                    let yes = can(object, sel_registerName(c"canEvaluatePolicy:error:".as_ptr()), BIOMETRICS, std::ptr::null_mut()) != 0;
                    let release = std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel)>(objc_msgSend);
                    release(object, sel_registerName(c"release".as_ptr()));
                    yes
                }
                None => false,
            };
            objc_autoreleasePoolPop(pool);
            answer
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A software P-256 key, made in memory and never kept: it stands in for
    /// the Secure Enclave's, which makes and uses keys the same way.
    fn software_key() -> SecKey {
        let mut options = GenerateKeyOptions::default();
        options.set_key_type(KeyType::ec_sec_prime_random()).set_size_in_bits(256).set_token(Token::Software);
        SecKey::new(&options).unwrap()
    }

    #[test]
    fn what_the_security_framework_signs_verifies_as_p256() {
        let key = software_key();
        let public = VerifyingKey::P256(public_bytes(&key).unwrap());
        let sig = key.create_signature(SIGN, &Purpose::TOKEN_ROOT.bytes(b"root")).unwrap();
        assert!(public.verify(Purpose::TOKEN_ROOT, b"root", &sig));
        assert!(!public.verify(Purpose::ACT, b"root", &sig));
        assert!(VerifyingKey::parse(&public.to_json()).is_some());
    }

    #[test]
    fn a_secret_sealed_to_a_public_key_opens_with_its_private_key() {
        let key = software_key();
        let public = VerifyingKey::P256(public_bytes(&key).unwrap());
        let sealed = wrap(&public, b"a root seed of thirty-two bytes!").unwrap();
        assert_ne!(sealed.as_slice(), b"a root seed of thirty-two bytes!");
        assert_eq!(open(&key, &sealed).unwrap(), b"a root seed of thirty-two bytes!");
        assert!(open(&software_key(), &sealed).is_err(), "another key");
        assert!(wrap(&VerifyingKey::Ed25519([1; 32]), b"x").is_err(), "not a P-256 key");
    }

    #[test]
    fn a_file_that_isnt_an_enclave_keys_doesnt_load() {
        let dir = std::env::temp_dir().join(format!("xenonet-keys-macos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("k.key");
        std::fs::write(&file, r#"{"v":1,"seed":"AAAA"}"#).unwrap();
        assert!(SecureEnclaveSigner::load(&file).is_err());
        std::fs::write(&file, r#"{"v":1,"kind":"secure-enclave","biometric":false,"toid":"","public":"AAAA"}"#).unwrap();
        assert!(SecureEnclaveSigner::load(&file).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "makes a key in this Mac's Secure Enclave (non-biometric: it doesn't prompt, but it uses the hardware)"]
    fn a_secure_enclave_key_signs_unwraps_and_loads_again() {
        let dir = std::env::temp_dir().join(format!("xenonet-keys-se-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("machine-sign.key");
        let made = SecureEnclaveSigner::create(&file, false).unwrap();
        let loaded = SecureEnclaveSigner::load(&file).unwrap();
        assert_eq!(made.public(), loaded.public());
        assert!(!loaded.asks_person());
        let sig = loaded.sign(Purpose::ROSTER, b"entry").unwrap();
        assert!(made.public().verify(Purpose::ROSTER, b"entry", &sig));
        let sealed = wrap(&made.public(), b"seed").unwrap();
        assert_eq!(unwrap(&loaded, &sealed).unwrap(), b"seed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[ignore = "makes a biometric Secure Enclave key and asks Touch ID: run by hand, at the Mac"]
    fn a_biometric_key_asks_touch_id_to_sign() {
        let dir = std::env::temp_dir().join(format!("xenonet-keys-bio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("device-sign.key");
        let made = SecureEnclaveSigner::create(&file, true).unwrap();
        assert!(made.asks_person());
        let sig = made.sign_with_reason(Purpose::ACT, b"act", "test XenoNet's Touch ID key").unwrap();
        assert!(made.public().verify(Purpose::ACT, b"act", &sig));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
