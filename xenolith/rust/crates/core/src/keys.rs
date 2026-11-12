//! The keys this Xenolith holds, each in a file of its own in <state>/keys/ that only its owner may
//! read (xenonet_os::private_file; on Windows the accounts named as readers too):
//!
//! - `context.key`, the context signing key (Ed25519). It signs what this Xenolith adds to a space:
//!   its notes, as curator its pages and chain entries, and its Hello to another Xenolith. Its id
//!   (`xns1_…`) is how the others name it.
//! - `transport.key`, its own endpoint's key between machines (Ed25519, so the iroh endpoint id),
//!   for QUIC alone. It signs no context, so a peer that has seen the endpoint can't speak for this
//!   Xenolith's context, and either key can change without the other.
//! - `owner-signing.key`, the standalone owner's key: it owns this machine's spaces while no person
//!   (XenoNet's) does. The service signs with it only for a request that carries the owner key
//!   (owner.rs). Any program running as you can read the file, so a space it owns is weaker
//!   (xenonet_log::space::Owner::Key): its chain lets it share into no network and move only to a
//!   person, never to another key, so an agent that reads the file can't take a space for a key of
//!   its own.
//!
//! A key file that is there and can't be read stops whoever loads it, with an error: a key is never
//! replaced by a new one, since a new key is a new identity to every peer.

use crate::owner::{self, OwnerKey};
use crate::paths::AppPaths;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use xenonet_id::codec::{b64, random, unb64_n};
pub use xenonet_id::sign::Signer;
use xenonet_id::sign::{KeyHome, SeedSigner, VerifyingKey};

/// The keys' folder: in the state folder, never the shared context folder (whose files anyone with
/// the folder can write).
pub fn dir(paths: &AppPaths) -> PathBuf {
    paths.state_dir.join("keys")
}

/// What each key is for, which its file names and records, so one can't be put to another use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Context,
    Transport,
    OwnerSigning,
}

impl Use {
    pub const ALL: [Use; 3] = [Use::Context, Use::Transport, Use::OwnerSigning];

    pub fn name(self) -> &'static str {
        match self {
            Use::Context => "context",
            Use::Transport => "transport",
            Use::OwnerSigning => "owner-signing",
        }
    }

    pub fn file(self, paths: &AppPaths) -> PathBuf {
        dir(paths).join(format!("{}.key", self.name()))
    }
}

fn public_of(seed: &[u8; 32]) -> Result<VerifyingKey, String> {
    SeedSigner::from_seed(seed, KeyHome::File).map(|k| k.public()).map_err(|e| e.message)
}

fn unreadable(file: &Path, why: &str) -> String {
    format!("{} {why}; it was left as it is (a new key would be a new identity), so move it aside to make one", file.display())
}

/// A key's seed from its file (`{v, use, seed, keyId}`), made when there's none, for its owner and
/// `readers` alone.
fn seed(paths: &AppPaths, what: Use, readers: &[String]) -> Result<[u8; 32], String> {
    let file = what.file(paths);
    for _ in 0..2 {
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                let v: Value = serde_json::from_str(&text).map_err(|_| unreadable(&file, "isn't a key file"))?;
                let seed = v["seed"].as_str().and_then(unb64_n::<32>).ok_or_else(|| unreadable(&file, "holds no key"))?;
                if v["use"] != what.name() {
                    return Err(unreadable(&file, &format!("is a key for {}, not for {}", v["use"].as_str().unwrap_or("something else"), what.name())));
                }
                if v["keyId"] != public_of(&seed)?.id().as_str() {
                    return Err(unreadable(&file, "names another key than the one it holds"));
                }
                return Ok(seed);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let seed = random::<32>().map_err(|e| e.message)?;
                let text = format!("{}\n", json!({ "v": 1, "use": what.name(), "seed": b64(&seed), "keyId": public_of(&seed)?.id() }));
                match xenonet_os::private_file::write_new(&file, text.as_bytes(), readers) {
                    Ok(()) => return Ok(seed),
                    // Another process made it first: read the one it made.
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return Err(format!("cannot write {}: {e}", file.display())),
                }
            }
            Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
        }
    }
    Err(format!("{} came and went while it was read", file.display()))
}

/// This Xenolith's own endpoint's key. Its secret goes to the transport alone.
pub struct TransportKey {
    seed: [u8; 32],
    public: VerifyingKey,
}

impl std::fmt::Debug for TransportKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[the transport key {}]", self.public.id())
    }
}

impl TransportKey {
    pub fn public(&self) -> &VerifyingKey {
        &self.public
    }

    /// The Ed25519 secret, for the endpoint (iroh's SecretKey) and nothing else.
    pub fn secret(&self) -> [u8; 32] {
        self.seed
    }
}

/// The standalone owner's signing key. It signs only for a request that presented the owner key the
/// running service made, so a process that can't read that file can't sign as the owner.
#[derive(Debug)]
pub struct OwnerSigner(SeedSigner);

impl OwnerSigner {
    pub fn public(&self) -> VerifyingKey {
        self.0.public()
    }

    /// The key to sign as the owner with, for a request whose `presented` owner key is the running
    /// service's.
    pub fn signer(&self, real: &OwnerKey, presented: &str) -> Result<&dyn Signer, String> {
        if !owner::matches(&real.key, presented) {
            return Err("signing as the owner needs the owner key".into());
        }
        Ok(&self.0)
    }
}

/// The three keys, as the service holds them.
#[derive(Debug)]
pub struct Keys {
    pub context: SeedSigner,
    pub transport: TransportKey,
    pub owner: OwnerSigner,
}

impl Keys {
    /// The keys in their files, each made when it's missing. `readers` may read the context key on
    /// Windows too (the service writes it and the curator, which runs as the user, signs with it);
    /// the transport and owner keys are the service's alone.
    pub fn load_or_create(paths: &AppPaths, readers: &[String]) -> Result<Keys, String> {
        let context = SeedSigner::from_seed(&seed(paths, Use::Context, readers)?, KeyHome::File).map_err(|e| e.message)?;
        let t = seed(paths, Use::Transport, &[])?;
        let transport = TransportKey { public: public_of(&t)?, seed: t };
        let owner = OwnerSigner(SeedSigner::from_seed(&seed(paths, Use::OwnerSigning, &[])?, KeyHome::File).map_err(|e| e.message)?);
        Ok(Keys { context, transport, owner })
    }

    /// What /status shows: each key's id and where it lives. Never a secret.
    pub fn describe(&self) -> Value {
        json!({
            "context": { "id": self.context.public().id(), "home": self.context.home().as_str() },
            "transport": { "id": self.transport.public().id(), "home": KeyHome::File.as_str() },
            // A file any program running as you can read: what it owns is weaker.
            "owner": { "id": self.owner.public().id(), "home": KeyHome::File.as_str(), "weaker": true },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> (PathBuf, AppPaths) {
        let home = std::env::temp_dir().join(format!("xenolith-keys-{}", crate::text::random_hex(8)));
        let env = [("XENOLITH_HOME".to_string(), home.to_string_lossy().to_string())].into_iter().collect();
        (home.clone(), AppPaths::from_env(&env))
    }

    #[test]
    fn three_keys_each_for_its_own_owner_alone_and_the_same_after_a_restart() {
        let (home, paths) = home();
        let a = Keys::load_or_create(&paths, &[]).unwrap();
        let ids = [a.context.public().id(), a.transport.public().id(), a.owner.public().id()];
        assert!(ids[0] != ids[1] && ids[1] != ids[2] && ids[0] != ids[2], "three keys, not one: {ids:?}");
        let b = Keys::load_or_create(&paths, &[]).unwrap();
        assert_eq!([b.context.public().id(), b.transport.public().id(), b.owner.public().id()], ids, "loaded, not made again");
        assert_eq!(a.transport.secret(), b.transport.secret());
        let d = a.describe();
        assert_eq!(d["owner"]["weaker"], true);
        assert_eq!(d["context"]["home"], "file");
        assert!(!d.to_string().contains(&b64(&a.transport.secret())), "no secret in what /status shows");
        let mut never = |_: &str, _: &[&str]| -> Result<String, String> { unreachable!("no commands off Windows") };
        for u in Use::ALL {
            let f = u.file(&paths);
            if cfg!(windows) {
                let mut run = |c: &str, args: &[&str]| xenonet_os::private_file::run_private(c, args);
                assert_eq!(xenonet_os::private_file::other_readers(&f.to_string_lossy(), true, &mut run).unwrap(), Vec::<String>::new(), "{}", f.display());
            } else {
                assert_eq!(xenonet_os::private_file::other_readers(&f.to_string_lossy(), false, &mut never).unwrap(), Vec::<String>::new(), "{}", f.display());
            }
        }
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_key_file_that_cant_be_read_is_never_replaced() {
        let (home, paths) = home();
        Keys::load_or_create(&paths, &[]).unwrap();
        let context = Use::Context.file(&paths);
        let transport = std::fs::read_to_string(Use::Transport.file(&paths)).unwrap();
        for (text, why) in [("not json", "isn't a key file"), (transport.as_str(), "is a key for transport, not for context")] {
            std::fs::write(&context, text).unwrap();
            let e = Keys::load_or_create(&paths, &[]).unwrap_err();
            assert!(e.contains(why), "{e}");
            assert_eq!(std::fs::read_to_string(&context).unwrap(), text, "left as it is");
        }
        let mut v: Value = serde_json::from_str(&transport).unwrap();
        v["use"] = json!("context");
        v["keyId"] = json!("xns1_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        std::fs::write(&context, v.to_string()).unwrap();
        assert!(Keys::load_or_create(&paths, &[]).unwrap_err().contains("names another key"));
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_owner_key_signs_only_with_the_owner_key_presented() {
        let (home, paths) = home();
        let keys = Keys::load_or_create(&paths, &[]).unwrap();
        let real = owner::create(&paths, 47999, &[]).unwrap();
        assert!(keys.owner.signer(&real, "0".repeat(64).as_str()).is_err());
        assert!(keys.owner.signer(&real, "").is_err());
        let signer = keys.owner.signer(&real, &real.key).unwrap();
        assert_eq!(signer.public(), keys.owner.public());
        assert_eq!(signer.home(), KeyHome::File, "a file: what it owns is weaker");
        let _ = std::fs::remove_dir_all(home);
    }

    /// Revision 21, with this Xenolith's own keys under XenoNet's space policy: an agent that can
    /// read the standalone owner key can sign as it, and still can't move a space to a key of its
    /// own, or share it into a network.
    #[test]
    fn a_space_the_standalone_key_owns_goes_to_a_person_or_nowhere() {
        use xenonet_log::space::{Level, Op, Owner, Space, SpacePolicy, Subject, found};
        use xenonet_model::ids::EntityRef;
        let (home, paths) = home();
        let keys = Keys::load_or_create(&paths, &[]).unwrap();
        let real = owner::create(&paths, 47999, &[]).unwrap();
        let owner = keys.owner.signer(&real, &real.key).unwrap();
        let about = EntityRef::Name { kind: "project".into(), name: "atlas".into() };
        let g = found(about, Owner::Key(keys.owner.public()), &keys.context, Some(owner), "atlas", [7; 16], 1).unwrap();
        let mut sp = Space::found(SpacePolicy::default(), g).unwrap();
        assert!(sp.view().weaker(), "a key-owned space is weaker");
        let signed = |mut e: xenonet_log::chain::Entry<SpacePolicy>| {
            e.sign(&keys.owner.public().id(), owner, None).unwrap();
            e.sign(&keys.context.public().id(), &keys.context, None).unwrap();
            e
        };
        // What the key may do, it does: share with another standalone Xenolith.
        let agent = SeedSigner::from_seed(&[9; 32], KeyHome::File).unwrap();
        let peer = Op::Share { subject: Subject::Key(agent.public().id()), level: Level::Append, endpoint: Some("ab".repeat(32)), after: vec![] };
        assert_eq!(sp.offer(signed(sp.next(peer, None, 2))).unwrap(), xenonet_log::space::Placed::Applied);
        // An agent's own key, as the new owner: refused, though the owner and the curator sign it.
        assert!(sp.offer(signed(sp.next(Op::MoveOwner { to: agent.public().id() }, None, 2))).is_err());
        assert_eq!(sp.view().owner, Owner::Key(keys.owner.public()));
        // Nor into a network.
        let net = Subject::Network("net_00112233445566778899aabbccddeeff".into());
        assert!(sp.offer(signed(sp.next(Op::Share { subject: net, level: Level::Read, endpoint: None, after: vec![] }, None, 2))).is_err());
        let _ = std::fs::remove_dir_all(home);
    }
}
