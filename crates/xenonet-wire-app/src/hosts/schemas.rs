//! What the hosts' Rust asks of the schemas (zod's, in the twins): a host
//! entry and a relay's hello checked against host/list/v1 and
//! host/hello/v1, and what another host sends checked against its schema
//! and copied with only what that declares (core/hosts/merge.js's clean,
//! with declared.js's walk), as XenoNet's JSON Schemas in schemas/ have
//! them. Everything a host sends is untrusted.
//!
//! The check is xenonet-schema's, which says what zod says. The copy walks
//! the JSON Schema: an object keeps the fields it declares, a record its
//! keys, a union the option the value fits (by its types, constants and
//! fields: the value has already passed the whole schema), and a schema that
//! takes anything keeps none of an object's own.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use xenonet_hosts::merge;
use xenonet_schema::check::{compile, Checker, Data};

const SESSION: &str = include_str!("../../../../schemas/session/v1.json");
const PROJECT: &str = include_str!("../../../../schemas/project/v1.json");
const IDENTITY: &str = include_str!("../../../../schemas/identity/v1.json");
const LIVE_SESSION: &str = include_str!("../../../../schemas/live/session/v1.json");
const LIVE_ACTIVITY: &str = include_str!("../../../../schemas/live/activity/v1.json");
const ENGINE_AGENT: &str = include_str!("../../../../schemas/engine/agent/v1.json");
const ENGINE_MESSAGE: &str = include_str!("../../../../schemas/engine/message/v1.json");
const HOST_HELLO: &str = include_str!("../../../../schemas/host/hello/v1.json");
const HOST_LIST: &str = include_str!("../../../../schemas/host/list/v1.json");
const GRAPH: &str = include_str!("../../../../schemas/graph/v1.json");

/// The kinds of node another host's map may hold (graph.js's GRAPH_NODES):
/// the core and host nodes are this machine's to make.
const NODE_KINDS: [&str; 7] = ["session", "project", "native", "zone", "wiki", "page", "resource"];

pub struct HostSchemas {
    docs: Vec<Value>,
    by_uri: HashMap<String, usize>,
    checker: Option<Checker>,
    /// The documents' indexes by what they check.
    named: HashMap<String, usize>,
}

fn parse(text: &str) -> Value {
    xenonet_base::json::parse(text).unwrap_or(Value::Null)
}

fn is_object(v: &Value) -> bool {
    v.is_object()
}

// The kind a node's option of graph/v1 is for: its kind field's constant,
// in it or in a part of it.
fn kind_of(node: &Value, docs: &[Value], by_uri: &HashMap<String, usize>) -> Option<String> {
    let direct = node.get("properties").and_then(|p| p.get("kind")).and_then(|k| k.get("const")).and_then(Value::as_str).map(str::to_string);
    if direct.is_some() {
        return direct;
    }
    if let Some(r) = node.get("$ref").and_then(Value::as_str) {
        return by_uri.get(r.split('#').next().unwrap_or("")).and_then(|i| kind_of(&docs[*i], docs, by_uri));
    }
    node.get("allOf").and_then(Value::as_array).and_then(|members| members.iter().find_map(|m| kind_of(m, docs, by_uri)))
}

impl HostSchemas {
    pub fn new() -> Self {
        let mut docs: Vec<Value> = [SESSION, PROJECT, IDENTITY, LIVE_SESSION, LIVE_ACTIVITY, ENGINE_AGENT, ENGINE_MESSAGE, HOST_HELLO].iter().map(|t| parse(t)).collect();
        let names = ["session-doc", "project-doc", "identity", "live-session", "live-activity", "engine-agent", "engine-message", "hello"];
        let mut named: HashMap<String, usize> = names.iter().enumerate().map(|(i, n)| (n.to_string(), i)).collect();
        let by_uri_of = |docs: &[Value]| -> HashMap<String, usize> { docs.iter().enumerate().filter_map(|(i, d)| Some((d.get("$id")?.as_str()?.to_string(), i))).collect() };
        // A host entry: host/list/v1's hosts' items, as a document of its own.
        let list = parse(HOST_LIST);
        if let Some(entry) = list.get("properties").and_then(|p| p.get("hosts")).and_then(|h| h.get("items")) {
            let mut e = entry.clone();
            e["$id"] = json!("xenonet:host/entry");
            named.insert("entry".into(), docs.len());
            docs.push(e);
        }
        // Each kind of node: its option of graph/v1's nodes.
        let graph = parse(GRAPH);
        let options: Vec<Value> =
            graph.get("properties").and_then(|p| p.get("nodes")).and_then(|n| n.get("items")).and_then(|i| i.get("anyOf")).and_then(Value::as_array).cloned().unwrap_or_default();
        let known = by_uri_of(&docs);
        for o in options {
            let Some(kind) = kind_of(&o, &docs, &known) else { continue };
            if !NODE_KINDS.contains(&kind.as_str()) {
                continue;
            }
            let mut o = o;
            o["$id"] = json!(format!("xenonet:graph/node/{kind}"));
            named.insert(format!("node:{kind}"), docs.len());
            docs.push(o);
        }
        let by_uri = by_uri_of(&docs);
        let checker = compile(&docs).ok();
        HostSchemas { docs, by_uri, checker, named }
    }

    /// check(schema, value) as explain() words it, or None when it passes;
    /// a schema that isn't there refuses.
    fn explain(&self, name: &str, v: &Value) -> Option<String> {
        let (Some(c), Some(&i)) = (&self.checker, self.named.get(name)) else { return Some("(the value): XenoNet can't check it".into()) };
        let r = c.check(c.root(i), &Data::from(v), false);
        if r.ok() {
            return None;
        }
        let issues: Vec<(String, String)> = r.issues.into_iter().map(|f| (f.path, f.message)).collect();
        Some(xenonet_schema::check::explain(&issues, &r.undeclared))
    }

    pub fn check_entry(&self, v: &Value) -> Option<String> {
        self.explain("entry", v)
    }

    pub fn check_hello(&self, v: &Value) -> Option<String> {
        self.explain("hello", v)
    }

    fn passes(&self, name: &str, v: &Value) -> bool {
        let (Some(c), Some(&i)) = (&self.checker, self.named.get(name)) else { return false };
        c.check(c.root(i), &Data::from(v), false).ok()
    }

    /// clean(kind, value): checked, then copied with what it declares.
    pub fn clean_named(&self, name: &str, v: &Value) -> Option<Value> {
        let &i = self.named.get(name)?;
        if !self.passes(name, v) {
            return None;
        }
        Some(self.declared(i, &self.docs[i], v, 0))
    }

    fn resolve(&self, doc: usize, r: &str) -> Option<(usize, &Value)> {
        let (base, fragment) = r.split_once('#').unwrap_or((r, ""));
        let at = if base.is_empty() { doc } else { *self.by_uri.get(base)? };
        let mut node = &self.docs[at];
        if !fragment.is_empty() {
            for part in fragment.strip_prefix('/')?.split('/') {
                let part = part.replace("~1", "/").replace("~0", "~");
                node = match node {
                    Value::Object(m) => m.get(&part)?,
                    Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
                    _ => return None,
                };
            }
        }
        Some((at, node))
    }

    // The fields an object schema declares, an intersection's from every
    // part (a later part's over an earlier's), as (doc, field schema).
    fn fields_of<'a>(&'a self, doc: usize, node: &'a Value, depth: usize) -> Option<Vec<(String, usize, &'a Value)>> {
        if depth > 64 {
            return None;
        }
        let m = node.as_object()?;
        if let Some(r) = m.get("$ref").and_then(Value::as_str) {
            let (d, t) = self.resolve(doc, r)?;
            return self.fields_of(d, t, depth + 1);
        }
        if let Some(Value::Array(members)) = m.get("allOf") {
            let mut out: Vec<(String, usize, &Value)> = Vec::new();
            for member in members {
                for (k, d, f) in self.fields_of(doc, member, depth + 1).unwrap_or_default() {
                    match out.iter_mut().find(|(x, ..)| *x == k) {
                        Some(slot) => *slot = (k, d, f),
                        None => out.push((k, d, f)),
                    }
                }
            }
            return Some(out);
        }
        let props = m.get("properties")?.as_object()?;
        Some(props.iter().map(|(k, v)| (k.clone(), doc, v)).collect())
    }

    // Whether a value fits a schema by its types, constants and fields, to
    // pick a union's option.
    fn fits(&self, doc: usize, node: &Value, v: &Value, depth: usize) -> bool {
        if depth > 64 {
            return false;
        }
        let m = match node {
            Value::Bool(b) => return *b,
            Value::Object(m) => m,
            _ => return true,
        };
        if let Some(r) = m.get("$ref").and_then(Value::as_str) {
            return self.resolve(doc, r).is_some_and(|(d, t)| self.fits(d, t, v, depth + 1));
        }
        if let Some(c) = m.get("const") {
            return c == v;
        }
        if let Some(Value::Array(e)) = m.get("enum") {
            return e.contains(v);
        }
        if let Some(Value::Array(options)) = m.get("anyOf").or_else(|| m.get("oneOf")) {
            return options.iter().any(|o| self.fits(doc, o, v, depth + 1));
        }
        if let Some(Value::Array(members)) = m.get("allOf") {
            return members.iter().all(|o| self.fits(doc, o, v, depth + 1));
        }
        let type_ok = |t: &str| match t {
            "string" => v.is_string(),
            "number" => v.is_number(),
            "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            "array" => v.is_array(),
            "object" => is_object(v),
            _ => true,
        };
        match m.get("type") {
            Some(Value::String(t)) if !type_ok(t) => return false,
            Some(Value::Array(ts)) if !ts.iter().filter_map(Value::as_str).any(type_ok) => return false,
            _ => {}
        }
        if let (Some(props), Value::Object(o)) = (m.get("properties").and_then(Value::as_object), v) {
            let required: Vec<&str> = m.get("required").and_then(Value::as_array).map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
            if required.iter().any(|k| !o.contains_key(*k)) {
                return false;
            }
            return props.iter().all(|(k, s)| o.get(k).is_none_or(|x| self.fits(doc, s, x, depth + 1)));
        }
        if let (Some(items), Value::Array(a)) = (m.get("items"), v) {
            return a.iter().all(|x| self.fits(doc, items, x, depth + 1));
        }
        true
    }

    // A schema that takes anything: an object keeps none of its own.
    fn any(v: &Value) -> Value {
        match v {
            Value::Array(a) => Value::Array(a.iter().map(Self::any).collect()),
            Value::Object(_) => Value::Object(Map::new()),
            other => other.clone(),
        }
    }

    /// declared(schema, value): a copy with what the schema declares, and
    /// nothing else, at every level.
    fn declared(&self, doc: usize, node: &Value, v: &Value, depth: usize) -> Value {
        if depth > 128 {
            return Value::Null;
        }
        let m = match node {
            Value::Object(m) => m,
            Value::Bool(true) => return Self::any(v),
            _ => return v.clone(),
        };
        if let Some(r) = m.get("$ref").and_then(Value::as_str) {
            return match self.resolve(doc, r) {
                Some((d, t)) => self.declared(d, t, v, depth + 1),
                None => v.clone(),
            };
        }
        if m.contains_key("const") || m.contains_key("enum") {
            return v.clone();
        }
        if let Some(Value::Array(options)) = m.get("anyOf").or_else(|| m.get("oneOf")) {
            if v.is_null() && options.iter().any(|o| o.get("type").and_then(Value::as_str) == Some("null")) {
                return Value::Null;
            }
            return match options.iter().find(|o| self.fits(doc, o, v, depth + 1)) {
                Some(o) => self.declared(doc, o, v, depth + 1),
                None => v.clone(),
            };
        }
        let ty = m.get("type").and_then(Value::as_str);
        if m.contains_key("allOf") && ty != Some("string") {
            let Value::Object(o) = v else { return v.clone() };
            let fields = self.fields_of(doc, node, depth + 1).unwrap_or_default();
            let mut out = Map::new();
            for (k, d, f) in fields {
                if let Some(x) = o.get(&k) {
                    out.insert(k, self.declared(d, f, x, depth + 1));
                }
            }
            return Value::Object(out);
        }
        match ty {
            Some("array") => match v {
                Value::Array(a) => {
                    let prefix = m.get("prefixItems").and_then(Value::as_array);
                    Value::Array(
                        a.iter()
                            .enumerate()
                            .map(|(i, x)| match prefix.and_then(|p| p.get(i)).or_else(|| m.get("items")) {
                                Some(s) => self.declared(doc, s, x, depth + 1),
                                None => x.clone(),
                            })
                            .collect(),
                    )
                }
                _ => v.clone(),
            },
            Some("object") | None if m.contains_key("properties") || m.contains_key("propertyNames") || ty == Some("object") => {
                let Value::Object(o) = v else { return v.clone() };
                if m.contains_key("propertyNames")
                    || (!m.contains_key("properties") && m.get("additionalProperties").is_some_and(|a| a.is_object() && !a.as_object().is_some_and(Map::is_empty)))
                {
                    let value = m.get("additionalProperties").cloned().unwrap_or(Value::Bool(true));
                    return Value::Object(o.iter().map(|(k, x)| (k.clone(), self.declared(doc, &value, x, depth + 1))).collect());
                }
                let mut out = Map::new();
                if let Some(props) = m.get("properties").and_then(Value::as_object) {
                    for (k, s) in props {
                        if let Some(x) = o.get(k) {
                            out.insert(k.clone(), self.declared(doc, s, x, depth + 1));
                        }
                    }
                }
                Value::Object(out)
            }
            None if m.keys().all(|k| matches!(k.as_str(), "description" | "title" | "$comment" | "default" | "examples" | "x-xenonet")) => Self::any(v),
            _ => v.clone(),
        }
    }
}

impl Default for HostSchemas {
    fn default() -> Self {
        Self::new()
    }
}

/// The merge's asks, answered: the schemas, and graph.js's activityOf.
pub struct MergeSchemas<'a>(pub &'a HostSchemas);

impl merge::Schemas for MergeSchemas<'_> {
    fn clean(&mut self, kind: merge::Kind, value: &Value) -> Option<Value> {
        let name = match kind {
            merge::Kind::Node(k) => format!("node:{k}"),
            merge::Kind::Identity => "identity".into(),
            merge::Kind::LiveSession => "live-session".into(),
            merge::Kind::LiveActivity => "live-activity".into(),
            merge::Kind::EngineAgent => "engine-agent".into(),
            merge::Kind::EngineMessage => "engine-message".into(),
        };
        self.0.clean_named(&name, value)
    }

    fn activity(&mut self, updated_at: &Value, now: f64, windows: &Value) -> Value {
        json!(xenonet_state::graph::activity_of(Some(updated_at), now, Some(windows)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_the_hosts_read_compiles() {
        let s = HostSchemas::new();
        assert!(s.checker.is_some());
        for k in NODE_KINDS {
            assert!(s.named.contains_key(&format!("node:{k}")), "{k}");
        }
    }

    #[test]
    fn a_hosts_entry_and_hello_as_zod_checks_them() {
        let s = HostSchemas::new();
        let entry =
            json!({ "id": "lab", "transport": "local", "ssh": null, "port": 47840, "command": null, "since": "7d", "detail": false, "key": null, "name": null, "addedAt": 1 });
        assert_eq!(s.check_entry(&entry), None);
        let mut bad = entry.clone();
        bad["port"] = json!(0);
        assert!(s.check_entry(&bad).unwrap().starts_with("port: "));
        assert!(s.check_hello(&json!({ "t": "hello" })).is_some());
    }

    #[test]
    fn what_a_host_sends_keeps_only_what_its_schema_declares() {
        let s = HostSchemas::new();
        let mut m = MergeSchemas(&s);
        use merge::Schemas;
        let wiki = json!({ "id": "wiki", "kind": "wiki", "name": "Agent Wiki", "path": "~/wiki", "pages": 3, "script": "<b>x</b>" });
        if let Some(out) = m.clean(merge::Kind::Node("wiki"), &wiki) {
            assert!(out.get("script").is_none(), "{out}");
            assert_eq!(out["kind"], "wiki");
        }
        assert_eq!(m.clean(merge::Kind::Node("core"), &json!({ "id": "core", "kind": "core" })), None, "the core is this machine's");
        assert_eq!(m.clean(merge::Kind::Node("session"), &json!({ "kind": "session" })), None, "one that doesn't pass is left out");
        let any = HostSchemas::any(&json!({ "a": [1, { "b": 2 }] }));
        assert_eq!(any, json!({}));
    }
}
