//! Drift: vendor data that differs from XenoNet's schema for it, one record
//! per difference (`drift/record/v1`), and the log that keeps them
//! (`drift/log/v1`).

use crate::num::{Ms, Uint};
use crate::schema::{Document, Nullable, Opt, Ref};
use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

crate::tag!(pub RecordTag = "drift/record/v1");

// What differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    NewField,
    MissingField,
    TypeChange,
    NewVariant,
    NewVersion,
    Unparsable,
}

/// A discriminator's value: a string, a number or a boolean.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    String(String),
    Number(f64),
    Bool(bool),
}

impl JsonSchema for Scalar {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Scalar")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": ["string", "number", "boolean"] })
    }
}

/// A drift record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub schema: RecordTag,
    /// <vendor>/<product>/<source>, the vendor schema's path without vendor/: anthropic/claude-code/transcript-line
    pub source: String,
    /// The version of the vendor's app that wrote the data, if it says
    pub vendor_version: Nullable<String>,
    pub kind: Kind,
    /// Where in the record: message.content[].citations; null where there's no field, as for a line that isn't JSON
    pub path: Nullable<String>,
    /// The kind of record it's in: the values of the discriminators around it, outermost first, such as ["assistant", "text"] for a text block in a reply; only values the schema lists. Left out where there's none
    #[expect(clippy::doc_link_with_quotes, reason = "the quoted list is the description's text, not a link")]
    #[serde(default, skip_serializing_if = "Opt::is_none")]
    #[schemars(length(min = 1, max = 8))]
    pub variant: Opt<Vec<Scalar>>,
    /// Types only, at most four levels deep: object keys and their value types, array element types. Never a value, except a declared discriminator's (a line's type) and vendor versions
    pub shape: Nullable<String>,
    pub first_seen: Ms,
    pub last_seen: Ms,
    #[schemars(range(min = 1))]
    pub count: Uint,
}

impl Document for Record {
    const ID: &'static str = "drift/record/v1";
    const TITLE: &'static str = "A drift record";
    const DESCRIPTION: &'static str =
        "Vendor data that differs from XenoNet's schema for it: a new or missing field, a field of another type, a new variant, a new version, or a line that isn't JSON.";
}

crate::constant!(
    /// drift.json's own version, always 1.
    pub LogVersion = 1
);

/// XenoNet's drift log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    /// drift.json's own version; its records each say what they are
    pub version: LogVersion,
    /// When you last looked at the drift (xenonet drift, or the console; never npm run contracts): what's first seen after it is new, and the doctor says so
    pub looked_at: Nullable<Ms>,
    /// The drift kept, merged across vendor versions, at most 500: the ones seen least lately go first
    #[schemars(length(max = 500))]
    pub records: Vec<Ref<Record>>,
}

impl Document for Log {
    const ID: &'static str = "drift/log/v1";
    const TITLE: &'static str = "XenoNet's drift log";
    const DESCRIPTION: &'static str = "drift.json in the cache folder: the drift the lenient readers have found in the other apps' data, merged across the processes that write it (the server, xenonet drift, npm run contracts), and when you last looked.";
}

/// The documents this module writes.
pub const DOCUMENTS: &[crate::schema::Writer] = &[crate::schema::file::<Log>, crate::schema::file::<Record>];
