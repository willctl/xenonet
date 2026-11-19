use crate::{EidolonId, FacetId, ModelError, Wall};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use xenonet_id::codec::{b64, unb64_n};
use xenonet_id::sign::VerifyingKey;

/// A machine's Ed25519 public key, written as base64url.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HolderKey([u8; 32]);

impl HolderKey {
    pub fn from_bytes(bytes: [u8; 32]) -> HolderKey {
        HolderKey(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The key as xenonet-id checks signatures with.
    pub fn verifying_key(&self) -> VerifyingKey {
        VerifyingKey::Ed25519(self.0)
    }

    /// The key from xenonet-id, when it is an Ed25519 one.
    pub fn from_verifying_key(key: &VerifyingKey) -> Option<HolderKey> {
        match key {
            VerifyingKey::Ed25519(k) => Some(HolderKey(*k)),
            VerifyingKey::P256(_) => None,
        }
    }

    /// The base64url text, as it appears in JSON and in a seat's signed bytes.
    pub fn to_b64(&self) -> String {
        b64(&self.0)
    }
}

impl fmt::Debug for HolderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HolderKey({})", self.to_b64())
    }
}

impl Serialize for HolderKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_b64())
    }
}

impl<'de> Deserialize<'de> for HolderKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<HolderKey, D::Error> {
        let text = String::deserialize(d)?;
        key32(&text).map(HolderKey).ok_or_else(|| serde::de::Error::custom(ModelError::BadKey { what: "holder key" }))
    }
}

/// 32 bytes from base64url, accepted only in the canonical spelling.
fn key32(text: &str) -> Option<[u8; 32]> {
    unb64_n::<32>(text).filter(|k| b64(k) == text)
}

mod b64_32 {
    use super::{b64, key32, Deserialize, Deserializer, ModelError, Serializer};

    pub fn serialize<S: Serializer>(k: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&b64(k))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let text = String::deserialize(d)?;
        key32(&text).ok_or_else(|| serde::de::Error::custom(ModelError::BadKey { what: "key" }))
    }
}

/// A machine that may hold the eidolon: a label, the key that signs its
/// seats, and the X25519 key the eidolon's key is wrapped to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    /// A label such as `desk`.
    pub name: String,
    pub sign: HolderKey,
    #[serde(with = "b64_32")]
    pub seal: [u8; 32],
}

/// What a facet is. More kinds (a git identity, ssh keys) can follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetKind {
    Folder,
    Browser,
}

/// One part of an eidolon's state and where it opens inside a cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FacetSpec {
    pub id: FacetId,
    pub kind: FacetKind,
    /// `a-z 0-9 -`, 1 to 48.
    pub name: String,
    /// Where it opens inside a cell: an absolute guest path.
    pub path: String,
    /// Relative glob-like patterns for what the facet does not carry,
    /// such as `cache2/**`.
    pub leave_out: Vec<String>,
}

impl FacetSpec {
    /// Whether the name and path are in their formats. Leave-out patterns
    /// are the policy's to judge (`eidolon-policy`).
    pub fn validate(&self) -> Result<(), ModelError> {
        let name_ok = (1..=48).contains(&self.name.len()) && self.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !name_ok {
            return Err(ModelError::BadValue { what: "facet name", rule: "1 to 48 of a-z, 0-9 or -" });
        }
        if !self.path.starts_with('/') || self.path.split('/').any(|part| part == "..") || self.path.contains('\0') {
            return Err(ModelError::BadValue { what: "facet path", rule: "an absolute path without `..`" });
        }
        Ok(())
    }
}

/// An eidolon as its owner declares it: the wall it opens behind, the
/// machines that may hold it and its facets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EidolonSpec {
    pub id: EidolonId,
    pub wall: Wall,
    pub holders: Vec<Holder>,
    pub facets: Vec<FacetSpec>,
}

impl EidolonSpec {
    /// Every facet valid and no two facets sharing a name or an id.
    pub fn validate(&self) -> Result<(), ModelError> {
        for (i, f) in self.facets.iter().enumerate() {
            f.validate()?;
            if self.facets[..i].iter().any(|g| g.name == f.name || g.id == f.id) {
                return Err(ModelError::BadValue { what: "facet list", rule: "free of repeated names and ids" });
            }
        }
        Ok(())
    }

    /// Whether this machine's key is on the holder list.
    pub fn holder_by_key(&self, key: &HolderKey) -> Option<&Holder> {
        self.holders.iter().find(|h| h.sign == *key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facet(name: &str, path: &str) -> FacetSpec {
        FacetSpec { id: FacetId::from_bytes([2; 16]), kind: FacetKind::Folder, name: name.into(), path: path.into(), leave_out: vec![] }
    }

    #[test]
    fn facet_names_and_paths_are_checked() {
        assert!(facet("notes", "/home/agent/notes").validate().is_ok());
        for name in ["", "Notes", "a_b", &"a".repeat(49)] {
            assert!(facet(name, "/x").validate().is_err(), "{name:?}");
        }
        for path in ["relative", "/a/../b", ""] {
            assert!(facet("a", path).validate().is_err(), "{path:?}");
        }
    }

    #[test]
    fn two_facets_may_not_share_a_name() {
        let spec = EidolonSpec {
            id: EidolonId::from_bytes([1; 16]),
            wall: "wall_work".parse().unwrap(),
            holders: vec![],
            facets: vec![facet("a", "/a"), FacetSpec { id: FacetId::from_bytes([3; 16]), ..facet("a", "/b") }],
        };
        assert!(spec.validate().is_err());
    }

    #[test]
    fn keys_have_one_spelling() {
        let k = HolderKey::from_bytes([9; 32]);
        let json = serde_json::to_string(&k).unwrap_or_default();
        assert_eq!(serde_json::from_str::<HolderKey>(&json).ok(), Some(k));
        assert!(serde_json::from_str::<HolderKey>("\"AAAA\"").is_err());
        assert!(serde_json::from_str::<HolderKey>(&format!("\"{}=\"", k.to_b64())).is_err(), "padding");
        assert_eq!(HolderKey::from_verifying_key(&k.verifying_key()), Some(k));
    }
}
