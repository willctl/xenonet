//! core/state.js: what the service knows, refreshed. XenoNetState gathers
//! the sessions, scans the projects folder, builds the map (`graph`), links
//! identities, and follows the agents live. The gathering, the timers and
//! the events are whoever runs it; here are its decisions, and what it
//! remembers from one refresh to the next (`Memory`).

use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_base::json::{number, stringify};
use xenonet_base::paths::{is_under, path_key};
use xenonet_base::text::{js_number, js_string, truthy};

fn get<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    v.as_object().and_then(|m| m.get(k))
}

fn nn(v: Option<&Value>) -> Option<&Value> {
    v.filter(|v| !v.is_null())
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    get(v, k).and_then(Value::as_str)
}

fn items(v: Option<&Value>) -> &[Value] {
    v.and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// A source's status after a run: { name, ok, ms, count } (count for a
/// list), or { name, ok: false, ms, error }.
pub fn source_status(name: &str, ok: bool, ms: Option<&Value>, value: Option<&Value>, error: Option<&Value>) -> Value {
    let ms = ms.cloned().unwrap_or(Value::Null);
    if ok {
        let count = value.and_then(Value::as_array).map_or(Value::Null, |a| Value::from(a.len()));
        json!({ "name": name, "ok": true, "ms": ms, "count": count })
    } else {
        let mut m = Map::new();
        m.insert("name".into(), Value::String(name.into()));
        m.insert("ok".into(), Value::Bool(false));
        m.insert("ms".into(), ms);
        if let Some(e) = error {
            m.insert("error".into(), e.clone());
        }
        Value::Object(m)
    }
}

/// Whether the map shows an MCP server's use: not one you hid, nor Claude
/// desktop's own (ccd_…).
pub fn visible_resource(server: &str, hidden: &HashSet<String>) -> bool {
    !hidden.contains(server) && !server.starts_with("ccd_")
}

/// Background runs (codex exec, subagents), which never wait for you.
pub fn automated(n: Option<&Value>) -> bool {
    let Some(n) = n else { return false };
    let source = get(n, "source").filter(|v| !v.is_null()).map(js_string).unwrap_or_default();
    s(n, "source") == Some("exec") || source.starts_with("subAgent") || s(n, "originator") == Some("codex_exec") || s(n, "parentKind") == Some("subagent")
}

/// The sessions a refresh maps: each app's fresh list, or for an app whose
/// listing failed, the sessions it had; then the runs XenoNet observes,
/// unless it reads the same run itself; with the demo's own names.
pub fn merge_sessions(prev: &[Value], codex: (bool, &[Value]), claude: (bool, &[Value]), observed: &[Value], demo_titles: &HashMap<String, Value>) -> Vec<Value> {
    let keep = |(ok, fresh): (bool, &[Value]), app: &str| -> Vec<Value> {
        if ok {
            fresh.to_vec()
        } else {
            prev.iter().filter(|n| s(n, "app") == Some(app)).cloned().collect()
        }
    };
    let mut sessions = keep(codex, "codex");
    sessions.extend(keep(claude, "claude"));
    let id = |v: &Value| get(v, "id").map_or_else(|| "\u{0}undefined".into(), js_string);
    let read: HashSet<String> = sessions.iter().map(id).collect();
    for o in observed {
        if !read.contains(&id(o)) {
            sessions.push(o.clone());
        }
    }
    for s in sessions.iter_mut() {
        if let Some(t) = demo_titles.get(&id(s)) {
            if let Some(m) = s.as_object_mut() {
                m.insert("title".into(), t.clone());
            }
        }
    }
    sessions
}

/// The folders sessions ran in, the scan's evidence.
pub fn session_dirs(sessions: &[Value]) -> Vec<Value> {
    sessions.iter().filter_map(|s| get(s, "cwd").filter(|c| truthy(Some(c))).cloned()).collect()
}

/// What the projects scan's status says of it.
pub fn projects_note(scan: &Value) -> String {
    let n = |k: &str| get(scan, k).map_or_else(|| "undefined".into(), js_string);
    let groups = js_number(get(scan, "groups"));
    format!(
        "{} projects in {} group{}, {} folders read{}",
        n("projects"),
        n("groups"),
        if groups == 1.0 { "" } else { "s" },
        n("dirsRead"),
        if truthy(get(scan, "truncated")) { format!("; stopped at the {}-folder limit", n("maxDirs")) } else { String::new() }
    )
}

/// The rules a scan of `root` follows: Codex's picks for that folder, then
/// yours on top.
pub fn project_rules(codex: bool, root: &str, picks: Option<&Value>, yours: &Value) -> Value {
    let mut rules = Map::new();
    if codex {
        for p in items(picks.and_then(|c| get(c, "picks"))) {
            let rel = s(p, "path").unwrap_or("");
            let parts: Vec<&str> = std::iter::once(root).chain(rel.split('/')).collect();
            let dir = xenonet_base::paths::join_all(&parts);
            let Some(key) = path_key(&dir) else { continue };
            rules.insert(key, json!({ "path": dir, "as": get(p, "as").cloned().unwrap_or(Value::Null), "by": "codex", "reason": get(p, "reason").cloned().unwrap_or(Value::Null) }));
        }
    }
    for (key, r) in yours.as_object().into_iter().flatten() {
        let mut rule = r.as_object().cloned().unwrap_or_default();
        rule.insert("by".into(), Value::String("you".into()));
        rules.insert(key.clone(), Value::Object(rule));
    }
    Value::Object(xenonet_base::jsstr::js_key_order(rules))
}

/// Why a rule for a folder can't be set, or None. `is_dir` says whether the
/// folder is there.
pub fn rule_problem(as_: Option<&Value>, full: &str, projects_dir: &str, is_dir: bool) -> Option<String> {
    // null clears a rule; anything else has to be one of the three.
    let as_str = as_.and_then(Value::as_str);
    if !matches!(as_, Some(Value::Null)) && !matches!(as_str, Some("project" | "group" | "ignore")) {
        return Some("unknown rule".into());
    }
    if !truthy(as_) {
        return None;
    }
    let key = path_key(full);
    let root = path_key(projects_dir);
    let inside = match (&key, &root) {
        (Some(k), Some(r)) => is_under(k, r) && k != r,
        _ => false,
    };
    if !inside {
        return Some(format!("Only folders inside {} can have rules", xenonet_base::paths::base_name(projects_dir)));
    }
    if !is_dir {
        return Some("That folder doesn't exist".into());
    }
    None
}

/// The scan's state: when it ran, what it found, your rules (each saying
/// whether it's in this projects folder, newest first) and Codex's picks
/// for this folder, stale once its folders changed since Codex looked.
pub fn project_detection(root: &str, scan: Option<&Value>, rules: &Value, codex: Option<&Value>, current: Option<&Value>) -> Value {
    let root_key = path_key(root);
    let mut list: Vec<Value> = rules
        .as_object()
        .into_iter()
        .flatten()
        .map(|(_, r)| {
            let key = get(r, "path").filter(|p| truthy(Some(p))).and_then(|p| path_key(&js_string(p)));
            let in_root = match (&key, &root_key) {
                (Some(k), Some(rk)) => is_under(k, rk) && k != rk,
                _ => false,
            };
            let mut m = r.as_object().cloned().unwrap_or_default();
            m.insert("inRoot".into(), Value::Bool(in_root));
            Value::Object(m)
        })
        .collect();
    list.sort_by(|a, b| js_number(get(b, "at")).partial_cmp(&js_number(get(a, "at"))).unwrap_or(std::cmp::Ordering::Equal));
    let codex = nn(codex).filter(|c| truthy(Some(c))).map_or(Value::Null, |c| {
        let mut m = c.as_object().cloned().unwrap_or_default();
        m.insert("stale".into(), Value::Bool(current.unwrap_or(&Value::Null) != get(c, "fingerprint").unwrap_or(&Value::Null)));
        Value::Object(m)
    });
    json!({ "root": root, "scan": scan.cloned().unwrap_or(Value::Null), "rules": list, "codex": codex })
}

/// The project (or zone) a session lives in: up the map from it, the first
/// node that isn't a session, or a native project's parent.
pub fn home_of(id: &str, kind_of: &mut dyn FnMut(&str) -> Option<String>, parent_of: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    let mut cur = Some(id.to_string());
    for _ in 0..8 {
        let Some(c) = cur.filter(|c| !c.is_empty()) else { return None };
        if let Some(kind) = kind_of(&c) {
            if kind != "session" {
                return if kind == "native" { parent_of(&c) } else { Some(c) };
            }
        }
        cur = parent_of(&c);
    }
    None
}

/// A node's title, else its name, else its id.
pub fn title_of(title: Option<&Value>, name: Option<&Value>, id: &Value) -> Value {
    nn(title).or(nn(name)).cloned().unwrap_or_else(|| id.clone())
}

/// The name you give an identity: one line of at most 40 units.
pub fn clean_identity_name(name: Option<&Value>) -> Option<String> {
    let name = name.and_then(Value::as_str)?;
    let no_controls: String = name.chars().filter(|c| !matches!(*c as u32, 0..=0x1f | 0x7f)).collect();
    let clean = xenonet_base::jsstr::head(xenonet_base::text::js_trim(&no_controls), 40);
    (!clean.is_empty()).then_some(clean)
}

/// Whether XenoNet knows an identity: on the map, or seen before.
pub fn known_identity(id: Option<&Value>, identities: &[Value], known: &Value) -> Option<String> {
    let id = id.and_then(Value::as_str)?;
    (identities.iter().any(|i| s(i, "id") == Some(id)) || truthy(get(known, id))).then(|| id.to_string())
}

/// What identityDetection says: whether it's on, when the map was made, the
/// identities, your pins (newest first) and the last sign-ins of each app.
pub fn identity_detection(enabled: bool, at: Option<&Value>, identities: Option<&Value>, saved: &Value, node: &mut dyn FnMut(&str) -> Option<Value>) -> Value {
    let mut pins: Vec<Value> = get(saved, "pins")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(id, p)| {
            let n = node(id);
            let field = |k: &str| n.as_ref().and_then(|n| nn(get(n, k))).cloned();
            let mut m = Map::new();
            m.insert("node".into(), Value::String(id.clone()));
            m.insert("kind".into(), field("kind").unwrap_or(Value::Null));
            m.insert("title".into(), field("title").or_else(|| field("name")).unwrap_or(Value::Null));
            for (k, v) in p.as_object().into_iter().flatten() {
                m.insert(k.clone(), v.clone());
            }
            Value::Object(m)
        })
        .collect();
    pins.sort_by(|a, b| js_number(get(b, "at")).partial_cmp(&js_number(get(a, "at"))).unwrap_or(std::cmp::Ordering::Equal));
    let signins: Map<String, Value> = get(saved, "signins")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(app, spans)| {
            let list = items(Some(spans));
            (app.clone(), Value::Array(list[list.len().saturating_sub(8)..].to_vec()))
        })
        .collect();
    json!({ "enabled": enabled, "at": at.cloned().unwrap_or(Value::Null), "identities": identities.cloned().unwrap_or(json!([])), "pins": pins, "signins": signins })
}

/// The kind of node an identity can be pinned to, or the error.
pub fn pin_problem(kind: Option<&str>) -> Option<&'static str> {
    (!matches!(kind, Some("session" | "project" | "pinned"))).then_some("Only a session or a project can belong to an identity")
}

/// The live snapshot as the page gets it: what you've seen, what's
/// archived and what runs on its own doesn't wait for you; collisions
/// name their sessions and branches; plan limits name the account they were
/// read for. `node` looks a node up on the map; `acks` are the times you
/// dismissed each session's wait.
pub fn live_snapshot(mut snap: Value, hidden: &HashSet<String>, acks: &Value, signed_in: Option<&Value>, node: &mut dyn FnMut(&str) -> Option<Value>) -> Value {
    let visible = |r: &Value| s(r, "server").is_some_and(|srv| visible_resource(srv, hidden));
    if let Some(m) = snap.as_object_mut() {
        if let Some(Value::Array(rs)) = m.get_mut("resources") {
            rs.retain(visible);
        }
        if let Some(Value::Array(sessions)) = m.get_mut("sessions") {
            for sess in sessions.iter_mut() {
                let id = s(sess, "id").unwrap_or("").to_string();
                let n = node(&id);
                let gone = n.as_ref().is_none_or(|n| truthy(get(n, "archived")));
                let since = js_number(sess.get("needsYou").and_then(|w| get(w, "since")));
                let acked = nn(get(acks, &id)).map_or(0.0, |v| js_number(Some(v)));
                let o = sess.as_object_mut().unwrap();
                if let Some(Value::Array(mcp)) = o.get_mut("mcp") {
                    mcp.retain(visible);
                }
                if truthy(o.get("needsYou")) && (gone || automated(n.as_ref()) || acked >= since) {
                    o.insert("needsYou".into(), Value::Null);
                }
                if truthy(o.get("stuck")) && gone {
                    o.insert("stuck".into(), Value::Null);
                }
            }
        }
        if let Some(Value::Array(cs)) = m.get_mut("collisions") {
            let mut kept = Vec::new();
            for c in cs.drain(..) {
                let (a_id, b_id) = (s(&c, "a").unwrap_or("").to_string(), s(&c, "b").unwrap_or("").to_string());
                let (a, b) = (node(&a_id), node(&b_id));
                let field = |n: &Option<Value>, k: &str| n.as_ref().and_then(|n| nn(get(n, k))).cloned();
                let mut o = c.as_object().cloned().unwrap_or_default();
                let (ab, bb) = (field(&a, "branch"), field(&b, "branch"));
                o.insert("branches".into(), json!([ab.clone().unwrap_or(Value::Null), bb.clone().unwrap_or(Value::Null)]));
                o.insert("sameBranch".into(), Value::Bool(truthy(ab.as_ref()) && ab == bb));
                o.insert("titles".into(), json!([field(&a, "title").unwrap_or_else(|| json!(a_id)), field(&b, "title").unwrap_or_else(|| json!(b_id))]));
                if a.is_some() && b.is_some() {
                    kept.push(Value::Object(o));
                }
            }
            *cs = kept;
        }
        // Plan limits are an account's, not a machine's, so each reading
        // names the account signed in now.
        let named = |reading: Option<&Value>, identity: Option<&Value>| -> Value {
            match reading.filter(|r| truthy(Some(r))) {
                Some(r) => {
                    let mut o = r.as_object().cloned().unwrap_or_default();
                    o.insert("identity".into(), nn(identity).cloned().unwrap_or(Value::Null));
                    Value::Object(o)
                }
                None => reading.cloned().unwrap_or(Value::Null),
            }
        };
        let limits = m.get("limits").cloned().unwrap_or(Value::Null);
        let mut l = limits.as_object().cloned().unwrap_or_default();
        l.insert("claude".into(), named(get(&limits, "claude"), signed_in.and_then(|s| get(s, "claude"))));
        l.insert("codex".into(), named(get(&limits, "codex"), signed_in.and_then(|s| get(s, "chatgpt"))));
        m.insert("limits".into(), Value::Object(l));
    }
    snap
}

/// The tray's compact status.
pub fn summary(snap: &Value, title_of: &mut dyn FnMut(&Value) -> Value, url: &str) -> Value {
    let sessions = items(get(snap, "sessions"));
    let waits = |s: &Value| get(s, "needsYou").filter(|n| truthy(Some(n))).cloned();
    let kind = |s: &Value| waits(s).and_then(|n| get(&n, "kind").cloned());
    let need: Vec<Value> = sessions
        .iter()
        .filter(|s| waits(s).is_some() && kind(s).as_ref().and_then(Value::as_str) != Some("review"))
        .map(|s| {
            let w = waits(s).unwrap();
            let id = get(s, "id").cloned().unwrap_or(Value::Null);
            json!({ "id": id, "app": get(s, "app").cloned().unwrap_or(Value::Null), "title": title_of(&id), "kind": get(&w, "kind").cloned().unwrap_or(Value::Null), "text": get(&w, "text").cloned().unwrap_or(Value::Null), "since": nn(get(&w, "since")).cloned().unwrap_or(Value::Null) })
        })
        .collect();
    let stuck: Vec<Value> = sessions
        .iter()
        .filter(|s| truthy(get(s, "stuck")))
        .map(|s| {
            let id = get(s, "id").cloned().unwrap_or(Value::Null);
            json!({ "id": id, "title": title_of(&id), "reasons": get(&s["stuck"], "reasons").cloned().unwrap_or(Value::Null) })
        })
        .collect();
    json!({
        "working": sessions.iter().filter(|s| s.get("status").and_then(Value::as_str) != Some("idle")).count(),
        "needsYou": need,
        "review": sessions.iter().filter(|s| kind(s).as_ref().and_then(Value::as_str) == Some("review")).count(),
        "stuck": stuck,
        "collisions": items(get(snap, "collisions")).len(),
        "tpm": get(snap, "tpm").cloned().unwrap_or(Value::Null),
        "url": url,
    })
}

/// A handoff's notes, from what the tailer saw, filled in from a page of
/// the transcript where it saw too little.
pub fn handoff_notes(notes: &Value, page: Option<&Value>) -> Value {
    let Some(page) = page else { return notes.clone() };
    let entries = items(get(page, "entries"));
    let with_text = |role: &str| -> Vec<&Value> { entries.iter().filter(|e| s(e, "role") == Some(role) && truthy(get(e, "text"))).collect() };
    let (users, agents) = (with_text("user"), with_text("assistant"));
    let text = |e: &Value| json!({ "text": get(e, "text").cloned().unwrap_or(Value::Null) });
    let mut m = notes.as_object().cloned().unwrap_or_default();
    let first = nn(get(notes, "firstPrompt")).cloned().unwrap_or_else(|| users.first().map_or(Value::Null, |u| text(u)));
    m.insert("firstPrompt".into(), first);
    if items(get(notes, "prompts")).is_empty() {
        m.insert("prompts".into(), Value::Array(users[users.len().saturating_sub(2)..].iter().map(|u| text(u)).collect()));
    }
    if items(get(notes, "messages")).is_empty() {
        m.insert("messages".into(), Value::Array(agents[agents.len().saturating_sub(1)..].iter().map(|a| text(a)).collect()));
    }
    Value::Object(m)
}

/// Whether a handoff needs the transcript: the tailer saw no first prompt
/// or no reply.
pub fn handoff_needs_transcript(notes: &Value) -> bool {
    !truthy(get(notes, "firstPrompt")) || items(get(notes, "messages")).is_empty()
}

/// What state.js keeps from one refresh to the next: when each session
/// was last written to, the map's signature, the last nudges to commit,
/// and who was signed in.
#[derive(Default)]
pub struct Memory {
    last_seen: HashMap<String, Option<Value>>,
    signature: Option<String>,
    ship_it_at: HashMap<String, f64>,
    signed_in: Option<Map<String, Value>>,
}

impl Memory {
    pub fn new() -> Self {
        Memory::default()
    }

    /// The sessions written to since the last refresh, which pulse on the
    /// map.
    pub fn updated(&mut self, sessions: &[Value]) -> Vec<Value> {
        let mut out = Vec::new();
        for s in sessions {
            let id = get(s, "id").map_or_else(|| "\u{0}undefined".into(), js_string);
            // A session last seen without a time (undefined) can't have
            // been written to since.
            let now = get(s, "updatedAt").cloned();
            if let Some(Some(before)) = self.last_seen.get(&id) {
                if truthy(now.as_ref()) && now.as_ref() != Some(before) {
                    out.push(get(s, "id").cloned().unwrap_or(Value::Null));
                }
            }
            self.last_seen.insert(id, now);
        }
        out
    }

    /// Whether the map differs from the last one in anything the page
    /// shows.
    pub fn changed(&mut self, graph: &Value) -> bool {
        let f = |n: &Value, k: &str| get(n, k).cloned().unwrap_or(Value::Null);
        let sub = |n: &Value, a: &str, b: &str| get(n, a).and_then(|x| get(x, b)).cloned().unwrap_or(Value::Null);
        let nodes: Vec<Value> = items(get(graph, "nodes"))
            .iter()
            .map(|n| {
                let ids = get(n, "identities").and_then(Value::as_array).map(|l| {
                    Value::String(l.iter().map(|x| format!("{}:{}:{}", get(x, "role").map_or("undefined".into(), js_string), get(x, "id").map_or("undefined".into(), js_string), get(x, "confidence").map_or("undefined".into(), js_string))).collect::<Vec<_>>().join(","))
                });
                json!([
                    f(n, "id"), f(n, "updatedAt"), f(n, "archived"), f(n, "title"), f(n, "activity"), f(n, "active"), f(n, "sessionCount"),
                    sub(n, "repo", "dirty"), sub(n, "repo", "branch"), f(n, "pinned"),
                    get(n, "inbox").and_then(Value::as_array).map_or(Value::Null, |a| Value::from(a.len())),
                    f(n, "updated"), f(n, "role"), f(n, "parent"), f(n, "detected"), sub(n, "rule", "as"), sub(n, "ranAs", "id"), sub(n, "ranAs", "confidence"),
                    ids.unwrap_or(Value::Null),
                ])
            })
            .collect();
        let ids: Vec<Value> = items(get(graph, "identities")).iter().map(|i| json!([f(i, "id"), f(i, "handle"), f(i, "name"), f(i, "hidden"), f(i, "signedIn"), f(i, "active")])).collect();
        let signature = stringify(&json!([nodes, items(get(graph, "relations")).len(), ids]), 0);
        let changed = self.signature.as_deref() != Some(&signature);
        self.signature = Some(signature);
        changed
    }

    /// The sign-ins now (Claude Code's, Codex's, each gh host's active
    /// account) from what identities found, and the activity a change since
    /// the last refresh makes.
    pub fn note_signins(&mut self, accounts: &Value, now: f64) -> Vec<Value> {
        let mut current = Map::new();
        let id_of = |k: &str| get(accounts, k).and_then(|a| nn(get(a, "id"))).cloned().unwrap_or(Value::Null);
        current.insert("claude".into(), id_of("claude"));
        current.insert("chatgpt".into(), id_of("chatgpt"));
        for h in items(get(accounts, "gh")) {
            let host = get(h, "host").map_or_else(|| "undefined".into(), js_string);
            let active = get(h, "active").filter(|a| truthy(Some(a))).map(|a| Value::String(format!("github:{host}/{}", js_string(a).to_lowercase())));
            current.insert(format!("gh:{host}"), active.unwrap_or(Value::Null));
        }
        let mut events = Vec::new();
        if let Some(before) = &self.signed_in {
            for (app, id) in &current {
                let was = before.get(app).cloned().unwrap_or(Value::Null);
                if &was != id && (truthy(Some(&was)) || truthy(Some(id))) {
                    events.push(json!({ "kind": "identity", "app": app.split(':').next().unwrap_or(""), "identity": id, "before": was, "ts": number(now) }));
                }
            }
        }
        self.signed_in = Some(current);
        events
    }

    pub fn signed_in(&self) -> Option<Value> {
        self.signed_in.clone().map(Value::Object)
    }

    /// A nudge to commit when an agent ends a turn in a repo with
    /// uncommitted work, at most every 30 minutes a project; not for a turn
    /// you stopped. `home` is the session's home on the map, with its name
    /// and its repo's status.
    pub fn ship_it(&mut self, ev: &Value, home: Option<(&str, &Value, &Value)>, now: f64) -> Option<Value> {
        const EVERY_MS: f64 = 30.0 * 60e3;
        if s(ev, "kind") != Some("turn-end") || truthy(get(ev, "interrupted")) {
            return None;
        }
        let (home, name, repo) = home?;
        let dirty = get(repo, "dirty").filter(|d| truthy(Some(d)))?;
        if now - self.ship_it_at.get(home).copied().unwrap_or(0.0) < EVERY_MS {
            return None;
        }
        self.ship_it_at.insert(home.to_string(), now);
        let n = js_number(Some(dirty));
        Some(json!({
            "sessionId": get(ev, "sessionId").cloned().unwrap_or(Value::Null),
            "app": get(ev, "app").cloned().unwrap_or(Value::Null),
            "kind": "ship-it",
            "detail": format!("{} uncommitted file{} in {}", js_string(dirty), if n == 1.0 { "" } else { "s" }, js_string(name)),
            "projectId": home,
            "ts": number(now),
        }))
    }
}

/// The note on a source's status for the projects scan: how many it found.
pub fn identities_note(identities: &[Value]) -> String {
    let mut kinds: Vec<(String, usize)> = Vec::new();
    for i in identities {
        let k = get(i, "kind").map_or_else(|| "undefined".into(), js_string);
        match kinds.iter_mut().find(|(x, _)| *x == k) {
            Some((_, n)) => *n += 1,
            None => kinds.push((k, 1)),
        }
    }
    if kinds.is_empty() {
        "none found".into()
    } else {
        kinds.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", ")
    }
}

/// What a native reader's status says: ok when its file or folder is,
/// with how many runs it read.
pub fn reader_status(name: &str, state: &str, runs: usize, what: &str) -> Value {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name.into()));
    m.insert("ok".into(), Value::Bool(state == "ok"));
    m.insert("count".into(), Value::from(runs));
    if state != "ok" {
        m.insert("error".into(), Value::String(format!("{what} is {state}")));
    }
    Value::Object(m)
}

/// The tray's address for this instance.
pub fn url(host: &str, port: &Value) -> String {
    format!("http://{host}:{}/", js_string(port))
}

/// When a renamed session's name went somewhere it says so: Claude desktop
/// shows it at its next start, or kept its old one.
pub fn rename_note(desktop: bool, desktop_error: Option<&Value>) -> Value {
    if desktop {
        return Value::String("Claude desktop shows it the next time it starts.".into());
    }
    match desktop_error.filter(|e| truthy(Some(e))) {
        Some(e) => Value::String(format!("Claude desktop kept its old name: {}", js_string(e))),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_a_refresh_decides() {
        let prev = vec![json!({ "id": "claude:a", "app": "claude" }), json!({ "id": "codex:x", "app": "codex" })];
        let fresh = vec![json!({ "id": "codex:b", "app": "codex" })];
        let observed = vec![json!({ "id": "codex:b", "app": "codex", "observed": "numbat" }), json!({ "id": "gemini-cli:z" })];
        let mut titles = HashMap::new();
        titles.insert("claude:a".to_string(), json!("Mine"));
        let merged = merge_sessions(&prev, (true, &fresh), (false, &[]), &observed, &titles);
        assert_eq!(merged.iter().map(|s| s["id"].as_str().unwrap()).collect::<Vec<_>>(), ["codex:b", "claude:a", "gemini-cli:z"]);
        assert_eq!(merged[1]["title"], "Mine");

        let mut m = Memory::new();
        assert!(m.updated(&[json!({ "id": "a", "updatedAt": 1 })]).is_empty());
        assert_eq!(m.updated(&[json!({ "id": "a", "updatedAt": 2 })]), [json!("a")]);
        let g = json!({ "nodes": [{ "id": "a" }], "relations": [], "identities": [] });
        assert!(m.changed(&g) && !m.changed(&g));
        assert!(m.note_signins(&json!({ "claude": { "id": "claude:x:y" }, "gh": [] }), 1.0).is_empty());
        assert_eq!(m.note_signins(&json!({ "claude": null, "gh": [] }), 2.0)[0]["before"], "claude:x:y");
        let ev = json!({ "kind": "turn-end", "sessionId": "s" });
        assert!(m.ship_it(&ev, Some(("project:p", &json!("p"), &json!({ "dirty": 1 }))), 1e9).unwrap()["detail"] == "1 uncommitted file in p");
        assert!(m.ship_it(&ev, Some(("project:p", &json!("p"), &json!({ "dirty": 1 }))), 1e9 + 1.0).is_none());

        assert_eq!(home_of("s", &mut |id| Some(if id == "n" { "native" } else if id == "p" { "project" } else { "session" }.into()), &mut |id| Some(if id == "s" { "n" } else { "p" }.into())).as_deref(), Some("p"));
        assert_eq!(clean_identity_name(Some(&json!("  Ada\u{7}  "))).as_deref(), Some("Ada"));
        assert_eq!(rule_problem(Some(&json!("nope")), "/p/x", "/p", true).as_deref(), Some("unknown rule"));
        assert_eq!(rule_problem(Some(&json!("project")), "/q/x", "/p", true).as_deref(), Some("Only folders inside p can have rules"));
        assert_eq!(rule_problem(Some(&Value::Null), "/q/x", "/p", false), None);
        assert_eq!(rule_problem(None, "/q/x", "/p", false).as_deref(), Some("unknown rule"));
        assert_eq!(identities_note(&[json!({ "kind": "git" }), json!({ "kind": "git" }), json!({ "kind": "claude" })]), "2 git, 1 claude");
    }
}
