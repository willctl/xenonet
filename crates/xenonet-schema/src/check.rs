//! Data checked against XenoNet's schemas, as core/schema/check.js checks
//! it with zod: the same issues (paths, codes and zod's messages), and with
//! `strict`, the keys the data has that its schema doesn't declare.
//!
//! A schema here is its JSON Schema document, as npm run schemas writes it
//! into schemas/ from the zod definitions. zod writes each kind of schema
//! in its own shape, so the shapes say what zod would do: a `type` array is
//! a union of bare types, `anyOf` with `{ "type": "null" }` last is
//! .nullable(), `oneOf` is a discriminated union, `allOf` an intersection,
//! `propertyNames` a record, `additionalProperties: false` a strict object
//! and `{}` a loose one, `prefixItems` a tuple.
//!
//! What zod does is followed where the output shows it: a type that's
//! wrong stops the checks after it, but a length check still runs on
//! anything with a length; a union that fits no option reports every
//! option's issues (check() then keeps the closest), unless only one option
//! got as far as its checks, which then speaks for the union; a
//! discriminated union with an unknown discriminator names the ones it
//! knows. A value is `Data`, which holds what JSON can't: undefined, NaN,
//! the infinities, and JS objects JSON has no form for (a Date, a Map).
//!
//! What a JSON Schema document no longer says: the order of a string's or
//! a number's checks (taken as zod's usual order: a format, the least, the
//! most, then patterns), whether equal least and most lengths were one
//! .length() (taken as one), and whether `additionalProperties: false` was
//! a strict object or z.object() (taken as strict; XenoNet's objects are
//! all loose or strict).

use crate::jsregex::{self, Regex};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fmt::Write as _;
use xenonet_base::{json, text};

// ---------- values ----------

/// A JS value, as check() sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Data>),
    /// Anything `typeof` calls an object, but null and arrays: a Date or a
    /// Map is one too, with no keys of its own.
    Obj(Object),
    /// A function, with its length (how many parameters it names).
    Fn(f64),
    /// What `typeof` calls a bigint or a symbol.
    Other(&'static str),
}

/// An object's own enumerable keys and their values, in JS's order; for one
/// that isn't a plain object, its constructor's name, which zod's messages
/// give ("received Date"), and whether zod's isPlainObject takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct Object {
    entries: Vec<(String, Data)>,
    index: HashMap<String, usize>,
    pub class: Option<String>,
    pub plain: bool,
}

impl Default for Object {
    fn default() -> Self {
        Object { entries: Vec::new(), index: HashMap::new(), class: None, plain: true }
    }
}

impl Object {
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a key, which keeps its place when it's set again.
    pub fn insert(&mut self, key: String, value: Data) {
        match self.index.get(&key) {
            Some(&i) => self.entries[i].1 = value,
            None => {
                self.index.insert(key.clone(), self.entries.len());
                self.entries.push((key, value));
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&Data> {
        self.index.get(key).map(|&i| &self.entries[i].1)
    }

    pub fn entries(&self) -> &[(String, Data)] {
        &self.entries
    }
}

impl From<&Value> for Data {
    fn from(v: &Value) -> Self {
        match v {
            Value::Null => Data::Null,
            Value::Bool(b) => Data::Bool(*b),
            Value::Number(n) => Data::Num(n.as_f64().unwrap_or(f64::NAN)),
            Value::String(s) => Data::Str(s.clone()),
            Value::Array(a) => Data::Arr(a.iter().map(Data::from).collect()),
            Value::Object(m) => {
                let mut o = Object::new();
                for (k, x) in m {
                    o.insert(k.clone(), Data::from(x));
                }
                Data::Obj(o)
            }
        }
    }
}

impl Data {
    // An object to zod's object schemas: typeof "object", not null, not an array.
    fn is_object(&self) -> bool {
        matches!(self, Data::Obj(_))
    }

    fn nullish(&self) -> bool {
        matches!(self, Data::Null | Data::Undefined)
    }
}

// The type zod names in "received …": util.parsedType, and a number that
// isn't finite as String() writes it.
fn received(v: &Data) -> String {
    match v {
        Data::Undefined => "undefined".into(),
        Data::Null => "null".into(),
        Data::Bool(_) => "boolean".into(),
        Data::Num(n) if n.is_nan() => "NaN".into(),
        Data::Num(n) if n.is_infinite() => json::number_string(*n),
        Data::Num(_) => "number".into(),
        Data::Str(_) => "string".into(),
        Data::Arr(_) => "array".into(),
        Data::Obj(o) => o.class.clone().unwrap_or_else(|| "object".into()),
        Data::Fn(_) => "function".into(),
        Data::Other(kind) => (*kind).into(),
    }
}

// SameValueZero, as a Set or Map of zod's compares a value with a literal.
fn same(x: &Value, v: &Data) -> bool {
    match (x, v) {
        (Value::Null, Data::Null) => true,
        (Value::Bool(a), Data::Bool(b)) => a == b,
        (Value::Number(a), Data::Num(b)) => a.as_f64() == Some(*b),
        (Value::String(a), Data::Str(b)) => a == b,
        _ => false,
    }
}

fn same_data(a: &Data, b: &Data) -> bool {
    match (a, b) {
        (Data::Undefined, Data::Undefined) | (Data::Null, Data::Null) => true,
        (Data::Bool(x), Data::Bool(y)) => x == y,
        (Data::Num(x), Data::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        (Data::Str(x), Data::Str(y)) => x == y,
        _ => false,
    }
}

// String(value), for a discriminator's value in a message.
fn data_string(v: &Data) -> String {
    match v {
        Data::Undefined => "undefined".into(),
        Data::Null => "null".into(),
        Data::Bool(b) => b.to_string(),
        Data::Num(n) => json::number_string(*n),
        Data::Str(s) => s.clone(),
        _ => "[object Object]".into(),
    }
}

// ---------- paths ----------

/// A step in a path: an index, or a key.
#[derive(Clone, Debug, PartialEq)]
pub enum PathKey {
    Num(f64),
    Key(String),
}

// /^[A-Za-z_$][\w$-]*$/
fn is_ident(k: &str) -> bool {
    let b = k.as_bytes();
    let head = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c == b'$';
    let rest = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-';
    !b.is_empty() && head(b[0]) && b[1..].iter().all(|&c| rest(c))
}

/// A path as it reads in a message: `nodes[3].ranAs.id`, `layout["project:/x"]`.
pub fn path_text(path: &[PathKey]) -> String {
    let mut out = String::new();
    for key in path {
        match key {
            PathKey::Num(n) => {
                let _ = write!(out, "[{}]", json::number_string(*n));
            }
            PathKey::Key(k) if is_ident(k) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(k);
            }
            PathKey::Key(k) => {
                let _ = write!(out, "[{}]", json::stringify(&Value::String(k.clone()), 0));
            }
        }
    }
    out
}

// ---------- issues ----------

// Whether an issue lets the checks after it run: zod's `continue`, which a
// check sets (true, or false to stop even length checks) and a schema's own
// type issue leaves out.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cont {
    Unset,
    Stop,
    Go,
}

#[derive(Clone, Debug)]
enum What {
    InvalidType { expected: &'static str, received: String },
    InvalidValue { values: Vec<Value> },
    TooBig { origin: &'static str, maximum: f64, inclusive: bool, exact: bool },
    TooSmall { origin: &'static str, minimum: f64, inclusive: bool, exact: bool },
    InvalidFormat { format: String, pattern: Option<String>, text: Option<String> },
    NotMultipleOf { divisor: f64 },
    UnrecognizedKeys { keys: Vec<String> },
    InvalidKey,
    InvalidUnion { errors: Vec<Vec<Issue>>, options: Vec<Data>, more_than_one: bool },
}

#[derive(Clone, Debug)]
struct Issue {
    path: Vec<PathKey>,
    what: What,
    cont: Cont,
}

impl Issue {
    fn new(what: What, cont: Cont) -> Self {
        Issue { path: Vec::new(), what, cont }
    }
}

fn invalid_type(expected: &'static str, v: &Data) -> Issue {
    Issue::new(What::InvalidType { expected, received: received(v) }, Cont::Unset)
}

fn prefixed(key: PathKey, issues: Vec<Issue>) -> impl Iterator<Item = Issue> {
    issues.into_iter().map(move |mut i| {
        i.path.insert(0, key.clone());
        i
    })
}

fn aborted(issues: &[Issue]) -> bool {
    issues.iter().any(|i| i.cont != Cont::Go)
}

fn sizing(origin: &str) -> Option<&'static str> {
    match origin {
        "string" => Some("characters"),
        "file" => Some("bytes"),
        "array" | "set" => Some("items"),
        "map" => Some("entries"),
        _ => None,
    }
}

// What each string format is called in zod's English messages.
fn format_name(format: &str) -> &str {
    match format {
        "regex" | "template_literal" => "input",
        "email" => "email address",
        "url" => "URL",
        "emoji" => "emoji",
        "uuid" => "UUID",
        "uuidv4" => "UUIDv4",
        "uuidv6" => "UUIDv6",
        "nanoid" => "nanoid",
        "guid" => "GUID",
        "cuid" => "cuid",
        "cuid2" => "cuid2",
        "ulid" => "ULID",
        "xid" => "XID",
        "ksuid" => "KSUID",
        "datetime" => "ISO datetime",
        "date" => "ISO date",
        "time" => "ISO time",
        "duration" => "ISO duration",
        "ipv4" => "IPv4 address",
        "ipv6" => "IPv6 address",
        "mac" => "MAC address",
        "cidrv4" => "IPv4 range",
        "cidrv6" => "IPv6 range",
        "base64" => "base64-encoded string",
        "base64url" => "base64url-encoded string",
        "json_string" => "JSON string",
        "e164" => "E.164 number",
        "currency_code" => "currency code",
        "credit_card" => "credit card number",
        "iban" => "IBAN",
        "jwt" => "JWT",
        other => other,
    }
}

// util.stringifyPrimitive: a string in quotes, anything else as String().
fn primitive(v: &Value) -> String {
    match v {
        Value::String(s) => format!("\"{s}\""),
        other => text::js_string(other),
    }
}

impl What {
    fn code(&self) -> &'static str {
        match self {
            What::InvalidType { .. } => "invalid_type",
            What::InvalidValue { .. } => "invalid_value",
            What::TooBig { .. } => "too_big",
            What::TooSmall { .. } => "too_small",
            What::InvalidFormat { .. } => "invalid_format",
            What::NotMultipleOf { .. } => "not_multiple_of",
            What::UnrecognizedKeys { .. } => "unrecognized_keys",
            What::InvalidKey => "invalid_key",
            What::InvalidUnion { .. } => "invalid_union",
        }
    }

    // zod's English messages (zod/v4/locales/en.js).
    fn message(&self) -> String {
        let n = json::number_string;
        match self {
            What::InvalidType { expected, received } => format!("Invalid input: expected {expected}, received {received}"),
            What::InvalidValue { values } if values.len() == 1 => format!("Invalid input: expected {}", primitive(&values[0])),
            What::InvalidValue { values } => format!("Invalid option: expected one of {}", values.iter().map(primitive).collect::<Vec<_>>().join("|")),
            What::TooBig { origin, maximum, inclusive, exact } => {
                let adj = if *exact {
                    "exactly "
                } else if *inclusive {
                    "<="
                } else {
                    "<"
                };
                match sizing(origin) {
                    Some(unit) => format!("Too big: expected {origin} to have {adj}{} {unit}", n(*maximum)),
                    None => format!("Too big: expected {origin} to be {adj}{}", n(*maximum)),
                }
            }
            What::TooSmall { origin, minimum, inclusive, exact } => {
                let adj = if *exact {
                    "exactly "
                } else if *inclusive {
                    ">="
                } else {
                    ">"
                };
                match sizing(origin) {
                    Some(unit) => format!("Too small: expected {origin} to have {adj}{} {unit}", n(*minimum)),
                    None => format!("Too small: expected {origin} to be {adj}{}", n(*minimum)),
                }
            }
            What::InvalidFormat { format, pattern, text } => match format.as_str() {
                "starts_with" => format!("Invalid string: must start with \"{}\"", text.as_deref().unwrap_or("")),
                "ends_with" => format!("Invalid string: must end with \"{}\"", text.as_deref().unwrap_or("")),
                "includes" => format!("Invalid string: must include \"{}\"", text.as_deref().unwrap_or("")),
                "regex" => format!("Invalid string: must match pattern {}", pattern.as_deref().unwrap_or("")),
                f => format!("Invalid {}", format_name(f)),
            },
            What::NotMultipleOf { divisor } => format!("Invalid number: must be a multiple of {}", n(*divisor)),
            What::UnrecognizedKeys { keys } => {
                let s = if keys.len() > 1 { "s" } else { "" };
                format!("Unrecognized key{s}: {}", keys.iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(", "))
            }
            What::InvalidKey => "Invalid key in record".into(),
            What::InvalidUnion { options, .. } if !options.is_empty() => {
                format!("Invalid discriminator value. Expected {}", options.iter().map(|o| format!("'{}'", data_string(o))).collect::<Vec<_>>().join(" | "))
            }
            What::InvalidUnion { more_than_one: true, .. } => "Invalid input: more than one option matched".into(),
            What::InvalidUnion { .. } => "Invalid input".into(),
        }
    }
}

// ---------- compiled schemas ----------

#[derive(Clone, Debug)]
enum Check {
    Format { format: String, regex: Option<Regex>, pattern: Option<String>, text: Option<String> },
    MinLen(f64),
    MaxLen(f64),
    ExactLen(f64),
    Pattern(Regex, String),
    Int,
    Min(f64, bool),
    Max(f64, bool),
    MultipleOf(f64),
}

impl Check {
    // Length checks run on anything with a length, after a wrong type too
    // (zod's `when`); the rest stop once something has failed.
    fn on_length(&self) -> bool {
        matches!(self, Check::MinLen(_) | Check::MaxLen(_) | Check::ExactLen(_))
    }
}

#[derive(Clone, Debug)]
struct Prop {
    key: String,
    node: usize,
    required: bool,
}

#[derive(Clone, Debug)]
enum Extra {
    Strict,
    Strip,
    Loose(usize),
}

#[derive(Clone, Debug)]
enum Kind {
    Ref(usize),
    Any,
    Never,
    Values(Vec<Value>),
    Types(Vec<String>),
    Str(Vec<Check>),
    Num(Vec<Check>),
    Bool,
    Null,
    Array { items: usize, checks: Vec<Check> },
    Tuple { items: Vec<usize>, rest: Option<usize>, min: usize, closed: bool },
    Object { props: Vec<Prop>, index: HashMap<String, usize>, extra: Extra, passthrough: bool },
    Record { key: usize, value: usize, keys: Option<Vec<Value>>, exhaustive: bool },
    Nullable(usize),
    Union(Vec<usize>),
    Variants { key: String, by_value: Vec<(Data, Option<usize>)>, options: Vec<usize> },
    Xor(Vec<usize>),
    All { members: Vec<usize>, passthrough: bool },
}

/// JSON Schema documents, compiled for checking: each document's root by
/// its place in the list.
#[derive(Clone, Debug)]
pub struct Checker {
    nodes: Vec<Kind>,
    roots: Vec<usize>,
}

const ANY: usize = 0;
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

// Keywords that constrain a value; the rest are notes.
const CONSTRAINTS: &[&str] = &[
    "type",
    "const",
    "enum",
    "anyOf",
    "oneOf",
    "allOf",
    "not",
    "properties",
    "required",
    "additionalProperties",
    "propertyNames",
    "patternProperties",
    "items",
    "prefixItems",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "pattern",
    "format",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
];

struct Compiler<'a> {
    docs: &'a [Value],
    by_uri: HashMap<&'a str, usize>,
    nodes: Vec<Kind>,
    seen: HashMap<*const Value, usize>,
}

/// Compiles documents, which may refer to each other by $id.
pub fn compile(docs: &[Value]) -> Result<Checker, String> {
    let by_uri = docs.iter().enumerate().filter_map(|(i, d)| Some((d.get("$id")?.as_str()?, i))).collect();
    let mut c = Compiler { docs, by_uri, nodes: vec![Kind::Any], seen: HashMap::new() };
    let mut roots = Vec::new();
    for (i, d) in docs.iter().enumerate() {
        roots.push(c.go(i, d)?);
    }
    Ok(Checker { nodes: c.nodes, roots })
}

fn num(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn xs<'v>(m: &'v Map<String, Value>, key: &str) -> Option<&'v Value> {
    m.get("x-xenonet").and_then(|x| x.get(key))
}

// The text a starts_with, ends_with or includes pattern was made from
// (util.escapeRegex undone).
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn regex(source: &str) -> Result<Regex, String> {
    jsregex::compile(source).map_err(|e| format!("can't check the pattern /{source}/: {e}"))
}

// The length checks of a string or an array: one exact check when the least
// and the most are the same (.length()).
fn lengths(min: Option<f64>, max: Option<f64>) -> Vec<Check> {
    match (min, max) {
        (Some(a), Some(b)) if a == b => vec![Check::ExactLen(a)],
        _ => min.map(Check::MinLen).into_iter().chain(max.map(Check::MaxLen)).collect(),
    }
}

impl<'a> Compiler<'a> {
    fn resolve(&self, doc: usize, r: &str) -> Result<(usize, &'a Value), String> {
        let (base, fragment) = r.split_once('#').unwrap_or((r, ""));
        let at = if base.is_empty() { doc } else { *self.by_uri.get(base).ok_or_else(|| format!("no schema {base} to refer to"))? };
        let mut node = &self.docs[at];
        if !fragment.is_empty() {
            for part in fragment.strip_prefix('/').ok_or_else(|| format!("can't follow {r}"))?.split('/') {
                let part = part.replace("~1", "/").replace("~0", "~");
                node = match node {
                    Value::Object(m) => m.get(&part),
                    Value::Array(a) => part.parse::<usize>().ok().and_then(|i| a.get(i)),
                    _ => None,
                }
                .ok_or_else(|| format!("can't follow {r}"))?;
            }
        }
        Ok((at, node))
    }

    fn go(&mut self, doc: usize, node: &'a Value) -> Result<usize, String> {
        if let Some(&i) = self.seen.get(&std::ptr::from_ref::<Value>(node)) {
            return Ok(i);
        }
        let at = self.nodes.len();
        self.nodes.push(Kind::Any);
        self.seen.insert(std::ptr::from_ref::<Value>(node), at);
        let kind = self.kind(doc, node)?;
        self.nodes[at] = kind;
        Ok(at)
    }

    fn kind(&mut self, doc: usize, node: &'a Value) -> Result<Kind, String> {
        let m = match node {
            Value::Bool(true) => return Ok(Kind::Any),
            Value::Bool(false) => return Ok(Kind::Never),
            Value::Object(m) => m,
            _ => return Ok(Kind::Any),
        };
        if let Some(r) = m.get("$ref") {
            if let Some(k) = m.keys().find(|k| CONSTRAINTS.contains(&k.as_str())) {
                return Err(format!("a $ref beside {k} isn't something zod writes"));
            }
            let r = r.as_str().ok_or("a $ref that isn't text")?;
            let (d, target) = self.resolve(doc, r)?;
            return Ok(Kind::Ref(self.go(d, target)?));
        }
        if m.contains_key("not") {
            return Ok(Kind::Never);
        }
        if let Some(c) = m.get("const") {
            return Ok(Kind::Values(vec![c.clone()]));
        }
        if let Some(e) = m.get("enum") {
            return Ok(Kind::Values(e.as_array().cloned().unwrap_or_default()));
        }
        if let Some(Value::Array(options)) = m.get("anyOf") {
            let null_last = options.len() == 2 && options[1].as_object().is_some_and(|o| o.len() == 1 && o.get("type") == Some(&Value::String("null".into())));
            if null_last {
                return Ok(Kind::Nullable(self.go(doc, &options[0])?));
            }
            let compiled = options.iter().map(|o| self.go(doc, o)).collect::<Result<Vec<_>, _>>()?;
            return Ok(match compiled.len() {
                0 => Kind::Never,
                1 => Kind::Ref(compiled[0]),
                _ => Kind::Union(compiled),
            });
        }
        if let Some(Value::Array(options)) = m.get("oneOf") {
            let compiled = options.iter().map(|o| self.go(doc, o)).collect::<Result<Vec<_>, _>>()?;
            let key = xs(m, "discriminator").and_then(Value::as_str).map(str::to_string).or_else(|| self.discriminator(&compiled));
            return Ok(match key {
                Some(key) => {
                    let mut by_value: Vec<(Data, Option<usize>)> = Vec::new();
                    for &o in &compiled {
                        for v in self.prop_values(o, &key).unwrap_or_default() {
                            match by_value.iter_mut().find(|(x, _)| same_data(x, &v)) {
                                // Two options that can both leave it out claim undefined: neither gets it.
                                Some(slot) => slot.1 = None,
                                None => by_value.push((v, Some(o))),
                            }
                        }
                    }
                    Kind::Variants { key, by_value, options: compiled }
                }
                None => Kind::Xor(compiled),
            });
        }
        let ty = m.get("type");
        if let Some(Value::Array(members)) = m.get("allOf") {
            if ty != Some(&Value::String("string".into())) {
                let compiled = members.iter().map(|o| self.go(doc, o)).collect::<Result<Vec<_>, _>>()?;
                return Ok(Kind::All { members: compiled, passthrough: text::truthy(xs(m, "passthrough")) });
            }
        }
        match ty {
            Some(Value::Array(types)) => {
                let types: Vec<String> = types.iter().filter_map(|t| t.as_str().map(str::to_string)).collect();
                Ok(Kind::Types(types))
            }
            Some(Value::String(t)) => match t.as_str() {
                "string" => Self::string(m),
                "number" | "integer" => Ok(Self::number(m, t == "integer")),
                "boolean" => Ok(Kind::Bool),
                "null" => Ok(Kind::Null),
                "array" => self.array(doc, m),
                "object" => self.object(doc, m),
                other => Err(format!("a type of {other} isn't something zod writes")),
            },
            _ if m.contains_key("properties") => self.object(doc, m),
            _ => Ok(Kind::Any),
        }
    }

    fn string(m: &Map<String, Value>) -> Result<Kind, String> {
        let mut patterns: Vec<String> = m.get("pattern").and_then(Value::as_str).map(str::to_string).into_iter().collect();
        for p in m.get("allOf").and_then(Value::as_array).into_iter().flatten() {
            if let Some(p) = p.get("pattern").and_then(Value::as_str) {
                patterns.push(p.to_string());
            }
        }
        let mut checks = Vec::new();
        let mut patterns = patterns.into_iter();
        if let Some(f) = m.get("format").and_then(Value::as_str) {
            let format = match f {
                "date-time" => "datetime",
                "uri" => "url",
                "json-string" => "json_string",
                other => other,
            }
            .to_string();
            // A format's own pattern comes first: the format is its first check.
            let pattern = patterns.next();
            let text = match (format.as_str(), pattern.as_deref()) {
                ("starts_with", Some(p)) => Some(unescape(p.strip_prefix('^').unwrap_or(p).strip_suffix(".*").unwrap_or(p))),
                ("ends_with", Some(p)) => Some(unescape(p.strip_prefix(".*").unwrap_or(p).strip_suffix('$').unwrap_or(p))),
                ("includes", Some(p)) => Some(unescape(p)),
                _ => None,
            };
            let regex = pattern.as_deref().map(regex).transpose()?;
            checks.push(Check::Format { format, regex, pattern: pattern.map(|p| format!("/{p}/")), text });
        }
        checks.extend(lengths(num(m.get("minLength")), num(m.get("maxLength"))));
        for p in patterns {
            checks.push(Check::Pattern(regex(&p)?, format!("/{p}/")));
        }
        Ok(Kind::Str(checks))
    }

    fn number(m: &Map<String, Value>, int: bool) -> Kind {
        let mut checks = Vec::new();
        if int {
            checks.push(Check::Int);
        }
        // An int's own range is its first check, not a minimum or maximum of its own.
        let min = num(m.get("exclusiveMinimum")).map(|v| (v, false)).or_else(|| num(m.get("minimum")).map(|v| (v, true)));
        let max = num(m.get("exclusiveMaximum")).map(|v| (v, false)).or_else(|| num(m.get("maximum")).map(|v| (v, true)));
        if let Some((v, inclusive)) = min {
            if !(int && inclusive && v == -MAX_SAFE) {
                checks.push(Check::Min(v, inclusive));
            }
        }
        if let Some((v, inclusive)) = max {
            if !(int && inclusive && v == MAX_SAFE) {
                checks.push(Check::Max(v, inclusive));
            }
        }
        if let Some(d) = num(m.get("multipleOf")) {
            checks.push(Check::MultipleOf(d));
        }
        Kind::Num(checks)
    }

    fn array(&mut self, doc: usize, m: &'a Map<String, Value>) -> Result<Kind, String> {
        if let Some(Value::Array(prefix)) = m.get("prefixItems") {
            let items = prefix.iter().map(|o| self.go(doc, o)).collect::<Result<Vec<_>, _>>()?;
            let (rest, closed) = match m.get("items") {
                Some(Value::Bool(false)) => (None, true),
                Some(x) => (Some(self.go(doc, x)?), false),
                None => (Some(ANY), false),
            };
            let min = num(m.get("minItems")).map_or(0, |n| n as usize);
            return Ok(Kind::Tuple { items, rest, min, closed });
        }
        let items = match m.get("items") {
            Some(x) => self.go(doc, x)?,
            None => ANY,
        };
        Ok(Kind::Array { items, checks: lengths(num(m.get("minItems")), num(m.get("maxItems"))) })
    }

    fn object(&mut self, doc: usize, m: &'a Map<String, Value>) -> Result<Kind, String> {
        if m.contains_key("patternProperties") {
            return Err("patternProperties (z.looseRecord with a pattern) isn't supported".into());
        }
        let additional = m.get("additionalProperties");
        if let Some(names) = m.get("propertyNames") {
            let key = self.go(doc, names)?;
            let value = match additional {
                Some(a) => self.go(doc, a)?,
                None => ANY,
            };
            let keys = self.values_of(key).map(|vs| vs.iter().filter_map(data_value).collect());
            return Ok(Kind::Record { key, value, keys, exhaustive: m.contains_key("required") });
        }
        let required: Vec<&str> = m.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut props = Vec::new();
        let mut index = HashMap::new();
        if let Some(Value::Object(p)) = m.get("properties") {
            for (k, v) in p {
                let node = self.go(doc, v)?;
                index.insert(k.clone(), props.len());
                props.push(Prop { key: k.clone(), node, required: required.contains(&k.as_str()) });
            }
        }
        let extra = match additional {
            None => Extra::Strip,
            Some(Value::Bool(false)) => Extra::Strict,
            Some(Value::Bool(true)) => Extra::Loose(ANY),
            Some(a) => Extra::Loose(self.go(doc, a)?),
        };
        Ok(Kind::Object { props, index, extra, passthrough: text::truthy(xs(m, "passthrough")) })
    }

    fn deref(&self, mut n: usize) -> usize {
        for _ in 0..64 {
            match self.nodes[n] {
                Kind::Ref(t) => n = t,
                _ => break,
            }
        }
        n
    }

    // The values a schema holds to, as zod's `values` has them, for a
    // discriminator: literals and enums, null, and those made nullable.
    fn values_of(&self, n: usize) -> Option<Vec<Data>> {
        match &self.nodes[self.deref(n)] {
            Kind::Values(vs) => Some(vs.iter().map(Data::from).collect()),
            Kind::Null => Some(vec![Data::Null]),
            Kind::Types(t) if t.iter().all(|t| t == "null") => Some(vec![Data::Null]),
            Kind::Nullable(inner) => self.values_of(*inner).map(|mut v| {
                v.push(Data::Null);
                v
            }),
            Kind::Union(options) => {
                let mut out = Vec::new();
                for o in options {
                    out.extend(self.values_of(*o)?);
                }
                Some(out)
            }
            _ => None,
        }
    }

    // An option's values for the discriminator (zod's propValues), with
    // undefined when it may be left out.
    fn prop_values(&self, n: usize, key: &str) -> Option<Vec<Data>> {
        match &self.nodes[self.deref(n)] {
            Kind::Object { props, index, .. } => {
                let p = &props[*index.get(key)?];
                let mut vs = self.values_of(p.node)?;
                if !p.required {
                    vs.push(Data::Undefined);
                }
                Some(vs)
            }
            Kind::Variants { options, .. } => {
                let mut out: Vec<Data> = Vec::new();
                let mut undefined = 0;
                for o in options {
                    let vs = self.prop_values(*o, key)?;
                    if vs.contains(&Data::Undefined) {
                        undefined += 1;
                    }
                    for v in vs {
                        if !out.iter().any(|x| same_data(x, &v)) {
                            out.push(v);
                        }
                    }
                }
                if undefined > 1 {
                    out.retain(|v| *v != Data::Undefined);
                }
                Some(out)
            }
            _ => None,
        }
    }

    // The key a oneOf's options are told apart by, which the document
    // doesn't name: the first key of the first option that every option
    // holds to values for, no two options sharing one (zod refuses a
    // discriminated union where they do, so no other key can be it).
    fn discriminator(&self, options: &[usize]) -> Option<String> {
        self.keys_of(*options.first()?).into_iter().find(|k| {
            let mut seen: Vec<Data> = Vec::new();
            options.iter().all(|o| match self.prop_values(*o, k) {
                Some(vs) if !vs.is_empty() => vs.into_iter().filter(|v| *v != Data::Undefined).all(|v| {
                    let fresh = !seen.iter().any(|x| same_data(x, &v));
                    seen.push(v);
                    fresh
                }),
                _ => false,
            })
        })
    }

    // The keys an option has, to look for a discriminator among: an
    // object's own, or a discriminated union's key and its first option's.
    fn keys_of(&self, n: usize) -> Vec<String> {
        match &self.nodes[self.deref(n)] {
            Kind::Object { props, .. } => props.iter().map(|p| p.key.clone()).collect(),
            Kind::Variants { key, options, .. } => {
                let mut keys = vec![key.clone()];
                keys.extend(options.first().map(|o| self.keys_of(*o)).unwrap_or_default().into_iter().filter(|k| k != key));
                keys
            }
            _ => Vec::new(),
        }
    }
}

fn data_value(d: &Data) -> Option<Value> {
    match d {
        Data::Str(s) => Some(Value::String(s.clone())),
        Data::Num(n) => Some(json::number(*n)),
        _ => None,
    }
}

// A value's length, as `when` sees it for a length check: a string's code
// points, an array's items, or an object's own `length`; None for what has
// no length.
fn length_of(v: &Data) -> Option<f64> {
    match v {
        Data::Str(s) => Some(s.chars().count() as f64),
        Data::Arr(a) => Some(a.len() as f64),
        Data::Obj(o) => match o.get("length") {
            None | Some(Data::Undefined) => None,
            Some(l) => Some(to_number(l)),
        },
        Data::Fn(n) => Some(*n),
        _ => None,
    }
}

// Number(value), for an odd `length` compared with a bound.
fn to_number(v: &Data) -> f64 {
    match v {
        Data::Num(n) => *n,
        Data::Null => 0.0,
        Data::Bool(b) => f64::from(u8::from(*b)),
        Data::Str(s) => text::js_number(Some(&Value::String(s.clone()))),
        Data::Arr(a) if a.is_empty() => 0.0,
        Data::Arr(a) if a.len() == 1 => match &a[0] {
            Data::Undefined | Data::Null => 0.0,
            x @ (Data::Num(_) | Data::Str(_) | Data::Bool(_)) => to_number(&Data::Str(data_string(x))),
            _ => f64::NAN,
        },
        _ => f64::NAN,
    }
}

fn length_origin(v: &Data) -> &'static str {
    match v {
        Data::Arr(_) => "array",
        Data::Str(_) => "string",
        _ => "unknown",
    }
}

fn is_number_string(k: &str) -> bool {
    // /^-?\d+(?:\.\d+)?$/
    let s = k.strip_prefix('-').unwrap_or(k);
    let (int, frac) = match s.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    !int.is_empty() && int.bytes().all(|b| b.is_ascii_digit()) && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// One of check()'s issues: where, zod's code, and its message.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub path: String,
    pub code: &'static str,
    pub message: String,
}

/// What check() gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checked {
    pub issues: Vec<Found>,
    pub undeclared: Vec<String>,
}

impl Checked {
    pub fn ok(&self) -> bool {
        self.issues.is_empty() && self.undeclared.is_empty()
    }
}

// zod's issues, flattened: a union that failed every option reports the
// option that came closest.
fn flatten(issues: &[Issue], at: &[PathKey], out: &mut Vec<Found>) {
    for issue in issues {
        let mut path = at.to_vec();
        path.extend(issue.path.iter().cloned());
        match &issue.what {
            What::InvalidUnion { errors, .. } if !errors.is_empty() => {
                let closest = errors.iter().skip(1).fold(&errors[0], |best, e| if e.len() < best.len() { e } else { best });
                flatten(closest, &path, out);
            }
            what => out.push(Found { path: path_text(&path), code: what.code(), message: what.message() }),
        }
    }
}

impl Checker {
    /// The root of the document at `i` in the list compiled.
    pub fn root(&self, i: usize) -> usize {
        self.roots[i]
    }

    /// check(): `data` against the schema at `root`.
    pub fn check(&self, root: usize, data: &Data, strict: bool) -> Checked {
        let issues = self.run(root, data);
        let mut out = Checked::default();
        flatten(&issues, &[], &mut out.issues);
        if strict && issues.is_empty() {
            self.undeclared(root, data, &mut Vec::new(), &mut out.undeclared);
        }
        out
    }

    fn run(&self, n: usize, v: &Data) -> Vec<Issue> {
        match &self.nodes[n] {
            Kind::Ref(t) => self.run(*t, v),
            Kind::Any => Vec::new(),
            Kind::Never => vec![invalid_type("never", v)],
            Kind::Values(vals) => {
                if vals.iter().any(|x| same(x, v)) {
                    Vec::new()
                } else {
                    vec![Issue::new(What::InvalidValue { values: vals.clone() }, Cont::Unset)]
                }
            }
            Kind::Types(types) => {
                if types.iter().any(|t| fits_type(t, v)) {
                    return Vec::new();
                }
                // Each option fails the same way, so the first speaks for them.
                let expected = match types.first().map(String::as_str) {
                    Some("string") => "string",
                    Some("number") => "number",
                    Some("integer") if matches!(v, Data::Num(n) if n.is_finite()) => "int",
                    Some("integer") => "number",
                    Some("boolean") => "boolean",
                    Some("null") => "null",
                    Some("array") => "array",
                    Some("object") => "object",
                    _ => "never",
                };
                vec![invalid_type(expected, v)]
            }
            Kind::Str(checks) => {
                let mut issues = Vec::new();
                if !matches!(v, Data::Str(_)) {
                    issues.push(invalid_type("string", v));
                }
                run_checks(checks, v, &mut issues);
                issues
            }
            Kind::Num(checks) => {
                let mut issues = Vec::new();
                if !matches!(v, Data::Num(n) if n.is_finite()) {
                    issues.push(invalid_type("number", v));
                }
                run_checks(checks, v, &mut issues);
                issues
            }
            Kind::Bool => match v {
                Data::Bool(_) => Vec::new(),
                _ => vec![invalid_type("boolean", v)],
            },
            Kind::Null => match v {
                Data::Null => Vec::new(),
                _ => vec![invalid_type("null", v)],
            },
            Kind::Array { items, checks } => {
                let mut issues = Vec::new();
                match v {
                    Data::Arr(a) => {
                        for (i, x) in a.iter().enumerate() {
                            issues.extend(prefixed(PathKey::Num(i as f64), self.run(*items, x)));
                        }
                    }
                    _ => issues.push(invalid_type("array", v)),
                }
                run_checks(checks, v, &mut issues);
                issues
            }
            Kind::Tuple { items, rest, min, closed } => self.tuple(items, *rest, *min, *closed, v),
            Kind::Object { props, index, extra, .. } => self.object(props, index, extra, v),
            Kind::Record { key, value, keys, exhaustive } => self.record(*key, *value, keys.as_deref(), *exhaustive, v),
            Kind::Nullable(inner) => match v {
                Data::Null => Vec::new(),
                _ => self.run(*inner, v),
            },
            Kind::Union(options) => {
                let mut results = Vec::new();
                for o in options {
                    let r = self.run(*o, v);
                    if r.is_empty() {
                        return r;
                    }
                    results.push(r);
                }
                let mut going = results.iter().filter(|r| !aborted(r));
                if let (Some(only), None) = (going.next(), going.next()) {
                    return only.clone();
                }
                vec![Issue::new(What::InvalidUnion { errors: results, options: Vec::new(), more_than_one: false }, Cont::Unset)]
            }
            Kind::Variants { key, by_value, .. } => {
                if !v.is_object() {
                    return vec![invalid_type("object", v)];
                }
                let d = match v {
                    Data::Obj(o) => o.get(key).unwrap_or(&Data::Undefined),
                    _ => &Data::Undefined,
                };
                if let Some((_, Some(option))) = by_value.iter().find(|(x, _)| same_data(x, d)) {
                    return self.run(*option, v);
                }
                let options = by_value.iter().filter(|(_, o)| o.is_some()).map(|(x, _)| x.clone()).collect();
                let mut issue = Issue::new(What::InvalidUnion { errors: Vec::new(), options, more_than_one: false }, Cont::Unset);
                issue.path.push(PathKey::Key(key.clone()));
                vec![issue]
            }
            Kind::Xor(options) => {
                let results: Vec<Vec<Issue>> = options.iter().map(|o| self.run(*o, v)).collect();
                match results.iter().filter(|r| r.is_empty()).count() {
                    1 => Vec::new(),
                    0 => vec![Issue::new(What::InvalidUnion { errors: results, options: Vec::new(), more_than_one: false }, Cont::Unset)],
                    _ => vec![Issue::new(What::InvalidUnion { errors: Vec::new(), options: Vec::new(), more_than_one: true }, Cont::Unset)],
                }
            }
            Kind::All { members, .. } => self.all(members, v),
        }
    }

    fn tuple(&self, items: &[usize], rest: Option<usize>, min: usize, closed: bool, v: &Data) -> Vec<Issue> {
        let Data::Arr(a) = v else { return vec![invalid_type("tuple", v)] };
        let mut issues = Vec::new();
        if closed {
            if a.len() < min {
                return vec![Issue::new(What::TooSmall { origin: "array", minimum: min as f64, inclusive: true, exact: false }, Cont::Unset)];
            }
            if a.len() > items.len() {
                issues.push(Issue::new(What::TooBig { origin: "array", maximum: items.len() as f64, inclusive: true, exact: false }, Cont::Unset));
            }
        }
        // Past the least, an item may be left out, or undefined, as .optional() lets it.
        let results: Vec<Vec<Issue>> = items
            .iter()
            .enumerate()
            .map(|(i, n)| match a.get(i) {
                Some(Data::Undefined) | None if i >= min => Vec::new(),
                x => self.run(*n, x.unwrap_or(&Data::Undefined)),
            })
            .collect();
        if let Some(rest) = rest {
            for (j, x) in a.iter().enumerate().skip(items.len()) {
                issues.extend(prefixed(PathKey::Num(j as f64), self.run(rest, x)));
            }
        }
        for (i, r) in results.into_iter().enumerate() {
            if i >= a.len() && i >= min {
                break;
            }
            issues.extend(prefixed(PathKey::Num(i as f64), r));
        }
        issues
    }

    fn object(&self, props: &[Prop], index: &HashMap<String, usize>, extra: &Extra, v: &Data) -> Vec<Issue> {
        if !v.is_object() {
            return vec![invalid_type("object", v)];
        }
        let obj = match v {
            Data::Obj(o) => Some(o),
            _ => None,
        };
        let mut issues = Vec::new();
        for p in props {
            let found = obj.and_then(|o| o.get(&p.key));
            let value = found.unwrap_or(&Data::Undefined);
            if !p.required {
                // .optional(): left out, or undefined, is fine.
                if found.is_none() || *value == Data::Undefined {
                    continue;
                }
                issues.extend(prefixed(PathKey::Key(p.key.clone()), self.run(p.node, value)));
                continue;
            }
            let r = self.run(p.node, value);
            if !r.is_empty() {
                issues.extend(prefixed(PathKey::Key(p.key.clone()), r));
            } else if found.is_none() {
                let mut missing = invalid_type("nonoptional", &Data::Undefined);
                missing.path.push(PathKey::Key(p.key.clone()));
                issues.push(missing);
            }
        }
        let Some(o) = obj else { return issues };
        match extra {
            Extra::Strict => {
                let keys: Vec<String> = o.entries().iter().filter(|(k, _)| !index.contains_key(k)).map(|(k, _)| k.clone()).collect();
                if !keys.is_empty() {
                    // It says what the data is, not that it's wrong to read: it never aborts.
                    issues.push(Issue::new(What::UnrecognizedKeys { keys }, Cont::Go));
                }
            }
            Extra::Loose(n) if *n != ANY => {
                for (k, x) in o.entries() {
                    if k != "__proto__" && !index.contains_key(k) {
                        issues.extend(prefixed(PathKey::Key(k.clone()), self.run(*n, x)));
                    }
                }
            }
            _ => {}
        }
        issues
    }

    fn record(&self, key: usize, value: usize, keys: Option<&[Value]>, exhaustive: bool, v: &Data) -> Vec<Issue> {
        let o = match v {
            Data::Obj(o) if o.plain => o,
            _ => return vec![invalid_type("record", v)],
        };
        let mut issues = Vec::new();
        let mut unrecognized = Vec::new();
        match keys {
            Some(values) if exhaustive => {
                let mut own: Vec<String> = Vec::new();
                for k in values {
                    let name = text::js_string(k);
                    own.push(name.clone());
                    if name == "__proto__" {
                        continue;
                    }
                    let step = match k {
                        Value::Number(n) => PathKey::Num(n.as_f64().unwrap_or(f64::NAN)),
                        _ => PathKey::Key(name.clone()),
                    };
                    let kr = self.run(key, &Data::from(k));
                    if !kr.is_empty() {
                        let mut bad = Issue::new(What::InvalidKey, Cont::Unset);
                        bad.path.push(step);
                        issues.push(bad);
                        continue;
                    }
                    issues.extend(prefixed(step, self.run(value, o.get(&name).unwrap_or(&Data::Undefined))));
                }
                for (k, _) in o.entries() {
                    if !own.contains(k) {
                        unrecognized.push(k.clone());
                    }
                }
            }
            _ => {
                for (k, x) in o.entries() {
                    if k == "__proto__" {
                        continue;
                    }
                    let mut kr = self.run(key, &Data::Str(k.clone()));
                    // A numeric key is tried as a number too.
                    if !kr.is_empty() && is_number_string(k) {
                        let again = self.run(key, &Data::Num(k.parse().unwrap_or(f64::NAN)));
                        if again.is_empty() {
                            kr = again;
                        }
                    }
                    if !kr.is_empty() {
                        if keys.is_some() {
                            unrecognized.push(k.clone());
                        } else {
                            let mut bad = Issue::new(What::InvalidKey, Cont::Unset);
                            bad.path.push(PathKey::Key(k.clone()));
                            issues.push(bad);
                        }
                        continue;
                    }
                    issues.extend(prefixed(PathKey::Key(k.clone()), self.run(value, x)));
                }
            }
        }
        if !unrecognized.is_empty() {
            issues.push(Issue::new(What::UnrecognizedKeys { keys: unrecognized }, Cont::Go));
        }
        issues
    }

    // An intersection: every member's issues, but a key one member doesn't
    // know is reported only when none of them knows it.
    fn all(&self, members: &[usize], v: &Data) -> Vec<Issue> {
        let mut issues = Vec::new();
        let mut first_unknown: Option<Issue> = None;
        let mut key_issues: Vec<(String, Issue)> = Vec::new();
        let mut rejected: Vec<(String, Vec<bool>)> = Vec::new();
        for (side, m) in members.iter().enumerate() {
            for issue in self.run(*m, v) {
                let keys: Vec<String> = match &issue.what {
                    What::UnrecognizedKeys { keys } if issue.path.is_empty() => {
                        if first_unknown.is_none() {
                            first_unknown = Some(issue.clone());
                        }
                        keys.clone()
                    }
                    What::InvalidKey if issue.path.len() == 1 => {
                        let k = match &issue.path[0] {
                            PathKey::Key(k) => k.clone(),
                            PathKey::Num(n) => json::number_string(*n),
                        };
                        if !key_issues.iter().any(|(x, _)| *x == k) {
                            key_issues.push((k.clone(), issue.clone()));
                        }
                        vec![k]
                    }
                    _ => {
                        issues.push(issue);
                        continue;
                    }
                };
                for k in keys {
                    match rejected.iter_mut().find(|(x, _)| *x == k) {
                        Some((_, sides)) => sides[side] = true,
                        None => {
                            let mut sides = vec![false; members.len()];
                            sides[side] = true;
                            rejected.push((k, sides));
                        }
                    }
                }
            }
        }
        let by_all: Vec<String> = rejected.into_iter().filter(|(_, s)| s.iter().all(|x| *x)).map(|(k, _)| k).collect();
        if !by_all.is_empty() {
            let mut aggregated = Vec::new();
            if let Some(first) = first_unknown {
                if let What::UnrecognizedKeys { keys } = &first.what {
                    aggregated = by_all.iter().filter(|k| keys.contains(k)).cloned().collect();
                }
                if !aggregated.is_empty() {
                    issues.push(Issue { what: What::UnrecognizedKeys { keys: aggregated.clone() }, ..first });
                }
            }
            for k in &by_all {
                if !aggregated.contains(k) {
                    if let Some((_, issue)) = key_issues.iter().find(|(x, _)| x == k) {
                        issues.push(issue.clone());
                    }
                }
            }
        }
        issues
    }

    // ---------- strictly ----------

    fn fields_of(&self, n: usize) -> Option<Vec<(String, usize)>> {
        match &self.nodes[n] {
            Kind::Ref(t) => self.fields_of(*t),
            Kind::Object { props, .. } => Some(props.iter().map(|p| (p.key.clone(), p.node)).collect()),
            Kind::All { members, .. } => {
                let mut out: Option<Vec<(String, usize)>> = None;
                for m in members {
                    if let Some(fields) = self.fields_of(*m) {
                        let all = out.get_or_insert_with(Vec::new);
                        for (k, f) in fields {
                            // The later member's field, as a Map made from both keeps it.
                            match all.iter_mut().find(|(x, _)| *x == k) {
                                Some(slot) => slot.1 = f,
                                None => all.push((k, f)),
                            }
                        }
                    }
                }
                out
            }
            _ => None,
        }
    }

    fn passes_through(&self, n: usize) -> bool {
        match &self.nodes[n] {
            Kind::Ref(t) => self.passes_through(*t),
            Kind::Object { passthrough, .. } => *passthrough,
            Kind::All { members, passthrough } => *passthrough || members.iter().any(|m| self.passes_through(*m)),
            _ => false,
        }
    }

    // The keys in `v` that the schema doesn't declare, as paths.
    fn undeclared(&self, n: usize, v: &Data, at: &mut Vec<PathKey>, out: &mut Vec<String>) {
        match &self.nodes[n] {
            Kind::Ref(t) => self.undeclared(*t, v, at, out),
            Kind::Nullable(inner) => {
                if !v.nullish() {
                    self.undeclared(*inner, v, at, out);
                }
            }
            Kind::Array { items, .. } => {
                if let Data::Arr(a) = v {
                    for (i, x) in a.iter().enumerate() {
                        at.push(PathKey::Num(i as f64));
                        self.undeclared(*items, x, at, out);
                        at.pop();
                    }
                }
            }
            Kind::Tuple { items, rest, .. } => {
                if let Data::Arr(a) = v {
                    for (i, x) in a.iter().enumerate() {
                        if let Some(node) = items.get(i).copied().or(*rest) {
                            at.push(PathKey::Num(i as f64));
                            self.undeclared(node, x, at, out);
                            at.pop();
                        }
                    }
                }
            }
            Kind::Record { value, .. } => {
                if let Data::Obj(o) = v {
                    for (k, x) in o.entries() {
                        at.push(PathKey::Key(k.clone()));
                        self.undeclared(*value, x, at, out);
                        at.pop();
                    }
                }
            }
            Kind::Union(options) | Kind::Xor(options) | Kind::Variants { options, .. } => {
                if let Some(o) = options.iter().find(|o| self.run(**o, v).is_empty()) {
                    self.undeclared(*o, v, at, out);
                }
            }
            Kind::Object { .. } | Kind::All { .. } => {
                if !v.is_object() {
                    return;
                }
                let Data::Obj(o) = v else { return };
                let fields = self.fields_of(n).unwrap_or_default();
                let open = self.passes_through(n);
                for (k, x) in o.entries() {
                    match fields.iter().find(|(f, _)| f == k) {
                        Some((_, f)) => {
                            at.push(PathKey::Key(k.clone()));
                            self.undeclared(*f, x, at, out);
                            at.pop();
                        }
                        None if !open && *x != Data::Undefined => {
                            at.push(PathKey::Key(k.clone()));
                            out.push(path_text(at));
                            at.pop();
                        }
                        None => {}
                    }
                }
            }
            _ => {}
        }
    }
}

fn fits_type(t: &str, v: &Data) -> bool {
    match (t, v) {
        ("string", Data::Str(_)) | ("boolean", Data::Bool(_)) | ("null", Data::Null) | ("array", Data::Arr(_)) => true,
        ("number", Data::Num(n)) => n.is_finite(),
        ("integer", Data::Num(n)) => n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE,
        ("object", d) => d.is_object(),
        _ => false,
    }
}

// A schema's checks, in order, as zod's runChecks runs them.
fn run_checks(checks: &[Check], v: &Data, issues: &mut Vec<Issue>) {
    let mut stopped = aborted(issues);
    for c in checks {
        if c.on_length() {
            if issues.iter().any(|i| i.cont == Cont::Stop) {
                continue;
            }
            if v.nullish() || length_of(v).is_none() {
                continue;
            }
        } else if stopped {
            continue;
        }
        let before = issues.len();
        run_check(c, v, issues);
        if !stopped {
            stopped = aborted(&issues[before..]);
        }
    }
}

fn run_check(c: &Check, v: &Data, issues: &mut Vec<Issue>) {
    let push = |issues: &mut Vec<Issue>, what: What| issues.push(Issue::new(what, Cont::Go));
    match c {
        Check::Format { format, regex, pattern, text } => {
            let Data::Str(s) = v else { return };
            if regex.as_ref().is_none_or(|r| r.test(s)) {
                return;
            }
            push(issues, What::InvalidFormat { format: format.clone(), pattern: pattern.clone(), text: text.clone() });
        }
        Check::Pattern(regex, pattern) => {
            let Data::Str(s) = v else { return };
            if !regex.test(s) {
                push(issues, What::InvalidFormat { format: "regex".into(), pattern: Some(pattern.clone()), text: None });
            }
        }
        Check::MinLen(min) => {
            // A length that isn't a number (NaN) fails, as JS's >= says.
            let len = length_of(v).unwrap_or(f64::NAN);
            if len.partial_cmp(min).is_none_or(std::cmp::Ordering::is_lt) {
                push(issues, What::TooSmall { origin: length_origin(v), minimum: *min, inclusive: true, exact: false });
            }
        }
        Check::MaxLen(max) => {
            let len = length_of(v).unwrap_or(f64::NAN);
            if len.partial_cmp(max).is_none_or(std::cmp::Ordering::is_gt) {
                push(issues, What::TooBig { origin: length_origin(v), maximum: *max, inclusive: true, exact: false });
            }
        }
        Check::ExactLen(want) => {
            let len = length_of(v).unwrap_or(f64::NAN);
            if len == *want {
                return;
            }
            let origin = length_origin(v);
            push(
                issues,
                if len > *want {
                    What::TooBig { origin, maximum: *want, inclusive: true, exact: true }
                } else {
                    What::TooSmall { origin, minimum: *want, inclusive: true, exact: true }
                },
            );
        }
        Check::Int => {
            let Data::Num(n) = v else { return };
            if n.fract() != 0.0 || !n.is_finite() {
                issues.push(Issue::new(What::InvalidType { expected: "int", received: received(v) }, Cont::Stop));
            } else if n.abs() > MAX_SAFE {
                if *n > 0.0 {
                    push(issues, What::TooBig { origin: "int", maximum: MAX_SAFE, inclusive: true, exact: false });
                } else {
                    push(issues, What::TooSmall { origin: "int", minimum: -MAX_SAFE, inclusive: true, exact: false });
                }
            }
        }
        Check::Min(min, inclusive) => {
            let Data::Num(n) = v else { return };
            if !(if *inclusive { *n >= *min } else { *n > *min }) {
                push(issues, What::TooSmall { origin: "number", minimum: *min, inclusive: *inclusive, exact: false });
            }
        }
        Check::Max(max, inclusive) => {
            let Data::Num(n) = v else { return };
            if !(if *inclusive { *n <= *max } else { *n < *max }) {
                push(issues, What::TooBig { origin: "number", maximum: *max, inclusive: *inclusive, exact: false });
            }
        }
        Check::MultipleOf(d) => {
            let Data::Num(n) = v else { return };
            if *d == 0.0 || (n / d).fract() != 0.0 {
                push(issues, What::NotMultipleOf { divisor: *d });
            }
        }
    }
}

/// What check() found, as lines for an error message.
pub fn explain(issues: &[(String, String)], undeclared: &[String]) -> String {
    let mut lines: Vec<String> = issues.iter().map(|(path, message)| format!("{}: {message}", if path.is_empty() { "(the value)" } else { path })).collect();
    lines.extend(undeclared.iter().map(|p| format!("{p}: not in the schema")));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn checker(doc: Value) -> Checker {
        compile(&[doc]).unwrap()
    }

    fn found(c: &Checker, v: Value, strict: bool) -> Vec<(String, String)> {
        let r = c.check(c.root(0), &Data::from(&v), strict);
        r.issues.into_iter().map(|f| (f.path, f.message)).chain(r.undeclared.into_iter().map(|p| (p, "undeclared".into()))).collect()
    }

    fn leaf() -> Value {
        json!({
            "type": "object",
            "properties": {
                "at": { "type": "number" },
                "tag": { "anyOf": [{ "type": "string", "enum": ["a", "b"] }, { "type": "string" }] },
                "n": { "type": "integer", "minimum": 0, "maximum": 9007199254740991_i64 },
                "name": { "type": "string", "minLength": 1, "maxLength": 3 },
                "kind": { "oneOf": [
                    { "type": "object", "properties": { "kind": { "type": "string", "const": "x" }, "x": { "type": "number" } }, "required": ["kind", "x"], "additionalProperties": {} },
                    { "type": "object", "properties": { "kind": { "type": "string", "const": "y" } }, "required": ["kind"], "additionalProperties": {} }
                ] },
                "maybe": { "anyOf": [{ "type": "string", "pattern": "^#[0-9a-f]{6}$" }, { "type": "null" }] },
                "strict": { "type": "object", "properties": { "a": { "type": "string" } }, "additionalProperties": false }
            },
            "required": ["at", "tag"],
            "additionalProperties": {}
        })
    }

    #[test]
    fn says_what_zod_says() {
        let c = checker(leaf());
        assert_eq!(found(&c, json!({ "at": 1, "tag": "z" }), true), []);
        assert_eq!(found(&c, json!(null), false), [(String::new(), "Invalid input: expected object, received null".into())]);
        assert_eq!(
            found(&c, json!({ "at": "soon" }), false),
            [("at".into(), "Invalid input: expected number, received string".into()), ("tag".into(), "Invalid option: expected one of \"a\"|\"b\"".into())]
        );
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "n": 1.5 }), false), [("n".into(), "Invalid input: expected int, received number".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "n": -1 }), false), [("n".into(), "Too small: expected number to be >=0".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "name": "" }), false), [("name".into(), "Too small: expected string to have >=1 characters".into())]);
        assert_eq!(
            found(&c, json!({ "at": 1, "tag": "a", "name": [1, 2, 3, 4] }), false),
            [("name".into(), "Invalid input: expected string, received array".into()), ("name".into(), "Too big: expected array to have <=3 items".into())],
            "a length check runs on anything with a length"
        );
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "kind": { "kind": "z" } }), false), [("kind.kind".into(), "Invalid discriminator value. Expected 'x' | 'y'".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "kind": { "kind": "x" } }), false), [("kind.x".into(), "Invalid input: expected number, received undefined".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "maybe": "red" }), false), [("maybe".into(), "Invalid string: must match pattern /^#[0-9a-f]{6}$/".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": "a", "strict": { "a": "x", "b": 1, "c": 2 } }), false), [("strict".into(), "Unrecognized keys: \"b\", \"c\"".into())]);
        assert_eq!(found(&c, json!({ "at": 1, "tag": 5 }), false), [("tag".into(), "Invalid option: expected one of \"a\"|\"b\"".into())], "the closest option");
    }

    #[test]
    fn strictly_names_undeclared_keys() {
        let c = checker(leaf());
        assert_eq!(
            found(&c, json!({ "at": 1, "tag": "a", "kind": { "kind": "x", "x": 1, "why": 1 }, "top": 1 }), true),
            [("kind.why".to_string(), "undeclared".to_string()), ("top".into(), "undeclared".into())]
        );
        let mut o = Object::new();
        o.insert("at".into(), Data::Num(1.0));
        o.insert("tag".into(), Data::Str("a".into()));
        o.insert("gone".into(), Data::Undefined);
        assert!(c.check(c.root(0), &Data::Obj(o), true).ok(), "an undefined key isn't one");
    }

    #[test]
    fn unions_report_the_option_that_got_furthest() {
        let c = checker(json!({ "anyOf": [
            { "type": "object", "properties": { "a": { "type": "string" }, "n": { "type": "number" } }, "required": ["a", "n"], "additionalProperties": {} },
            { "type": "object", "properties": { "b": { "type": "string" } }, "required": ["b"], "additionalProperties": {} }
        ] }));
        assert_eq!(found(&c, json!({ "a": "x", "n": "1" }), false), [("n".into(), "Invalid input: expected number, received string".into())]);
        let short = checker(json!({ "anyOf": [{ "type": "string", "maxLength": 2 }, { "type": "number" }] }));
        assert_eq!(found(&short, json!("abc"), false), [(String::new(), "Too big: expected string to have <=2 characters".into())], "the one option that got to its checks");
        let types = checker(json!({ "type": ["string", "null"] }));
        assert_eq!(found(&types, json!(1), false), [(String::new(), "Invalid input: expected string, received number".into())]);
    }

    #[test]
    fn finds_what_the_document_no_longer_names() {
        // Both keys have a value in every option, but only kind's tell them apart.
        let du = checker(json!({ "oneOf": [
            { "type": "object", "properties": { "app": { "type": "string", "enum": ["a", "b"] }, "kind": { "type": "string", "const": "x" } }, "required": ["app", "kind"], "additionalProperties": {} },
            { "type": "object", "properties": { "app": { "type": "string", "enum": ["a"] }, "kind": { "type": "string", "const": "y" } }, "required": ["app", "kind"], "additionalProperties": {} }
        ] }));
        assert_eq!(found(&du, json!({ "app": "a", "kind": "z" }), false), [("kind".into(), "Invalid discriminator value. Expected 'x' | 'y'".into())]);
        // An int that isn't one stops even the length check of the array it's in.
        let ints = checker(json!({ "type": "array", "items": { "type": "integer", "minimum": -9007199254740991_i64, "maximum": 9007199254740991_i64 }, "maxItems": 1 }));
        assert_eq!(found(&ints, json!([1.5, 2]), false), [("[0]".into(), "Invalid input: expected int, received number".into())]);
        assert_eq!(found(&ints, json!([1, 2]), false), [(String::new(), "Too big: expected array to have <=1 items".into())]);
        // A tuple's items past its least may be left out.
        let pair = checker(json!({ "type": "array", "prefixItems": [{ "type": "string" }, { "type": "number" }], "items": false, "minItems": 1, "maxItems": 2 }));
        assert_eq!(found(&pair, json!(["a"]), false), []);
        assert_eq!(found(&pair, json!([]), false), [(String::new(), "Too small: expected array to have >=1 items".into())]);
        assert_eq!(found(&pair, json!(["a", 1, 2]), false), [(String::new(), "Too big: expected array to have <=2 items".into())], "and none past its most");
    }

    #[test]
    fn paths_read_as_code() {
        let p = |k: &[PathKey]| path_text(k);
        assert_eq!(p(&[PathKey::Key("nodes".into()), PathKey::Num(3.0), PathKey::Key("ranAs".into())]), "nodes[3].ranAs");
        assert_eq!(p(&[PathKey::Key("layout".into()), PathKey::Key("project:/x".into())]), "layout[\"project:/x\"]");
        assert_eq!(p(&[PathKey::Num(0.0), PathKey::Key("a".into())]), "[0].a");
        assert_eq!(explain(&[(String::new(), "m".into())], &["a.b".into()]), "(the value): m\na.b: not in the schema");
    }
}
