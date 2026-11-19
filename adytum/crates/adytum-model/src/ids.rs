use std::fmt;
use std::str::FromStr;

use crate::error::{invalid, ModelError};

const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const ENCODED_LEN: usize = 26;

/// 16 bytes as 26 lowercase base32 characters (RFC 4648 alphabet, no padding).
fn encode(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(ENCODED_LEN);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ALPHABET[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    if bits > 0 {
        out.push(char::from(ALPHABET[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

/// The inverse of `encode`. Refuses anything `encode` would not have written,
/// so each id has exactly one spelling.
fn decode(text: &str, what: &'static str) -> Result<[u8; 16], ModelError> {
    let rule = "26 lowercase base32 characters (a-z, 2-7) after the prefix";
    if text.len() != ENCODED_LEN {
        return Err(invalid(what, rule));
    }
    let mut out = [0u8; 16];
    let mut written = 0;
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for c in text.bytes() {
        let value = ALPHABET.iter().position(|&a| a == c).ok_or_else(|| invalid(what, rule))?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if let Some(slot) = out.get_mut(written) {
                *slot = ((buffer >> bits) & 0xff) as u8;
                written += 1;
            }
            buffer &= (1 << bits) - 1;
        }
    }
    if buffer != 0 {
        return Err(invalid(what, "the last character must have its spare bits zero"));
    }
    Ok(out)
}

macro_rules! typed_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal, $what:literal) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name([u8; 16]);

        impl $name {
            /// The prefix every spelling of this id starts with.
            pub const PREFIX: &'static str = $prefix;

            /// A fresh id from the system's random source.
            pub fn new_random() -> Result<Self, ModelError> {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes).map_err(|_| ModelError::Random)?;
                Ok(Self(bytes))
            }

            /// An id made of these bytes.
            pub const fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(bytes)
            }

            /// The id's 16 bytes.
            pub const fn as_bytes(&self) -> &[u8; 16] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, encode(&self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self)
            }
        }

        impl FromStr for $name {
            type Err = ModelError;

            fn from_str(text: &str) -> Result<Self, ModelError> {
                let rest = text
                    .strip_prefix($prefix)
                    .ok_or_else(|| invalid($what, concat!("it starts with ", $prefix)))?;
                decode(rest, $what).map(Self)
            }
        }

        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let text = String::deserialize(d)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

typed_id!(
    /// A cell's id: `cell_` and 26 base32 characters.
    CellId,
    "cell_",
    "cell id"
);
typed_id!(
    /// An image's id: `img_` and 26 base32 characters.
    ImageId,
    "img_",
    "image id"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_is_prefix_and_26_lowercase_base32() {
        let id = CellId::from_bytes([0xff; 16]);
        let text = id.to_string();
        assert!(text.starts_with("cell_"));
        assert_eq!(text.len(), 5 + 26);
        assert_eq!(text, "cell_77777777777777777777777774");
        assert_eq!(ImageId::from_bytes([0; 16]).to_string(), "img_aaaaaaaaaaaaaaaaaaaaaaaaaa");
    }

    #[test]
    fn round_trips_through_text_and_json() {
        let id = CellId::new_random().unwrap();
        assert_eq!(id.to_string().parse::<CellId>().unwrap(), id);
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<CellId>(&json).unwrap(), id);
    }

    #[test]
    fn refuses_what_is_not_an_id() {
        for bad in [
            "",
            "cell_",
            "img_aaaaaaaaaaaaaaaaaaaaaaaaaa",   // wrong prefix for a cell
            "cell_AAAAAAAAAAAAAAAAAAAAAAAAAA",  // upper case
            "cell_aaaaaaaaaaaaaaaaaaaaaaaaa",   // 25 characters
            "cell_aaaaaaaaaaaaaaaaaaaaaaaaaaa", // 27 characters
            "cell_aaaaaaaaaaaaaaaaaaaaaaaaa1",  // outside the alphabet
            "cell_aaaaaaaaaaaaaaaaaaaaaaaaab",  // spare bits set
        ] {
            assert!(bad.parse::<CellId>().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn two_random_ids_differ() {
        assert_ne!(CellId::new_random().unwrap(), CellId::new_random().unwrap());
    }
}
