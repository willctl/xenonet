//! The JSON Schema documents, written from the types: each document type
//! says its id (`Document`), and `documents()` renders every one the way
//! the files in `schemas/` have it.
//!
//! The shapes follow what XenoNet's data has always been written in: a
//! loose object says `"additionalProperties": {}`, a record has
//! `propertyNames`, a nullable field is `anyOf` with null last or a `type`
//! list, an integer has no `format`. The transforms below put schemars's
//! output into those shapes, so a reader of the files sees one dialect.

use schemars::generate::SchemaSettings;
use schemars::transform::{transform_subschemas, Transform};
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::marker::PhantomData;

/// Where every schema's `$id` lives.
pub const BASE: &str = "https://xenonet.dev/schemas";
/// The meta-schema every document names.
pub const META: &str = "https://json-schema.org/draft/2020-12/schema";
/// The line at the top of every file saying where it comes from.
pub const COMMENT: &str = "Written from crates/xenonet-model by cargo test -p xenonet-model. Change the type there, not this file.";

/// A writer of one document's file: `file::<T>`.
pub type Writer = fn() -> (String, String);

/// A type that is one of XenoNet's schema documents: `ID` is its short id
/// (`engine/claim/v1`), the file is `schemas/<ID>.json`.
pub trait Document: JsonSchema {
    const ID: &'static str;
    /// The file's title.
    const TITLE: &'static str;
    /// The file's description.
    const DESCRIPTION: &'static str;

    fn uri() -> String {
        format!("{BASE}/{}.json", Self::ID)
    }
}

/// A field whose schema refers to another document by its `$id`, as the
/// files have always referred to each other, and whose value is that
/// document's type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Ref<T>(pub T);

impl<T: Document> JsonSchema for Ref<T> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("ref:{}", T::ID))
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("ref:{}", T::ID))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut m = Map::new();
        m.insert("$ref".into(), Value::String(T::uri()));
        Schema::from(m)
    }
}

impl<T> std::ops::Deref for Ref<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

/// A field that may be left out, and is never null when it's there: zod's
/// `.optional()`, where `Option<T>` is a nullable field. Mark the field
/// `#[serde(default, skip_serializing_if = "Opt::is_none")]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Opt<T>(pub Option<T>);

impl<T> Default for Opt<T> {
    fn default() -> Self {
        Opt(None)
    }
}

impl<T> Opt<T> {
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
    pub fn as_ref(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T> From<Option<T>> for Opt<T> {
    fn from(o: Option<T>) -> Self {
        Opt(o)
    }
}

impl<T> From<T> for Opt<T> {
    fn from(v: T) -> Self {
        Opt(Some(v))
    }
}

impl<T: JsonSchema> JsonSchema for Opt<T> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Opt_of_{}", T::schema_name()))
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("Opt<{}>", T::schema_id()))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        g.subschema_for::<T>()
    }
}

/// A field that is there, and may be null: zod's `.nullable()`. The key
/// is required; the schema is `anyOf` of the type and null, with the
/// type's own description kept inside, as the files have it. (`Option<T>`
/// alone is an optional field to schemars.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Nullable<T>(pub Option<T>);

impl<T> Default for Nullable<T> {
    fn default() -> Self {
        Nullable(None)
    }
}

impl<T> Nullable<T> {
    pub fn is_none(&self) -> bool {
        self.0.is_none()
    }
    pub fn as_ref(&self) -> Option<&T> {
        self.0.as_ref()
    }
}

impl<T> From<Option<T>> for Nullable<T> {
    fn from(o: Option<T>) -> Self {
        Nullable(o)
    }
}

impl<T: JsonSchema> JsonSchema for Nullable<T> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Nullable_of_{}", T::schema_name()))
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("Nullable<{}>", T::schema_id()))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let inner = g.subschema_for::<T>();
        let mut m = Map::new();
        m.insert("anyOf".into(), Value::Array(vec![inner.to_value(), Value::Object([("type".to_string(), Value::String("null".into()))].into_iter().collect())]));
        Schema::from(m)
    }
}

/// Any JSON at all: the `{}` schema. (`serde_json::Value`'s schema is
/// `true`, which means the same but isn't how the files write it.)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Any(pub Value);

impl JsonSchema for Any {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("Any")
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        Schema::from(Map::new())
    }
}

/// A string with a known bound: at most `MAX` characters, and at least
/// `MIN` when that's more than 0. For a field whose bounds can't go on
/// the field itself, as inside a `Nullable`: `Nullable<Text<0, 200>>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Text<const MIN: usize, const MAX: usize>(pub String);

impl<const MIN: usize, const MAX: usize> From<String> for Text<MIN, MAX> {
    fn from(s: String) -> Self {
        Text(s)
    }
}

impl<const MIN: usize, const MAX: usize> std::ops::Deref for Text<MIN, MAX> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const MIN: usize, const MAX: usize> JsonSchema for Text<MIN, MAX> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Text_{MIN}_{MAX}"))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        let mut m = Map::new();
        m.insert("type".into(), Value::String("string".into()));
        if MIN > 0 {
            m.insert("minLength".into(), Value::from(MIN));
        }
        if MAX < usize::MAX {
            m.insert("maxLength".into(), Value::from(MAX));
        }
        Schema::from(m)
    }
}

/// A record: keys of one shape (`K`, a string by default), values of one
/// type, in the order they were written. `propertyNames` is the key's
/// schema and `additionalProperties` the value's, as a record has always
/// been written.
#[derive(Clone, Debug, PartialEq)]
pub struct Record<V, K = String>(pub Vec<(K, V)>);

impl<V, K> Default for Record<V, K> {
    fn default() -> Self {
        Record(Vec::new())
    }
}

impl<V, K: PartialEq> Record<V, K> {
    pub fn get(&self, k: &K) -> Option<&V> {
        self.0.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }
    pub fn insert(&mut self, k: K, v: V) {
        match self.0.iter_mut().find(|(x, _)| *x == k) {
            Some(e) => e.1 = v,
            None => self.0.push((k, v)),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&K, &V)> {
        self.0.iter().map(|(k, v)| (k, v))
    }
}

impl<V: Serialize, K: Serialize> Serialize for Record<V, K> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

impl<'de, V: Deserialize<'de>, K: Deserialize<'de>> Deserialize<'de> for Record<V, K> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<V, K>(PhantomData<(V, K)>);
        impl<'de, V: Deserialize<'de>, K: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<V, K> {
            type Value = Record<V, K>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::with_capacity(a.size_hint().unwrap_or(0));
                while let Some(e) = a.next_entry()? {
                    out.push(e);
                }
                Ok(Record(out))
            }
        }
        d.deserialize_map(Visitor(PhantomData))
    }
}

impl<V: JsonSchema, K: JsonSchema> JsonSchema for Record<V, K> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Record_of_{}_by_{}", V::schema_name(), K::schema_name()))
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("Record<{}, {}>", V::schema_id(), K::schema_id()))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let mut m = Map::new();
        m.insert("type".into(), Value::String("object".into()));
        m.insert("propertyNames".into(), g.subschema_for::<K>().to_value());
        m.insert("additionalProperties".into(), g.subschema_for::<V>().to_value());
        Schema::from(m)
    }
}

/// Writes the shapes the files have always had (see the module's doc).
#[derive(Clone, Copy, Debug, Default)]
pub struct Shapes;

fn is_object_schema(m: &Map<String, Value>) -> bool {
    match m.get("type") {
        Some(Value::String(t)) => t == "object",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "object"),
        _ => m.contains_key("properties"),
    }
}

fn is_record(m: &Map<String, Value>) -> bool {
    m.contains_key("propertyNames")
}

impl Transform for Shapes {
    fn transform(&mut self, schema: &mut Schema) {
        transform_subschemas(self, schema);
        let Some(m) = schema.as_object_mut() else { return };
        // A loose object says so.
        if is_object_schema(m) && !is_record(m) && !m.contains_key("additionalProperties") {
            m.insert("additionalProperties".into(), Value::Object(Map::new()));
        }
        // A number is a number: no float or integer width.
        let numeric = matches!(m.get("type"), Some(Value::String(t)) if t == "integer" || t == "number");
        if numeric {
            m.remove("format");
        }
        // Nothing of ours is a title but the document's.
        m.remove("title");
    }
}

fn settings() -> SchemaSettings {
    SchemaSettings::draft2020_12().with(|s| {
        s.inline_subschemas = true;
        s.meta_schema = None;
    })
}

/// A document's JSON, as its file has it: `$schema`, `$id`, `$comment`,
/// `title` and `description` first, then the type's schema.
pub fn document<T: Document>() -> Value {
    let mut g = settings().with_transform(Shapes).into_generator();
    let schema = g.root_schema_for::<T>();
    let mut body = match schema.to_value() {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("const".into(), other);
            m
        }
    };
    body.remove("title");
    body.remove("description");
    body.remove("$schema");
    let mut out = Map::new();
    out.insert("$schema".into(), Value::String(META.into()));
    out.insert("$id".into(), Value::String(T::uri()));
    out.insert("$comment".into(), Value::String(COMMENT.into()));
    out.insert("title".into(), Value::String(T::TITLE.into()));
    out.insert("description".into(), Value::String(T::DESCRIPTION.into()));
    for (k, v) in body {
        out.insert(k, v);
    }
    Value::Object(out)
}

/// A document's file: its path under `schemas/` and its text, two-space
/// indented with a final newline, as every file there is written.
pub fn file<T: Document>() -> (String, String) {
    let text = serde_json::to_string_pretty(&document::<T>()).unwrap_or_default();
    (format!("{}.json", T::ID), format!("{text}\n"))
}

/// Every document, by id.
pub fn documents() -> Vec<(String, String)> {
    // Each module's list, flattened: the modules are the crate's own.
    let mut out = crate::DOCUMENTS.iter().flat_map(|m| m.iter()).map(|f| f()).collect::<Vec<_>>();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A unit type that is always one string: a document's `schema` tag, a
/// constant kind. `tag!(pub Name = "text")` makes one.
#[macro_export]
macro_rules! tag {
    ($(#[$m:meta])* $v:vis $name:ident = $text:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        $v struct $name;
        impl $name {
            pub const TEXT: &'static str = $text;
        }
        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str($text)
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <::std::borrow::Cow<'de, str> as ::serde::Deserialize>::deserialize(d)?;
                if s == $text {
                    Ok($name)
                } else {
                    Err(::serde::de::Error::invalid_value(::serde::de::Unexpected::Str(&s), &$text))
                }
            }
        }
        impl ::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut ::schemars::SchemaGenerator) -> ::schemars::Schema {
                ::schemars::json_schema!({ "type": "string", "const": $text })
            }
        }
    };
}

/// A unit type that is always one JSON number or boolean, with the
/// schema `{"type": ..., "const": ...}`: a format's version, a flag that
/// is only ever set. `constant!(pub Version2 = 2)` or
/// `constant!(pub True = true)` makes one.
#[macro_export]
macro_rules! constant {
    ($(#[$m:meta])* $v:vis $name:ident = true) => {
        $crate::constant!(@bool $(#[$m])* $v $name = true, false);
    };
    ($(#[$m:meta])* $v:vis $name:ident = false) => {
        $crate::constant!(@bool $(#[$m])* $v $name = false, true);
    };
    ($(#[$m:meta])* $v:vis $name:ident = $value:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        $v struct $name;
        impl $name {
            pub const VALUE: u64 = $value;
        }
        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_u64(Self::VALUE)
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                // A number however it's written: 1 and 1.0 are the same value.
                let n = <f64 as ::serde::Deserialize>::deserialize(d)?;
                if n == Self::VALUE as f64 {
                    Ok($name)
                } else {
                    Err(::serde::de::Error::invalid_value(::serde::de::Unexpected::Float(n), &stringify!($value)))
                }
            }
        }
        impl ::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut ::schemars::SchemaGenerator) -> ::schemars::Schema {
                ::schemars::json_schema!({ "type": "number", "const": $value })
            }
        }
    };
    (@bool $(#[$m:meta])* $v:vis $name:ident = $value:literal, $other:literal) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        $v struct $name;
        impl $name {
            pub const VALUE: bool = $value;
        }
        impl ::serde::Serialize for $name {
            fn serialize<S: ::serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_bool(Self::VALUE)
            }
        }
        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                if <bool as ::serde::Deserialize>::deserialize(d)? == Self::VALUE {
                    Ok($name)
                } else {
                    Err(::serde::de::Error::invalid_value(::serde::de::Unexpected::Bool($other), &stringify!($value)))
                }
            }
        }
        impl ::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut ::schemars::SchemaGenerator) -> ::schemars::Schema {
                ::schemars::json_schema!({ "type": "boolean", "const": $value })
            }
        }
    };
}

crate::constant!(
    /// A flag that is only ever `true`: left out, or null, otherwise.
    pub True = true
);

/// A vocabulary with room for words a reader doesn't know yet: the known
/// words as an enum, and the value as one of them or any other string.
/// `open!(Kind of KnownKind { A = "a", B = "b" })` makes both.
#[macro_export]
macro_rules! open {
    ($(#[$m:meta])* $open:ident of $known:ident { $($var:ident = $text:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize, ::schemars::JsonSchema)]
        pub enum $known {
            $(#[serde(rename = $text)] $var,)+
        }
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, ::serde::Serialize, ::serde::Deserialize, ::schemars::JsonSchema)]
        #[serde(untagged)]
        pub enum $open {
            Known($known),
            Other(String),
        }
    };
}
