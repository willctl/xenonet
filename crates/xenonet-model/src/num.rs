//! Numbers as XenoNet's data writes them: JSON numbers, whole where a
//! count or an id is, never wider than what every reader keeps exactly
//! (2^53 - 1).

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{de, Deserialize, Deserializer, Serialize};
use std::borrow::Cow;

/// The widest whole number every reader keeps exactly.
pub const MAX_SAFE: u64 = 9_007_199_254_740_991;

// What a whole number between `min` and `max` is, in words: a bound that
// is the widest every reader keeps goes unsaid.
fn whole_words(min: i128, max: i128) -> String {
    let safe = i128::from(MAX_SAFE);
    match (min > -safe, max < safe) {
        (true, true) => format!("a whole number from {min} to {max}"),
        (true, false) => format!("a whole number from {min}"),
        (false, true) => format!("a whole number of at most {max}"),
        (false, false) => "a whole number".to_string(),
    }
}

// Reads a whole number from `min` to `max`, however JSON writes it: 5 and
// 5.0 are the same number, 5.5 isn't a whole one.
struct Whole {
    min: i128,
    max: i128,
}

impl Whole {
    fn check<E: de::Error>(&self, n: i128) -> Result<i128, E> {
        if (self.min..=self.max).contains(&n) {
            return Ok(n);
        }
        let shown = match u64::try_from(n) {
            Ok(u) => de::Unexpected::Unsigned(u),
            Err(_) => de::Unexpected::Signed(n as i64),
        };
        let safe = i128::from(MAX_SAFE);
        let words = if n > safe {
            format!("a whole number of at most {safe}")
        } else if n < -safe {
            format!("a whole number of at least {}", -safe)
        } else {
            whole_words(self.min, self.max)
        };
        Err(E::invalid_value(shown, &words.as_str()))
    }
}

impl de::Visitor<'_> for Whole {
    type Value = i128;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&whole_words(self.min, self.max))
    }
    fn visit_i64<E: de::Error>(self, n: i64) -> Result<i128, E> {
        self.check(i128::from(n))
    }
    fn visit_u64<E: de::Error>(self, n: u64) -> Result<i128, E> {
        self.check(i128::from(n))
    }
    fn visit_i128<E: de::Error>(self, n: i128) -> Result<i128, E> {
        self.check(n)
    }
    fn visit_u128<E: de::Error>(self, n: u128) -> Result<i128, E> {
        self.check(i128::try_from(n).unwrap_or(i128::MAX))
    }
    fn visit_f64<E: de::Error>(self, n: f64) -> Result<i128, E> {
        // Past 2^63 every float is whole, and out of range anyway.
        if n.is_finite() && n.trunc() == n && n.abs() < 9.3e18 {
            self.check(n as i128)
        } else {
            Err(E::invalid_value(de::Unexpected::Float(n), &whole_words(self.min, self.max).as_str()))
        }
    }
}

fn read_whole<'de, D: Deserializer<'de>>(d: D, min: i128, max: i128) -> Result<i128, D::Error> {
    d.deserialize_any(Whole { min, max })
}

macro_rules! whole {
    ($(#[$m:meta])* $name:ident($inner:ty), $min:expr => $schema:tt) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub $inner);
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!($schema)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                // In range, so it fits.
                read_whole(d, $min, i128::from(MAX_SAFE)).map(|n| $name(n as $inner))
            }
        }
        impl From<$inner> for $name {
            fn from(n: $inner) -> Self {
                $name(n)
            }
        }
    };
}

whole! {
    /// A count, an index, an id: a whole number from 0.
    Uint(u64), 0 => { "type": "integer", "minimum": 0, "maximum": 9007199254740991_i64 }
}
whole! {
    /// A whole number from 1: a process id, a port, a size that isn't empty.
    PosInt(u64), 1 => { "type": "integer", "exclusiveMinimum": 0, "maximum": 9007199254740991_i64 }
}
whole! {
    /// Any whole number every reader keeps exactly.
    Int(i64), -i128::from(MAX_SAFE) => { "type": "integer", "minimum": -9007199254740991_i64, "maximum": 9007199254740991_i64 }
}

/// A whole number from `MIN` to `MAX` (at most the widest every reader
/// keeps): a port, an HTTP status, a count with a cap. Its schema is
/// `{"type": "integer", "minimum": MIN, "maximum": MAX}`, and reading one
/// refuses a number outside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Ranged<const MIN: u64, const MAX: u64 = MAX_SAFE>(pub u64);

impl<const MIN: u64, const MAX: u64> Default for Ranged<MIN, MAX> {
    fn default() -> Self {
        Ranged(MIN)
    }
}

impl<const MIN: u64, const MAX: u64> From<u64> for Ranged<MIN, MAX> {
    fn from(n: u64) -> Self {
        Ranged(n)
    }
}

impl<const MIN: u64, const MAX: u64> From<Ranged<MIN, MAX>> for Uint {
    fn from(n: Ranged<MIN, MAX>) -> Self {
        Uint(n.0)
    }
}

impl<'de, const MIN: u64, const MAX: u64> Deserialize<'de> for Ranged<MIN, MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let max = MAX.min(MAX_SAFE);
        read_whole(d, i128::from(MIN), i128::from(max)).map(|n| Ranged(n as u64))
    }
}

impl<const MIN: u64, const MAX: u64> JsonSchema for Ranged<MIN, MAX> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Ranged_{MIN}_{MAX}"))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "integer", "minimum": MIN, "maximum": MAX.min(MAX_SAFE) })
    }
}

/// A time in epoch milliseconds, as every clock in XenoNet's data reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ms(pub f64);

impl JsonSchema for Ms {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Ms")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "number", "description": "Epoch milliseconds" })
    }
}

impl From<f64> for Ms {
    fn from(n: f64) -> Self {
        Ms(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refused<T: de::DeserializeOwned>(v: serde_json::Value) -> String {
        serde_json::from_value::<T>(v).err().map(|e| e.to_string()).unwrap_or_default()
    }

    #[test]
    fn whole_numbers_refuse_fractions_and_what_is_out_of_range() {
        assert_eq!(serde_json::from_value::<Uint>(json!(5)).unwrap(), Uint(5));
        assert_eq!(serde_json::from_value::<Uint>(json!(5.0)).unwrap(), Uint(5), "5.0 is 5");
        assert_eq!(refused::<Uint>(json!(5.5)), "invalid value: floating point `5.5`, expected a whole number from 0");
        assert_eq!(refused::<Uint>(json!(-1)), "invalid value: integer `-1`, expected a whole number from 0");
        assert_eq!(refused::<Uint>(json!(MAX_SAFE + 1)), "invalid value: integer `9007199254740992`, expected a whole number of at most 9007199254740991");
        assert_eq!(serde_json::from_value::<Uint>(json!(MAX_SAFE)).unwrap(), Uint(MAX_SAFE));
        assert_eq!(refused::<PosInt>(json!(0)), "invalid value: integer `0`, expected a whole number from 1");
        assert_eq!(serde_json::from_value::<Int>(json!(-3)).unwrap(), Int(-3));
        assert_eq!(refused::<Int>(json!(-9007199254740992_i64)), "invalid value: integer `-9007199254740992`, expected a whole number of at least -9007199254740991");
        assert!(refused::<Int>(json!("3")).starts_with("invalid type: string \"3\", expected a whole number"));
        assert!(refused::<Uint>(json!(1e300)).starts_with("invalid value: floating point"));
    }

    #[test]
    fn a_ranged_number_holds_its_range() {
        type Port = Ranged<1, 65535>;
        assert_eq!(serde_json::from_value::<Port>(json!(22)).unwrap(), Ranged(22));
        assert_eq!(refused::<Port>(json!(0)), "invalid value: integer `0`, expected a whole number from 1 to 65535");
        assert_eq!(refused::<Port>(json!(65536)), "invalid value: integer `65536`, expected a whole number from 1 to 65535");
        assert_eq!(refused::<Ranged<1>>(json!(0)), "invalid value: integer `0`, expected a whole number from 1");
        assert_eq!(serde_json::to_value(Port::from(8080)).unwrap(), json!(8080));
    }
}
