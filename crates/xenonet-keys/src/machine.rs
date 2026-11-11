//! The machine signing key in a secure element that keeps keys by name
//! (the TPM on Windows), and the rule for when a file key stands in
//! (docs/design/direction.md, "The machine key"; phase 6, revision 32).
//!
//! The machine's identity rests on this key, and there is no rekey, so a
//! key that can't be had is never quietly replaced by a new one:
//!
//! - A key file names where the key is. A file that names a TPM key opens
//!   that key or fails, saying why; it never falls back to a file key.
//! - With no key file, the key is made in the element when the element is
//!   there. A key by this folder's name already in the element (made by an
//!   earlier start that couldn't write the file) is the same identity, so
//!   it's kept. Only when the element is missing, or refuses to make a key,
//!   is a file key made, which is weaker (`KeyHome::File`), and its file
//!   says why the element didn't hold it.
//! - A file key stays a file key: one made before the element was used
//!   isn't moved into it.
//!
//! `NamedKeys` is the element as this rule sees it, so a fake stands in for
//! the TPM in the tests on every platform.

use crate::{enclave, held, Held, MACHINE_FILE};
use serde_json::{json, Map, Value};
use std::path::Path;
use std::time::Instant;
use xenonet_id::codec::{b64, hex, random, sha256, unb64};
use xenonet_id::keys::write_private;
use xenonet_id::sign::{KeyHome, Purpose, SeedSigner, Signer, VerifyingKey};
use xenonet_id::{IdError, R};

/// What a TPM key's file says it is (`kind`).
pub const TPM: &str = "tpm";

/// A secure element that keeps non-exportable P-256 keys by name, in the
/// store of the account XenoNet runs as. Nothing here prompts.
pub trait NamedKeys {
    /// Whether the element is there for XenoNet to use.
    fn present(&self) -> bool;
    /// The account whose key store new keys go in and keys open from.
    fn account(&self) -> String;
    /// The key called `name`, or None when the store has none by that name.
    fn open(&self, name: &str) -> R<Option<Box<dyn Signer>>>;
    /// A new key called `name`, which can't be exported. Fails when there's
    /// one by that name already.
    fn create(&self, name: &str) -> R<Box<dyn Signer>>;
}

/// The name of the machine signing key for the key folder `dir`: each
/// instance's folder has its own, so the installed XenoNet and XenoNet Dev
/// never share a key.
pub fn key_name(dir: &Path) -> String {
    format!("XenoNet machine signing key {}", hex(&sha256(dir.to_string_lossy().as_bytes())[..8]))
}

/// A TPM key's file: its name, the account whose store holds it, and its
/// public key, which the key opened from the TPM must match.
struct Named {
    name: String,
    account: String,
    public: VerifyingKey,
}

impl Named {
    fn read(file: &Path) -> R<Named> {
        let bad = || IdError::internal(format!("{} isn't a readable TPM key's file", file.display()));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(file)?).map_err(|_| bad())?;
        let s = |k: &str| v[k].as_str().map(str::to_owned).ok_or_else(bad);
        let public = s("public")?;
        let public = unb64(&public).filter(|p| p.len() == 65 && p[0] == 4).ok_or_else(bad)?;
        Ok(Named { name: s("name")?, account: s("account")?, public: VerifyingKey::P256(public) })
    }

    fn write(&self, file: &Path) -> R<()> {
        let v = json!({ "v": 1, "kind": TPM, "name": self.name, "account": self.account, "public": b64(self.public.bytes()), "keyId": self.public.id() });
        write_private(file, format!("{v}\n").as_bytes())
    }
}

/// The machine signing key in `dir`, by the rule above. `element` is the
/// TPM where this build has one; macOS's Secure Enclave key is made by the
/// caller and only opened here.
pub(crate) fn signer(dir: &Path, element: Option<&dyn NamedKeys>) -> R<Box<dyn Signer>> {
    let file = dir.join(MACHINE_FILE);
    match held(&file)? {
        Some(Held::Enclave) => enclave(&file),
        Some(Held::Tpm) => open_named(&file, element),
        Some(Held::Seed) => Ok(Box::new(SeedSigner::load_or_create(&file)?)),
        None => {
            let why = match element {
                Some(el) if el.present() => match make_named(dir, &file, el)? {
                    Ok(key) => return Ok(key),
                    Err(why) => why,
                },
                Some(_) => "this machine has no TPM XenoNet can use".to_owned(),
                None => "this machine has no secure element XenoNet uses".to_owned(),
            };
            file_key(&file, &why)
        }
    }
}

/// The TPM key `file` names, or an error that says why it can't be had.
fn open_named(file: &Path, element: Option<&dyn NamedKeys>) -> R<Box<dyn Signer>> {
    let named = Named::read(file)?;
    let refuse = |why: String| {
        IdError::internal(format!(
            "this machine's signing key is {} in the TPM, and {why}. XenoNet won't make a new key, since this machine's identity rests on it; xenonet doctor says what to do",
            named.name
        ))
    };
    let Some(el) = element else { return Err(refuse("only Windows opens a TPM key".into())) };
    match el.open(&named.name) {
        Ok(Some(key)) if key.public() == named.public => Ok(key),
        Ok(Some(key)) => Err(refuse(format!("the TPM's key by that name is {}, not {} as {} says", key.public().id(), named.public.id(), file.display()))),
        Ok(None) if el.account() != named.account => Err(refuse(format!("it's in the key store of {}, while XenoNet runs as {}", named.account, el.account()))),
        Ok(None) => Err(refuse("the TPM doesn't have it any more (a TPM that was cleared or reset loses its keys)".into())),
        Err(e) => Err(refuse(format!("the TPM didn't open it: {}", e.message))),
    }
}

/// A new key in the element, its file written: Ok(Err(why)) when the
/// element couldn't make one, and a file key stands in. A key that's there
/// by this name but fails to open is an error, not a reason for a file key,
/// since it may be this machine's identity.
fn make_named(dir: &Path, file: &Path, el: &dyn NamedKeys) -> R<Result<Box<dyn Signer>, String>> {
    let name = key_name(dir);
    let opened = |e: IdError| IdError::internal(format!("the TPM has a key called {name} that didn't open: {}", e.message));
    let key = match el.open(&name).map_err(opened)? {
        Some(key) => key,
        None => match el.create(&name) {
            Ok(key) => key,
            // Another XenoNet of this folder made it just now, or the TPM
            // can't make keys.
            Err(e) => match el.open(&name).map_err(opened)? {
                Some(key) => key,
                None => return Ok(Err(format!("the TPM didn't make a key: {}", e.message))),
            },
        },
    };
    let public = key.public();
    if !matches!(public, VerifyingKey::P256(_)) {
        return Err(IdError::internal(format!("the TPM's key {name} isn't a P-256 key")));
    }
    Named { name, account: el.account(), public }.write(file)?;
    Ok(Ok(key))
}

/// A file key, its file saying why no element holds it.
fn file_key(file: &Path, why: &str) -> R<Box<dyn Signer>> {
    let seed = random::<32>()?;
    let key = SeedSigner::from_seed(&seed, KeyHome::File)?;
    let v = json!({ "v": 1, "seed": b64(&seed), "keyId": key.public().id(), "weaker": why });
    write_private(file, format!("{v}\n").as_bytes())?;
    Ok(Box::new(key))
}

/// The doctor's row for the machine signing key in `dir` (its verdict:
/// status, detail and fix), or None before the key is made. Opening a key
/// and timing one signature never prompts.
pub(crate) fn standing(dir: &Path, element: Option<&dyn NamedKeys>) -> Option<Value> {
    let file = dir.join(MACHINE_FILE);
    let verdict = |status: &str, detail: String, fix: Option<String>| {
        let mut m = Map::new();
        m.insert("status".into(), json!(status));
        m.insert("detail".into(), json!(detail));
        if let Some(f) = fix {
            m.insert("fix".into(), json!(f));
        }
        Value::Object(m)
    };
    let kind = match held(&file) {
        Ok(None) => return None,
        Ok(Some(k)) => k,
        Err(e) => return Some(verdict("fail", e.message, Some(format!("{} isn't a key XenoNet can read: restore it from a backup", file.display())))),
    };
    let row = match kind {
        Held::Seed => {
            let why = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|v| v["weaker"].as_str().map(str::to_owned));
            let why = why.unwrap_or_else(|| match element {
                Some(el) if el.present() => "it was made before XenoNet kept this key in the TPM, and a key that exists isn't moved".to_owned(),
                _ => "it was made where XenoNet had no secure element to keep it in".to_owned(),
            });
            verdict("info", format!("a file key, which is weaker: any program of this account that reads it can sign as this machine ({why})"), None)
        }
        Held::Enclave => match enclave(&file) {
            Ok(key) => signs(key.as_ref(), "in the Secure Enclave: it can't be copied off this Mac, though any program of this account can ask it to sign".into()),
            Err(e) => verdict(
                "fail",
                e.message,
                Some(
                    "If this Mac's Secure Enclave was reset, the key is gone: remove machine-sign.key from XenoNet's identity folder and add this machine to its network again."
                        .into(),
                ),
            ),
        },
        Held::Tpm => {
            let account = Named::read(&file).map(|n| n.account).unwrap_or_default();
            match open_named(&file, element) {
                Ok(key) => signs(
                    key.as_ref(),
                    format!("in the TPM, in the key store of {account}, the account XenoNet runs as: it can't be copied off this machine, though any program of this account can ask the TPM to sign with it"),
                ),
                Err(e) => verdict(
                    "fail",
                    e.message,
                    Some(format!(
                        "Run XenoNet as {account}, whose key store holds the key. If the TPM was cleared or reset, the key is gone for good: remove machine-sign.key from XenoNet's identity folder and add this machine to its network again."
                    )),
                ),
            }
        }
    };
    Some(row)
}

/// A key in hardware that opened: ok with how long one probe signature
/// took, or a failure when it doesn't sign.
fn signs(key: &dyn Signer, detail: String) -> Value {
    let started = Instant::now();
    match key.sign(Purpose::PROBE, b"") {
        Ok(_) => json!({ "status": "ok", "detail": format!("{detail}; one signature took {} ms", started.elapsed().as_millis()) }),
        Err(e) => json!({ "status": "fail", "detail": format!("the key opens but doesn't sign: {}", e.message) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A TPM in memory: named software keys, which it can be told to lose,
    /// to fail to open, or to refuse to make.
    struct FakeTpm {
        present: bool,
        account: String,
        keys: Mutex<HashMap<String, Vec<u8>>>,
        refuse_create: bool,
        fail_open: bool,
    }

    impl FakeTpm {
        fn new() -> FakeTpm {
            FakeTpm { present: true, account: "NEO-REX\\will".into(), keys: Mutex::new(HashMap::new()), refuse_create: false, fail_open: false }
        }
    }

    /// A P-256 key from ring, standing in for the TPM's.
    struct Soft(ring::signature::EcdsaKeyPair);

    impl Soft {
        fn pkcs8() -> Vec<u8> {
            ring::signature::EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &ring::rand::SystemRandom::new()).unwrap().as_ref().to_vec()
        }
        fn new(pkcs8: &[u8]) -> Soft {
            Soft(ring::signature::EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8, &ring::rand::SystemRandom::new()).unwrap())
        }
    }

    impl Signer for Soft {
        fn public(&self) -> VerifyingKey {
            use ring::signature::KeyPair;
            VerifyingKey::P256(self.0.public_key().as_ref().to_vec())
        }
        fn home(&self) -> KeyHome {
            KeyHome::Hardware
        }
        fn sign(&self, purpose: Purpose, msg: &[u8]) -> R<Vec<u8>> {
            Ok(self.0.sign(&ring::rand::SystemRandom::new(), &purpose.bytes(msg)).unwrap().as_ref().to_vec())
        }
    }

    impl NamedKeys for FakeTpm {
        fn present(&self) -> bool {
            self.present
        }
        fn account(&self) -> String {
            self.account.clone()
        }
        fn open(&self, name: &str) -> R<Option<Box<dyn Signer>>> {
            if self.fail_open {
                return Err(IdError::internal("TPM_E_DEACTIVATED"));
            }
            Ok(self.keys.lock().unwrap().get(name).map(|s| Box::new(Soft::new(s)) as Box<dyn Signer>))
        }
        fn create(&self, name: &str) -> R<Box<dyn Signer>> {
            if self.refuse_create {
                return Err(IdError::internal("NTE_DEVICE_NOT_FOUND"));
            }
            let mut keys = self.keys.lock().unwrap();
            if keys.contains_key(name) {
                return Err(IdError::internal("NTE_EXISTS"));
            }
            let pkcs8 = Soft::pkcs8();
            let key = Soft::new(&pkcs8);
            keys.insert(name.to_owned(), pkcs8);
            Ok(Box::new(key))
        }
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("xenonet-keys-machine-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn status(row: Option<&Value>) -> &str {
        row.and_then(|r| r["status"].as_str()).unwrap_or("none")
    }

    #[test]
    fn a_tpm_key_is_made_once_and_opened_after() {
        let d = dir("made");
        let tpm = FakeTpm::new();
        assert_eq!(standing(&d, Some(&tpm)), None, "no key, no row");
        let key = signer(&d, Some(&tpm)).unwrap();
        assert_eq!(key.home(), KeyHome::Hardware);
        assert!(matches!(key.public(), VerifyingKey::P256(_)));
        let again = signer(&d, Some(&tpm)).unwrap();
        assert_eq!(again.public(), key.public(), "the same key, opened by name");
        let sig = again.sign(Purpose::FEED, b"entry").unwrap();
        assert!(key.public().verify(Purpose::FEED, b"entry", &sig));
        let file: Value = serde_json::from_str(&std::fs::read_to_string(d.join(MACHINE_FILE)).unwrap()).unwrap();
        assert_eq!((file["kind"].as_str(), file["account"].as_str()), (Some(TPM), Some("NEO-REX\\will")));
        assert_eq!(file["name"].as_str(), Some(key_name(&d).as_str()));
        assert!(file.get("seed").is_none(), "no secret in the file");
        let row = standing(&d, Some(&tpm));
        assert_eq!(status(row.as_ref()), "ok", "{row:?}");
        let detail = row.as_ref().unwrap()["detail"].as_str().unwrap();
        assert!(detail.starts_with("in the TPM, in the key store of NEO-REX\\will") && detail.contains("one signature took"), "{detail}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_tpm_key_that_cant_be_had_is_refused_never_replaced() {
        let d = dir("refused");
        let tpm = FakeTpm::new();
        let key = signer(&d, Some(&tpm)).unwrap();
        let file = std::fs::read(d.join(MACHINE_FILE)).unwrap();
        let unchanged = || assert_eq!(std::fs::read(d.join(MACHINE_FILE)).unwrap(), file, "the key file is left as it was");

        // The TPM was cleared.
        let cleared = FakeTpm::new();
        let e = signer(&d, Some(&cleared)).err().unwrap();
        assert!(e.message.contains("doesn't have it any more") && e.message.contains("won't make a new key"), "{}", e.message);
        assert!(cleared.keys.lock().unwrap().is_empty(), "no new key in the TPM");
        unchanged();
        let row = standing(&d, Some(&cleared));
        assert_eq!(status(row.as_ref()), "fail", "{row:?}");
        assert!(row.as_ref().unwrap()["fix"].as_str().unwrap().contains("NEO-REX\\will"), "{row:?}");

        // XenoNet runs as another account.
        let other = FakeTpm { account: "NEO-REX\\svc".into(), ..FakeTpm::new() };
        let e = signer(&d, Some(&other)).err().unwrap();
        assert!(e.message.contains("key store of NEO-REX\\will, while XenoNet runs as NEO-REX\\svc"), "{}", e.message);

        // The TPM fails, or isn't there now.
        let failing = FakeTpm { fail_open: true, keys: Mutex::new(tpm.keys.lock().unwrap().clone()), ..FakeTpm::new() };
        assert!(signer(&d, Some(&failing)).err().unwrap().message.contains("didn't open it: TPM_E_DEACTIVATED"));
        assert!(signer(&d, None).err().unwrap().message.contains("only Windows opens a TPM key"));
        assert_eq!(status(standing(&d, None).as_ref()), "fail");

        // Another key by the same name isn't this machine's.
        let swapped = FakeTpm::new();
        swapped.create(&key_name(&d)).unwrap();
        let e = signer(&d, Some(&swapped)).err().unwrap();
        assert!(e.message.contains("the TPM's key by that name is") && e.message.contains(&key.public().id()), "{}", e.message);
        unchanged();

        // And the TPM that has it still opens it.
        assert_eq!(signer(&d, Some(&tpm)).unwrap().public(), key.public());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_key_only_where_no_tpm_was_used_and_marked_weaker() {
        // No TPM: a file key, which says so.
        let d = dir("none");
        let absent = FakeTpm { present: false, ..FakeTpm::new() };
        let key = signer(&d, Some(&absent)).unwrap();
        assert_eq!(key.home(), KeyHome::File);
        let row = standing(&d, Some(&absent));
        assert_eq!(status(row.as_ref()), "info");
        assert!(row.as_ref().unwrap()["detail"].as_str().unwrap().contains("weaker") && row.as_ref().unwrap()["detail"].as_str().unwrap().contains("no TPM"), "{row:?}");

        // A TPM there later doesn't move the key: the identity stays.
        let tpm = FakeTpm::new();
        assert_eq!(signer(&d, Some(&tpm)).unwrap().public(), key.public());
        assert!(tpm.keys.lock().unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);

        // A TPM that won't make a key: a file key, and the reason kept.
        let d = dir("refusing");
        let refusing = FakeTpm { refuse_create: true, ..FakeTpm::new() };
        assert_eq!(signer(&d, Some(&refusing)).unwrap().home(), KeyHome::File);
        let detail = standing(&d, Some(&refusing)).unwrap()["detail"].as_str().unwrap().to_owned();
        assert!(detail.contains("the TPM didn't make a key: NTE_DEVICE_NOT_FOUND"), "{detail}");
        let _ = std::fs::remove_dir_all(&d);

        // No element in this build at all (Linux).
        let d = dir("linux");
        assert_eq!(signer(&d, None).unwrap().home(), KeyHome::File);
        assert!(standing(&d, None).unwrap()["detail"].as_str().unwrap().contains("no secure element"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_key_in_the_tpm_without_its_file_is_kept_and_one_that_fails_is_no_reason_for_a_file_key() {
        // A start made the key and didn't write its file: the next keeps it.
        let d = dir("orphan");
        let tpm = FakeTpm::new();
        let made = tpm.create(&key_name(&d)).unwrap();
        assert_eq!(signer(&d, Some(&tpm)).unwrap().public(), made.public());
        assert_eq!(tpm.keys.lock().unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&d);

        // A TPM that fails to open by name: an error, no file key.
        let d = dir("failing");
        let failing = FakeTpm { fail_open: true, ..FakeTpm::new() };
        assert!(signer(&d, Some(&failing)).is_err());
        assert!(!d.join(MACHINE_FILE).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn each_folder_names_its_own_key() {
        assert_ne!(key_name(Path::new("C:\\Users\\w\\xenonet\\identity")), key_name(Path::new("C:\\Users\\w\\xenonet-dev\\identity")));
        assert!(key_name(Path::new("x")).starts_with("XenoNet machine signing key "));
    }
}
