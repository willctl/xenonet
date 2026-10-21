//! core/hosts/merge.js: other machines in this XenoNet's view, as pure
//! functions over what hosts sent and what this XenoNet has.
//!
//! Every id from a host becomes `@<host>:<its id>`. An id that already
//! starts with @ is dropped, with anything that points at it, so a host
//! can't pass on another host's nodes or pose as this machine; the host's
//! core becomes its host node, and everything else it sent hangs from that
//! node in one tree. Nodes are checked one at a time against graph/v1 and
//! dropped when they don't fit, and a host has at most 5,000 on the map.
//!
//! Nothing a host sent is passed on as it came: what has a schema is
//! copied with only the fields it declares (`Schemas::clean`, zod's check
//! and declared.rs), and the rest is rebuilt field by field. Actions that
//! would open something on this machine are cut to Preview and Stats.
//! Times move by the host's clock offset, and activity is worked out again
//! by this machine's clock.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use xenonet_app::jstext::{locale_compare, slice16, utf16_len};
use xenonet_app::jsv::truthy;
use xenonet_base::text::{js_number, js_string};

pub const MAX_NODES: usize = 5000;
const MAX_ID: usize = 2048;
// The most of anything else one host may put on the map.
const MAX_LIVE: usize = 1000;
const MAX_PAIRS: usize = 200;
const MAX_RELATIONS: usize = 4 * MAX_NODES;
const MAX_IDENTITIES: usize = 500;
const MAX_ENGINE: usize = 500;
const LINKS: [&str; 11] = ["contains", "nested", "satellite", "native", "zone", "session", "lineage", "lineage-cross", "wiki", "page", "resource"];
const RELATIONS: [&str; 2] = ["wikilink", "mentions"];
const NODE_TIMES: [&str; 6] = ["createdAt", "updatedAt", "lastActivity", "lastSessionAt", "modifiedAt", "updated"];
const BRIDGES_PER_REPO: usize = 8;

/// Which schema a value from a host is checked against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind<'a> {
    /// graph/v1's node of this kind.
    Node(&'a str),
    Identity,
    LiveSession,
    LiveActivity,
    EngineAgent,
    EngineMessage,
}

/// What the merge asks of the JS around it: the schemas (zod's), and
/// graph.js's activity.
pub trait Schemas {
    /// The value checked against the schema, and copied with only what it
    /// declares; None when it doesn't pass, or there's no such schema.
    fn clean(&mut self, kind: Kind<'_>, value: &Value) -> Option<Value>;
    /// activityOf(updatedAt, now, windows).
    fn activity(&mut self, updated_at: &Value, now: f64, windows: &Value) -> Value;
}

fn is_object(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Object(_)))
}

fn finite(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn num(v: Option<&Value>) -> f64 {
    finite(v).unwrap_or(0.0)
}

fn n(x: f64) -> Value {
    xenonet_base::json::number(x)
}

fn str_of(v: Option<&Value>, max: usize) -> Value {
    match v {
        Some(Value::String(s)) => Value::String(slice16(s, max)),
        _ => Value::Null,
    }
}

fn list(v: Option<&Value>, max: usize) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.iter().take(max).cloned().collect(),
        _ => Vec::new(),
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(String::from)
}

/// A host's id as this XenoNet writes it, or None for one that can't be the
/// host's own: not a string, too long, or already qualified.
pub fn remote_id(host: &str, id: Option<&Value>) -> Option<String> {
    let id = id?.as_str()?;
    if id.is_empty() || utf16_len(id) > MAX_ID || id.starts_with('@') {
        return None;
    }
    xenonet_base::ids::qualify(id, Some(host), None).ok()
}

/// An identity's id from a host: account ids are global, so they stay as
/// they are, except openai:api-key, which is that machine's own key.
pub fn identity_id(host: &str, id: Option<&Value>) -> Option<String> {
    let id = id?.as_str()?;
    if id.is_empty() || utf16_len(id) > 512 || id.starts_with('@') {
        return None;
    }
    if id == "openai:api-key" {
        return xenonet_base::ids::qualify(id, Some(host), None).ok();
    }
    Some(id.to_string())
}

/// A node a host sent, as this XenoNet keeps it: checked against graph/v1
/// for its kind and copied with only what that declares; None for a node
/// that doesn't fit.
pub fn clean_node(schemas: &mut dyn Schemas, node: &Value) -> Option<Value> {
    let kind = node.get("kind").and_then(Value::as_str)?;
    if !node.is_object() {
        return None;
    }
    schemas.clean(Kind::Node(kind), node)
}

/// clean_node remembered for one host, by each node's text: a host sends
/// its whole map with each change, and only a few of its nodes differ.
#[derive(Default)]
pub struct CleanOnce {
    known: HashMap<String, (String, Option<Value>)>,
    next: HashMap<String, (String, Option<Value>)>,
}

impl CleanOnce {
    pub fn clean(&mut self, schemas: &mut dyn Schemas, node: &Value) -> Option<Value> {
        let text = xenonet_base::json::stringify(node, 0);
        let id = node.get("id").map(js_string).unwrap_or_default();
        let kept = match self.known.get(&id) {
            Some((t, k)) if *t == text => k.clone(),
            _ => clean_node(schemas, node),
        };
        self.next.insert(id, (text, kept.clone()));
        kept
    }

    /// What the last map had, kept for the next; the rest forgotten.
    pub fn done(&mut self) {
        self.known = std::mem::take(&mut self.next);
    }
}

// How a node is kept: once per host, or each time.
pub enum Cleaner<'a> {
    Once(&'a mut CleanOnce),
    Each,
}

impl Cleaner<'_> {
    fn clean(&mut self, schemas: &mut dyn Schemas, node: &Value) -> Option<Value> {
        match self {
            Cleaner::Once(c) => c.clean(schemas, node),
            Cleaner::Each => clean_node(schemas, node),
        }
    }
    fn done(&mut self) {
        if let Cleaner::Once(c) = self {
            c.done();
        }
    }
}

fn placed(mut node: Map<String, Value>, layout: &Value) -> Map<String, Value> {
    let p = node.get("id").and_then(Value::as_str).and_then(|id| layout.get(id));
    let xyz: Option<Vec<f64>> = p.and_then(|p| ["x", "y", "z"].iter().map(|k| finite(p.get(*k))).collect());
    match xyz {
        Some(v) => {
            node.insert("fx".into(), n(v[0]));
            node.insert("fy".into(), n(v[1]));
            node.insert("fz".into(), n(v[2]));
            node.insert("pinned".into(), json!(true));
        }
        None => {
            for k in ["fx", "fy", "fz"] {
                node.insert(k.into(), Value::Null);
            }
            node.insert("pinned".into(), json!(false));
        }
    }
    node
}

fn up_kind(kind: Option<&str>) -> Option<&'static str> {
    Some(match kind? {
        "session" => "session",
        "project" => "contains",
        "native" => "native",
        "zone" => "zone",
        "wiki" => "wiki",
        "page" => "page",
        "resource" => "resource",
        _ => return None,
    })
}

/// A host's view, qualified: its host node first, then its nodes, links,
/// relations and identities, and its stats for adding up.
pub struct Part {
    pub nodes: Vec<Value>,
    pub links: Vec<Value>,
    pub relations: Vec<Value>,
    pub identities: Vec<Value>,
    pub stats: Map<String, Value>,
    pub truncated: bool,
    pub dropped: usize,
}

struct Shift(f64);

impl Shift {
    fn of(host: &Value) -> Shift {
        Shift(finite(host.get("offsetMs")).unwrap_or(0.0))
    }
    fn at(&self, t: Option<&Value>) -> Value {
        finite(t).map_or(Value::Null, |t| n(t - self.0))
    }
}

pub fn qualify_graph(graph: &Value, host: &Value, layout: &Value, now: f64, windows: &Value, clean: &mut Cleaner<'_>, schemas: &mut dyn Schemas) -> Part {
    let h = s(host, "id").unwrap_or_default();
    let shift = Shift::of(host);
    let stale = host.get("state").and_then(Value::as_str) != Some("live");
    let sent = list(graph.get("nodes"), usize::MAX);
    let core_at = sent.iter().position(|x| is_object(Some(x)) && x.get("id") == Some(&json!("core")) && x.get("kind") == Some(&json!("core")));
    let core = core_at.map(|i| sent[i].clone());
    let mut dropped = 0;
    let mut candidates = Vec::new();
    for (i, x) in sent.iter().enumerate() {
        if Some(i) == core_at {
            continue;
        }
        let kept = if is_object(Some(x)) && x.get("id") != Some(&json!("core")) && remote_id(&h, x.get("id")).is_some() { clean.clean(schemas, x) } else { None };
        match kept {
            Some(k) => candidates.push(k),
            None => dropped += 1,
        }
    }
    clean.done();
    let mut truncated = false;
    if candidates.len() > MAX_NODES - 1 {
        truncated = true;
        let (mut sessions, places): (Vec<Value>, Vec<Value>) = candidates.into_iter().partition(|x| x.get("kind") == Some(&json!("session")));
        let at = |x: &Value| x.get("updatedAt").filter(|v| !v.is_null()).map_or(0.0, |v| js_number(Some(v)));
        xenonet_base::sort::sort_by(&mut sessions, |a, b| (at(b) - at(a)).partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal));
        candidates = places.into_iter().chain(sessions).take(MAX_NODES - 1).collect();
    }

    let mut nodes: Map<String, Value> = Map::new();
    for x in &candidates {
        let Some(id) = remote_id(&h, x.get("id")) else { continue };
        if nodes.contains_key(&id) {
            continue;
        }
        let mut out = x.as_object().cloned().unwrap_or_default();
        out.insert("id".into(), json!(id));
        out.insert("host".into(), json!(h));
        out.insert("stale".into(), json!(stale));
        for k in NODE_TIMES {
            if out.contains_key(k) {
                let t = shift.at(out.get(k));
                out.insert(k.into(), t);
            }
        }
        if let Some(Value::Object(repo)) = out.get("repo") {
            let mut r = repo.clone();
            r.insert("checkedAt".into(), shift.at(repo.get("checkedAt")));
            out.insert("repo".into(), Value::Object(r));
        }
        if let Some(Value::Array(writes)) = out.get("writes") {
            let w: Vec<Value> = writes
                .iter()
                .map(|w| {
                    let mut m = w.as_object().cloned().unwrap_or_default();
                    m.insert("ts".into(), shift.at(w.get("ts")));
                    Value::Object(m)
                })
                .collect();
            out.insert("writes".into(), Value::Array(w));
        }
        let kind = out.get("kind").and_then(Value::as_str).map(String::from);
        if kind.as_deref() == Some("session") {
            let parent = out.get("parentId").filter(|p| !p.is_null()).and_then(|p| remote_id(&h, Some(p)));
            out.insert("parentId".into(), parent.clone().map_or(Value::Null, Value::String));
            if parent.is_none() {
                out.insert("parentKind".into(), Value::Null);
            }
            let a = schemas.activity(out.get("updatedAt").unwrap_or(&Value::Null), now, windows);
            let activity = if truthy(out.get("archived")) && a == json!("active") { json!("recent") } else { a };
            out.insert("activity".into(), activity);
            if let Some(Value::Object(ran)) = out.get("ranAs") {
                let next = identity_id(&h, ran.get("id")).map_or(Value::Null, |id| {
                    let mut r = ran.clone();
                    r.insert("id".into(), json!(id));
                    Value::Object(r)
                });
                out.insert("ranAs".into(), next);
            }
            out.insert("actions".into(), json!([{ "id": "preview", "label": "Preview transcript", "client": true }, { "id": "stats", "label": "Stats", "client": true }]));
        } else {
            out.insert("actions".into(), json!([]));
        }
        if kind.as_deref() == Some("project") {
            if let Some(Value::Array(ids)) = out.get("identities") {
                let kept: Vec<Value> = ids
                    .iter()
                    .filter_map(|l| {
                        let mut m = l.as_object().cloned().unwrap_or_default();
                        let id = identity_id(&h, l.get("id"));
                        m.insert("id".into(), id.clone().map_or(Value::Null, Value::String));
                        id.map(|_| Value::Object(m))
                    })
                    .collect();
                out.insert("identities".into(), Value::Array(kept));
            }
        }
        nodes.insert(id, Value::Object(placed(out, layout)));
    }

    let host_id = format!("@{h}:core");
    let core_r = core.as_ref();
    let hello = host.get("hello");
    let hello_id = hello.and_then(|x| x.get("id"));
    let calls = hello_id.filter(|x| truthy(Some(x)) && **x != json!(h)).cloned().unwrap_or(Value::Null);
    let field = |v: Option<&Value>| v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
    let mut hn = Map::new();
    hn.insert("id".into(), json!(host_id));
    hn.insert("kind".into(), json!("host"));
    hn.insert("host".into(), json!(h));
    hn.insert("name".into(), json!(h));
    hn.insert("path".into(), str_of(core_r.and_then(|c| c.get("name")), 200));
    hn.insert("calls".into(), calls);
    if let Some(st) = host.get("state") {
        hn.insert("state".into(), st.clone());
    }
    if let Some(since) = host.get("since") {
        hn.insert("since".into(), since.clone());
    }
    hn.insert("lastHeard".into(), field(host.get("lastHeard")));
    hn.insert("offsetMs".into(), finite(host.get("offsetMs")).map_or(Value::Null, n));
    hn.insert("version".into(), field(hello.and_then(|x| x.get("version"))));
    hn.insert("platform".into(), field(hello.and_then(|x| x.get("platform"))));
    hn.insert("demo".into(), field(hello.and_then(|x| x.get("demo"))));
    hn.insert("error".into(), field(host.get("error").and_then(|e| e.get("message"))));
    hn.insert("cached".into(), json!(truthy(host.get("cached"))));
    hn.insert("truncated".into(), json!(truncated));
    hn.insert("stale".into(), json!(stale));
    let count = core_r.and_then(|c| c.get("sessionCount")).and_then(Value::as_f64).filter(|c| c.fract() == 0.0 && *c >= 0.0).unwrap_or(0.0);
    hn.insert("sessionCount".into(), n(count));
    hn.insert("activeCount".into(), json!(0));
    hn.insert("lastActivity".into(), shift.at(core_r.and_then(|c| c.get("lastActivity"))));
    hn.insert("lastSessionAt".into(), shift.at(core_r.and_then(|c| c.get("lastSessionAt"))));
    hn.insert("actions".into(), json!([]));
    let mut host_node = placed(hn, layout);

    // One tree under the host node: each node keeps the first parent the
    // host gave it, and a node whose parent didn't come, or that a loop of
    // links cut off, hangs from the host node itself.
    let mut parent_of: Map<String, Value> = Map::new();
    let mut cross = Vec::new();
    for l in list(graph.get("links"), usize::MAX) {
        if !is_object(Some(&l)) || !l.get("kind").and_then(Value::as_str).is_some_and(|k| LINKS.contains(&k)) {
            continue;
        }
        let source = if l.get("source") == Some(&json!("core")) { Some(host_id.clone()) } else { remote_id(&h, l.get("source")) };
        let target = remote_id(&h, l.get("target"));
        let (Some(source), Some(target)) = (source, target) else { continue };
        if !nodes.contains_key(&target) || !(source == host_id || nodes.contains_key(&source)) || source == target {
            continue;
        }
        let kind = l["kind"].clone();
        if kind == "lineage-cross" {
            if cross.len() < MAX_NODES {
                cross.push(json!({ "source": source, "target": target, "kind": kind }));
            }
        } else if !parent_of.contains_key(&target) {
            parent_of.insert(target.clone(), json!({ "source": source, "target": target, "kind": kind }));
        }
    }
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for l in parent_of.values() {
        children.entry(js_string(&l["source"])).or_default().push(js_string(&l["target"]));
    }
    let mut reached: HashSet<String> = HashSet::new();
    let reach = |from: &str, reached: &mut HashSet<String>, children: &HashMap<String, Vec<String>>| {
        let mut queue = VecDeque::from([from.to_string()]);
        reached.insert(from.to_string());
        while let Some(x) = queue.pop_front() {
            for c in children.get(&x).into_iter().flatten() {
                if reached.insert(c.clone()) {
                    queue.push_back(c.clone());
                }
            }
        }
    };
    reach(&host_id, &mut reached, &children);
    let ids: Vec<String> = nodes.keys().cloned().collect();
    for id in ids {
        if reached.contains(&id) {
            continue;
        }
        let kind = up_kind(nodes[&id].get("kind").and_then(Value::as_str));
        let mut link = Map::new();
        link.insert("source".into(), json!(host_id));
        link.insert("target".into(), json!(id));
        if let Some(k) = kind {
            link.insert("kind".into(), json!(k));
        }
        parent_of.insert(id.clone(), Value::Object(link));
        children.entry(host_id.clone()).or_default().push(id.clone());
        reach(&id, &mut reached, &children);
    }
    let mut links = vec![json!({ "source": "core", "target": host_id, "kind": "host" })];
    links.extend(parent_of.values().cloned());
    links.extend(cross);

    let mut relations = Vec::new();
    for r in list(graph.get("relations"), MAX_RELATIONS) {
        if !is_object(Some(&r)) || !r.get("kind").and_then(Value::as_str).is_some_and(|k| RELATIONS.contains(&k)) {
            continue;
        }
        if let (Some(a), Some(b)) = (remote_id(&h, r.get("a")), remote_id(&h, r.get("b"))) {
            if nodes.contains_key(&a) && nodes.contains_key(&b) {
                relations.push(json!({ "a": a, "b": b, "kind": r["kind"] }));
            }
        }
    }

    // What's active, by this machine's clock.
    for (_, x) in &mut nodes {
        if x.get("kind") != Some(&json!("session")) {
            x["activeCount"] = json!(0);
        }
    }
    let up: HashMap<String, String> = parent_of.values().map(|l| (js_string(&l["target"]), js_string(&l["source"]))).collect();
    let actives: Vec<String> =
        nodes.iter().filter(|(_, x)| x.get("kind") == Some(&json!("session")) && x.get("activity") == Some(&json!("active"))).map(|(id, _)| id.clone()).collect();
    for sid in actives {
        let mut seen: HashSet<String> = HashSet::new();
        let mut cur = up.get(&sid).cloned();
        while let Some(c) = cur.filter(|c| !seen.contains(c)) {
            seen.insert(c.clone());
            if c == host_id {
                let a = num(host_node.get("activeCount"));
                host_node.insert("activeCount".into(), n(a + 1.0));
            } else if let Some(x) = nodes.get_mut(&c) {
                if x.get("kind") != Some(&json!("session")) {
                    let a = num(x.get("activeCount"));
                    x["activeCount"] = n(a + 1.0);
                }
            }
            cur = up.get(&c).cloned();
        }
    }
    let project_ms = finite(windows.get("projectActiveMs")).unwrap_or(3600e3);
    for (_, x) in &mut nodes {
        if matches!(x.get("kind").and_then(Value::as_str), Some("project" | "native" | "zone")) {
            let last = x.get("lastSessionAt").filter(|v| !v.is_null()).map(|v| js_number(Some(v)));
            x["active"] = json!(last.is_some_and(|l| now - l <= project_ms));
        }
    }

    let mut identities = Vec::new();
    for i in list(graph.get("identities"), MAX_IDENTITIES) {
        let Some(id) = identity_id(&h, i.get("id")) else { continue };
        if let Some(Value::Object(mut kept)) = schemas.clean(Kind::Identity, &i) {
            kept.insert("id".into(), json!(id));
            identities.push(Value::Object(kept));
        }
    }

    let all: Vec<Value> = nodes.into_iter().map(|(_, v)| v).collect();
    let kind_is = |x: &Value, k: &str| x.get("kind").and_then(Value::as_str) == Some(k);
    let sessions: Vec<&Value> = all.iter().filter(|x| kind_is(x, "session")).collect();
    let projects: Vec<&Value> = all.iter().filter(|x| kind_is(x, "project")).collect();
    let mut stats = Map::new();
    stats.insert("wikiPages".into(), json!(all.iter().filter(|x| kind_is(x, "page")).count()));
    stats.insert("dirtyRepos".into(), json!(projects.iter().filter(|x| js_number(x.get("repo").and_then(|r| r.get("dirty"))) > 0.0).count()));
    stats.insert("projects".into(), json!(projects.iter().filter(|x| !truthy(x.get("group"))).count()));
    stats.insert("groups".into(), json!(projects.iter().filter(|x| truthy(x.get("group"))).count()));
    stats.insert("sessions".into(), json!(sessions.len()));
    stats.insert("claude".into(), json!(sessions.iter().filter(|x| x.get("app") == Some(&json!("claude"))).count()));
    stats.insert("codex".into(), json!(sessions.iter().filter(|x| x.get("app") == Some(&json!("codex"))).count()));
    stats.insert("active".into(), json!(sessions.iter().filter(|x| x.get("activity") == Some(&json!("active"))).count()));
    stats.insert("archived".into(), json!(sessions.iter().filter(|x| truthy(x.get("archived"))).count()));
    let mut out_nodes = vec![Value::Object(host_node)];
    out_nodes.extend(all);
    Part { nodes: out_nodes, links, relations, identities, stats, truncated, dropped }
}

/// Bridges: a same-repo relation between checkouts of one repo on
/// different machines, by repo key; one repo gets a few at most.
pub fn same_repo(nodes: &[Value], cap: usize) -> Vec<Value> {
    // By repo key, then by host: the ids of the repo's checkouts there.
    type ByHost = Vec<(String, Vec<Value>)>;
    let mut repos: Vec<(String, ByHost)> = Vec::new();
    for x in nodes {
        if x.get("kind") != Some(&json!("project")) || truthy(x.get("group")) || truthy(x.get("satelliteOf")) {
            continue;
        }
        let Some(key) = x.get("repoKey").and_then(Value::as_str).filter(|k| !k.is_empty()) else { continue };
        let host = x.get("host").filter(|h| !h.is_null()).map_or_else(String::new, js_string);
        let at = match repos.iter().position(|(k, _)| k == key) {
            Some(i) => i,
            None => {
                repos.push((key.to_string(), Vec::new()));
                repos.len() - 1
            }
        };
        let by_host = &mut repos[at].1;
        match by_host.iter_mut().find(|(hh, _)| *hh == host) {
            Some((_, ids)) => ids.push(x.get("id").cloned().unwrap_or(Value::Null)),
            None => by_host.push((host, vec![x.get("id").cloned().unwrap_or(Value::Null)])),
        }
    }
    let mut out = Vec::new();
    for (_, groups) in repos {
        let mut made = 0;
        for i in 0..groups.len() {
            for j in i + 1..groups.len() {
                for a in &groups[i].1 {
                    for b in &groups[j].1 {
                        if made < cap {
                            out.push(json!({ "a": a, "b": b, "kind": "same-repo" }));
                        }
                        made += 1;
                    }
                }
            }
        }
    }
    out
}

fn own<'a>(o: &'a Value, k: Option<&Value>) -> Option<&'a Value> {
    k.and_then(Value::as_str).and_then(|k| o.get(k))
}

/// Identities from every machine, one per id: counts add up, a handle
/// comes from whichever machine knows it, and `hosts` and `signedInOn` say
/// where (null for this one). Your names and hidden ones apply to every
/// machine's, and a name given on a host fills in where you gave none.
pub fn merge_identities(local: &[Value], remotes: &[(String, Vec<Value>)], names: &Value, hidden: &Value) -> Vec<Value> {
    let mut by_id: Map<String, Value> = Map::new();
    for i in local {
        let mut m = i.as_object().cloned().unwrap_or_default();
        m.insert("sources".into(), Value::Array(list(i.get("sources"), usize::MAX)));
        m.insert("hosts".into(), json!([null]));
        m.insert("signedInOn".into(), if truthy(i.get("signedIn")) { json!([null]) } else { json!([]) });
        by_id.insert(i.get("id").map(js_string).unwrap_or_default(), Value::Object(m));
    }
    for (host, identities) in remotes {
        for r in identities {
            let sources: Vec<Value> = list(r.get("sources"), usize::MAX).into_iter().filter_map(|x| x.as_str().map(|t| json!(format!("{host}: {t}")))).collect();
            let id = r.get("id").map(js_string).unwrap_or_default();
            let Some(cur) = by_id.get_mut(&id) else {
                let mut m = r.as_object().cloned().unwrap_or_default();
                m.insert("sources".into(), Value::Array(sources));
                let name = own(names, r.get("id")).filter(|x| !x.is_null()).or_else(|| r.get("name").filter(|x| !x.is_null())).cloned().unwrap_or(Value::Null);
                m.insert("name".into(), name);
                m.insert("hidden".into(), json!(truthy(own(hidden, r.get("id")))));
                m.insert("hosts".into(), json!([host]));
                m.insert("signedInOn".into(), if truthy(r.get("signedIn")) { json!([host]) } else { json!([]) });
                by_id.insert(id, Value::Object(m));
                continue;
            };
            cur["sessions"] = n(num(cur.get("sessions")) + num(r.get("sessions")));
            cur["projects"] = n(num(cur.get("projects")) + num(r.get("projects")));
            if cur.get("handle").is_none_or(Value::is_null) {
                cur["handle"] = r.get("handle").filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
            }
            let mut detail = r.get("detail").and_then(Value::as_object).cloned().unwrap_or_default();
            if let Some(Value::Object(mine)) = cur.get("detail") {
                for (k, v) in mine {
                    detail.insert(k.clone(), v.clone());
                }
            }
            cur["detail"] = Value::Object(detail);
            if cur.get("name").is_none_or(Value::is_null) {
                cur["name"] = own(names, r.get("id")).filter(|x| !x.is_null()).or_else(|| r.get("name").filter(|x| !x.is_null())).cloned().unwrap_or(Value::Null);
            }
            cur["active"] = json!(truthy(cur.get("active")) || truthy(r.get("active")));
            if truthy(r.get("signedIn")) {
                cur["signedIn"] = json!(true);
                if let Some(Value::Array(on)) = cur.get_mut("signedInOn") {
                    on.push(json!(host));
                }
            }
            if let Some(f) = finite(r.get("firstSeen")) {
                let had = finite(cur.get("firstSeen")).unwrap_or(f);
                cur["firstSeen"] = n(had.min(f));
            }
            if let Some(t) = finite(r.get("seenAt")) {
                let had = finite(cur.get("seenAt")).unwrap_or(0.0);
                cur["seenAt"] = n(had.max(t));
            }
            if let Some(Value::Array(hs)) = cur.get_mut("hosts") {
                hs.push(json!(host));
            }
            if let Some(Value::Array(src)) = cur.get_mut("sources") {
                src.extend(sources);
            }
        }
    }
    let mut out: Vec<Value> = by_id.into_iter().map(|(_, v)| v).collect();
    xenonet_base::sort::sort_by(&mut out, |a, b| {
        let d = js_number(b.get("sessions")) - js_number(a.get("sessions"));
        let d = if d == 0.0 || d.is_nan() { js_number(b.get("projects")) - js_number(a.get("projects")) } else { d };
        if d == 0.0 || d.is_nan() {
            locale_compare(&a.get("id").map(js_string).unwrap_or_default(), &b.get("id").map(js_string).unwrap_or_default())
        } else if d < 0.0 {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });
    out
}

/// How a host stands, for graph.hosts and /api/hosts.
pub fn host_summary(host: &Value, part: Option<&Part>) -> Value {
    let hello = host.get("hello");
    let field = |v: Option<&Value>| v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
    let mut m = Map::new();
    for k in ["id", "state", "since"] {
        if let Some(v) = host.get(k) {
            m.insert(k.into(), v.clone());
        }
    }
    m.insert("lastHeard".into(), field(host.get("lastHeard")));
    m.insert("offsetMs".into(), finite(host.get("offsetMs")).map_or(Value::Null, n));
    m.insert("version".into(), field(hello.and_then(|x| x.get("version"))));
    m.insert("platform".into(), field(hello.and_then(|x| x.get("platform"))));
    m.insert("demo".into(), field(hello.and_then(|x| x.get("demo"))));
    m.insert("error".into(), field(host.get("error").and_then(|e| e.get("message"))));
    m.insert("cached".into(), json!(truthy(host.get("cached"))));
    m.insert("truncated".into(), json!(part.is_some_and(|p| p.truncated)));
    m.insert("nodes".into(), json!(part.map_or(0, |p| p.nodes.len() as i64 - 1)));
    m.insert("sessions".into(), part.and_then(|p| p.stats.get("sessions").cloned()).unwrap_or(json!(0)));
    Value::Object(m)
}

/// A host as the merge takes it: how it stands, its last snapshot, and
/// how its nodes are cleaned.
pub struct HostIn<'a> {
    pub info: Value,
    pub snapshot: Value,
    pub clean: Option<&'a mut CleanOnce>,
}

/// repoKey(origin, aliases) from schema/ids.js: a remote's host (through
/// the SSH aliases) and its repo's path, normalized.
pub fn repo_key(origin: Option<&Value>, aliases: &HashMap<String, String>) -> Option<String> {
    let url = origin.filter(|o| !o.is_null()).map_or_else(String::new, js_string);
    let (host, repo) = xenonet_base::ids::remote_parts(&url)?;
    let h = aliases.get(&host).cloned().unwrap_or(host);
    Some(xenonet_base::ids::normalize_remote(&format!("{h}/{repo}")))
}

/// This XenoNet's map with every host's in it. With no hosts, the map is
/// this XenoNet's own.
#[expect(clippy::too_many_arguments, reason = "the JS's mergeGraph and its options, which xenonet-node passes one by one")]
pub fn merge_graph(
    local: &Value,
    hosts: &mut [HostIn<'_>],
    layout: &Value,
    now: f64,
    windows: &Value,
    self_id: &Value,
    names: &Value,
    hidden: &Value,
    aliases: &HashMap<String, String>,
    schemas: &mut dyn Schemas,
) -> Value {
    if hosts.is_empty() {
        return local.clone();
    }
    let empty = json!({ "nodes": [], "links": [] });
    let mut parts = Vec::new();
    for h in hosts.iter_mut() {
        let graph = h.snapshot.get("graph").filter(|g| !g.is_null()).cloned().unwrap_or_else(|| empty.clone());
        let mut cleaner = match h.clean.as_deref_mut() {
            Some(c) => Cleaner::Once(c),
            None => Cleaner::Each,
        };
        parts.push(qualify_graph(&graph, &h.info, layout, now, windows, &mut cleaner, schemas));
    }
    let mut nodes: Vec<Value> = list(local.get("nodes"), usize::MAX)
        .into_iter()
        .map(|x| {
            if x.get("kind") != Some(&json!("project")) {
                return x;
            }
            match repo_key(x.get("git").and_then(|g| g.get("origin")), aliases) {
                Some(key) => {
                    let mut m = x.as_object().cloned().unwrap_or_default();
                    m.insert("repoKey".into(), json!(key));
                    Value::Object(m)
                }
                None => x,
            }
        })
        .collect();
    for p in &parts {
        nodes.extend(p.nodes.iter().cloned());
    }
    let mut stats = local.get("stats").and_then(Value::as_object).cloned().unwrap_or_default();
    for p in &parts {
        for (k, v) in &p.stats {
            let sum = num(stats.get(k)) + num(Some(v));
            stats.insert(k.clone(), n(sum));
        }
    }
    let mut out = local.as_object().cloned().unwrap_or_default();
    let mut links = list(local.get("links"), usize::MAX);
    let mut relations = list(local.get("relations"), usize::MAX);
    for p in &parts {
        links.extend(p.links.iter().cloned());
    }
    for p in &parts {
        relations.extend(p.relations.iter().cloned());
    }
    relations.extend(same_repo(&nodes, BRIDGES_PER_REPO));
    let remotes: Vec<(String, Vec<Value>)> = hosts.iter().zip(&parts).map(|(h, p)| (s(&h.info, "id").unwrap_or_default(), p.identities.clone())).collect();
    let identities = merge_identities(&list(local.get("identities"), usize::MAX), &remotes, names, hidden);
    out.insert("nodes".into(), Value::Array(nodes));
    out.insert("links".into(), Value::Array(links));
    out.insert("relations".into(), Value::Array(relations));
    out.insert("stats".into(), Value::Object(stats));
    out.insert("identities".into(), Value::Array(identities));
    out.insert("hosts".into(), Value::Array(hosts.iter().zip(&parts).map(|(h, p)| host_summary(&h.info, Some(p))).collect()));
    out.insert("self".into(), self_id.clone());
    Value::Object(out)
}

// ---------- live state ----------

const LIVE_TIMES: [&str; 4] = ["turnStartedAt", "lastTurnEndAt", "interruptedAt", "lastEventAt"];

fn strings(v: Option<&Value>) -> Value {
    Value::Array(match v {
        Some(Value::Array(a)) => a.iter().filter(|x| x.is_string()).take(50).cloned().collect(),
        _ => Vec::new(),
    })
}

// A host's collision, or its sessions sharing files, rebuilt from the
// fields live/snapshot/v1 gives them and nothing else; None for a
// collision with no time it happened.
fn pair_of(c: &Value, h: &str, shift: &Shift, shared: bool) -> Option<Value> {
    if !c.is_object() {
        return None;
    }
    let a = remote_id(h, c.get("a"))?;
    let b = remote_id(h, c.get("b"))?;
    if shared {
        return Some(json!({ "a": a, "b": b, "kind": "file", "files": strings(c.get("files")) }));
    }
    let ts = shift.at(c.get("ts"));
    if ts.is_null() {
        return None;
    }
    let branch = |i: usize| str_of(c.get("branches").and_then(|b| b.get(i)), 200);
    let title = |i: usize, or: &str| match str_of(c.get("titles").and_then(|t| t.get(i)), 400) {
        Value::Null => json!(or),
        t => t,
    };
    Some(
        json!({ "a": a, "b": b, "files": strings(c.get("files")), "paths": strings(c.get("paths")), "ts": ts, "branches": [branch(0), branch(1)], "sameBranch": c.get("sameBranch") == Some(&json!(true)), "titles": [title(0, &a), title(1, &b)] }),
    )
}

// A host's plan limit reading, rebuilt from the fields that fit, with its
// times moved; None for one with no time it was read.
fn reading_of(app: &str, r: &Value, shift: &Shift) -> Option<Value> {
    if !r.is_object() {
        return None;
    }
    let nn = |v: Option<&Value>| finite(v).map_or(Value::Null, n);
    let observed = shift.at(r.get("observedAt"));
    if app == "claude" && r.get("stale") == Some(&json!(true)) {
        return Some(json!({ "fiveHour": null, "sevenDay": null, "observedAt": observed, "stale": true, "reason": str_of(r.get("reason"), 200) }));
    }
    if observed.is_null() {
        return None;
    }
    if app == "claude" {
        let resets = r.get("resets");
        let source = if r.get("source") == Some(&json!("claude-desktop")) { "claude-desktop" } else { "anthropic" };
        return Some(
            json!({ "fiveHour": nn(r.get("fiveHour")), "sevenDay": nn(r.get("sevenDay")), "resets": { "fiveHour": shift.at(resets.and_then(|x| x.get("fiveHour"))), "sevenDay": shift.at(resets.and_then(|x| x.get("sevenDay"))) }, "observedAt": observed, "source": source }),
        );
    }
    let window = |w: Option<&Value>| match w {
        Some(w @ Value::Object(_)) => json!({ "pct": nn(w.get("pct")), "windowMinutes": nn(w.get("windowMinutes")), "resetsAt": shift.at(w.get("resetsAt")) }),
        _ => Value::Null,
    };
    Some(json!({ "primary": window(r.get("primary")), "secondary": window(r.get("secondary")), "observedAt": observed }))
}

/// One host's live state, qualified.
pub struct LiveIn {
    pub sessions: Vec<Value>,
    pub collisions: Vec<Value>,
    pub collab: Vec<Value>,
    pub series: Value,
    pub at: Value,
    pub tpm: f64,
    pub today: f64,
    pub total24h: f64,
    pub readings: Vec<Value>,
}

fn shift_each(list: Option<&Value>, fields: &[&str], shift: &Shift) -> Value {
    match list {
        Some(Value::Array(a)) => Value::Array(
            a.iter()
                .map(|x| {
                    let mut m = x.as_object().cloned().unwrap_or_default();
                    for f in fields {
                        m.insert((*f).into(), shift.at(x.get(*f)));
                    }
                    Value::Object(m)
                })
                .collect(),
        ),
        other => other.cloned().unwrap_or(Value::Null),
    }
}

/// One host's live state, qualified. A host that isn't live shows its
/// sessions idle, its rates at nothing and a question that was waiting as
/// of when it was last heard; its collisions are dropped.
pub fn qualify_live(live: &Value, host: &Value, schemas: &mut dyn Schemas) -> LiveIn {
    let h = s(host, "id").unwrap_or_default();
    let shift = Shift::of(host);
    let stale = host.get("state").and_then(Value::as_str) != Some("live");
    let mut sessions = Vec::new();
    for x in list(live.get("sessions"), MAX_LIVE) {
        let Some(id) = remote_id(&h, x.get("id")) else { continue };
        let Some(Value::Object(mut out)) = schemas.clean(Kind::LiveSession, &x) else { continue };
        out.insert("id".into(), json!(id));
        out.insert("host".into(), json!(h));
        out.insert("stale".into(), json!(stale));
        for k in LIVE_TIMES {
            if out.contains_key(k) {
                let t = shift.at(out.get(k));
                out.insert(k.into(), t);
            }
        }
        if let Some(nv) = out.get("needsYou").filter(|v| truthy(Some(v))).cloned() {
            let mut m = nv.as_object().cloned().unwrap_or_default();
            m.insert("since".into(), shift.at(nv.get("since")));
            if stale {
                m.insert("asOf".into(), host.get("lastHeard").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null));
            }
            out.insert("needsYou".into(), Value::Object(m));
        }
        if let Some(t) = out.get("lastTool").filter(|v| truthy(Some(v))).cloned() {
            let mut m = t.as_object().cloned().unwrap_or_default();
            m.insert("ts".into(), shift.at(t.get("ts")));
            out.insert("lastTool".into(), Value::Object(m));
        }
        let files = shift_each(out.get("files"), &["ts"], &shift);
        out.insert("files".into(), files);
        let mcp = shift_each(out.get("mcp"), &["ts"], &shift);
        out.insert("mcp".into(), mcp);
        let subagents = shift_each(out.get("subagents"), &["startedAt", "lastAt"], &shift);
        out.insert("subagents".into(), subagents);
        if stale {
            out.insert("status".into(), json!("idle"));
            out.insert("tpm".into(), json!(0));
            out.insert("stuck".into(), Value::Null);
            out.insert("turnStartedAt".into(), Value::Null);
        }
        sessions.push(Value::Object(out));
    }
    let pairs = |v: Option<&Value>, shared: bool| -> Vec<Value> {
        if stale {
            return Vec::new();
        }
        list(v, MAX_PAIRS).iter().filter_map(|c| pair_of(c, &h, &shift, shared)).collect()
    };
    let mut readings = Vec::new();
    for app in limited() {
        let r = live.get("limits").and_then(|l| l.get(app));
        if let Some(Value::Object(mut reading)) = r.and_then(|r| reading_of(app, r, &shift)) {
            let identity = identity_id(&h, r.and_then(|r| r.get("identity"))).map_or(Value::Null, Value::String);
            reading.insert("identity".into(), identity);
            readings.push(json!({ "app": app, "host": h, "reading": reading }));
        }
    }
    LiveIn {
        sessions,
        collisions: pairs(live.get("collisions"), false),
        collab: pairs(live.get("collab"), true),
        series: if stale || !is_object(live.get("series")) { Value::Null } else { live["series"].clone() },
        at: shift.at(live.get("at")),
        tpm: if stale { 0.0 } else { num(live.get("tpm")) },
        today: num(live.get("today")),
        total24h: num(live.get("total24h")),
        readings,
    }
}

// The apps with plan limits a host reads (Claude Code's and Codex's), and
// the ones whose token rates it keeps a series of (the tailed ones).
fn limited() -> impl Iterator<Item = &'static str> {
    xenonet_base::apps::apps().iter().filter(|a| a.limits.is_some()).map(|a| a.id.as_str())
}

fn tailed() -> impl Iterator<Item = &'static str> {
    xenonet_base::apps::with("lines").map(|a| a.id.as_str())
}

// Adds a host's per-second series into this machine's, lined up by time:
// `lag` is how many seconds the host's ends before this one's.
fn add_series(into: &mut Map<String, Value>, from: &Value, lag: f64) {
    for app in tailed() {
        let b: Vec<Value> = match from.get(app) {
            Some(Value::Array(b)) => b.clone(),
            _ => Vec::new(),
        };
        let Some(Value::Array(a)) = into.get_mut(app) else { continue };
        let len = a.len() as f64;
        for (i, x) in b.iter().enumerate() {
            let j = len - b.len() as f64 + i as f64 - lag;
            if j >= 0.0 && j < len {
                if let Some(v) = finite(Some(x)) {
                    let at = j as usize;
                    let sum = js_number(Some(&a[at])) + v;
                    a[at] = n(sum);
                }
            }
        }
    }
}

/// Plan limits are an account's, not a machine's: a newer reading of this
/// machine's account from another one takes its place, and accounts signed
/// in only elsewhere are kept as their own, the newest of each.
pub fn merge_limits(local: &Value, readings: &[Value]) -> Value {
    merge_limits_with(local, readings).0
}

/// merge_limits, with the apps whose reading another machine's took the
/// place of (the rest are this machine's, as they were).
pub fn merge_limits_with(local: &Value, readings: &[Value]) -> (Value, Vec<String>) {
    let mut replaced: Vec<String> = Vec::new();
    let mut limits = local.as_object().cloned().unwrap_or_default();
    let mut others: Map<String, Value> = Map::new();
    let at = |r: &Value| finite(r.get("observedAt")).unwrap_or(f64::NEG_INFINITY);
    for x in readings {
        let app = x.get("app").map(js_string).unwrap_or_default();
        let host = x.get("host").cloned().unwrap_or(Value::Null);
        let reading = x.get("reading").cloned().unwrap_or(Value::Null);
        let identity = reading.get("identity").filter(|v| truthy(Some(v))).cloned();
        if let (Some(mine), Some(id)) = (limits.get(&app).filter(|m| truthy(Some(m))).cloned(), identity.as_ref()) {
            if mine.get("identity") == Some(id) {
                if !truthy(reading.get("stale")) && at(&reading) > at(&mine) {
                    let mut r = reading.as_object().cloned().unwrap_or_default();
                    r.insert("host".into(), host.clone());
                    limits.insert(app.clone(), Value::Object(r));
                    if !replaced.contains(&app) {
                        replaced.push(app.clone());
                    }
                }
                continue;
            }
        }
        // As the JS's ??: an identity that's there but empty still names one.
        let named = reading.get("identity").filter(|v| !v.is_null()).cloned();
        let who = named.as_ref().map_or_else(|| format!("@{}", js_string(&host)), js_string);
        let key = format!("{app} {who}");
        let newer = match others.get(&key) {
            None => true,
            Some(prev) => at(&reading) > at(&prev["reading"]),
        };
        if newer {
            others.insert(key, json!({ "app": app, "identity": named.unwrap_or(Value::Null), "host": host, "reading": reading }));
        }
    }
    limits.insert("others".into(), Value::Array(others.into_iter().map(|(_, v)| v).collect()));
    (Value::Object(limits), replaced)
}

/// This XenoNet's live state with every host's in it.
pub fn merge_live(local: &Value, hosts: &[(Value, Value)], schemas: &mut dyn Schemas) -> Value {
    merge_live_with(local, hosts, schemas).0
}

/// merge_live, with the apps whose limits reading another machine's took
/// the place of.
pub fn merge_live_with(local: &Value, hosts: &[(Value, Value)], schemas: &mut dyn Schemas) -> (Value, Vec<String>) {
    if hosts.is_empty() {
        return (local.clone(), Vec::new());
    }
    let mut out = local.as_object().cloned().unwrap_or_default();
    let mut sessions = list(local.get("sessions"), usize::MAX);
    let mut collisions = list(local.get("collisions"), usize::MAX);
    let mut collab = list(local.get("collab"), usize::MAX);
    let mut series = Map::new();
    for app in tailed() {
        series.insert(app.into(), Value::Array(list(local.get("series").and_then(|x| x.get(app)), usize::MAX)));
    }
    let mut readings = Vec::new();
    let (mut tpm, mut today, mut total) = (num(local.get("tpm")), num(local.get("today")), num(local.get("total24h")));
    let mut touched = false;
    for (info, snapshot) in hosts {
        let Some(live) = snapshot.get("live").filter(|l| truthy(Some(l))) else { continue };
        touched = true;
        let q = qualify_live(live, info, schemas);
        sessions.extend(q.sessions);
        collisions.extend(q.collisions);
        collab.extend(q.collab);
        tpm += q.tpm;
        today += q.today;
        total += q.total24h;
        if let (false, Some(qa), Some(la)) = (q.series.is_null(), finite(Some(&q.at)), finite(local.get("at"))) {
            add_series(&mut series, &q.series, xenonet_base::units::js_round((la - qa) / 1000.0));
        }
        readings.extend(q.readings);
    }
    out.insert("sessions".into(), Value::Array(sessions));
    out.insert("collisions".into(), Value::Array(collisions));
    out.insert("collab".into(), Value::Array(collab));
    out.insert("series".into(), Value::Object(series));
    if touched {
        out.insert("tpm".into(), n(tpm));
        out.insert("today".into(), n(today));
        out.insert("total24h".into(), n(total));
    }
    let (limits, replaced) = merge_limits_with(local.get("limits").unwrap_or(&json!({})), &readings);
    out.insert("limits".into(), limits);
    (Value::Object(out), replaced)
}

/// The summary the tray, the menu bar and xenonet status read, with every
/// live host's in it. A host that isn't live stays out.
pub fn merge_summary(local: &Value, hosts: &[(Value, Value)], schemas: &mut dyn Schemas) -> Value {
    if hosts.is_empty() {
        return local.clone();
    }
    let mut out = local.as_object().cloned().unwrap_or_default();
    let mut needs = list(local.get("needsYou"), usize::MAX);
    let mut stuck = list(local.get("stuck"), usize::MAX);
    let mut sums: Vec<(&str, f64)> = Vec::new();
    for (info, snapshot) in hosts {
        let Some(live) = snapshot.get("live").filter(|l| truthy(Some(l))) else { continue };
        if info.get("state").and_then(Value::as_str) != Some("live") {
            continue;
        }
        let h = s(info, "id").unwrap_or_default();
        let q = qualify_live(live, info, schemas);
        let mut titles: HashMap<String, Value> = HashMap::new();
        for x in list(snapshot.get("graph").and_then(|g| g.get("nodes")), MAX_NODES) {
            if x.is_object() && x.get("kind") == Some(&json!("session")) {
                if let Some(id) = remote_id(&h, x.get("id")) {
                    titles.insert(id, str_of(x.get("title"), 200));
                }
            }
        }
        let title = |id: &str| titles.get(id).cloned().unwrap_or_else(|| json!(id));
        for x in &q.sessions {
            let id = js_string(&x["id"]);
            if let Some(nv) = x.get("needsYou").filter(|v| truthy(Some(v))) {
                if nv.get("kind") != Some(&json!("review")) {
                    needs.push(json!({ "id": id, "app": x.get("app"), "title": title(&id), "kind": nv.get("kind"), "text": nv.get("text"), "host": h }));
                }
            }
            if let Some(st) = x.get("stuck").filter(|v| truthy(Some(v))) {
                stuck.push(json!({ "id": id, "title": title(&id), "reasons": st.get("reasons"), "host": h }));
            }
        }
        let working = q.sessions.iter().filter(|x| x.get("status") != Some(&json!("idle"))).count() as f64;
        let review = q.sessions.iter().filter(|x| x.get("needsYou").and_then(|nv| nv.get("kind")) == Some(&json!("review"))).count() as f64;
        sums.push(("working", working));
        sums.push(("review", review));
        sums.push(("collisions", q.collisions.len() as f64));
        sums.push(("tpm", q.tpm));
    }
    out.insert("needsYou".into(), Value::Array(needs));
    out.insert("stuck".into(), Value::Array(stuck));
    for (k, v) in sums {
        let total = num(out.get(k)) + v;
        out.insert(k.into(), n(total));
    }
    Value::Object(out)
}

// ---------- events and the engine ----------

/// One of a host's events as this XenoNet passes it on, with its ids
/// qualified, its time moved and only the fields its kind has, or None.
pub fn qualify_event(name: &str, data: &Value, host: &Value, schemas: &mut dyn Schemas) -> Option<Value> {
    let h = s(host, "id").unwrap_or_default();
    let shift = Shift::of(host);
    if !data.is_object() {
        return None;
    }
    match name {
        "activity" => {
            let session = remote_id(&h, data.get("sessionId"))?;
            let Some(Value::Object(mut out)) = schemas.clean(Kind::LiveActivity, data) else { return None };
            out.insert("sessionId".into(), json!(session));
            out.insert("ts".into(), shift.at(data.get("ts")));
            out.insert("host".into(), json!(h));
            if out.contains_key("projectId") {
                let p = remote_id(&h, out.get("projectId"))?;
                out.insert("projectId".into(), json!(p));
            }
            Some(Value::Object(out))
        }
        "tokens" => {
            let session = remote_id(&h, data.get("sessionId"))?;
            // An app this XenoNet doesn't tail (one a newer host knows, say)
            // isn't counted as Claude Code's: its tokens are left out, and
            // event_note says so.
            let app = xenonet_base::apps::tailed_or(data.get("app"), None)?;
            let tokens = finite(data.get("n"))?;
            Some(json!({ "sessionId": session, "app": app, "n": n(tokens), "ts": shift.at(data.get("ts")), "host": h }))
        }
        "pulse" => {
            let node = if data.get("nodeId") == Some(&json!("core")) { Some(format!("@{h}:core")) } else { remote_id(&h, data.get("nodeId")) }?;
            let reason = match str_of(data.get("reason"), 40) {
                Value::Null => json!("updated"),
                r => r,
            };
            Some(json!({ "nodeId": node, "reason": reason, "host": h }))
        }
        "agent-message" => {
            let from = remote_id(&h, data.get("from"))?;
            let id = data.get("id")?;
            let safe_int = id.as_f64().is_some_and(|x| x.fract() == 0.0 && x.abs() <= 9007199254740991.0);
            if !safe_int && !id.is_string() {
                return None;
            }
            let to: Vec<Value> = list(data.get("to"), usize::MAX).iter().filter_map(|x| remote_id(&h, Some(x))).map(Value::String).collect();
            let text = match str_of(data.get("text"), 4000) {
                Value::Null => json!(""),
                t => t,
            };
            Some(
                json!({ "id": format!("@{h}:{}", slice16(&js_string(id), 64)), "from": from, "to": to, "target": str_of(data.get("target"), 200), "text": text, "ts": shift.at(data.get("ts")), "host": h }),
            )
        }
        _ => None,
    }
}

/// What the hub notes in the log of an event it left out for its app, as
/// the drift log notes a vendor's new variant: tokens for an app this
/// XenoNet doesn't tail. None for any other event.
pub fn event_note(name: &str, data: &Value, host: &Value) -> Option<String> {
    if name != "tokens" || !data.is_object() || xenonet_base::apps::tailed_or(data.get("app"), None).is_some() {
        return None;
    }
    let app = xenonet_base::json::stringify(data.get("app").unwrap_or(&Value::Null), 0);
    let id = s(host, "id").unwrap_or_else(|| "undefined".into());
    Some(format!("{id} sent tokens for {}, an app this XenoNet doesn't tail, and they were left out", slice16(&app, 60)))
}

/// A host's engine under qualified ids: the agents and messages that fit
/// engine/agent/v1 and engine/message/v1, with only what those declare.
pub fn qualify_engine(engine: &Value, host: &Value, schemas: &mut dyn Schemas) -> Value {
    let h = s(host, "id").unwrap_or_default();
    let mut agents = Vec::new();
    for a in list(engine.get("agents"), MAX_ENGINE) {
        let Some(id) = (if a.is_object() { remote_id(&h, a.get("id")) } else { None }) else { continue };
        if let Some(Value::Object(mut kept)) = schemas.clean(Kind::EngineAgent, &a) {
            kept.insert("id".into(), json!(id));
            agents.push(Value::Object(kept));
        }
    }
    let mut messages = Vec::new();
    for m in list(engine.get("messages"), MAX_ENGINE) {
        let Some(from) = (if m.is_object() { remote_id(&h, m.get("from")) } else { None }) else { continue };
        let Some(Value::Object(mut kept)) = schemas.clean(Kind::EngineMessage, &m) else { continue };
        let to: Vec<Value> = list(kept.get("to"), usize::MAX)
            .into_iter()
            .filter(Value::is_object)
            .filter_map(|d| {
                let id = remote_id(&h, d.get("id"))?;
                let mut x = d.as_object().cloned().unwrap_or_default();
                x.insert("id".into(), json!(id));
                Some(Value::Object(x))
            })
            .collect();
        let id = m.get("id").map_or_else(|| "undefined".to_string(), js_string);
        kept.insert("id".into(), json!(format!("@{h}:{}", slice16(&id, 64))));
        kept.insert("from".into(), json!(from));
        kept.insert("to".into(), Value::Array(to));
        messages.push(Value::Object(kept));
    }
    json!({ "agents": agents, "messages": messages })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_for_an_app_this_xenonet_doesnt_tail_are_noted_not_counted() {
        let host = json!({ "id": "vm1" });
        let tokens = |app: Value| json!({ "sessionId": "claude:0190d3e0-0000-7000-8000-000000000001", "app": app, "n": 40 });
        assert_eq!(
            event_note("tokens", &tokens(json!("grok-build")), &host).as_deref(),
            Some(r#"vm1 sent tokens for "grok-build", an app this XenoNet doesn't tail, and they were left out"#)
        );
        assert_eq!(event_note("tokens", &json!({ "n": 40 }), &host).as_deref(), Some("vm1 sent tokens for null, an app this XenoNet doesn't tail, and they were left out"));
        assert_eq!(event_note("tokens", &tokens(json!("codex")), &host), None);
        assert_eq!(event_note("pulse", &json!({}), &host), None);
        assert_eq!(event_note("tokens", &json!(5), &host), None, "not an event at all");
    }
}
