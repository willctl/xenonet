//! The bounds in the schemas of what another machine, an actor or a relay
//! sends hold as it's read: for each such document, values are made from
//! its schema (every property, and each branch of every union), each one
//! reads, and each with one bound pushed past (a string too long or too
//! short, a pattern not matched, a number out of range or not whole, a list
//! too long or too short) is refused.

#[cfg(test)]
mod made {
    use serde::de::DeserializeOwned;
    use serde_json::{Map, Value};
    use std::collections::{BTreeMap, BTreeSet};
    use xenonet_model::schema::{self, Document};
    use xenonet_model::{actor, host, network};

    pub type Read = fn(&Value) -> Result<(), String>;

    fn read<T: DeserializeOwned>(v: &Value) -> Result<(), String> {
        serde_json::from_value::<T>(v.clone()).map(|_| ()).map_err(|e| e.to_string())
    }

    pub struct Doc {
        pub id: &'static str,
        pub schema: Value,
        pub read: Read,
    }

    fn doc<T: Document + DeserializeOwned>() -> Doc {
        Doc { id: T::ID, schema: schema::document::<T>(), read: read::<T> }
    }

    /// Every document read from outside: another host's frames, a relay's
    /// hello, and what actors send.
    pub fn untrusted() -> Vec<Doc> {
        vec![
            doc::<network::Claims>(),
            doc::<network::Frame>(),
            doc::<network::Handoff>(),
            doc::<network::Message>(),
            doc::<network::Operator>(),
            doc::<network::Presence>(),
            doc::<network::Receipt>(),
            doc::<network::Relay>(),
            doc::<network::RosterEntry>(),
            doc::<network::State>(),
            doc::<network::Svc>(),
            doc::<host::Hello>(),
            doc::<actor::Registration>(),
            doc::<actor::Report>(),
            doc::<actor::Console>(),
            doc::<actor::Events>(),
            doc::<actor::Requests>(),
            doc::<actor::GatewayAsk>(),
        ]
    }

    /// Every document by its `$id`, for a `$ref`.
    pub fn by_uri() -> BTreeMap<String, Value> {
        schema::documents().into_iter().map(|(_, text)| serde_json::from_str::<Value>(&text).unwrap()).map(|v| (v["$id"].as_str().unwrap().to_string(), v)).collect()
    }

    /// A string that matches each pattern the documents use.
    pub fn sample(pattern: &str) -> Option<&'static str> {
        Some(match pattern {
            "^[a-z][a-z0-9-]{1,31}$" => "vm1",
            "^[a-z0-9][a-z0-9-]{0,31}$" | "^[a-z0-9][a-z0-9-]{0,63}$" | "^[a-z0-9][a-z0-9-]{0,40}$" => "kite",
            "^[A-Za-z0-9_-]*$" => "AAAA",
            "^[0-9a-f]{32}$" => "0123456789abcdef0123456789abcdef",
            "^xnk1_[a-z2-7]{32}$" => "xnk1_abcdefghijklmnopqrstuvwxyz234567",
            "^xns1_[a-z2-7]{32}$" => "xns1_abcdefghijklmnopqrstuvwxyz234567",
            "^prs_[a-z2-7]{32}$" => "prs_abcdefghijklmnopqrstuvwxyz234567",
            "^net_[0-9a-f]{32}$" => "net_0123456789abcdef0123456789abcdef",
            "^sub_[0-9a-f]{32}$" => "sub_0123456789abcdef0123456789abcdef",
            "^svq_[0-9a-f]{32}$" => "svq_0123456789abcdef0123456789abcdef",
            "^msg_[0-9a-f-]{36}$" => "msg_00000000-0000-4000-8000-000000000000",
            "^frm_[0-9a-f-]{36}$" => "frm_00000000-0000-4000-8000-000000000000",
            "^hof_[0-9a-f-]{36}$" => "hof_00000000-0000-4000-8000-000000000000",
            "^dev_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => "dev_00000000-0000-4000-8000-000000000000",
            "^prj_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => "prj_00000000-0000-4000-8000-000000000000",
            "^r_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => "r_00000000-0000-4000-8000-000000000000",
            "^(claude|codex|grok):[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$" => "claude:00000000-0000-4000-8000-000000000000",
            "^cn_[0-9a-f]{24}$" => "cn_0123456789abcdef01234567",
            "^gr_[0-9a-f]{24}$" => "gr_0123456789abcdef01234567",
            "^att_[0-9a-f]{24}$" => "att_0123456789abcdef01234567",
            "^xenonet_actor_[0-9a-f]{64}$" => "xenonet_actor_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "^xenonet_gateway_[0-9a-f]{64}$" => "xenonet_gateway_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "^actor:[a-z0-9][a-z0-9-]{0,31}$" | "^(@[a-z][a-z0-9-]{1,31}:)?actor:[a-z0-9][a-z0-9-]{0,31}$" => "actor:kite",
            "^(@[a-z][a-z0-9-]{1,31}:)?[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{6,}$" | "^(@[a-z][a-z0-9-]{1,31}:)?[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{0,}$" => {
                "claude:4f1c2a3b"
            }
            "^[a-z][a-z0-9-]*$" | "^[a-z0-9][a-z0-9-]*$" => "claude",
            "^[A-Za-z0-9._:-]{1,64}$" => "e1",
            "^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$" => "studio.local",
            "^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$" => "will",
            r#"^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$"# => "~/.ssh/id_xenonet",
            r"^(?:xenonet|\/[A-Za-z0-9._+/-]{1,1023})$" => "xenonet",
            r"^\d{1,4}[mhd]$" => "7d",
            "^[0-9a-f]{7,64}$" => "abcdef1",
            "^[A-Z]{1,16}$" => "GET",
            "^[a-z-]{1,32}$" => "not-found",
            "^[A-Z0-9]{10}$" => "ABCDEFGHIJ",
            "^repo:.+$" => "repo:github.com/willctl/xenonet",
            "^[a-z0-9][a-z0-9-]*(/[a-z0-9][a-z0-9-]*)*/v[1-9][0-9]*$" => "live/snapshot/v1",
            "^#[0-9a-fA-F]{6}$" => "#00ff88",
            r"^[a-z0-9-]+\/[a-z0-9-]+$" => "willctl/xenonet",
            r"^~(\/.*)?$" => "~/x",
            r"^(?:(?:\d\d[2468][048]|\d\d[13579][26]|\d\d0[48]|[02468][048]00|[13579][26]00)-02-29|\d{4}-(?:(?:0[13578]|1[02])-(?:0[1-9]|[12]\d|3[01])|(?:0[469]|11)-(?:0[1-9]|[12]\d|30)|(?:02)-(?:0[1-9]|1\d|2[0-8])))T(?:(?:[01]\d|2[0-3]):[0-5]\d:[0-5]\d(?:\.\d+)?(?:Z))$" => {
                "2026-11-09T12:00:00Z"
            }
            _ => return None,
        })
    }

    #[derive(Clone, Debug)]
    pub enum Step {
        Key(String),
        Index(usize),
    }

    /// A bound met in a made value: where the value is, the schema keyword
    /// and the node that has it.
    #[derive(Clone, Debug)]
    pub struct Site {
        pub at: Vec<Step>,
        /// The keyword's place in the schema: `<document>#<pointer>/<keyword>`.
        pub ptr: String,
        pub keyword: &'static str,
        pub node: Map<String, Value>,
        pub value: Value,
    }

    pub struct Maker<'a> {
        pub refs: &'a BTreeMap<String, Value>,
        /// Which branch to take at a union, by its pointer; the first that
        /// isn't null otherwise.
        pub choose: &'a BTreeMap<String, usize>,
        /// The unions met, with how many branches each has.
        pub unions: BTreeMap<String, usize>,
        pub sites: Vec<Site>,
        pub missing: BTreeSet<String>,
    }

    fn is_null(s: &Value) -> bool {
        s.get("type") == Some(&Value::String("null".into()))
    }

    fn num(m: &Map<String, Value>, k: &str) -> Option<f64> {
        m.get(k).and_then(Value::as_f64)
    }

    pub const KEYWORDS: [&str; 9] = ["maxLength", "minLength", "pattern", "maximum", "minimum", "exclusiveMinimum", "maxItems", "minItems", "integer"];

    impl Maker<'_> {
        pub fn make(&mut self, s: &Value, ptr: &str, at: &[Step]) -> Value {
            let Some(m) = s.as_object() else { return Value::Null };
            if let Some(r) = m.get("$ref").and_then(Value::as_str) {
                let target = self.refs.get(r).unwrap_or_else(|| panic!("{ptr}: no document {r}")).clone();
                return self.make(&target, &format!("{r}#"), at);
            }
            let v = self.value(m, ptr, at);
            self.bounds(m, ptr, at, &v);
            v
        }

        fn value(&mut self, m: &Map<String, Value>, ptr: &str, at: &[Step]) -> Value {
            if let Some(c) = m.get("const") {
                return c.clone();
            }
            if let Some(e) = m.get("enum").and_then(Value::as_array) {
                return e[0].clone();
            }
            for key in ["anyOf", "oneOf"] {
                if let Some(branches) = m.get(key).and_then(Value::as_array) {
                    let p = format!("{ptr}/{key}");
                    self.unions.insert(p.clone(), branches.len());
                    let first = branches.iter().position(|b| !is_null(b)).unwrap_or(0);
                    let i = self.choose.get(&p).copied().unwrap_or(first);
                    return self.make(&branches[i], &format!("{p}/{i}"), at);
                }
            }
            if let Some(parts) = m.get("allOf").and_then(Value::as_array) {
                let mut out = Map::new();
                for (i, part) in parts.iter().enumerate() {
                    if let Value::Object(o) = self.make(part, &format!("{ptr}/allOf/{i}"), at) {
                        out.extend(o);
                    }
                }
                if let Value::Object(o) = self.object(m, ptr, at) {
                    out.extend(o);
                }
                return Value::Object(out);
            }
            let ty = match m.get("type") {
                Some(Value::String(t)) => t.as_str(),
                Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).find(|t| *t != "null").unwrap_or("null"),
                _ if m.contains_key("properties") => "object",
                _ => "any",
            };
            match ty {
                "string" => {
                    if let Some(p) = m.get("pattern").and_then(Value::as_str) {
                        match sample(p) {
                            Some(s) => Value::String(s.into()),
                            None => {
                                self.missing.insert(p.to_string());
                                Value::String(String::new())
                            }
                        }
                    } else {
                        let min = num(m, "minLength").unwrap_or(0.0) as usize;
                        let max = num(m, "maxLength").map_or(usize::MAX, |n| n as usize);
                        Value::String("x".repeat(min.max(1).min(max)))
                    }
                }
                "integer" | "number" => {
                    let n = num(m, "minimum").or_else(|| num(m, "exclusiveMinimum").map(|n| n + 1.0)).unwrap_or(1.0).max(1.0).min(num(m, "maximum").unwrap_or(f64::MAX));
                    number(n)
                }
                "boolean" => Value::Bool(true),
                "null" => Value::Null,
                "array" => {
                    if let Some(items) = m.get("prefixItems").and_then(Value::as_array) {
                        let mut at = at.to_vec();
                        return Value::Array(
                            items
                                .iter()
                                .enumerate()
                                .map(|(i, s)| {
                                    at.push(Step::Index(i));
                                    let v = self.make(s, &format!("{ptr}/prefixItems/{i}"), &at);
                                    at.pop();
                                    v
                                })
                                .collect(),
                        );
                    }
                    let min = num(m, "minItems").unwrap_or(0.0) as usize;
                    let max = num(m, "maxItems").map_or(usize::MAX, |n| n as usize);
                    let n = min.max(1).min(max);
                    let items = m.get("items").cloned().unwrap_or(Value::Bool(true));
                    let mut at = at.to_vec();
                    Value::Array(
                        (0..n)
                            .map(|i| {
                                at.push(Step::Index(i));
                                let v = self.make(&items, &format!("{ptr}/items"), &at);
                                at.pop();
                                v
                            })
                            .collect(),
                    )
                }
                "object" => self.object(m, ptr, at),
                _ => Value::Null,
            }
        }

        fn object(&mut self, m: &Map<String, Value>, ptr: &str, at: &[Step]) -> Value {
            let mut out = Map::new();
            let mut at = at.to_vec();
            if let Some(props) = m.get("properties").and_then(Value::as_object) {
                for (k, s) in props {
                    at.push(Step::Key(k.clone()));
                    let v = self.make(s, &format!("{ptr}/properties/{k}"), &at);
                    at.pop();
                    out.insert(k.clone(), v);
                }
            }
            // A record: one entry, its key made from the key's schema.
            if let (Some(keys), Some(values)) = (m.get("propertyNames"), m.get("additionalProperties")) {
                let key = match self.make(keys, &format!("{ptr}/propertyNames"), &[]) {
                    Value::String(k) => k,
                    _ => "k".to_string(),
                };
                // A site is a value: the key's own bounds aren't pushed.
                self.sites.retain(|s| !s.ptr.starts_with(&format!("{ptr}/propertyNames/")));
                at.push(Step::Key(key.clone()));
                let v = self.make(values, &format!("{ptr}/additionalProperties"), &at);
                at.pop();
                out.insert(key, v);
            }
            Value::Object(out)
        }

        fn bounds(&mut self, m: &Map<String, Value>, ptr: &str, at: &[Step], v: &Value) {
            for keyword in KEYWORDS {
                if has(m, keyword) {
                    self.sites.push(Site { at: at.to_vec(), ptr: format!("{ptr}/{keyword}"), keyword, node: m.clone(), value: v.clone() });
                }
            }
        }
    }

    // Whether a schema node has a bound of the kind.
    fn has(m: &Map<String, Value>, keyword: &str) -> bool {
        match keyword {
            "integer" => m.get("type") == Some(&Value::String("integer".into())),
            "minLength" | "minItems" => num(m, keyword).is_some_and(|n| n > 0.0),
            _ => m.contains_key(keyword),
        }
    }

    /// Every bound in a schema and the documents it refers to, by the
    /// pointer `Maker` gives its site.
    pub fn sites_in(s: &Value, ptr: &str, refs: &BTreeMap<String, Value>, seen: &mut BTreeSet<String>, out: &mut BTreeSet<String>) {
        let Some(m) = s.as_object() else { return };
        if let Some(r) = m.get("$ref").and_then(Value::as_str) {
            if seen.insert(r.to_string()) {
                sites_in(&refs[r], &format!("{r}#"), refs, seen, out);
            }
            return;
        }
        for keyword in KEYWORDS {
            if has(m, keyword) {
                out.insert(format!("{ptr}/{keyword}"));
            }
        }
        for key in ["not", "if", "then", "else", "patternProperties", "contains", "dependentSchemas"] {
            assert!(!m.contains_key(key), "{ptr}: {key} isn't walked");
        }
        for (key, v) in m {
            match (key.as_str(), v) {
                ("properties", Value::Object(ps)) => {
                    for (k, v) in ps {
                        sites_in(v, &format!("{ptr}/properties/{k}"), refs, seen, out);
                    }
                }
                ("anyOf" | "oneOf" | "allOf" | "prefixItems", Value::Array(bs)) => {
                    for (i, v) in bs.iter().enumerate() {
                        sites_in(v, &format!("{ptr}/{key}/{i}"), refs, seen, out);
                    }
                }
                ("items" | "additionalProperties" | "propertyNames", v) => sites_in(v, &format!("{ptr}/{key}"), refs, seen, out),
                _ => {}
            }
        }
    }

    // A number as JSON writes it: a whole one as an integer, as 2^53 is
    // when it's pushed past the widest a reader keeps.
    fn number(n: f64) -> Value {
        if n.fract() == 0.0 && n.abs() < 9.2e18 {
            Value::from(n as i64)
        } else {
            Value::from(n)
        }
    }

    fn units(s: &str) -> usize {
        s.encode_utf16().count()
    }

    /// The site's value pushed past its bound.
    pub fn pushed(site: &Site) -> Option<Value> {
        let n = |k: &str| num(&site.node, k);
        Some(match (site.keyword, &site.value) {
            ("maxLength", Value::String(s)) => {
                let max = n("maxLength")? as usize;
                Value::String(format!("{s}{}", "a".repeat(max + 1 - units(s))))
            }
            ("minLength", Value::String(s)) => {
                let min = n("minLength")? as usize;
                Value::String(s.chars().take(min - 1).collect())
            }
            ("pattern", Value::String(s)) => Value::String(format!("!{s}")),
            ("maximum", _) => number(n("maximum")? + 1.0),
            ("minimum", _) => number(n("minimum")? - 1.0),
            ("exclusiveMinimum", _) => number(n("exclusiveMinimum")?),
            ("integer", Value::Number(x)) => Value::from(x.as_f64()? + 0.5),
            ("maxItems", Value::Array(a)) => {
                let max = n("maxItems")? as usize;
                let last = a.last()?.clone();
                let mut a = a.clone();
                a.resize(max + 1, last);
                Value::Array(a)
            }
            ("minItems", Value::Array(a)) => {
                let min = n("minItems")? as usize;
                Value::Array(a[..min - 1].to_vec())
            }
            _ => return None,
        })
    }

    /// `root` with the value at `at` replaced.
    pub fn with(root: &Value, at: &[Step], v: Value) -> Value {
        let mut out = root.clone();
        let mut here = &mut out;
        for step in at {
            here = match step {
                Step::Key(k) => here.get_mut(k.as_str()).unwrap(),
                Step::Index(i) => here.get_mut(*i).unwrap(),
            };
        }
        *here = v;
        out
    }
}

use made::{by_uri, pushed, untrusted, with, Maker};
use std::collections::{BTreeMap, BTreeSet};

// Values for `doc`: the first branch of every union, then each other
// branch of each union met, until every branch has been taken once.
fn values_of(doc: &made::Doc, refs: &BTreeMap<String, serde_json::Value>, missing: &mut BTreeSet<String>) -> Vec<(serde_json::Value, Vec<made::Site>, String)> {
    let mut out = Vec::new();
    let mut todo = vec![BTreeMap::new()];
    let mut taken = BTreeSet::new();
    while let Some(choose) = todo.pop() {
        let mut maker = Maker { refs, choose: &choose, unions: BTreeMap::new(), sites: Vec::new(), missing: BTreeSet::new() };
        let v = maker.make(&doc.schema, &format!("{}#", doc.id), &[]);
        missing.extend(maker.missing);
        for (p, n) in &maker.unions {
            for i in 0..*n {
                if !choose.contains_key(p) && taken.insert((p.clone(), i)) {
                    let mut next = choose.clone();
                    next.insert(p.clone(), i);
                    todo.push(next);
                }
            }
        }
        out.push((v, maker.sites, format!("{choose:?}")));
    }
    out
}

#[test]
fn every_bound_of_what_comes_from_outside_holds_as_it_is_read() {
    let refs = by_uri();
    let mut missing = BTreeSet::new();
    let mut problems = Vec::new();
    let mut checked = BTreeSet::new();
    let mut every = BTreeSet::new();
    for doc in untrusted() {
        made::sites_in(&doc.schema, &format!("{}#", doc.id), &refs, &mut BTreeSet::new(), &mut every);
        for (value, sites, choice) in values_of(&doc, &refs, &mut missing) {
            if let Err(e) = (doc.read)(&value) {
                problems.push(format!("{}: a value made from its schema ({choice}) doesn't read: {e}\n    {value}", doc.id));
                continue;
            }
            for site in &sites {
                let Some(bad) = pushed(site) else {
                    problems.push(format!("{}: can't push {} at {:?}", doc.id, site.ptr, site.at));
                    continue;
                };
                if (doc.read)(&with(&value, &site.at, bad.clone())).is_ok() {
                    let shown: String = bad.to_string().chars().take(80).collect();
                    problems.push(format!("{}: {} isn't held: {shown} at {:?} reads", doc.id, site.ptr, site.at));
                }
                checked.insert(site.ptr.clone());
            }
        }
    }
    assert!(missing.is_empty(), "no sample for the patterns:\n  {}", missing.into_iter().collect::<Vec<_>>().join("\n  "));
    assert!(problems.is_empty(), "{} problems:\n  {}", problems.len(), problems.join("\n  "));
    // Every bound in the schemas was pushed. (None of these documents
    // bounds a record's keys, which the made values don't push: one that
    // did would show here.)
    let unchecked: Vec<_> = every.difference(&checked).collect();
    assert!(unchecked.is_empty(), "bounds no made value met:\n  {}", unchecked.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n  "));
    for keyword in made::KEYWORDS {
        assert!(checked.iter().any(|p| p.ends_with(&format!("/{keyword}"))), "no {keyword} was checked");
    }
}

#[test]
fn every_pattern_in_the_schemas_compiles_and_its_sample_matches() {
    let mut patterns = BTreeSet::new();
    fn walk(v: &serde_json::Value, out: &mut BTreeSet<String>) {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(p) = m.get("pattern").and_then(serde_json::Value::as_str) {
                    out.insert(p.to_string());
                }
                m.values().for_each(|v| walk(v, out));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    for v in by_uri().values() {
        walk(v, &mut patterns);
    }
    for p in patterns {
        let pattern = xenonet_model::schema::Pattern::new(Box::leak(p.clone().into_boxed_str()));
        assert!(pattern.compiles(), "/{p}/ doesn't compile");
        if let Some(s) = made::sample(&p) {
            assert!(pattern.test(s), "{s:?} doesn't match /{p}/");
            assert!(!pattern.test(&format!("!{s}")), "!{s:?} matches /{p}/");
        }
    }
}

// The network's known answers (test/fixtures/net, which crates/xenonet-net
// writes and checks) are real frames, a service request and roster
// entries: they read. (Its message body is a sketch, not a whole
// network/message/v2.)
#[test]
fn the_networks_known_answers_read_as_the_model_types() {
    use xenonet_model::network;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/net/known-answers.json");
    let k: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let read = |what: &str, r: Result<(), String>| assert!(r.is_ok(), "{what}: {}", r.unwrap_err());
    fn of<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Result<(), String> {
        serde_json::from_value::<T>(v.clone()).map(|_| ()).map_err(|e| e.to_string())
    }
    for f in ["frame", "svcFrame", "gatewayFrame"] {
        read(f, of::<network::Frame>(&k[f]["frame"]));
    }
    read("svcFrame.body", of::<network::Svc>(&k["svcFrame"]["body"]));
    let roster = k["roster"].as_array().unwrap();
    assert_ne!(roster.len(), 0, "the known answers have a roster");
    for (i, e) in roster.iter().enumerate() {
        read(&format!("roster.{i}"), of::<network::RosterEntry>(e));
    }
}
