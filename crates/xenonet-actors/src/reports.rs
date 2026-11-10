//! What a registered actor reports, with its token (docs/design/actors/
//! interface.md, section 2: Heartbeat, Events, Targets), taken into the
//! registry a part at a time. A part that can't be taken is dropped with a
//! reason and doesn't fail the rest.
//!
//! An actor reports facts, never text: XenoNet writes every event's detail
//! from them. Its paths land on the project or folder holding them, and a
//! path outside every place XenoNet knows is dropped, so an actor can't add
//! places to the map. Each kind and relation needs the capability you
//! allowed it, and on macOS a claim to have spawned a session held by a
//! process outside the actor's own is dropped, while a spawned report that
//! names no session waits two minutes for one to start under the actor's
//! own processes (Actors#awaitSession).
//!
//! What a report asks of the registry, the map, the engine's pins and the
//! process table goes through `Host`. A report's parts come as JSON: a key
//! left out is one the actor didn't send, a null one it did.

use crate::detail::status_line;
use crate::registration::{ActorError, Fail, Result};
use crate::registry::node_kind;
use crate::store;
use serde_json::{json, Map, Value};
use xenonet_adapter::apps::is_session_id;
use xenonet_os::nodepath::{style_of, NATIVE};
use xenonet_text::json::number;
use xenonet_text::text::{display, truthy};

const DAY_MS: f64 = 24.0 * 3600e3;
const AHEAD_MS: f64 = 60e3;
const KINDS: [&str; 6] = ["spawned", "edited", "ran", "messaged", "finished", "failed"];
const PLATFORMS: [&str; 12] = ["telegram", "discord", "slack", "whatsapp", "signal", "matrix", "mattermost", "email", "sms", "imessage", "teams", "other"];
const SUBJECTS: [&str; 5] = ["session", "task", "job", "turn", "command"];

/// What a report asks of the world: the registry, the map, the engine's
/// pins, and the actor's own processes.
pub trait Host {
    /// Whether the actor was allowed a capability.
    fn can(&mut self, id: &str, capability: &str) -> bool;
    /// The project or folder holding an absolute path, as a node id, or
    /// null.
    fn place_of(&mut self, path: &Value) -> Value;
    /// One of the actor's instances says it's there.
    fn report(&mut self, id: &str, report: &Value) -> Result<()>;
    /// Something the actor did, as facts.
    fn record(&mut self, id: &str, record: &Value) -> Result<()>;
    /// The nodes the actor acts on ({ id, relation }).
    fn targets_of(&mut self, id: &str) -> Vec<Value>;
    fn untarget(&mut self, id: &str, node: &str, relation: &str) -> Result<()>;
    fn target(&mut self, id: &str, node: &str, relation: &str, at: f64) -> Result<()>;
    /// The processes XenoNet saw holding a session (each with its `pid`).
    fn pins(&mut self, session: &str) -> Result<Vec<Value>>;
    /// Whether the actor reported an event with this ref since `since`.
    fn has_actor_ref(&mut self, id: &str, r: &Value, since: f64) -> Result<bool>;
    /// Whether the map has had its first look, so a node it hasn't is one
    /// it doesn't know.
    fn map_ready(&mut self) -> bool;
    fn map_has(&mut self, id: &str) -> bool;
    /// Whether the actor's own processes are known (and there are some).
    fn own_known(&mut self) -> bool;
    /// Whether a process is one of the actor's own; None when they aren't
    /// known.
    fn owns(&mut self, pid: &Value) -> Option<bool>;
    /// A process and the ones above it, as pids; None where XenoNet can't
    /// look.
    fn lineage(&mut self, pid: &Value) -> Option<Vec<Value>>;
    /// Whether the part at `index` was dropped already, before the report
    /// got here.
    fn skipped(&mut self, index: usize) -> bool;
    /// A spawned report that named no session waits for one to start under
    /// the actor's own processes in that place.
    fn await_session(&mut self, id: &str, app: &Value, place: &Value, at: &Value) -> Result<()>;
    /// The targets to take away (each { id | path, relation? }).
    fn remove_list(&mut self) -> Vec<Value>;
}

/// The capability each kind of event, and each relation, needs.
fn needs(kind: &str) -> Option<&'static str> {
    match kind {
        "spawned" => Some("spawn-sessions"),
        "edited" => Some("edit-repos"),
        "ran" => Some("run-commands"),
        "messaged" => Some("message-people"),
        _ => None,
    }
}

/// The relations an actor may set (messaged is the engine's), and what each
/// points at (the interface's table of relations).
fn points_at(relation: &str) -> Option<&'static [&'static str]> {
    match relation {
        "works-on" => Some(&["project", "native", "zone"]),
        "spawned" => Some(&["session"]),
        "edited" => Some(&["project", "zone"]),
        "watches" => Some(&["project", "native", "zone", "session"]),
        "reviews" => Some(&["session", "project"]),
        _ => None,
    }
}

fn word(kind: Option<&str>) -> &'static str {
    match kind {
        Some("project" | "native") => "a project",
        Some("zone") => "a folder",
        Some("session") => "a session",
        _ => "undefined",
    }
}

/// Number(v), as a time in a report is compared with the clock.
fn to_number(v: &Value) -> f64 {
    xenonet_text::text::to_number(Some(v))
}

/// Whether a field is missing or null.
fn nullish(v: Option<&Value>) -> bool {
    v.is_none_or(Value::is_null)
}

fn one_of(set: &[&str], v: Option<&Value>) -> bool {
    v.and_then(Value::as_str).is_some_and(|s| set.contains(&s))
}

/// nodeKind(id), with `String(id ?? "")`.
fn kind_of(id: Option<&Value>) -> Option<&'static str> {
    node_kind(&if nullish(id) { String::new() } else { display(id.unwrap_or(&Value::Null)) })
}

/// `a === b` for two values a report may hold.
fn strict_eq(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Null, Value::Null) => true,
        _ => false,
    }
}

/// Set#has's SameValueZero: as `===`, with NaN equal to itself.
fn same_value_zero(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64(), y.as_f64());
            x == y || (x.is_some_and(f64::is_nan) && y.is_some_and(f64::is_nan))
        }
        _ => strict_eq(a, b),
    }
}

/// What a path that isn't text is refused with: the type it was, in the
/// words of the runtime that checked it.
fn not_a_path(v: &Value) -> store::Error {
    let received = match v {
        Value::Null => "null".to_string(),
        Value::Number(_) => format!("type number ({})", display(v)),
        Value::Bool(b) => format!("type boolean ({b})"),
        Value::Array(_) => "an instance of Array".to_string(),
        Value::Object(_) => "an instance of Object".to_string(),
        Value::String(s) => s.clone(),
    };
    store::Error::coded(format!("The \"path\" argument must be of type string. Received {received}"), "ERR_INVALID_ARG_TYPE")
}

/// Whether a path is absolute on this machine; a path that isn't text is
/// refused.
fn absolute(p: &Value) -> Result<bool> {
    match p {
        Value::String(s) => Ok(NATIVE.is_absolute(s)),
        other => Err(Fail::Error(not_a_path(other))),
    }
}

/// A path already judged absolute, resolved in its own style.
fn resolved(p: &Value) -> Value {
    let p = display(p);
    Value::String(style_of(&p).resolve(&[&p]))
}

enum Node {
    Id(Value),
    Why(&'static str),
}

/// A node of the map from a report's `target` (a node id) or `path` (an
/// absolute path, landing on the project or folder holding it), or why it
/// can't be one. A session not on the map yet is given two minutes (the
/// registry keeps it that long), in case its transcript is on the way.
fn node_of(host: &mut dyn Host, target: Option<&Value>, p: Option<&Value>) -> Result<Node> {
    if !nullish(target) {
        let target = target.unwrap_or(&Value::Null);
        let kind = kind_of(Some(target));
        if kind.is_none() || kind == Some("actor") {
            return Ok(Node::Why("a target is a project, folder or session on XenoNet's map, never an actor"));
        }
        if kind == Some("session") && !is_session_id(&display(target)) {
            return Ok(Node::Why("that isn't a session's id"));
        }
        // Before XenoNet's first look at the map, at start, every node is
        // taken; the registry lets go of the ones that never turn up.
        if kind != Some("session") && host.map_ready() && !host.map_has(&display(target)) {
            return Ok(Node::Why("XenoNet doesn't know that node"));
        }
        return Ok(Node::Id(target.clone()));
    }
    if !nullish(p) {
        let p = p.unwrap_or(&Value::Null);
        if !absolute(p)? {
            return Ok(Node::Why("a path is absolute"));
        }
        let id = host.place_of(p);
        return Ok(if truthy(Some(&id)) { Node::Id(id) } else { Node::Why("that path isn't in a project or folder XenoNet knows") });
    }
    Ok(Node::Id(Value::Null))
}

/// Whether a session the actor says it spawned runs under its own
/// processes: the session's pins (the processes XenoNet saw holding it)
/// against their lineage. None when XenoNet can't tell: not macOS, no
/// pins, or the actor's processes unknown.
fn holds_session(host: &mut dyn Host, session: &Value) -> Result<Option<bool>> {
    if !host.own_known() {
        return Ok(None);
    }
    let mut seen = false;
    for pin in host.pins(&display(session))? {
        let Some(line) = host.lineage(pin.get("pid").unwrap_or(&Value::Null)) else { return Ok(None) };
        if line.is_empty() {
            continue;
        }
        seen = true;
        for pid in &line {
            if host.owns(pid) == Some(true) {
                return Ok(Some(true));
            }
        }
    }
    Ok(if seen { Some(false) } else { None })
}

/// A heartbeat (POST /api/actors/heartbeat): presence, a status, the folder
/// it works in and its process, for one of its instances. A status that
/// looks like it holds a key is refused (400); a folder outside every place
/// XenoNet knows is left out. Paths are kept as they resolve (no .. in
/// them), the form placeOf judged.
pub fn take_heartbeat(host: &mut dyn Host, id: &str, body: &Value, now: f64) -> Result<()> {
    let status = body.get("status");
    if let Some(status) = status {
        if let Err(reason) = status_line(Some(status)) {
            return Err(Fail::Actor(ActorError { status: 400.0, code: "bad-request", message: reason.into(), extra: json!({}) }));
        }
    }
    let cwd = body.get("cwd");
    if let Some(cwd) = cwd {
        if !absolute(cwd)? {
            return Err(Fail::Actor(ActorError { status: 400.0, code: "bad-request", message: "cwd is an absolute path".into(), extra: json!({}) }));
        }
    }
    let cwd = cwd.map(|cwd| if truthy(Some(&host.place_of(cwd))) { resolved(cwd) } else { Value::Null });
    let mut report = Map::new();
    report.insert("instance".into(), body.get("instance").filter(|i| !i.is_null()).cloned().unwrap_or_else(|| json!("main")));
    if let Some(p) = body.get("presence") {
        report.insert("presence".into(), p.clone());
    }
    if let Some(s) = status {
        report.insert("status".into(), s.clone());
    }
    if let Some(p) = body.get("pid") {
        report.insert("pid".into(), p.clone());
    }
    if let Some(c) = cwd {
        report.insert("cwd".into(), c);
    }
    report.insert("via".into(), json!("registered"));
    report.insert("at".into(), number(now));
    host.report(id, &Value::Object(report))?;
    Ok(())
}

/// What a report took: how many parts, and the ones dropped, by index.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    pub accepted: usize,
    pub dropped: Vec<(usize, String)>,
}

impl Taken {
    pub fn to_json(&self) -> Value {
        let dropped: Vec<Value> = self.dropped.iter().map(|(i, r)| json!({ "index": number(*i as f64), "reason": r })).collect();
        json!({ "accepted": number(self.accepted as f64), "dropped": dropped })
    }
}

/// Edited files (`e.paths`, when there are some): None for none. Text has
/// a length but no items to go through, so it's refused.
fn paths_of(e: &Value) -> Result<Option<Vec<Value>>> {
    match e.get("paths") {
        Some(Value::Array(items)) if !items.is_empty() => Ok(Some(items.clone())),
        Some(Value::String(s)) if !s.is_empty() => Err(Fail::Error(store::Error::new("e.paths.some is not a function"))),
        _ => Ok(None),
    }
}

/// What it did (POST /api/actors/events), each as facts. `events` are the
/// report's, in order, null where a part was dropped already.
pub fn take_events(host: &mut dyn Host, id: &str, events: &[Value], now: f64) -> Result<Taken> {
    let mut out = Taken::default();
    let mut refs: Vec<Value> = Vec::new();
    for (index, e) in events.iter().enumerate() {
        if e.is_null() || host.skipped(index) {
            continue;
        }
        let mut drop = |reason: String| out.dropped.push((index, reason));
        let kind = e.get("kind");
        if !one_of(&KINDS, kind) {
            drop("XenoNet doesn't take that kind in a report: presence and status come from heartbeats, and said from the engine".into());
            continue;
        }
        let kind = kind.and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(need) = needs(&kind) {
            if !host.can(id, need) {
                // A message to someone outside XenoNet is easily mistaken for
                // one to an agent, which is the engine's, so the refusal says
                // which is which.
                let aside = if kind == "messaged" {
                    " (messaged is a message to someone outside XenoNet, by email or a chat app; to write to a session or another actor, use xenonet actor send)"
                } else {
                    ""
                };
                drop(format!("{kind} needs the {need} capability, which it wasn't allowed{aside}"));
                continue;
            }
        }
        let mut at = e.get("at").filter(|a| !a.is_null()).cloned().unwrap_or_else(|| number(now));
        if now - to_number(&at) > DAY_MS {
            drop("that was more than a day ago".into());
            continue;
        }
        if to_number(&at) - now > AHEAD_MS {
            at = number(now);
        }
        let reference = e.get("ref").filter(|r| !r.is_null());
        if let Some(r) = reference {
            if refs.iter().any(|x| same_value_zero(x, r)) || host.has_actor_ref(id, r, now - DAY_MS)? {
                drop("an event with that ref was reported already".into());
                continue;
            }
        }
        // Edited files: each in a place XenoNet knows.
        let mut paths = None;
        if kind == "edited" {
            if let Some(items) = paths_of(e)? {
                if items.iter().any(|p| match p {
                    Value::String(s) => !NATIVE.is_absolute(s),
                    _ => true,
                }) {
                    drop("an edited path is absolute".into());
                    continue;
                }
                let mut outside = false;
                for p in &items {
                    if !truthy(Some(&host.place_of(p))) {
                        outside = true;
                        break;
                    }
                }
                if outside {
                    drop("an edited path isn't in a project or folder XenoNet knows".into());
                    continue;
                }
                paths = Some(items.iter().map(resolved).collect::<Vec<_>>());
            }
        }
        let node = match node_of(host, e.get("target"), e.get("path"))? {
            Node::Why(why) => {
                drop(why.into());
                continue;
            }
            Node::Id(id) => id,
        };
        // Without a place of its own, an edit lands where its first file is.
        let target = match (&node, &paths) {
            (n, _) if !n.is_null() => node.clone(),
            (_, Some(p)) => host.place_of(&p[0]),
            _ => Value::Null,
        };
        if kind == "spawned" && kind_of(Some(&target)) == Some("session") && holds_session(host, &target)? == Some(false) {
            drop("that session runs under a process outside this actor's own".into());
            continue;
        }
        if let Some(r) = reference {
            refs.push(r.clone());
        }
        let app_given = e.get("app");
        let app = if kind == "spawned" && !nullish(app_given) {
            // The apps an actor's events name are those XenoNet hands
            // sessions to; any other is "other".
            if app_given.and_then(Value::as_str).is_some_and(|a| xenonet_adapter::apps::asked_or_text(Some(a), None).is_some()) {
                app_given.cloned()
            } else {
                Some(json!("other"))
            }
        } else {
            None
        };
        let platform = if kind == "messaged" {
            let p = e.get("platform");
            if one_of(&PLATFORMS, p) {
                p.cloned()
            } else if nullish(p) {
                None
            } else {
                Some(json!("other"))
            }
        } else {
            None
        };
        let subject = e.get("subject").filter(|s| one_of(&SUBJECTS, Some(s))).cloned();
        let mut record = Map::new();
        record.insert("kind".into(), json!(kind));
        record.insert("target".into(), target.clone());
        record.insert("via".into(), json!("registered"));
        record.insert("at".into(), at.clone());
        record.insert("ref".into(), reference.cloned().unwrap_or(Value::Null));
        if let Some(count) = e.get("count") {
            record.insert("count".into(), count.clone());
        }
        if let Some(p) = paths {
            record.insert("paths".into(), Value::Array(p));
        }
        if let Some(p) = platform {
            record.insert("platform".into(), p);
        }
        if let Some(s) = subject {
            record.insert("subject".into(), s);
        }
        if let Some(a) = &app {
            record.insert("app".into(), a.clone());
        }
        host.record(id, &Value::Object(record))?;
        // A session it started that it didn't name: one that starts under its
        // own processes in that place in the next two minutes is the one.
        if kind == "spawned" && kind_of(Some(&target)) != Some("session") && app.as_ref().and_then(Value::as_str) != Some("other") {
            host.await_session(id, app.as_ref().unwrap_or(&Value::Null), &target, &at)?;
        }
        out.accepted += 1;
    }
    Ok(out)
}

/// What it acts on (POST /api/actors/targets): `targets` merged, or with
/// `replace` the whole list of what an actor may set, and the host's
/// remove list taken away. The dropped indexes are into `targets`.
pub fn take_targets(host: &mut dyn Host, id: &str, targets: &[Value], replace: bool, now: f64) -> Result<Taken> {
    let mut out = Taken::default();
    let mut keep: Vec<(Value, Value)> = Vec::new();
    for (index, t) in targets.iter().enumerate() {
        if t.is_null() || host.skipped(index) {
            continue;
        }
        let mut drop = |reason: String| out.dropped.push((index, reason));
        let relation = t.get("relation").filter(|r| !r.is_null()).cloned().unwrap_or_else(|| json!("works-on"));
        let name = display(&relation);
        let Some(points) = points_at(&name) else {
            drop(
                if relation.as_str() == Some("messaged") {
                    "messaged is the engine's to set, when the actor sends a message"
                } else {
                    "an actor sets works-on, spawned, edited, watches or reviews"
                }
                .into(),
            );
            continue;
        };
        if let Some(need) = needs(&name) {
            if !host.can(id, need) {
                drop(format!("{name} needs the {need} capability, which it wasn't allowed"));
                continue;
            }
        }
        let node = match node_of(host, t.get("id"), t.get("path"))? {
            Node::Why(why) => {
                drop(why.into());
                continue;
            }
            Node::Id(n) if !truthy(Some(&n)) => {
                drop("a target names a node (id) or a path".into());
                continue;
            }
            Node::Id(n) => n,
        };
        let kind = kind_of(Some(&node));
        if !kind.is_some_and(|k| points.contains(&k)) {
            let mut words: Vec<&str> = Vec::new();
            for k in points {
                let w = word(Some(k));
                if !words.contains(&w) {
                    words.push(w);
                }
            }
            drop(format!("{name} points at {}, not {}", words.join(" or "), word(kind)));
            continue;
        }
        if name == "spawned" && holds_session(host, &node)? == Some(false) {
            drop("that session runs under a process outside this actor's own".into());
            continue;
        }
        keep.push((node, relation));
    }
    let mine = |relation: &Value| points_at(&display(relation)).is_some();
    let parts = |t: &Value| (t.get("id").cloned().unwrap_or(Value::Null), t.get("relation").cloned().unwrap_or(Value::Null));
    if replace {
        let wanted: Vec<String> = keep.iter().map(|(n, r)| format!("{}|{}", display(r), display(n))).collect();
        for t in host.targets_of(id) {
            let (tid, rel) = parts(&t);
            if mine(&rel) && !wanted.contains(&format!("{}|{}", display(&rel), display(&tid))) {
                host.untarget(id, &display(&tid), &display(&rel))?;
            }
        }
    }
    for r in host.remove_list() {
        let node = match node_of(host, r.get("id"), r.get("path"))? {
            Node::Id(n) if truthy(Some(&n)) => n,
            _ => continue,
        };
        let only = r.get("relation");
        for t in host.targets_of(id) {
            let (tid, rel) = parts(&t);
            if strict_eq(&tid, &node) && mine(&rel) && (nullish(only) || only.is_some_and(|o| strict_eq(o, &rel))) {
                host.untarget(id, &display(&tid), &display(&rel))?;
            }
        }
    }
    for (node, relation) in &keep {
        host.target(id, &display(node), &display(relation), now)?;
    }
    out.accepted = keep.len();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_isnt_text_is_refused_as_node_refuses_it() {
        assert_eq!(not_a_path(&json!(5)).message, "The \"path\" argument must be of type string. Received type number (5)");
        assert_eq!(not_a_path(&Value::Null).message, "The \"path\" argument must be of type string. Received null");
    }

    #[test]
    fn each_relation_says_what_it_points_at() {
        let words: Vec<&str> = points_at("watches").unwrap().iter().map(|k| word(Some(k))).collect();
        assert_eq!(words, ["a project", "a project", "a folder", "a session"]);
        assert_eq!(word(None), "undefined");
    }

    #[test]
    fn refs_compare_as_a_set_does() {
        assert!(same_value_zero(&json!(1), &json!(1.0)));
        assert!(!same_value_zero(&json!(1), &json!("1")));
    }
}
