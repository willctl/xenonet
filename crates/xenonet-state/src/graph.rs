//! core/graph.js: the hierarchy the page renders:
//!   core (the projects folder) → [groups] → projects → [Codex native
//!   project] → sessions → spawned or forked sessions
//! plus zones for sessions whose folder is outside the projects folder, the
//! shared context's wiki as its own constellation, and the shared systems
//! sessions reach through MCP. A group is a project node with `group` set.
//!
//! Which project orbits which (projects.js's projectRelations) comes in
//! as `parent_of`, worked out by whoever has the projects.

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_base::ids::normalize_remote;
use xenonet_base::json::number;
use xenonet_base::paths::{base_name, is_under, path_key, scratch_zone, tilde};
use xenonet_base::text::{js_number, js_string, truthy};

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn str_of<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    get(v, k).and_then(Value::as_str)
}

// pathKey(p): a key for a truthy path, as the paths twin gives it.
fn key_of(p: Option<&Value>) -> Option<String> {
    p.filter(|v| truthy(Some(v))).and_then(|v| path_key(&js_string(v)))
}

// normalizeRemote(url): null for no url.
fn remote_of(url: Option<&Value>) -> Option<String> {
    url.filter(|v| truthy(Some(v))).map(|v| normalize_remote(&js_string(v))).filter(|r| !r.is_empty())
}

/// How lately a session was written to: active, recent or idle.
pub fn activity_of(updated_at: Option<&Value>, now: f64, windows: Option<&Value>) -> &'static str {
    if !truthy(updated_at) {
        return "idle";
    }
    let age = now - js_number(updated_at);
    let w = |k: &str| js_number(windows.and_then(|w| get(w, k)));
    if age <= w("activeWindowMs") {
        "active"
    } else if age <= w("recentWindowMs") {
        "recent"
    } else {
        "idle"
    }
}

struct Graph {
    nodes: Vec<(String, Map<String, Value>)>,
    at: HashMap<String, usize>,
    links: Vec<Value>,
    parent_of_node: HashMap<String, String>,
}

impl Graph {
    fn has(&self, id: &str) -> bool {
        self.at.contains_key(id)
    }
    fn set(&mut self, id: &str, node: Map<String, Value>) {
        match self.at.get(id) {
            // Map.set on a key that's there keeps its place.
            Some(&i) => self.nodes[i].1 = node,
            None => {
                self.at.insert(id.to_string(), self.nodes.len());
                self.nodes.push((id.to_string(), node));
            }
        }
    }
    fn get_mut(&mut self, id: &str) -> Option<&mut Map<String, Value>> {
        self.at.get(id).map(|&i| &mut self.nodes[i].1)
    }
    fn get(&self, id: &str) -> Option<&Map<String, Value>> {
        self.at.get(id).map(|&i| &self.nodes[i].1)
    }
    fn link(&mut self, source: &str, target: &str, kind: &str) {
        self.links.push(json!({ "source": source, "target": target, "kind": kind }));
        self.parent_of_node.insert(target.to_string(), source.to_string());
    }
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// buildGraph(input), with `parent_of` (a project's key → the key of the
/// project it orbits) from projectRelations. `input` holds what buildGraph
/// takes, with `repos` as [[key, status], …].
pub fn build_graph(input: &Value, parent_of: &HashMap<String, String>) -> Value {
    let field = |k: &str| get(input, k).filter(|v| !v.is_null());
    let projects_dir = field("projectsDir").map(js_string).unwrap_or_default();
    let home_dir = field("homeDir").filter(|v| truthy(Some(v))).map(js_string);
    let projects = items(field("projects"));
    let sessions = items(field("sessions"));
    let native_projects = items(field("nativeProjects"));
    let resources = items(field("resources"));
    let wiki = field("wiki");
    let repos: Vec<(String, Value)> = items(field("repos")).iter().filter_map(|p| Some((js_string(p.get(0)?), p.get(1).cloned().unwrap_or(Value::Null)))).collect();
    let repo_of = |k: &str| repos.iter().rev().find(|(r, _)| r == k).map(|(_, v)| v.clone());
    let empty = Value::Object(Map::new());
    let layout = field("layout").unwrap_or(&empty);
    let now = field("now").map_or(f64::NAN, |v| js_number(Some(v)));
    let windows = field("windows");

    let mut g = Graph { nodes: Vec::new(), at: HashMap::new(), links: Vec::new(), parent_of_node: HashMap::new() };
    let root_key = path_key(&projects_dir);
    // Named for the projects folder in use: ~/Projects, ~/code/work, or the
    // whole path of one outside the home folder.
    let root_name = match &home_dir {
        Some(h) => tilde(&projects_dir, h).replace('\\', "/"),
        None => format!("~/{}", base_name(&projects_dir)),
    };
    g.set("core", obj(json!({ "id": "core", "kind": "core", "name": root_name, "path": projects_dir, "home": home_dir })));

    let pkey = |p: &Value| get(p, "key").map(js_string).unwrap_or_else(|| "undefined".into());
    for p in projects {
        let k = pkey(p);
        let id = format!("project:{k}");
        let mut n = Map::new();
        n.insert("id".into(), Value::String(id.clone()));
        n.insert("kind".into(), Value::String("project".into()));
        for (key, v) in p.as_object().into_iter().flatten() {
            n.insert(key.clone(), v.clone());
        }
        n.insert("group".into(), Value::Bool(str_of(p, "role") == Some("group")));
        n.insert("satelliteOf".into(), parent_of.get(&k).map_or(Value::Null, |s| Value::String(s.clone())));
        n.insert("repo".into(), repo_of(&k).unwrap_or(Value::Null));
        g.set(&id, n);
    }
    for p in projects {
        let k = pkey(p);
        let id = format!("project:{k}");
        if let Some(sat) = parent_of.get(&k) {
            g.link(&format!("project:{sat}"), &id, "satellite");
        } else if let Some(parent) = get(p, "parent").filter(|v| truthy(Some(v))).map(js_string).filter(|pp| g.has(&format!("project:{pp}"))) {
            g.link(&format!("project:{parent}"), &id, "nested");
        } else {
            g.link("core", &id, "contains");
        }
    }
    // The deepest project at or above a folder.
    let mut by_key: HashMap<String, &Value> = HashMap::new();
    for p in projects {
        by_key.insert(pkey(p), p);
    }
    let project_for_key = |key: &str| -> Option<&Value> {
        let mut k = key.to_string();
        while !k.is_empty() {
            if let Some(p) = by_key.get(&k) {
                return Some(p);
            }
            k.truncate(k.rfind('/').unwrap_or(0));
        }
        None
    };
    let mut by_remote: HashMap<String, &Value> = HashMap::new();
    for p in projects {
        if let Some(r) = remote_of(get(p, "git").and_then(|g| get(g, "origin"))) {
            by_remote.entry(r).or_insert(p);
        }
    }

    // Codex desktop projects sit inside the project that holds their root.
    let mut native_ids: HashSet<String> = HashSet::new();
    for np in native_projects {
        let raw = get(np, "id").map(js_string).unwrap_or_else(|| "undefined".into());
        let id = format!("native:codex:{raw}");
        let home = items(get(np, "roots")).iter().find_map(|r| key_of(Some(r)).and_then(|k| project_for_key(&k)));
        let home_id = home.map(|h| format!("project:{}", pkey(h)));
        g.set(&id, obj(json!({ "id": id, "kind": "native", "app": "codex", "name": get(np, "name").cloned().unwrap_or(Value::Null), "roots": get(np, "roots").cloned().unwrap_or(Value::Null) })));
        g.link(home_id.as_deref().unwrap_or("core"), &id, "native");
        native_ids.insert(raw);
    }

    let home_key = home_dir.as_deref().and_then(path_key);
    let zone_for = |g: &mut Graph, key: Option<&str>, session_path: Option<&Value>| -> String {
        // A folder in your home folder isn't Temp, even when home itself
        // sits in a temp folder, as the demo's does.
        let mut scratch = key.and_then(scratch_zone);
        if scratch.is_some_and(|z| z.id == "zone:temp") && home_key.as_deref().is_some_and(|h| key.is_some_and(|k| is_under(k, h))) {
            scratch = None;
        }
        let path = session_path.cloned().unwrap_or(Value::Null);
        let (id, name, zone_path, app) = match (key, scratch) {
            (None, _) => ("zone:unknown".to_string(), "Unknown folder".to_string(), Value::Null, Value::Null),
            (Some(_), Some(z)) => (z.id.to_string(), z.name.to_string(), Value::Null, z.app.map_or(Value::Null, |a| Value::String(a.into()))),
            (Some(k), None) if home_key.as_deref() == Some(k) => ("zone:home".into(), "Home folder".into(), path, Value::Null),
            (Some(k), None) => (format!("zone:{k}"), base_name(k), path, Value::Null),
        };
        if !g.has(&id) {
            g.set(&id, obj(json!({ "id": id, "kind": "zone", "name": name, "path": zone_path, "app": app, "external": true })));
            g.link("core", &id, "zone");
        }
        id
    };

    // Where a session lives, ignoring lineage: native project, project, or
    // zone.
    let home_of = |g: &mut Graph, s: &Value| -> String {
        // A run reported from another machine has no folder here.
        if str_of(s, "observed") == Some("gateway") {
            if !g.has("zone:elsewhere") {
                g.set("zone:elsewhere", obj(json!({ "id": "zone:elsewhere", "kind": "zone", "name": "Elsewhere", "path": null, "app": null, "external": true })));
                g.link("core", "zone:elsewhere", "zone");
            }
            return "zone:elsewhere".into();
        }
        if let Some(np) = get(s, "nativeProjectId").filter(|v| truthy(Some(v))).map(js_string).filter(|np| native_ids.contains(np)) {
            return format!("native:codex:{np}");
        }
        let key = key_of(get(s, "cwd"));
        let p = key.as_deref().filter(|k| root_key.as_deref().is_some_and(|r| is_under(k, r))).and_then(project_for_key);
        if let Some(p) = p {
            return format!("project:{}", pkey(p));
        }
        if let Some(by) = get(s, "origin").filter(|v| truthy(Some(v))).and_then(|o| remote_of(Some(o))).and_then(|r| by_remote.get(&r).copied()) {
            return format!("project:{}", pkey(by));
        }
        zone_for(g, key.as_deref(), get(s, "cwd"))
    };

    let sid = |s: &Value| get(s, "id").map(js_string).unwrap_or_else(|| "undefined".into());
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    for s in sessions {
        by_id.insert(sid(s), s);
    }
    for s in sessions {
        // An archived session can be touched by the app, but it isn't live
        // work.
        let activity = activity_of(get(s, "updatedAt"), now, windows);
        let mut n = s.as_object().cloned().unwrap_or_default();
        n.insert("kind".into(), Value::String("session".into()));
        n.insert("activity".into(), Value::String(if truthy(get(s, "archived")) && activity == "active" { "recent".into() } else { activity.into() }));
        g.set(&sid(s), n);
    }
    let mut homes: HashMap<String, String> = HashMap::new();
    for s in sessions {
        let h = home_of(&mut g, s);
        homes.insert(sid(s), h);
    }
    for s in sessions {
        let id = sid(s);
        let parent = get(s, "parentId").filter(|v| truthy(Some(v))).map(js_string).filter(|p| *p != id).and_then(|p| by_id.get(&p).copied());
        // Spawned or forked sessions nest under their parent when they share
        // a home; otherwise they stay in their own, with a cross-link.
        if let Some(parent) = parent {
            let pid = sid(parent);
            if homes.get(&pid) == homes.get(&id) {
                g.link(&pid, &id, "lineage");
                continue;
            }
        }
        let home = homes[&id].clone();
        g.link(&home, &id, "session");
        if let Some(parent) = parent {
            g.links.push(json!({ "source": sid(parent), "target": id, "kind": "lineage-cross" }));
        }
    }

    // The shared context as its own constellation. Its root takes its MCP
    // server's resource id, so beams from calls to that server land on it.
    let mut relations: Vec<Value> = Vec::new();
    if let Some(w) = wiki.filter(|w| truthy(Some(w))) {
        let server = get(w, "server").filter(|v| !v.is_null()).map_or_else(|| "agent-wiki".to_string(), js_string);
        let root = format!("resource:{server}");
        let pages = items(get(w, "pages"));
        let last_log = items(get(w, "log")).last().and_then(|l| get(l, "ts")).filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null);
        g.set(
            &root,
            obj(json!({ "id": root, "kind": "wiki", "name": xenonet_sources::xenolith::context_name(Some(&server)), "server": server, "path": get(w, "dir").cloned().unwrap_or(Value::Null), "pageCount": pages.len(), "inbox": get(w, "inbox").cloned().unwrap_or(Value::Null), "lastActivity": last_log })),
        );
        g.link("core", &root, "wiki");
        let slugs: HashSet<String> = pages.iter().map(|p| get(p, "slug").map(js_string).unwrap_or_default()).collect();
        let mut seen: HashSet<String> = HashSet::new();
        for p in pages {
            let slug = get(p, "slug").map(js_string).unwrap_or_else(|| "undefined".into());
            let id = format!("wiki:{slug}");
            let writes: Vec<Value> = items(get(p, "writes")).iter().rev().take(6).rev().cloned().collect();
            let f = |k: &str| get(p, k).cloned().unwrap_or(Value::Null);
            g.set(
                &id,
                obj(json!({ "id": id, "kind": "page", "name": f("title"), "slug": f("slug"), "type": f("type"), "summary": f("summary"), "tags": f("tags"), "updated": f("updated"), "updatedBy": f("updatedBy"), "path": f("path"), "writes": writes, "lastActivity": f("updated") })),
            );
            g.link(&root, &id, "page");
            for l in items(get(p, "links")) {
                let l = js_string(l);
                let mut pair = [slug.clone(), l.clone()];
                pair.sort_by(|a, b| xenonet_base::jsstr::cmp_units(a, b));
                let k = pair.join("|");
                if slugs.contains(&l) && !seen.contains(&k) {
                    seen.insert(k);
                    relations.push(json!({ "a": id, "b": format!("wiki:{l}"), "kind": "wikilink" }));
                }
            }
            for key in items(get(p, "projects")) {
                relations.push(json!({ "a": id, "b": format!("project:{}", js_string(key)), "kind": "mentions" }));
            }
        }
    }

    // Shared systems sessions reach through MCP (a browser, …).
    for r in resources {
        let id = format!("resource:{}", get(r, "server").map_or_else(|| "undefined".into(), js_string));
        if g.has(&id) {
            continue;
        }
        g.set(&id, obj(json!({ "id": id, "kind": "resource", "name": get(r, "server").cloned().unwrap_or(Value::Null), "lastActivity": get(r, "ts").cloned().unwrap_or(Value::Null) })));
        g.link("core", &id, "resource");
    }

    // Positions you dragged into place stay put.
    for (id, n) in g.nodes.iter_mut() {
        let p = get(layout, id).filter(|p| truthy(Some(p)));
        let (fx, fy, fz, pinned) = match p {
            Some(p) => (get(p, "x").cloned().unwrap_or(Value::Null), get(p, "y").cloned().unwrap_or(Value::Null), get(p, "z").cloned().unwrap_or(Value::Null), true),
            None => (Value::Null, Value::Null, Value::Null, false),
        };
        n.insert("fx".into(), fx);
        n.insert("fy".into(), fy);
        n.insert("fz".into(), fz);
        n.insert("pinned".into(), Value::Bool(pinned));
    }

    // Session counts and the latest activity rolled up the tree.
    for (_, n) in g.nodes.iter_mut() {
        if n.get("kind").and_then(Value::as_str) != Some("session") {
            let last = n.get("lastActivity").filter(|v| !v.is_null()).or(n.get("modifiedAt").filter(|v| !v.is_null())).cloned().unwrap_or(Value::Null);
            n.insert("sessionCount".into(), Value::from(0));
            n.insert("activeCount".into(), Value::from(0));
            n.insert("lastActivity".into(), last);
            n.insert("lastSessionAt".into(), Value::Null);
        }
    }
    for s in sessions {
        let id = sid(s);
        let active = g.get(&id).and_then(|n| n.get("activity")).and_then(Value::as_str) == Some("active");
        let updated = get(s, "updatedAt").filter(|v| truthy(Some(v))).map(|v| js_number(Some(v)));
        let mut cur = g.parent_of_node.get(&id).cloned();
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(c) = cur.filter(|c| !seen.contains(c)) {
            seen.insert(c.clone());
            if let Some(n) = g.get_mut(&c).filter(|n| n.get("kind").and_then(Value::as_str) != Some("session")) {
                let count = |n: &Map<String, Value>, k: &str| js_number(n.get(k));
                let sc = count(n, "sessionCount") + 1.0;
                n.insert("sessionCount".into(), number(sc));
                if active {
                    let ac = count(n, "activeCount") + 1.0;
                    n.insert("activeCount".into(), number(ac));
                }
                if let Some(u) = updated {
                    for k in ["lastActivity", "lastSessionAt"] {
                        let was = n.get(k).filter(|v| !v.is_null()).map_or(0.0, |v| js_number(Some(v)));
                        if u > was {
                            n.insert(k.into(), get(s, "updatedAt").cloned().unwrap());
                        }
                    }
                }
            }
            cur = g.parent_of_node.get(&c).cloned();
        }
    }
    // Places where a session ran lately are active.
    let active_ms = windows.and_then(|w| get(w, "projectActiveMs")).filter(|v| !v.is_null()).map_or(3600e3, |v| js_number(Some(v)));
    for (_, n) in g.nodes.iter_mut() {
        if matches!(n.get("kind").and_then(Value::as_str), Some("project" | "native" | "zone")) {
            let last = n.get("lastSessionAt").filter(|v| !v.is_null()).map(|v| js_number(Some(v)));
            n.insert("active".into(), Value::Bool(last.is_some_and(|l| now - l <= active_ms)));
        }
    }

    let count = |f: &dyn Fn(&Map<String, Value>) -> bool| g.nodes.iter().filter(|(_, n)| n.get("kind").and_then(Value::as_str) == Some("session") && f(n)).count();
    let stats = json!({
        "wikiPages": wiki.filter(|w| truthy(Some(w))).map_or(0, |w| items(get(w, "pages")).len()),
        "dirtyRepos": repos.iter().filter(|(_, r)| js_number(get(r, "dirty")) > 0.0).count(),
        "projects": projects.iter().filter(|p| str_of(p, "role") != Some("group")).count(),
        "groups": projects.iter().filter(|p| str_of(p, "role") == Some("group")).count(),
        "sessions": count(&|_| true),
        "claude": count(&|n| n.get("app").and_then(Value::as_str) == Some("claude")),
        "codex": count(&|n| n.get("app").and_then(Value::as_str) == Some("codex")),
        "active": count(&|n| n.get("activity").and_then(Value::as_str) == Some("active")),
        "archived": count(&|n| truthy(n.get("archived"))),
    });
    let relations: Vec<Value> = relations.into_iter().filter(|r| r["a"].as_str().is_some_and(|a| g.has(a)) && r["b"].as_str().is_some_and(|b| g.has(b))).collect();
    let nodes: Vec<Value> = g.nodes.into_iter().map(|(_, n)| Value::Object(n)).collect();
    json!({ "generatedAt": number(now), "nodes": nodes, "links": g.links, "relations": relations, "stats": stats })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_find_their_projects_and_zones() {
        let input = json!({
            "projectsDir": "/home/me/Projects",
            "homeDir": "/home/me",
            "projects": [{ "key": "/home/me/projects/atlas", "name": "atlas" }, { "key": "/home/me/projects/atlas-web", "name": "atlas-web" }],
            "sessions": [
                { "id": "claude:a", "app": "claude", "cwd": "/home/me/Projects/atlas/src", "updatedAt": 990 },
                { "id": "claude:b", "app": "claude", "cwd": "/home/me/elsewhere", "updatedAt": 10, "parentId": "claude:a" },
                { "id": "codex:c", "app": "codex", "cwd": "/home/me/Projects/atlas", "updatedAt": 995, "parentId": "claude:a", "archived": true },
            ],
            "layout": { "core": { "x": 1, "y": 2, "z": 3 } },
            "now": 1000,
            "windows": { "activeWindowMs": 60, "recentWindowMs": 600 },
        });
        let mut parent_of = HashMap::new();
        parent_of.insert("/home/me/projects/atlas-web".to_string(), "/home/me/projects/atlas".to_string());
        let g = build_graph(&input, &parent_of);
        let node = |id: &str| g["nodes"].as_array().unwrap().iter().find(|n| n["id"] == id).cloned().unwrap();
        assert_eq!(node("core")["name"], "~/Projects");
        assert_eq!(node("core")["pinned"], true);
        assert_eq!(node("project:/home/me/projects/atlas-web")["satelliteOf"], "/home/me/projects/atlas");
        assert_eq!(node("codex:c")["activity"], "recent", "archived work isn't live");
        assert_eq!(node("zone:/home/me/elsewhere")["kind"], "zone");
        let links: Vec<(String, String)> = g["links"].as_array().unwrap().iter().map(|l| (l["target"].as_str().unwrap().into(), l["kind"].as_str().unwrap().into())).collect();
        assert!(links.contains(&("codex:c".into(), "lineage".into())));
        assert!(links.contains(&("claude:b".into(), "lineage-cross".into())));
        assert_eq!(node("project:/home/me/projects/atlas")["sessionCount"], 2);
        assert_eq!(node("project:/home/me/projects/atlas")["active"], true);
        assert_eq!(g["stats"]["sessions"], 3);
    }
}
