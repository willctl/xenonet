use crate::ModelError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use xenonet_id::codec::{base32, random};

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// RFC 4648 base32, lowercase, no padding, back to 16 bytes. Only the
/// canonical text of 16 bytes passes, so an id has one spelling.
fn unbase32_16(text: &str) -> Option<[u8; 16]> {
    if text.len() != 26 {
        return None;
    }
    let (mut buffer, mut bits) = (0u32, 0u32);
    let mut out = Vec::with_capacity(16);
    for c in text.bytes() {
        let v = ALPHABET.iter().position(|a| *a == c)?;
        buffer = (buffer << 5) | u32::try_from(v).ok()?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
            buffer &= (1 << bits) - 1;
        }
    }
    let bytes: [u8; 16] = out.try_into().ok()?;
    (base32(&bytes) == text).then_some(bytes)
}

macro_rules! typed_id {
    ($name:ident, $prefix:literal, $what:literal) => {
        #[doc = concat!("A ", $what, " id: `", $prefix, "` and 26 lowercase base32 characters of 16 random bytes.")]
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name([u8; 16]);

        impl $name {
            /// A new random id.
            pub fn new_random() -> Result<$name, ModelError> {
                random::<16>().map($name).map_err(|_| ModelError::Random)
            }

            pub fn from_bytes(bytes: [u8; 16]) -> $name {
                $name(bytes)
            }

            pub fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, base32(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{self}")
            }
        }

        impl FromStr for $name {
            type Err = ModelError;
            fn from_str(s: &str) -> Result<$name, ModelError> {
                s.strip_prefix($prefix)
                    .and_then(unbase32_16)
                    .map($name)
                    .ok_or(ModelError::BadValue { what: $what, rule: concat!("`", $prefix, "` and 26 lowercase base32 characters") })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<$name, D::Error> {
                let text = String::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(EidolonId, "eid_", "eidolon id");
typed_id!(FacetId, "fct_", "facet id");

/// An opaque label for a wall: `wall_` and 1 to 64 of `a-z 0-9 _ -`. The
/// same format as Adytum's, kept as a type of its own.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Wall(String);

impl Wall {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Wall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Wall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Wall {
    type Err = ModelError;
    fn from_str(s: &str) -> Result<Wall, ModelError> {
        let ok = s.strip_prefix("wall_").is_some_and(|l| (1..=64).contains(&l.len()) && l.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-'));
        if ok {
            Ok(Wall(s.to_owned()))
        } else {
            Err(ModelError::BadValue { what: "wall", rule: "`wall_` and 1 to 64 of a-z, 0-9, _ or -" })
        }
    }
}

impl Serialize for Wall {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Wall {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Wall, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_have_a_prefix_26_characters_and_one_spelling() {
        let id = EidolonId::from_bytes([7; 16]);
        let text = id.to_string();
        assert!(text.starts_with("eid_") && text.len() == 30);
        assert_eq!(text.parse::<EidolonId>(), Ok(id));
        assert!(text.parse::<FacetId>().is_err(), "another prefix");
        assert!(text.to_uppercase().parse::<EidolonId>().is_err());
        assert!("eid_aaaa".parse::<EidolonId>().is_err(), "too short");
        // The last character carries two spare bits that must be zero.
        let mut bad = text.clone();
        bad.pop();
        bad.push('b');
        assert!(bad.parse::<EidolonId>().is_err());
        assert!(FacetId::new_random().is_ok_and(|f| f.to_string().starts_with("fct_")));
    }

    #[test]
    fn ids_go_through_json_as_strings() {
        let id = FacetId::from_bytes([1; 16]);
        let json = serde_json::to_string(&id).unwrap_or_default();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<FacetId>(&json).ok(), Some(id));
        assert!(serde_json::from_str::<FacetId>("\"fct_nope\"").is_err());
    }

    #[test]
    fn a_wall_is_wall_and_a_short_label() {
        for bad in ["", "wall_", "wall", "Wall_a", "wall_A", "wall_a b", "wall_a/b", &format!("wall_{}", "a".repeat(65))] {
            assert!(bad.parse::<Wall>().is_err(), "{bad:?}");
        }
        for good in ["wall_a", "wall_a-b_9", &format!("wall_{}", "a".repeat(64))] {
            assert!(good.parse::<Wall>().is_ok(), "{good:?}");
        }
        assert!(serde_json::from_str::<Wall>("\"nope\"").is_err());
    }
}
