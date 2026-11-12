//! A space folder's marker (`xenolith-space.json`; xenolith/FORMAT.md,
//! "The space marker"): which chain a folder in the shared context is,
//! what it's about and who owns it, signed by the context key of the
//! Xenolith that holds the chain. Xenolith writes it and XenoNet's map
//! reads it, so both take the one format from here.
//!
//! The folder is untrusted: any program can write the file. So the marker
//! is signed, and a reader trusts it only when the key that signed is one
//! it knows to be that Xenolith's (XenoNet: the `key(id)` in Xenolith's
//! actor grant). A marker decides nothing for Xenolith.

use super::{Owner, Space};
use serde_json::{json, Value};
use xenonet_id::codec::{b64, unb64};
use xenonet_id::sign::{Purpose, Signer, VerifyingKey};
use xenonet_model::ids::{EntityRef, SpaceId};

/// The file each space folder holds.
pub const FILE: &str = "xenolith-space.json";
/// What the signature covers first, so a marker can't be taken for a
/// chain entry, whose signed bytes are JSON.
const TAG: &str = "xenolith-space-marker/1";
/// The fields a marker has, and nothing else.
const FIELDS: [&str; 8] = ["v", "space", "about", "genesis", "owner", "ownerSigned", "key", "sig"];

/// A marker whose signature holds, read: what it says, and the key that
/// signed it. Whose key that is, the reader decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    /// The chain's `spc_`.
    pub space: String,
    /// What it's about, as an `EntityRef`'s text.
    pub about: String,
    pub genesis: String,
    /// `person:prs_…`, or `key:xns1_…` for a space a standalone owner key
    /// owns.
    pub owner: String,
    /// Whether the owner has signed an entry of the chain, not only been
    /// named by its founder (`View::owner_signed`): a space counts as its
    /// entity's only then.
    pub owner_signed: bool,
    pub key: VerifyingKey,
}

impl Marker {
    /// The person who owns the space, when a person does.
    pub fn person(&self) -> Option<&str> {
        self.owner.strip_prefix("person:")
    }

    /// What it's about, parsed.
    pub fn entity(&self) -> Option<EntityRef> {
        EntityRef::parse(&self.about).ok()
    }
}

fn owner_text(owner: &Owner) -> String {
    match owner {
        Owner::Person(p) => format!("person:{p}"),
        Owner::Key(k) => format!("key:{}", k.id()),
    }
}

fn message(space: &str, about: &str, genesis: &str, owner: &str, owner_signed: bool, key: &str) -> Vec<u8> {
    format!("{TAG}\n{space}\n{about}\n{genesis}\n{owner}\n{owner_signed}\n{key}").into_bytes()
}

/// `space`'s marker, signed by `context`, the context key of the Xenolith
/// that holds its chain.
pub fn sign(space: &Space, context: &dyn Signer) -> Result<Value, String> {
    let v = space.view();
    sign_parts(&v.id, &v.about.to_string(), &v.genesis, &owner_text(&v.owner), v.owner_signed, context)
}

/// A marker saying these parts, signed by `context`: `sign`'s, and a
/// test's that needs a marker for a chain it doesn't build.
pub fn sign_parts(space: &str, about: &str, genesis: &str, owner: &str, owner_signed: bool, context: &dyn Signer) -> Result<Value, String> {
    let key = context.public();
    let sig = context.sign(Purpose::SPACE, &message(space, about, genesis, owner, owner_signed, &key.id())).map_err(|e| e.message)?;
    Ok(json!({ "v": 1, "space": space, "about": about, "genesis": genesis, "owner": owner, "ownerSigned": owner_signed, "key": key.to_json(), "sig": b64(&sig) }))
}

/// What a marker says, when its signature holds by the key it names. A
/// field it doesn't have, one more, or a signature that doesn't hold is
/// None.
pub fn verify(v: &Value) -> Option<Marker> {
    let o = v.as_object()?;
    if o.len() != FIELDS.len() || o.keys().any(|k| !FIELDS.contains(&k.as_str())) || v["v"] != 1 {
        return None;
    }
    let text = |k: &str| v[k].as_str();
    let (space, about, genesis, owner) = (text("space")?, text("about")?, text("genesis")?, text("owner")?);
    let owner_signed = v["ownerSigned"].as_bool()?;
    let key = VerifyingKey::parse(&v["key"])?;
    let sig = unb64(text("sig")?)?;
    let ok = SpaceId::parse(space).is_ok() && key.verify(Purpose::SPACE, &message(space, about, genesis, owner, owner_signed, &key.id()), &sig);
    ok.then(|| Marker { space: space.into(), about: about.into(), genesis: genesis.into(), owner: owner.into(), owner_signed, key })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::{found, SpacePolicy};
    use xenonet_id::sign::{KeyHome, SeedSigner};
    use xenonet_model::ids::ProjectId;

    const PRS: &str = "prs_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn signer(byte: u8) -> SeedSigner {
        SeedSigner::from_seed(&[byte; 32], KeyHome::File).unwrap()
    }

    fn space(about: EntityRef, owner: Owner, owner_key: Option<&dyn Signer>, curator: &SeedSigner) -> Space {
        Space::found(SpacePolicy::default(), found(about, owner, curator, owner_key, "atlas", [7; 16], 1_792_512_000_000).unwrap()).unwrap()
    }

    #[test]
    fn a_marker_holds_by_its_key_and_an_edited_one_does_not() {
        let (curator, owner) = (signer(1), signer(2));
        let about = EntityRef::Name { kind: "project".into(), name: "atlas".into() };
        let s = space(about, Owner::Key(owner.public()), Some(&owner), &curator);
        let m = sign(&s, &curator).unwrap();
        let read = verify(&m).unwrap();
        assert_eq!((read.space.as_str(), read.about.as_str(), read.owner_signed), (s.view().id.as_str(), "name:project:atlas", true));
        assert_eq!((read.key.id(), read.owner.clone()), (curator.public().id(), format!("key:{}", owner.public().id())));
        assert_eq!(read.person(), None);
        for (k, v) in [("about", json!("prj_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")), ("owner", json!(format!("person:{PRS}"))), ("ownerSigned", json!(false)), ("v", json!(2))] {
            let mut edited = m.clone();
            edited[k] = v;
            assert!(verify(&edited).is_none(), "{k} edited");
        }
        let mut more = m.clone();
        more["note"] = json!("x");
        assert!(verify(&more).is_none(), "a field it doesn't have");
        let mut less = m.clone();
        less.as_object_mut().unwrap().remove("ownerSigned");
        assert!(verify(&less).is_none(), "a field it lacks");
    }

    #[test]
    fn a_space_founded_naming_a_person_says_they_signed_nothing() {
        let curator = signer(3);
        let about = EntityRef::Project(ProjectId::parse("prj_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap());
        let s = space(about, Owner::Person(PRS.into()), None, &curator);
        let read = verify(&sign(&s, &curator).unwrap()).unwrap();
        assert_eq!((read.person(), read.owner_signed), (Some(PRS), false));
        assert!(matches!(read.entity(), Some(EntityRef::Project(_))));
    }
}
