//! The tools' argument and answer schemas, declared as the JS servers
//! declare them with zod (v4): one declaration gives both the JSON Schema
//! that tools/list shows, as zod's toJSONSchema writes it (the same keys in
//! the same order), and the check of a call's arguments, with zod's issues
//! in zod's words, which the MCP SDK puts in its "Input validation error".
//!
//! Only what the servers use: strings (a pattern, a length, an ISO time),
//! numbers and integers with bounds, booleans, enums, literals, arrays,
//! objects (loose or strict), optional, nullable and unions, each with a
//! description.

use regex::Regex;
use serde_json::{json, Map, Value};
use xenonet_base::json::number_string;

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;
const SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";
/// zod's z.iso.datetime(): UTC (Z) only, seconds required, any fraction.
pub const ISO_PATTERN: &str = r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z))$";

#[derive(Clone)]
enum Check {
    Pattern(String, Regex),
    Max(usize),
    Min(usize),
    Iso(Regex),
}

#[derive(Clone)]
enum Kind {
    Str(Vec<Check>),
    Num { int: bool, min: Option<f64>, max: Option<f64> },
    Bool,
    Enum(Vec<&'static str>),
    Lit(Value),
    Arr { item: Box<Z>, min: Option<usize>, max: Option<usize> },
    Obj { shape: Vec<(&'static str, Z)>, strict: bool },
    /// z.record(z.string(), z.unknown()): any object, read as it is.
    Record,
    Opt(Box<Z>),
    Null(Box<Z>),
    Union(Vec<Z>),
}

/// A schema, as a zod schema is one.
#[derive(Clone)]
pub struct Z {
    kind: Kind,
    description: Option<String>,
}

// A pattern as JS's RegExp reads it, through the engine's translation (\d,
// \s, \w, \b and . are JS's, not the regex crate's).
fn compile(pattern: &str) -> Regex {
    xenonet_engine::jsre::js(pattern, false)
}

// zod's last pass over what it wrote (compactTypeUnion): an anyOf whose
// every branch is a type and nothing else, as a nullable string's is,
// becomes one type list, after the keys already there.
fn compact(m: &mut Map<String, Value>) {
    let Some(Value::Array(options)) = m.get("anyOf") else { return };
    if options.is_empty() || m.contains_key("type") {
        return;
    }
    let mut types: Vec<Value> = vec![];
    for o in options {
        let Some(o) = o.as_object().filter(|o| o.len() == 1) else { return };
        let members = match o.get("type") {
            Some(Value::String(t)) => vec![Value::String(t.clone())],
            Some(Value::Array(ts)) if ts.iter().all(Value::is_string) => ts.clone(),
            _ => return,
        };
        for t in members {
            if !types.contains(&t) {
                types.push(t);
            }
        }
    }
    m.shift_remove("anyOf");
    let t = if types.len() == 1 { types.remove(0) } else { Value::Array(types) };
    m.insert("type".into(), t);
}

fn of(kind: Kind) -> Z {
    Z { kind, description: None }
}

pub fn string() -> Z {
    of(Kind::Str(vec![]))
}
/// z.iso.datetime().
pub fn iso() -> Z {
    of(Kind::Str(vec![Check::Iso(compile(ISO_PATTERN))]))
}
pub fn number() -> Z {
    of(Kind::Num { int: false, min: None, max: None })
}
/// z.int(), or z.number().int(): both safe integers.
pub fn int() -> Z {
    of(Kind::Num { int: true, min: None, max: None })
}
pub fn boolean() -> Z {
    of(Kind::Bool)
}
pub fn enumeration(values: &[&'static str]) -> Z {
    of(Kind::Enum(values.to_vec()))
}
pub fn literal(v: Value) -> Z {
    of(Kind::Lit(v))
}
pub fn array(item: Z) -> Z {
    of(Kind::Arr { item: Box::new(item), min: None, max: None })
}
/// z.object(): keys it doesn't name are left out of what it reads.
pub fn object(shape: Vec<(&'static str, Z)>) -> Z {
    of(Kind::Obj { shape, strict: false })
}
/// z.record(z.string(), z.unknown()): any object, passed on whole.
pub fn record() -> Z {
    of(Kind::Record)
}
/// z.strictObject(): keys it doesn't name are refused.
pub fn strict(shape: Vec<(&'static str, Z)>) -> Z {
    of(Kind::Obj { shape, strict: true })
}
pub fn union(options: Vec<Z>) -> Z {
    of(Kind::Union(options))
}

impl Z {
    pub fn describe(mut self, text: &str) -> Z {
        self.description = Some(text.to_string());
        self
    }
    pub fn optional(self) -> Z {
        of(Kind::Opt(Box::new(self)))
    }
    pub fn nullable(self) -> Z {
        of(Kind::Null(Box::new(self)))
    }
    pub fn regex(mut self, pattern: &str) -> Z {
        if let Kind::Str(checks) = &mut self.kind {
            checks.push(Check::Pattern(pattern.to_string(), compile(pattern)));
        }
        self
    }
    /// .min(n) of a string, a number or an array.
    pub fn min(mut self, n: f64) -> Z {
        match &mut self.kind {
            Kind::Str(checks) => checks.push(Check::Min(n as usize)),
            Kind::Num { min, .. } => *min = Some(n),
            Kind::Arr { min, .. } => *min = Some(n as usize),
            _ => {}
        }
        self
    }
    /// .max(n) of a string, a number or an array.
    pub fn max(mut self, n: f64) -> Z {
        match &mut self.kind {
            Kind::Str(checks) => checks.push(Check::Max(n as usize)),
            Kind::Num { max, .. } => *max = Some(n),
            Kind::Arr { max, .. } => *max = Some(n as usize),
            _ => {}
        }
        self
    }

    fn is_optional(&self) -> bool {
        matches!(self.kind, Kind::Opt(_))
    }

    /// The JSON Schema zod writes for this, without the root's $schema.
    pub fn json(&self) -> Map<String, Value> {
        let mut m = Map::new();
        match &self.kind {
            Kind::Str(checks) => {
                m.insert("type".into(), json!("string"));
                // zod writes the lengths first, then the format and pattern,
                // whatever order they were declared in.
                for c in checks {
                    if let Check::Min(n) = c {
                        m.insert("minLength".into(), json!(n));
                    }
                }
                for c in checks {
                    if let Check::Max(n) = c {
                        m.insert("maxLength".into(), json!(n));
                    }
                }
                for c in checks {
                    match c {
                        Check::Iso(_) => {
                            m.insert("format".into(), json!("date-time"));
                            m.insert("pattern".into(), json!(ISO_PATTERN));
                        }
                        Check::Pattern(p, _) => {
                            m.insert("pattern".into(), json!(p));
                        }
                        _ => {}
                    }
                }
            }
            Kind::Num { int, min, max } => {
                m.insert("type".into(), json!(if *int { "integer" } else { "number" }));
                let (min, max) = if *int { (Some(min.unwrap_or(-MAX_SAFE)), Some(max.unwrap_or(MAX_SAFE))) } else { (*min, *max) };
                if let Some(n) = min {
                    m.insert("minimum".into(), xenonet_base::json::number(n));
                }
                if let Some(n) = max {
                    m.insert("maximum".into(), xenonet_base::json::number(n));
                }
            }
            Kind::Bool => {
                m.insert("type".into(), json!("boolean"));
            }
            Kind::Enum(values) => {
                m.insert("type".into(), json!("string"));
                m.insert("enum".into(), json!(values));
            }
            Kind::Lit(v) => {
                let t = match v {
                    Value::String(_) => "string",
                    Value::Bool(_) => "boolean",
                    Value::Null => "null",
                    _ => "number",
                };
                m.insert("type".into(), json!(t));
                m.insert("const".into(), v.clone());
            }
            Kind::Arr { item, min, max } => {
                if let Some(n) = min {
                    m.insert("minItems".into(), json!(n));
                }
                if let Some(n) = max {
                    m.insert("maxItems".into(), json!(n));
                }
                m.insert("type".into(), json!("array"));
                m.insert("items".into(), Value::Object(item.json()));
            }
            Kind::Record => {
                m.insert("type".into(), json!("object"));
                m.insert("propertyNames".into(), json!({ "type": "string" }));
                m.insert("additionalProperties".into(), json!({}));
            }
            Kind::Obj { shape, strict } => {
                m.insert("type".into(), json!("object"));
                m.insert("properties".into(), Value::Object(shape.iter().map(|(k, z)| (k.to_string(), Value::Object(z.json()))).collect()));
                let required: Vec<&str> = shape.iter().filter(|(_, z)| !z.is_optional()).map(|(k, _)| *k).collect();
                if !required.is_empty() {
                    m.insert("required".into(), json!(required));
                }
                if *strict {
                    m.insert("additionalProperties".into(), json!(false));
                }
            }
            Kind::Opt(inner) => {
                // An optional's description comes first: zod writes the
                // wrapper's own keys, then the schema it wraps.
                let mut wrapped = inner.json();
                if let Some(d) = &self.description {
                    wrapped.shift_remove("description");
                    m.insert("description".into(), json!(d));
                }
                m.extend(wrapped);
                return m;
            }
            Kind::Null(inner) => {
                m.insert("anyOf".into(), json!([Value::Object(inner.json()), { "type": "null" }]));
            }
            Kind::Union(options) => {
                m.insert("anyOf".into(), Value::Array(options.iter().map(|z| Value::Object(z.json())).collect()));
            }
        }
        if let Some(d) = &self.description {
            m.insert("description".into(), json!(d));
        }
        compact(&mut m);
        m
    }

    /// A tool's inputSchema, as the MCP SDK lists it: an object first.
    pub fn input_schema(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("type".into(), json!("object"));
        m.insert("$schema".into(), json!(SCHEMA));
        m.extend(self.json());
        m
    }

    /// A tool's outputSchema, as zod writes it.
    pub fn output_schema(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("$schema".into(), json!(SCHEMA));
        m.extend(self.json());
        m
    }

    /// Reads `v` as zod's safeParse does: what it reads (the keys a loose
    /// object doesn't name left out, the others in the schema's order), or
    /// the issues, each as the SDK writes it ("path: message"), joined.
    pub fn parse(&self, v: &Value) -> Result<Value, String> {
        let mut issues = vec![];
        let out = self.read(Some(v), &mut vec![], &mut issues);
        if issues.is_empty() {
            Ok(out.unwrap_or(Value::Null))
        } else {
            Err(issues.join(", "))
        }
    }

    fn read(&self, v: Option<&Value>, path: &mut Vec<String>, issues: &mut Vec<String>) -> Option<Value> {
        let wrong = |expected: &str| format!("Invalid input: expected {expected}, received {}", received(v));
        match &self.kind {
            Kind::Opt(inner) => match v {
                None => None,
                Some(_) => inner.read(v, path, issues),
            },
            Kind::Null(inner) => match v {
                Some(Value::Null) => Some(Value::Null),
                _ => inner.read(v, path, issues),
            },
            Kind::Str(checks) => {
                let Some(Value::String(s)) = v else {
                    say(issues, path, wrong("string"));
                    return None;
                };
                for c in checks {
                    match c {
                        Check::Pattern(p, re) if !re.is_match(s) => say(issues, path, format!("Invalid string: must match pattern /{p}/")),
                        Check::Max(n) if s.chars().count() > *n => say(issues, path, format!("Too big: expected string to have <={n} characters")),
                        Check::Min(n) if s.chars().count() < *n => say(issues, path, format!("Too small: expected string to have >={n} characters")),
                        Check::Iso(re) if !re.is_match(s) => say(issues, path, "Invalid ISO datetime".into()),
                        _ => {}
                    }
                }
                v.cloned()
            }
            Kind::Num { int, min, max } => {
                let Some(n) = v.and_then(Value::as_f64) else {
                    say(issues, path, wrong("number"));
                    return None;
                };
                if *int {
                    if n.fract() != 0.0 {
                        say(issues, path, "Invalid input: expected int, received number".into());
                        return None;
                    }
                    if n > MAX_SAFE {
                        say(issues, path, format!("Too big: expected int to be <={}", number_string(MAX_SAFE)));
                    }
                    if n < -MAX_SAFE {
                        say(issues, path, format!("Too small: expected int to be >={}", number_string(-MAX_SAFE)));
                    }
                }
                if let Some(m) = min.filter(|m| n < *m) {
                    say(issues, path, format!("Too small: expected number to be >={}", number_string(m)));
                }
                if let Some(m) = max.filter(|m| n > *m) {
                    say(issues, path, format!("Too big: expected number to be <={}", number_string(m)));
                }
                v.cloned()
            }
            Kind::Bool => {
                if !matches!(v, Some(Value::Bool(_))) {
                    say(issues, path, wrong("boolean"));
                }
                v.cloned()
            }
            Kind::Enum(values) => {
                if !matches!(v, Some(Value::String(s)) if values.contains(&s.as_str())) {
                    let quoted: Vec<String> = values.iter().map(|s| json!(s).to_string()).collect();
                    // One value reads as a literal does.
                    say(
                        issues,
                        path,
                        if quoted.len() == 1 { format!("Invalid input: expected {}", quoted[0]) } else { format!("Invalid option: expected one of {}", quoted.join("|")) },
                    );
                }
                v.cloned()
            }
            Kind::Lit(lit) => {
                if !v.is_some_and(|v| same(v, lit)) {
                    say(issues, path, format!("Invalid input: expected {}", xenonet_base::json::stringify(lit, 0)));
                }
                v.cloned()
            }
            Kind::Arr { item, min, max } => {
                let Some(Value::Array(items)) = v else {
                    say(issues, path, wrong("array"));
                    return None;
                };
                let mut out = vec![];
                for (i, x) in items.iter().enumerate() {
                    path.push(i.to_string());
                    out.push(item.read(Some(x), path, issues).unwrap_or(Value::Null));
                    path.pop();
                }
                if let Some(n) = min.filter(|n| items.len() < *n) {
                    say(issues, path, format!("Too small: expected array to have >={n} items"));
                }
                if let Some(n) = max.filter(|n| items.len() > *n) {
                    say(issues, path, format!("Too big: expected array to have <={n} items"));
                }
                Some(Value::Array(out))
            }
            Kind::Record => {
                if !matches!(v, Some(Value::Object(_))) {
                    say(issues, path, wrong("record"));
                }
                v.cloned()
            }
            Kind::Obj { shape, strict } => {
                let Some(Value::Object(given)) = v else {
                    say(issues, path, wrong("object"));
                    return None;
                };
                let mut out = Map::new();
                for (k, z) in shape {
                    path.push(k.to_string());
                    if let Some(x) = z.read(given.get(*k), path, issues) {
                        out.insert(k.to_string(), x);
                    }
                    path.pop();
                }
                if *strict {
                    let extra: Vec<String> = given.keys().filter(|k| !shape.iter().any(|(s, _)| s == k)).map(|k| json!(k).to_string()).collect();
                    let message = match extra.len() {
                        0 => None,
                        1 => Some(format!("Unrecognized key: {}", extra[0])),
                        _ => Some(format!("Unrecognized keys: {}", extra.join(", "))),
                    };
                    if let Some(message) = message {
                        say(issues, path, message);
                    }
                }
                Some(Value::Object(out))
            }
            Kind::Union(options) => {
                for z in options {
                    let mut own = vec![];
                    let out = z.read(v, &mut path.clone(), &mut own);
                    if own.is_empty() {
                        return out;
                    }
                }
                say(issues, path, "Invalid input".into());
                None
            }
        }
    }
}

// An issue, at its path.
fn say(issues: &mut Vec<String>, path: &[String], message: String) {
    issues.push(if path.is_empty() { message } else { format!("{}: {message}", path.join(".")) });
}

// === on two JSON values that aren't objects or arrays: 1 and 1.0 are
// the same number.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(_) | Value::Object(_), _) => false,
        _ => a == b,
    }
}

// What zod says it got instead.
fn received(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issues(z: &Z, v: Value) -> String {
        z.parse(&v).unwrap_err()
    }

    // The issues zod 4.6 gives for these, as the MCP SDK joins them.
    #[test]
    fn reads_as_zod_does() {
        let minutes = object(vec![("minutes", int().min(1.0).max(480.0).optional())]);
        assert_eq!(issues(&minutes, json!({ "minutes": 0.5 })), "minutes: Invalid input: expected int, received number");
        assert_eq!(issues(&minutes, json!({ "minutes": 0 })), "minutes: Too small: expected number to be >=1");
        assert_eq!(issues(&minutes, json!({ "minutes": "5" })), "minutes: Invalid input: expected number, received string");
        assert_eq!(issues(&minutes, json!({ "minutes": null })), "minutes: Invalid input: expected number, received null");
        assert_eq!(
            issues(&minutes, json!({ "minutes": 1152921504606846976.0 })),
            "minutes: Too big: expected int to be <=9007199254740991, minutes: Too big: expected number to be <=480"
        );
        assert_eq!(minutes.parse(&json!({ "minutes": 5.0 })).unwrap(), json!({ "minutes": 5.0 }));
        let s = object(vec![("s", string())]);
        assert_eq!(issues(&s, json!({})), "s: Invalid input: expected string, received undefined");
        assert_eq!(issues(&s, json!({ "s": [] })), "s: Invalid input: expected string, received array");
        let e = object(vec![("s", enumeration(&["a", "b"]))]);
        assert_eq!(issues(&e, json!({ "s": 5 })), r#"s: Invalid option: expected one of "a"|"b""#);
        let p = object(vec![("p", array(string()).max(2.0))]);
        assert_eq!(issues(&p, json!({ "p": ["a", "b", 3] })), "p.2: Invalid input: expected string, received number, p: Too big: expected array to have <=2 items");
        let st = strict(vec![("a", string().optional())]);
        assert_eq!(issues(&st, json!({ "b": 1, "c": 2, "a": 3 })), r#"a: Invalid input: expected string, received number, Unrecognized keys: "b", "c""#);
        assert_eq!(issues(&st, json!({ "b": 1 })), r#"Unrecognized key: "b""#);
        assert_eq!(issues(&st, json!([])), "Invalid input: expected object, received array");
        let loose = object(vec![("kind", enumeration(&["a"])), ("x", string().optional())]);
        assert_eq!(loose.parse(&json!({ "x": "q", "kind": "a", "y": 1 })).unwrap().to_string(), r#"{"kind":"a","x":"q"}"#, "the schema's keys, in its order");
        let r = object(vec![("a", string().regex("^prj_[0-9]{2}$").max(2.0))]);
        assert_eq!(issues(&r, json!({ "a": "abc" })), "a: Invalid string: must match pattern /^prj_[0-9]{2}$/, a: Too big: expected string to have <=2 characters");
        let t = object(vec![("a", iso())]);
        assert!(t.parse(&json!({ "a": "2026-10-07T00:00:00.123456Z" })).is_ok());
        assert_eq!(issues(&t, json!({ "a": "2026-10-07T00:00:00+01:00" })), "a: Invalid ISO datetime");
        assert_eq!(issues(&t, json!({ "a": "٢٠٢٦-10-07T00:00:00Z" })), "a: Invalid ISO datetime", "an ASCII digit, as in JS");
        let l = object(vec![("a", literal(json!("ro"))), ("b", union(vec![string().regex("^a$"), string().regex("^b$")]))]);
        assert_eq!(issues(&l, json!({ "a": "x", "b": "c" })), r#"a: Invalid input: expected "ro", b: Invalid input"#);
        let n = object(vec![("events", array(object(vec![("kind", enumeration(&["x", "y"])), ("count", int().min(1.0).optional())])).min(1.0))]);
        assert_eq!(
            issues(&n, json!({ "events": [{ "kind": "z", "count": 0 }, {}] })),
            r#"events.0.kind: Invalid option: expected one of "x"|"y", events.0.count: Too small: expected number to be >=1, events.1.kind: Invalid option: expected one of "x"|"y""#
        );
        assert_eq!(issues(&object(vec![("s", string().max(1.0))]), json!({ "s": "😀😀" })), "s: Too big: expected string to have <=1 characters");
        assert!(object(vec![("s", string().max(1.0))]).parse(&json!({ "s": "😀" })).is_ok(), "code points, as zod counts them");
    }

    // The JSON Schema zod writes for these.
    #[test]
    fn writes_json_schema_as_zod_does() {
        let z = strict(vec![
            ("a", string().max(240.0).nullable().describe("L")),
            ("b", int().min(0.0).nullable().describe("N").describe("M")),
            ("c", literal(json!(false))),
            ("d", union(vec![string().regex("^a$"), string()]).describe("U")),
            ("e", array(string()).max(3.0).min(1.0)),
            ("f", iso().optional().describe("D")),
        ]);
        assert_eq!(
            Value::Object(z.output_schema()).to_string(),
            r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object","properties":{"a":{"anyOf":[{"type":"string","maxLength":240},{"type":"null"}],"description":"L"},"b":{"anyOf":[{"type":"integer","minimum":0,"maximum":9007199254740991},{"type":"null"}],"description":"M"},"c":{"type":"boolean","const":false},"d":{"anyOf":[{"type":"string","pattern":"^a$"},{"type":"string"}],"description":"U"},"e":{"minItems":1,"maxItems":3,"type":"array","items":{"type":"string"}},"f":{"description":"D","type":"string","format":"date-time","pattern":""#
                .to_string()
                + &ISO_PATTERN.replace('\\', "\\\\")
                + r#""}},"required":["a","b","c","d","e"],"additionalProperties":false}"#
        );
        assert_eq!(
            Value::Object(object(vec![("p", array(string()).min(1.0)), ("r", int().optional())]).input_schema()).to_string(),
            r#"{"type":"object","$schema":"https://json-schema.org/draft/2020-12/schema","properties":{"p":{"minItems":1,"type":"array","items":{"type":"string"}},"r":{"type":"integer","minimum":-9007199254740991,"maximum":9007199254740991}},"required":["p"]}"#
        );
        // A nullable or a union whose branches are bare types is one type
        // list, after the keys before it, as mcp/console.js's nullable ids
        // are listed; "a" and "d" above, with constraints, stay an anyOf.
        assert_eq!(
            Value::Object(object(vec![("s", string().nullable().optional().describe("S")), ("t", union(vec![string(), boolean(), string()]))]).json()).to_string(),
            r#"{"type":"object","properties":{"s":{"description":"S","type":["string","null"]},"t":{"type":["string","boolean"]}},"required":["t"]}"#
        );
    }
}
