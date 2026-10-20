//! Vendor data checked leniently against XenoNet's vendor schemas
//! (core/schema/lenient.js; contracts, section 1), as npm run schemas
//! writes them in schemas/vendor/: JSON Schema.
//!
//! `compile` turns a schema file into a spec, and `inspect` walks a record
//! against it, the way the record would be read: the fields the schema
//! declares, each on its own. It finds
//!
//! - a required field that isn't there (missing-field),
//! - a field of another type (type-change), which `strip` takes out of the
//!   record, so a reader sees it as missing and falls back the way it does
//!   for a missing field,
//! - a discriminator's value the schema doesn't list (new-variant),
//! - a key a closed object doesn't list (new-field),
//!
//! and a record that isn't an object at all, which nothing can be read
//! from (drop). Other keys pass: vendor objects are open unless the schema
//! closes them. Arrays and maps are checked a sample of elements at a
//! time, since a record can hold hundreds.
//!
//! A compiled schema is an arena of nodes, since a schema can refer to
//! itself; a node is named by the schema and its place in it.

use crate::fields::is_field;
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
use xenonet_base::json;

/// `typeof`, as JSON sees a value: null, array, object, boolean, number,
/// string.
pub fn json_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Any,
    Types(BTreeSet<String>),
    Object { props: Vec<(String, usize, bool)>, closed: bool, known: BTreeSet<String> },
    Record { value: usize },
    Array { items: usize },
    Variants { key: String, by_value: Vec<(Value, usize)>, value_types: BTreeSet<String> },
    Union { options: Vec<usize> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Schema {
    pub nodes: Vec<Node>,
    pub root: usize,
}

/// A place in a compiled schema: one of its nodes, or the types a
/// discriminator takes (what a type change of one is checked against).
#[derive(Clone, Debug, PartialEq)]
pub enum SpecRef {
    Node(usize),
    Types(BTreeSet<String>),
}

const ANY: usize = 0;

fn is_empty(node: &Value) -> bool {
    match node {
        Value::Object(m) => m.keys().all(|k| k == "description" || k == "title" || k == "$comment"),
        // Anything else holds no constraint XenoNet reads.
        _ => true,
    }
}

/// A schema file, or a part of one, as the spec `inspect` walks.
pub fn compile(root: &Value) -> Schema {
    let mut c = Compiler { root, nodes: vec![Node::Any], seen: HashMap::new() };
    let r = c.go(root);
    Schema { nodes: c.nodes, root: r }
}

struct Compiler<'a> {
    root: &'a Value,
    nodes: Vec<Node>,
    seen: HashMap<*const Value, usize>,
}

impl<'a> Compiler<'a> {
    fn resolve(&self, r: &str) -> Option<&'a Value> {
        let name = r.strip_prefix("#/$defs/")?.replace("~1", "/").replace("~0", "~");
        self.root.get("$defs")?.get(&name)
    }

    fn go(&mut self, node: &'a Value) -> usize {
        if is_empty(node) {
            return ANY;
        }
        if let Some(&i) = self.seen.get(&std::ptr::from_ref(node)) {
            return i;
        }
        if let Some(r) = node.get("$ref") {
            return match r.as_str().and_then(|r| self.resolve(r)) {
                Some(target) => self.go(target),
                None => ANY,
            };
        }
        // A place for it first, so a schema that refers to itself finds it.
        let at = self.nodes.len();
        self.nodes.push(Node::Any);
        self.seen.insert(std::ptr::from_ref(node), at);
        let compiled = self.node(node);
        self.nodes[at] = compiled;
        at
    }

    fn node(&mut self, node: &'a Value) -> Node {
        let empty = Map::new();
        let xs = node.get("x-xenonet").and_then(Value::as_object).unwrap_or(&empty);
        let one_of = node.get("oneOf").and_then(Value::as_array);
        if let Some(options) = one_of.or_else(|| node.get("anyOf").and_then(Value::as_array)) {
            let compiled: Vec<usize> = options.iter().map(|o| self.go(o)).collect();
            let key = xs.get("discriminator").and_then(Value::as_str).map(str::to_string).or_else(|| one_of.and_then(|o| discriminator_of(o)));
            if let Some(key) = key {
                let mut by_value: Vec<(Value, usize)> = Vec::new();
                for (o, i) in options.iter().zip(&compiled) {
                    if let Some(v) = o.get("properties").and_then(|p| p.get(&key)).and_then(|p| p.get("const")) {
                        // As a Map's set: a value given twice keeps its first
                        // place and its last option.
                        match by_value.iter_mut().find(|(x, _)| same_value_zero(x, v)) {
                            Some(slot) => slot.1 = *i,
                            None => by_value.push((v.clone(), *i)),
                        }
                    }
                }
                let value_types = by_value.iter().map(|(v, _)| json_type(v).to_string()).collect();
                return Node::Variants { key, by_value, value_types };
            }
            return Node::Union { options: compiled };
        }
        let mut types: Option<Vec<String>> = match node.get("type") {
            Some(Value::String(t)) => Some(vec![t.clone()]),
            Some(Value::Array(a)) => Some(a.iter().map(|t| t.as_str().map_or_else(|| xenonet_base::text::js_string(t), str::to_string)).collect()),
            Some(Value::Null) | None => None,
            Some(other) => Some(vec![xenonet_base::text::js_string(other)]),
        };
        if types.is_none() {
            if let Some(c) = node.get("const") {
                types = Some(vec![json_type(c).to_string()]);
            }
        }
        if types.is_none() {
            if let Some(Value::Array(e)) = node.get("enum") {
                let mut seen: Vec<String> = Vec::new();
                for v in e {
                    let t = json_type(v).to_string();
                    if !seen.contains(&t) {
                        seen.push(t);
                    }
                }
                types = Some(seen);
            }
        }
        if types.is_none() && node.get("properties").is_some_and(|p| !p.is_null()) {
            types = Some(vec!["object".into()]);
        }
        let Some(types) = types else { return Node::Any };
        if types.len() == 1 && types[0] == "object" {
            if let Some(props) = node.get("properties").filter(|p| !p.is_null()) {
                let required: BTreeSet<String> =
                    node.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let props: Vec<(String, usize, bool)> =
                    props.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), self.go(v), required.contains(k))).collect()).unwrap_or_default();
                let known = xs.get("known").and_then(Value::as_array).map(|k| k.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
                let closed = xenonet_base::text::truthy(xs.get("closed"));
                return Node::Object { props, closed, known };
            }
            // A record (z.record): keyed by data, whatever its values are,
            // so its keys are never fields, nor any key inside it.
            let additional = node.get("additionalProperties");
            if node.get("propertyNames").is_some_and(|p| !p.is_null()) || additional.is_some_and(|a| !is_empty(a)) {
                let value = additional.map_or(ANY, |a| self.go(a));
                return Node::Record { value };
            }
            return Node::Types(types.into_iter().collect());
        }
        if types.len() == 1 && types[0] == "array" {
            let items = node.get("items").map_or(ANY, |i| self.go(i));
            return Node::Array { items };
        }
        Node::Types(types.into_iter().collect())
    }
}

// The key every option of a oneOf gives a const, as zod writes a
// discriminated union.
fn discriminator_of(options: &[Value]) -> Option<String> {
    let first = options.first()?.get("properties")?.as_object()?;
    first.keys().find(|k| options.iter().all(|o| o.get("properties").and_then(|p| p.get(k.as_str())).and_then(|p| p.get("const")).is_some())).cloned()
}

// SameValueZero, as a Map compares keys: equal primitives; never two
// objects or arrays (which a Map tells apart by identity).
fn same_value_zero(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(_) | Value::Object(_), _) | (_, Value::Array(_) | Value::Object(_)) => false,
        _ => a == b,
    }
}

/// A major of a vendor source's schema.
#[derive(Clone, Debug)]
pub struct Major {
    pub id: String,
    pub major: u32,
    pub range: Value,
    pub schema: Arc<Schema>,
}

/// The majors of a vendor source's schema in `dir`, newest first; none
/// when the files aren't there.
pub fn load_vendor(source: &str, dir: &Path) -> Vec<Major> {
    let folder = source.split('/').fold(dir.join("vendor"), |p, s| p.join(s));
    let Ok(names) = std::fs::read_dir(&folder) else { return Vec::new() };
    let mut out: Vec<Major> = names
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix('v')?.strip_suffix(".json")?;
            if n.is_empty() || n.starts_with('0') || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let text = std::fs::read_to_string(folder.join(&name)).ok()?;
            let file = json::parse(&text).ok()?;
            let range = file.get("x-xenonet").and_then(|x| x.get("vendorVersions")).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Value::String("*".into()));
            Some(Major { id: format!("vendor/{source}/v{n}"), major: n.parse().ok()?, range, schema: Arc::new(compile(&file)) })
        })
        .collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.major));
    out
}

/// Every vendor source with a schema file in `dir`, as
/// `<vendor>/<product>/<source>`, sorted.
pub fn vendor_sources(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, rel: &mut Vec<String>, out: &mut BTreeSet<String>) {
        let Ok(ents) = xenonet_base::paths::read_dir_sorted(&rel.iter().fold(base.to_path_buf(), |p, s| p.join(s))) else { return };
        for e in ents {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                rel.push(name);
                walk(base, rel, out);
                rel.pop();
            } else if name.strip_prefix('v').and_then(|n| n.strip_suffix(".json")).is_some_and(|n| !n.is_empty() && !n.starts_with('0') && n.bytes().all(|b| b.is_ascii_digit())) {
                out.insert(rel.join("/"));
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(&dir.join("vendor"), &mut Vec::new(), &mut out);
    out.into_iter().collect()
}

// ---------- checking ----------

/// A step to a place in a record: a key of an object, an element of an
/// array, an entry of a map.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Prop(String),
    Item(usize),
    Entry(String),
}

/// A key on the way to a place, as strip() takes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Name(String),
    Index(usize),
}

fn is_ident(k: &str) -> bool {
    // /^[A-Za-z_$][\w$-]*$/
    let b = k.as_bytes();
    !b.is_empty() && (b[0].is_ascii_alphabetic() || b[0] == b'_' || b[0] == b'$') && b[1..].iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c == b'-')
}

/// Where, as a drift record says it: `message.content[].citations`,
/// `changes.*.type`, `["https://api.openai.com/auth"].chatgpt_plan_type`.
/// None for the record itself.
pub fn path_of(steps: &[Step]) -> Option<String> {
    if steps.is_empty() {
        return None;
    }
    let mut s = String::new();
    for p in steps {
        match p {
            Step::Item(_) => s.push_str("[]"),
            Step::Entry(_) => s.push_str(if s.is_empty() { "*" } else { ".*" }),
            Step::Prop(k) if is_ident(k) => {
                if !s.is_empty() {
                    s.push('.');
                }
                s.push_str(k);
            }
            Step::Prop(k) => {
                s.push('[');
                s.push_str(&json::stringify(&Value::String(k.clone()), 0));
                s.push(']');
            }
        }
    }
    Some(s)
}

fn keys_of(steps: &[Step]) -> Vec<Key> {
    steps
        .iter()
        .map(|s| match s {
            Step::Prop(k) | Step::Entry(k) => Key::Name(k.clone()),
            Step::Item(i) => Key::Index(*i),
        })
        .collect()
}

fn segments_of(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|s| match s {
            Step::Prop(k) => k.clone(),
            Step::Item(_) => "[]".into(),
            Step::Entry(_) => "*".into(),
        })
        .collect()
}

fn in_data(steps: &[Step]) -> bool {
    steps.iter().any(|s| matches!(s, Step::Entry(_)))
}

/// The elements of an array worth a look: the first 16, then every 8th,
/// and the last.
pub fn sample_of(length: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < length {
        out.push(i);
        i += if i < 16 { 1 } else { 8 };
    }
    if length > 16 && (length - 1 - 16) % 8 != 0 {
        out.push(length - 1);
    }
    out
}

const MAX_ENTRIES: usize = 32;

fn fits(types: &BTreeSet<String>, v: &Value) -> bool {
    let t = json_type(v);
    types.contains(t) || (t == "number" && types.contains("integer") && v.as_f64().is_some_and(|n| n.fract() == 0.0 && n.is_finite()))
}

impl Schema {
    /// Whether a spec takes a value of this sort at all (without looking
    /// inside).
    pub fn accepts(&self, spec: &SpecRef, v: &Value) -> bool {
        match spec {
            SpecRef::Types(t) => fits(t, v),
            SpecRef::Node(i) => self.accepts_node(*i, v),
        }
    }

    fn accepts_node(&self, i: usize, v: &Value) -> bool {
        match &self.nodes[i] {
            Node::Any => true,
            Node::Types(t) => fits(t, v),
            Node::Object { .. } | Node::Record { .. } | Node::Variants { .. } => v.is_object(),
            Node::Array { .. } => v.is_array(),
            Node::Union { options } => options.iter().any(|o| self.accepts_node(*o, v)),
        }
    }

    /// Walks `value` against the schema (see `Inspection`).
    pub fn inspect(&self, value: &Value) -> Inspection {
        self.inspect_at(&SpecRef::Node(self.root), value)
    }

    /// Walks `value` against a place in the schema.
    pub fn inspect_at(&self, spec: &SpecRef, value: &Value) -> Inspection {
        let mut out = Inspection::default();
        self.walk(spec, value, &mut Vec::new(), None, &mut out);
        out
    }

    fn changed(steps: &[Step], v: &Value, spec: SpecRef, variant: Option<&[Value]>, out: &mut Inspection) {
        out.findings.push(Finding::TypeChange {
            path: path_of(steps),
            variant: variant.map(<[Value]>::to_vec),
            value: v.clone(),
            data: in_data(steps),
            keys: keys_of(steps),
            segments: segments_of(steps),
            spec,
        });
        if steps.is_empty() {
            out.drop = true;
        } else {
            out.strip.push(keys_of(steps));
        }
    }

    fn walk(&self, spec: &SpecRef, v: &Value, steps: &mut Vec<Step>, variant: Option<Vec<Value>>, out: &mut Inspection) {
        let i = match spec {
            SpecRef::Types(t) => {
                if !fits(t, v) {
                    Self::changed(steps, v, spec.clone(), variant.as_deref(), out);
                }
                return;
            }
            SpecRef::Node(i) => *i,
        };
        match &self.nodes[i] {
            Node::Any => {}
            Node::Types(t) => {
                if !fits(t, v) {
                    Self::changed(steps, v, spec.clone(), variant.as_deref(), out);
                }
            }
            Node::Array { items } => {
                let Value::Array(a) = v else { return Self::changed(steps, v, spec.clone(), variant.as_deref(), out) };
                for j in sample_of(a.len()) {
                    steps.push(Step::Item(j));
                    self.walk(&SpecRef::Node(*items), &a[j], steps, variant.clone(), out);
                    steps.pop();
                }
            }
            Node::Record { value } => {
                let Value::Object(m) = v else { return Self::changed(steps, v, spec.clone(), variant.as_deref(), out) };
                for (k, x) in m.iter().take(MAX_ENTRIES) {
                    steps.push(Step::Entry(k.clone()));
                    self.walk(&SpecRef::Node(*value), x, steps, variant.clone(), out);
                    steps.pop();
                }
            }
            Node::Object { props, closed, known } => {
                let Value::Object(m) = v else { return Self::changed(steps, v, spec.clone(), variant.as_deref(), out) };
                for (k, p, required) in props {
                    steps.push(Step::Prop(k.clone()));
                    match m.get(k) {
                        None => {
                            if *required {
                                out.findings.push(Finding::MissingField { path: path_of(steps), variant: variant.clone() });
                            }
                        }
                        Some(x) => self.walk(&SpecRef::Node(*p), x, steps, variant.clone(), out),
                    }
                    steps.pop();
                }
                // A key that reads as data rather than a field's name is never
                // kept: it shows as *.
                if *closed {
                    for (k, x) in m {
                        if props.iter().any(|(p, _, _)| p == k) || known.contains(k) {
                            continue;
                        }
                        steps.push(if is_field(k) { Step::Prop(k.clone()) } else { Step::Entry(k.clone()) });
                        out.findings.push(Finding::NewField { path: path_of(steps), variant: variant.clone(), value: x.clone(), data: in_data(steps) });
                        steps.pop();
                    }
                }
            }
            Node::Variants { key, by_value, value_types } => {
                let Value::Object(m) = v else { return Self::changed(steps, v, spec.clone(), variant.as_deref(), out) };
                steps.push(Step::Prop(key.clone()));
                let Some(d) = m.get(key) else {
                    out.findings.push(Finding::MissingField { path: path_of(steps), variant });
                    steps.pop();
                    return;
                };
                if let Some((_, option)) = by_value.iter().find(|(x, _)| same_value_zero(x, d)) {
                    steps.pop();
                    let mut inner = variant.unwrap_or_default();
                    inner.push(d.clone());
                    return self.walk(&SpecRef::Node(*option), v, steps, Some(inner), out);
                }
                // A value of the sort the schema lists is a new one; another
                // sort is a type change. Either is in the variant around it,
                // not one of its own.
                if value_types.contains(json_type(d)) {
                    out.findings.push(Finding::NewVariant { path: path_of(steps), variant, value: d.clone() });
                } else {
                    Self::changed(steps, d, SpecRef::Types(value_types.clone()), variant.as_deref(), out);
                }
                steps.pop();
            }
            Node::Union { options } => match options.iter().find(|o| self.accepts_node(**o, v)) {
                Some(o) => self.walk(&SpecRef::Node(*o), v, steps, variant, out),
                None => Self::changed(steps, v, spec.clone(), variant.as_deref(), out),
            },
        }
    }

    /// Each place `segments` lead to in `value` (every element for [],
    /// every entry for *) that `spec` doesn't take, as the keys to it: a
    /// type change found before, looked for again in a record that isn't
    /// checked whole.
    pub fn recheck(&self, value: &Value, segments: &[String], spec: &SpecRef) -> Vec<Vec<Key>> {
        let mut bad = Vec::new();
        self.recheck_at(value, segments, spec, &mut Vec::new(), &mut bad);
        bad
    }

    fn recheck_at(&self, v: &Value, segs: &[String], spec: &SpecRef, keys: &mut Vec<Key>, bad: &mut Vec<Vec<Key>>) {
        let Some((seg, rest)) = segs.split_first() else {
            if !self.accepts(spec, v) {
                bad.push(keys.clone());
            }
            return;
        };
        match (seg.as_str(), v) {
            ("[]", Value::Array(a)) => {
                for (j, x) in a.iter().enumerate() {
                    keys.push(Key::Index(j));
                    self.recheck_at(x, rest, spec, keys, bad);
                    keys.pop();
                }
            }
            ("[]", _) => {}
            ("*", Value::Object(m)) => {
                for (k, x) in m {
                    keys.push(Key::Name(k.clone()));
                    self.recheck_at(x, rest, spec, keys, bad);
                    keys.pop();
                }
            }
            ("*", _) => {}
            (name, Value::Object(m)) => {
                if let Some(x) = m.get(name) {
                    keys.push(Key::Name(name.to_string()));
                    self.recheck_at(x, rest, spec, keys, bad);
                    keys.pop();
                }
            }
            _ => {}
        }
    }
}

/// What a walk found. A type change also has the keys to strip it, and the
/// segments and spec to look for it again cheaply (see `recheck`). Each
/// says the variant it's in: the values of the discriminators the walk went
/// through to get there, outermost first, or None outside any. A new field
/// or type change inside an object keyed by data says `data`: its value's
/// shape keeps no key's name.
#[derive(Clone, Debug, PartialEq)]
pub enum Finding {
    MissingField { path: Option<String>, variant: Option<Vec<Value>> },
    TypeChange { path: Option<String>, variant: Option<Vec<Value>>, value: Value, data: bool, keys: Vec<Key>, segments: Vec<String>, spec: SpecRef },
    NewField { path: Option<String>, variant: Option<Vec<Value>>, value: Value, data: bool },
    NewVariant { path: Option<String>, variant: Option<Vec<Value>>, value: Value },
}

/// What `inspect` found, the keys to strip, and whether nothing can be read
/// (the record isn't an object).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inspection {
    pub findings: Vec<Finding>,
    pub strip: Vec<Vec<Key>>,
    pub drop: bool,
}

/// `record` without what's at the end of each of `paths`: a key taken out
/// of an object, an element out of an array. Later elements go first, so
/// taking one out doesn't move the others.
pub fn strip(record: &Value, paths: &[Vec<Key>]) -> Value {
    let mut order: Vec<&Vec<Key>> = paths.iter().collect();
    order.sort_by(|a, b| {
        for (x, y) in a.iter().zip(b.iter()) {
            if x == y {
                continue;
            }
            return match (x, y) {
                (Key::Index(i), Key::Index(j)) => j.cmp(i),
                (Key::Name(s), Key::Name(t)) => t.cmp(s),
                (Key::Index(_), Key::Name(_)) => std::cmp::Ordering::Greater,
                (Key::Name(_), Key::Index(_)) => std::cmp::Ordering::Less,
            };
        }
        a.len().cmp(&b.len())
    });
    let mut out = record.clone();
    for keys in order {
        out = without(out, keys);
    }
    out
}

fn without(v: Value, keys: &[Key]) -> Value {
    let Some((k, rest)) = keys.split_first() else { return v };
    match (v, k) {
        (Value::Array(mut a), Key::Index(i)) if *i < a.len() => {
            if rest.is_empty() {
                a.remove(*i);
            } else {
                let x = std::mem::take(&mut a[*i]);
                a[*i] = without(x, rest);
            }
            Value::Array(a)
        }
        (Value::Object(mut m), Key::Name(name)) if m.contains_key(name) => {
            if rest.is_empty() {
                m.shift_remove(name);
            } else {
                let x = m.get_mut(name).map(std::mem::take).unwrap_or_default();
                m.insert(name.clone(), without(x, rest));
            }
            Value::Object(m)
        }
        (v, _) => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema() -> Schema {
        compile(&json!({
            "oneOf": [
                { "type": "object", "properties": { "type": { "const": "user" }, "message": { "type": "object", "properties": { "content": { "anyOf": [{ "type": "string" }, { "type": "array", "items": { "$ref": "#/$defs/block" } }] } } } }, "required": ["type"] },
                { "type": "object", "properties": { "type": { "const": "assistant" }, "n": { "type": "integer" } }, "required": ["type", "n"], "x-xenonet": { "closed": true } }
            ],
            "$defs": { "block": { "type": "object", "properties": { "type": { "type": "string" }, "text": { "type": "string" } } } }
        }))
    }

    #[test]
    fn finds_what_differs() {
        let s = schema();
        let r = s.inspect(&json!({ "type": "user", "message": { "content": [{ "type": "text", "text": 5 }] } }));
        assert_eq!(r.findings.len(), 1);
        let Finding::TypeChange { path, variant, keys, segments, .. } = &r.findings[0] else { panic!() };
        assert_eq!((path.as_deref(), variant.clone()), (Some("message.content[].text"), Some(vec![json!("user")])));
        assert_eq!(segments, &["message", "content", "[]", "text"]);
        assert_eq!(
            strip(&json!({ "type": "user", "message": { "content": [{ "type": "text", "text": 5 }] } }), std::slice::from_ref(keys)),
            json!({ "type": "user", "message": { "content": [{ "type": "text" }] } })
        );
        let r = s.inspect(&json!({ "type": "assistant", "n": 1.5, "extra": 1, "0190d3e0-aaaa": 2 }));
        let kinds: Vec<_> = r
            .findings
            .iter()
            .map(|f| match f {
                Finding::TypeChange { path, .. } => format!("type-change {path:?}"),
                Finding::NewField { path, .. } => format!("new-field {path:?}"),
                f => format!("{f:?}"),
            })
            .collect();
        assert_eq!(kinds, ["type-change Some(\"n\")", "new-field Some(\"extra\")", "new-field Some(\"*\")"]);
        assert!(matches!(s.inspect(&json!({ "type": "system" })).findings[0], Finding::NewVariant { .. }));
        assert!(s.inspect(&json!([1])).drop);
    }

    #[test]
    fn samples_and_rechecks() {
        assert_eq!(sample_of(3), [0, 1, 2]);
        assert_eq!(sample_of(20).len(), 18, "16, then 16+8 is past it, then the last");
        let s = schema();
        let Finding::TypeChange { segments, spec, .. } = s.inspect(&json!({ "type": "user", "message": { "content": [{ "text": 5 }] } })).findings.remove(0) else { panic!() };
        let bad = s.recheck(&json!({ "message": { "content": [{ "text": "ok" }, { "text": [] }] } }), &segments, &spec);
        assert_eq!(bad, [vec![Key::Name("message".into()), Key::Name("content".into()), Key::Index(1), Key::Name("text".into())]]);
    }
}
