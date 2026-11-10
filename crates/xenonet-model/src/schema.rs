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
use std::sync::OnceLock;

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
/// Reading one refuses a string outside the bounds, its length counted in
/// UTF-16 code units, as JS counts a string's length.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Text<const MIN: usize, const MAX: usize>(pub String);

impl<const MIN: usize, const MAX: usize> Text<MIN, MAX> {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<const MIN: usize, const MAX: usize> From<String> for Text<MIN, MAX> {
    fn from(s: String) -> Self {
        Text(s)
    }
}

impl<const MIN: usize, const MAX: usize> From<&str> for Text<MIN, MAX> {
    fn from(s: &str) -> Self {
        Text(s.to_string())
    }
}

impl<const MIN: usize, const MAX: usize> From<Text<MIN, MAX>> for String {
    fn from(t: Text<MIN, MAX>) -> Self {
        t.0
    }
}

impl<const MIN: usize, const MAX: usize> std::ops::Deref for Text<MIN, MAX> {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const MIN: usize, const MAX: usize> std::fmt::Display for Text<MIN, MAX> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const MIN: usize, const MAX: usize> PartialEq<str> for Text<MIN, MAX> {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl<const MIN: usize, const MAX: usize> PartialEq<&str> for Text<MIN, MAX> {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// What a bounded string or list is expected to be, in words: "a string
/// of at most 200 characters", "a list of 1 to 4 items".
fn sized(what: &str, unit: &str, min: usize, max: usize) -> String {
    let units = |n: usize| if n == 1 { format!("{n} {unit}") } else { format!("{n} {unit}s") };
    match (min, max) {
        (0, usize::MAX) => what.to_string(),
        (0, max) => format!("{what} of at most {}", units(max)),
        (min, usize::MAX) => format!("{what} of at least {}", units(min)),
        (min, max) if min == max => format!("{what} of exactly {}", units(min)),
        (min, max) => format!("{what} of {min} to {}", units(max)),
    }
}

impl<'de, const MIN: usize, const MAX: usize> Deserialize<'de> for Text<MIN, MAX> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let n = s.encode_utf16().count();
        if (MIN..=MAX).contains(&n) {
            Ok(Text(s))
        } else {
            Err(serde::de::Error::invalid_length(n, &sized("a string", "character", MIN, MAX).as_str()))
        }
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
        m.insert(BOUNDS_LAST.into(), Value::Bool(true));
        Schema::from(m)
    }
}

/// A list with a known bound: at least `MIN` items and at most `MAX`
/// (`usize::MAX` for no limit). Its schema is the list's with `minItems`
/// and `maxItems`; reading one refuses a list outside the bounds, without
/// reading the items past `MAX`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Bounded<T, const MIN: usize, const MAX: usize>(pub Vec<T>);

impl<T, const MIN: usize, const MAX: usize> Default for Bounded<T, MIN, MAX> {
    fn default() -> Self {
        Bounded(Vec::new())
    }
}

impl<T, const MIN: usize, const MAX: usize> From<Vec<T>> for Bounded<T, MIN, MAX> {
    fn from(v: Vec<T>) -> Self {
        Bounded(v)
    }
}

impl<T, const MIN: usize, const MAX: usize> From<Bounded<T, MIN, MAX>> for Vec<T> {
    fn from(b: Bounded<T, MIN, MAX>) -> Self {
        b.0
    }
}

impl<T, const MIN: usize, const MAX: usize> FromIterator<T> for Bounded<T, MIN, MAX> {
    fn from_iter<I: IntoIterator<Item = T>>(i: I) -> Self {
        Bounded(i.into_iter().collect())
    }
}

impl<T, const MIN: usize, const MAX: usize> std::ops::Deref for Bounded<T, MIN, MAX> {
    type Target = Vec<T>;
    fn deref(&self) -> &Vec<T> {
        &self.0
    }
}

impl<T, const MIN: usize, const MAX: usize> std::ops::DerefMut for Bounded<T, MIN, MAX> {
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.0
    }
}

impl<T, const MIN: usize, const MAX: usize> IntoIterator for Bounded<T, MIN, MAX> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a, T, const MIN: usize, const MAX: usize> IntoIterator for &'a Bounded<T, MIN, MAX> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'de, T: Deserialize<'de>, const MIN: usize, const MAX: usize> Deserialize<'de> for Bounded<T, MIN, MAX> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Visitor<T, const MIN: usize, const MAX: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const MIN: usize, const MAX: usize> serde::de::Visitor<'de> for Visitor<T, MIN, MAX> {
            type Value = Bounded<T, MIN, MAX>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&sized("a list", "item", MIN, MAX))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                // A list known to be too long is refused before its items
                // are read; one whose length isn't known, at the first item
                // past the bound, counting the rest without reading them.
                if let Some(n) = a.size_hint().filter(|&n| n > MAX) {
                    return Err(serde::de::Error::invalid_length(n, &self));
                }
                let mut out = Vec::with_capacity(a.size_hint().unwrap_or(0).min(MAX));
                while out.len() < MAX {
                    match a.next_element()? {
                        Some(x) => out.push(x),
                        None => break,
                    }
                }
                let mut n = out.len();
                while a.next_element::<serde::de::IgnoredAny>()?.is_some() {
                    n += 1;
                }
                if n < MIN || n > MAX {
                    return Err(serde::de::Error::invalid_length(n, &self));
                }
                Ok(Bounded(out))
            }
        }
        d.deserialize_seq(Visitor::<T, MIN, MAX>(PhantomData))
    }
}

impl<T: JsonSchema, const MIN: usize, const MAX: usize> JsonSchema for Bounded<T, MIN, MAX> {
    fn schema_name() -> Cow<'static, str> {
        Cow::Owned(format!("Bounded_of_{}_{MIN}_{MAX}", T::schema_name()))
    }
    fn schema_id() -> Cow<'static, str> {
        Cow::Owned(format!("Bounded<{}, {MIN}, {MAX}>", T::schema_id()))
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut SchemaGenerator) -> Schema {
        let mut s = g.subschema_for::<Vec<T>>();
        if MIN > 0 {
            s.insert("minItems".into(), Value::from(MIN));
        }
        if MAX < usize::MAX {
            s.insert("maxItems".into(), Value::from(MAX));
        }
        s.insert(BOUNDS_LAST.into(), Value::Bool(true));
        s
    }
}

/// A schema's `pattern`, checked as a string is read: the pattern is the
/// source of a JS regular expression, as JSON Schema has it, and is
/// compiled once, the first time it's needed. A module's patterned string
/// type keeps one in a `static`.
#[derive(Debug)]
pub struct Pattern {
    source: &'static str,
    compiled: OnceLock<Option<regex::Regex>>,
}

impl Pattern {
    pub const fn new(source: &'static str) -> Self {
        Pattern { source, compiled: OnceLock::new() }
    }

    /// The pattern as the schema writes it.
    pub fn source(&self) -> &'static str {
        self.source
    }

    fn regex(&self) -> Option<&regex::Regex> {
        self.compiled.get_or_init(|| regex::RegexBuilder::new(&rust_syntax(self.source)).size_limit(1 << 26).build().ok()).as_ref()
    }

    /// Whether the pattern compiles: one that doesn't matches nothing.
    pub fn compiles(&self) -> bool {
        self.regex().is_some()
    }

    /// Whether `s` matches, as JS's `RegExp.prototype.test` says: anywhere
    /// in the string, unless the pattern anchors it.
    pub fn test(&self, s: &str) -> bool {
        self.regex().is_some_and(|r| r.is_match(s))
    }

    /// `s`, or the error that says it doesn't match.
    pub fn check<E: serde::de::Error>(&self, s: String) -> Result<String, E> {
        if self.test(&s) {
            Ok(s)
        } else {
            Err(E::invalid_value(serde::de::Unexpected::Str(shown(&s)), &format!("a string matching {}", self.source).as_str()))
        }
    }

    /// A string read from `d` that matches.
    pub fn read<'de, D: serde::Deserializer<'de>>(&self, d: D) -> Result<String, D::Error> {
        self.check(String::deserialize(d)?)
    }
}

/// The start of a refused string, as an error shows it: never all of a
/// long one.
pub fn shown(s: &str) -> &str {
    match s.char_indices().nth(64) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

// A JS pattern as the `regex` crate reads it. The patterns in the schemas
// are plain: anchors, classes, groups and counted repeats. Where the two
// differ on those, this writes JS's meaning: `\d` is ASCII's digits, `.`
// stops at JS's line terminators, `\uXXXX` is a code point, and an escaped
// punctuation mark is the mark itself. (JS without the u flag counts a
// character outside the BMP as two; here it is one.)
fn rust_syntax(js: &str) -> String {
    let mut out = String::with_capacity(js.len() + 16);
    let mut class = false;
    let mut chars = js.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('d') => out.push_str(if class { "0-9" } else { "[0-9]" }),
                Some('D') if !class => out.push_str("[^0-9]"),
                Some('u') => {
                    out.push_str("\\x{");
                    out.extend(chars.by_ref().take(4));
                    out.push('}');
                }
                Some(e) if e.is_ascii_alphanumeric() => {
                    out.push('\\');
                    out.push(e);
                }
                Some(e) => out.push_str(&regex::escape(&e.to_string())),
                None => out.push_str("\\\\"),
            },
            '[' if !class => {
                class = true;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    chars.next();
                    out.push('^');
                }
            }
            ']' if class => {
                class = false;
                out.push(']');
            }
            // Inside a class these mean sets to the regex crate, and only
            // themselves to JS.
            '[' | '&' | '~' if class => {
                out.push('\\');
                out.push(c);
            }
            '.' if !class => out.push_str(r"[^\n\r\x{2028}\x{2029}]"),
            _ => out.push(c),
        }
    }
    out
}

/// A string of one shape: a newtype over `String` whose schema is
/// `{"type": "string", "pattern": ...}` (with the type's doc comment as its
/// description), and which refuses a string that doesn't match as it's
/// read. `pattern!(pub struct KeyId(r"^xnk1_[a-z2-7]{32}$");)` makes one.
#[macro_export]
macro_rules! pattern {
    ($(#[$m:meta])* $v:vis struct $name:ident($pattern:tt);) => {
        #[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, ::serde::Serialize, ::schemars::JsonSchema)]
        $(#[$m])*
        #[serde(transparent)]
        $v struct $name(#[schemars(regex(pattern = $pattern))] pub String);

        impl $name {
            /// The pattern the string matches.
            pub fn pattern() -> &'static $crate::schema::Pattern {
                static PATTERN: $crate::schema::Pattern = $crate::schema::Pattern::new($pattern);
                &PATTERN
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> ::serde::Deserialize<'de> for $name {
            fn deserialize<D: ::serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                Self::pattern().read(d).map($name)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                $name(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                $name(s.to_string())
            }
        }

        impl ::std::ops::Deref for $name {
            type Target = str;
            fn deref(&self) -> &str {
                &self.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
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

/// A key a bounded type's schema carries until `Shapes` takes it off:
/// `true` puts the bounds (`minLength`, `maxLength`, `minItems`,
/// `maxItems`, `pattern`) after the description a field gives it, as the
/// files have a field's bounds; a field whose file has them first says
/// `#[schemars(extend("x-bounds-last" = false))]`.
pub const BOUNDS_LAST: &str = "x-bounds-last";

const BOUNDS: [&str; 5] = ["minLength", "maxLength", "minItems", "maxItems", "pattern"];

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
        if m.shift_remove(BOUNDS_LAST) == Some(Value::Bool(true)) {
            for k in BOUNDS {
                if let Some(v) = m.shift_remove(k) {
                    m.insert(k.into(), v);
                }
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refused<T: serde::de::DeserializeOwned>(v: Value) -> String {
        serde_json::from_value::<T>(v).err().map(|e| e.to_string()).unwrap_or_default()
    }

    #[test]
    fn a_text_counts_utf16_code_units() {
        type Name = Text<1, 4>;
        assert_eq!(serde_json::from_value::<Name>(json!("abcd")).unwrap().as_str(), "abcd");
        assert_eq!(serde_json::from_value::<Name>(json!("éééé")).unwrap().as_str(), "éééé", "é is one unit");
        assert!(serde_json::from_value::<Name>(json!("😀😀")).is_ok(), "two units each");
        assert_eq!(refused::<Name>(json!("😀😀a")), "invalid length 5, expected a string of 1 to 4 characters");
        assert_eq!(refused::<Name>(json!("")), "invalid length 0, expected a string of 1 to 4 characters");
        assert_eq!(refused::<Text<0, 200>>(json!("a".repeat(201))), "invalid length 201, expected a string of at most 200 characters");
        assert_eq!(refused::<Text<1, { usize::MAX }>>(json!("")), "invalid length 0, expected a string of at least 1 character");
        assert!(refused::<Text<0, 3>>(json!(3)).starts_with("invalid type: integer `3`"));
    }

    #[test]
    fn a_bounded_list_refuses_one_too_long_or_too_short() {
        type Few = Bounded<u8, 1, 2>;
        assert_eq!(serde_json::from_value::<Few>(json!([1, 2])).unwrap().0, vec![1, 2]);
        assert_eq!(refused::<Few>(json!([1, 2, 3])), "invalid length 3, expected a list of 1 to 2 items");
        assert_eq!(refused::<Few>(json!([])), "invalid length 0, expected a list of 1 to 2 items");
        // Read from text, the length isn't known ahead: the rest is counted.
        assert_eq!(serde_json::from_str::<Few>("[1, 2, 3, 4]").unwrap_err().to_string(), "invalid length 4, expected a list of 1 to 2 items at line 1 column 12");
        // The items past the bound aren't read, so their type doesn't matter.
        assert!(serde_json::from_str::<Few>(r#"[1, 2, "x"]"#).unwrap_err().to_string().starts_with("invalid length 3"));
        assert!(refused::<Few>(json!([1, "x"])).starts_with("invalid type: string \"x\""));
        assert_eq!(refused::<Bounded<u8, 0, 16>>(json!(vec![0; 17])), "invalid length 17, expected a list of at most 16 items");
    }

    #[test]
    fn a_pattern_is_read_as_js_reads_it() {
        assert_eq!(rust_syntax(r"^\d{1,4}[mhd]$"), "^[0-9]{1,4}[mhd]$");
        assert_eq!(rust_syntax(r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]$"#), r#"^(?:/|~/)[^"\\%\x{0000}-\x{001f}\x{007f}]$"#);
        assert_eq!(rust_syntax(r"^~(\/.*)?$"), r"^~(/[^\n\r\x{2028}\x{2029}]*)?$");
        let digits = Pattern::new(r"^\d+$");
        assert!(digits.test("2026") && !digits.test("٢٠٢٦"), "\\d is ASCII's digits");
        let home = Pattern::new(r"^~(\/.*)?$");
        assert!(home.test("~/a") && !home.test("~/a\nb") && !home.test("~/a\u{2028}"), ". stops at line terminators");
        let path = Pattern::new(r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"#);
        assert!(path.test("~/.ssh/id") && !path.test("~/a\"b") && !path.test("~/a\u{7f}") && !path.test("/a%b"));
        assert!(Pattern::new("^(?:@[a-z][a-z0-9-]{1,31}:)?project:").test("project:/x/y"), "unanchored at the end, as RegExp.test is");
        assert!(!Pattern::new("[").compiles() && !Pattern::new("[").test("["), "a pattern that doesn't compile matches nothing");
    }

    crate::pattern! {
        // A test id.
        struct TestId(r"^t_[0-9]{2}$");
    }

    #[test]
    fn a_patterned_string_refuses_one_that_does_not_match() {
        assert_eq!(serde_json::from_value::<TestId>(json!("t_01")).unwrap().as_str(), "t_01");
        assert_eq!(refused::<TestId>(json!("t_1")), r#"invalid value: string "t_1", expected a string matching ^t_[0-9]{2}$"#);
        let long = refused::<TestId>(json!("x".repeat(1000)));
        assert!(long.len() < 200, "a long string isn't shown whole: {long}");
    }
}
