//! Numbers as XenoNet's data writes them: JSON numbers, whole where a
//! count or an id is, never wider than what every reader keeps exactly
//! (2^53 - 1).

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// The widest whole number every reader keeps exactly.
pub const MAX_SAFE: u64 = 9_007_199_254_740_991;

macro_rules! whole {
    ($(#[$m:meta])* $name:ident($inner:ty) => $schema:tt) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd, Serialize, Deserialize)]
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
        impl From<$inner> for $name {
            fn from(n: $inner) -> Self {
                $name(n)
            }
        }
    };
}

whole! {
    /// A count, an index, an id: a whole number from 0.
    Uint(u64) => { "type": "integer", "minimum": 0, "maximum": 9007199254740991_i64 }
}
whole! {
    /// A whole number from 1: a process id, a port, a size that isn't empty.
    PosInt(u64) => { "type": "integer", "exclusiveMinimum": 0, "maximum": 9007199254740991_i64 }
}
whole! {
    /// Any whole number every reader keeps exactly.
    Int(i64) => { "type": "integer", "minimum": -9007199254740991_i64, "maximum": 9007199254740991_i64 }
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
