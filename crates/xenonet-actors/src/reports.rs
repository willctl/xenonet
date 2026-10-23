//! What a registered actor reports, with its token (docs/design/actors/
//! interface.md, section 2: Heartbeat, Events, Targets), taken into the
//! registry a part at a time: core/actors/reports.js. A part that can't be
//! taken is dropped with a reason and doesn't fail the rest.
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
//! The registry and the rest are called as the JS called them, through
//! `Calls`: "actors" is the registry, and the twin gives "own" (the
//! actor's processes, a Set), "lineage" (how to read a process's lineage),
//! "skip" (the indexes already dropped), "awaitSession" (the registry's,
//! with `own` as given) and "removeList" (what to take away, read when
//! the JS read it).

use crate::detail::status_line;
use crate::registration::{to_number, ActorError, Fail, Result};
use crate::registry::node_kind;
use serde_json::json;
use xenonet_base::ids::is_session_id;
use xenonet_base::nodepath::{style_of, NATIVE};
use xenonet_engine::peer::{list, opt_call, Calls, Thrown};
use xenonet_engine::sql::Arg;
use xenonet_engine::store::js_text;

const DAY_MS: f64 = 24.0 * 3600e3;
const AHEAD_MS: f64 = 60e3;
const KINDS: [&str; 6] = ["spawned", "edited", "ran", "messaged", "finished", "failed"];
const PLATFORMS: [&str; 12] = ["telegram", "discord", "slack", "whatsapp", "signal", "matrix", "mattermost", "email", "sms", "imessage", "teams", "other"];
const SUBJECTS: [&str; 5] = ["session", "task", "job", "turn", "command"];
const APPS: [&str; 3] = ["claude", "codex", "other"];

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

fn one_of(set: &[&str], v: &Arg) -> bool {
    v.as_str().is_some_and(|s| set.contains(&s))
}

/// nodeKind(id), with String(id ?? "") as the JS read it.
fn kind_of(id: &Arg) -> Option<&'static str> {
    node_kind(&if id.is_nullish() { String::new() } else { js_text(id) })
}

/// `a === b`.
fn strict_eq(a: &Arg, b: &Arg) -> bool {
    match (a, b) {
        (Arg::Num(x), Arg::Num(y)) => x == y,
        (Arg::Text(x), Arg::Text(y)) => x == y,
        (Arg::Bool(x), Arg::Bool(y)) => x == y,
        (Arg::Big(x), Arg::Big(y)) => x == y,
        (Arg::Undef, Arg::Undef) | (Arg::Null, Arg::Null) => true,
        _ => false,
    }
}

/// Set#has's SameValueZero.
fn same_value_zero(a: &Arg, b: &Arg) -> bool {
    match (a, b) {
        (Arg::Num(x), Arg::Num(y)) => x == y || (x.is_nan() && y.is_nan()),
        _ => strict_eq(a, b),
    }
}

/// What Node's path.isAbsolute throws for a path that isn't text.
fn not_a_path(v: &Arg) -> Thrown {
    let received = match v {
        Arg::Null => "null".to_string(),
        Arg::Undef => "undefined".to_string(),
        Arg::Num(n) => format!("type number ({})", js_text(&Arg::Num(*n))),
        Arg::Big(n) => format!("type bigint ({n}n)"),
        Arg::Bool(b) => format!("type boolean ({b})"),
        Arg::List(_) => "an instance of Array".to_string(),
        Arg::Bytes(_) => "an instance of Buffer".to_string(),
        _ => "an instance of Object".to_string(),
    };
    Thrown::coded(format!("The \"path\" argument must be of type string. Received {received}"), "ERR_INVALID_ARG_TYPE")
}

/// path.isAbsolute(p) on this machine.
fn absolute(p: &Arg) -> Result<bool> {
    match p {
        Arg::Text(s) => Ok(NATIVE.is_absolute(s)),
        other => Err(not_a_path(other).into()),
    }
}

/// `pathStyle(p).resolve(p)`, for a path already judged absolute.
fn resolved(p: &Arg) -> Arg {
    let p = js_text(p);
    Arg::text(style_of(&p).resolve(&[&p]))
}

/// Whether `<path>` is truthy, read without converting an object.
fn truthy_at(c: &mut dyn Calls, path: &str) -> Result<bool> {
    Ok(match c.kind(path)?.as_str() {
        "undefined" | "null" => false,
        "function" | "object" | "symbol" => true,
        _ => c.get(path)?.truthy(),
    })
}

enum Node {
    Id(Arg),
    Why(&'static str),
}

/// A node of the map from a report's `target` (a node id) or `path` (an
/// absolute path, landing on the project or folder holding it), or why it
/// can't be one. A session not on the map yet is given two minutes (the
/// registry keeps it that long), in case its transcript is on the way.
fn node_of(c: &mut dyn Calls, target: &Arg, p: &Arg) -> Result<Node> {
    if !target.is_nullish() {
        let kind = kind_of(target);
        if kind.is_none() || kind == Some("actor") {
            return Ok(Node::Why("a target is a project, folder or session on XenoNet's map, never an actor"));
        }
        if kind == Some("session") && !is_session_id(&js_text(target)) {
            return Ok(Node::Why("that isn't a session's id"));
        }
        // Before XenoNet's first look at the map, at start, every node is
        // taken; the registry lets go of the ones that never turn up.
        let ready = truthy_at(c, "actors.map.has")? && opt_call(c, "actors.map", "ready", vec![])? != Arg::Bool(false);
        if kind != Some("session") && ready && !c.call("actors.map", "has", vec![target.clone()])?.truthy() {
            return Ok(Node::Why("XenoNet doesn't know that node"));
        }
        return Ok(Node::Id(target.clone()));
    }
    if !p.is_nullish() {
        if !absolute(p)? {
            return Ok(Node::Why("a path is absolute"));
        }
        let id = c.call("actors", "placeOf", vec![p.clone()])?;
        return Ok(if id.truthy() { Node::Id(id) } else { Node::Why("that path isn't in a project or folder XenoNet knows") });
    }
    Ok(Node::Id(Arg::Null))
}

/// Whether a session the actor says it spawned runs under its own
/// processes ("own"): the session's pins (the processes XenoNet saw
/// holding it) against their lineage. None when XenoNet can't tell: not
/// macOS, no pins, or the actor's processes unknown.
fn holds_session(c: &mut dyn Calls, session: &Arg) -> Result<Option<bool>> {
    let has_own = !matches!(c.kind("own")?.as_str(), "undefined" | "null") && truthy_at(c, "own.size")?;
    if !has_own || !truthy_at(c, "lineage")? {
        return Ok(None);
    }
    let mut seen = false;
    for pin in list(c.call("actors.store", "pins", vec![session.clone()])?) {
        let line = c.call("", "lineage", vec![pin.field("pid")])?;
        if line.is_nullish() {
            return Ok(None);
        }
        let line = list(line);
        if line.is_empty() {
            continue;
        }
        seen = true;
        for pid in line {
            if c.call("own", "has", vec![pid])?.truthy() {
                return Ok(Some(true));
            }
        }
    }
    Ok(if seen { Some(false) } else { None })
}

/// Whether index `i` is one of those already dropped (`skip?.has(i)`).
fn skipped(c: &mut dyn Calls, i: usize) -> Result<bool> {
    if matches!(c.kind("skip")?.as_str(), "undefined" | "null") {
        return Ok(false);
    }
    Ok(c.call("skip", "has", vec![Arg::num(i as f64)])?.truthy())
}

/// A heartbeat (POST /api/actors/heartbeat): presence, a status, the folder
/// it works in and its process, for one of its instances. A status that
/// looks like it holds a key is refused (400); a folder outside every place
/// XenoNet knows is left out. Paths are kept as they resolve (no .. in
/// them), the form placeOf judged.
pub fn take_heartbeat(c: &mut dyn Calls, id: &Arg, body: &Arg, now: &Arg) -> Result<()> {
    let status = body.field("status");
    if !matches!(status, Arg::Undef) {
        if let Err(reason) = status_line(Some(&status.to_json())) {
            return Err(Fail::Actor(ActorError { status: 400.0, code: "bad-request", message: reason.into(), extra: json!({}) }));
        }
    }
    let cwd = body.field("cwd");
    if !matches!(cwd, Arg::Undef) && !absolute(&cwd)? {
        return Err(Fail::Actor(ActorError { status: 400.0, code: "bad-request", message: "cwd is an absolute path".into(), extra: json!({}) }));
    }
    let cwd = match cwd {
        Arg::Undef => Arg::Undef,
        cwd if c.call("actors", "placeOf", vec![cwd.clone()])?.truthy() => resolved(&cwd),
        _ => Arg::Null,
    };
    let instance = body.field("instance").or(Arg::text("main"));
    let report = Arg::Obj(vec![
        ("instance".into(), instance),
        ("presence".into(), body.field("presence")),
        ("status".into(), status),
        ("pid".into(), body.field("pid")),
        ("cwd".into(), cwd),
        ("via".into(), Arg::text("registered")),
        ("at".into(), now.clone()),
    ]);
    c.call("actors", "report", vec![id.clone(), report])?;
    Ok(())
}

/// What a report took: how many parts, and the ones dropped, by index.
#[derive(Debug, Default, PartialEq)]
pub struct Taken {
    pub accepted: usize,
    pub dropped: Vec<(usize, String)>,
}

impl Taken {
    pub fn to_arg(&self) -> Arg {
        let dropped = self.dropped.iter().map(|(i, r)| Arg::Obj(vec![("index".into(), Arg::num(*i as f64)), ("reason".into(), Arg::text(r.clone()))])).collect();
        Arg::Obj(vec![("accepted".into(), Arg::num(self.accepted as f64)), ("dropped".into(), Arg::List(dropped))])
    }
}

/// Edited files (`e.paths?.length`), as the JS read them: None for none.
fn paths_of(e: &Arg) -> Result<Option<Vec<Arg>>> {
    match e.field("paths") {
        Arg::List(items) if !items.is_empty() => Ok(Some(items)),
        Arg::List(_) | Arg::Undef | Arg::Null => Ok(None),
        // Text has a length but nothing to call some() on.
        Arg::Text(s) if !s.is_empty() => Err(Thrown { message: "e.paths.some is not a function".into(), code: None, handle: None, stdout: None }.into()),
        _ => Ok(None),
    }
}

/// What it did (POST /api/actors/events), each as facts. `events` are the
/// report's, in order.
pub fn take_events(c: &mut dyn Calls, id: &Arg, events: &[Arg], now: &Arg) -> Result<Taken> {
    let n = to_number(now);
    let mut out = Taken::default();
    let mut refs: Vec<Arg> = Vec::new();
    for (index, e) in events.iter().enumerate() {
        if e.is_nullish() || skipped(c, index)? {
            continue;
        }
        let mut drop = |reason: String| out.dropped.push((index, reason));
        let kind = e.field("kind");
        if !one_of(&KINDS, &kind) {
            drop("XenoNet doesn't take that kind in a report: presence and status come from heartbeats, and said from the engine".into());
            continue;
        }
        let kind = kind.as_str().unwrap_or_default().to_string();
        if let Some(need) = needs(&kind) {
            if !c.call("actors", "can", vec![id.clone(), Arg::text(need)])?.truthy() {
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
        let mut at = e.field("at").or(now.clone());
        if n - to_number(&at) > DAY_MS {
            drop("that was more than a day ago".into());
            continue;
        }
        if to_number(&at) - n > AHEAD_MS {
            at = now.clone();
        }
        let reference = e.field("ref");
        if !reference.is_nullish()
            && (refs.iter().any(|r| same_value_zero(r, &reference)) || c.call("actors.store", "hasActorRef", vec![id.clone(), reference.clone(), Arg::num(n - DAY_MS)])?.truthy())
        {
            drop("an event with that ref was reported already".into());
            continue;
        }
        // Edited files: each in a place XenoNet knows.
        let mut paths = None;
        if kind == "edited" {
            if let Some(items) = paths_of(e)? {
                if items.iter().any(|p| match p {
                    Arg::Text(s) => !NATIVE.is_absolute(s),
                    _ => true,
                }) {
                    drop("an edited path is absolute".into());
                    continue;
                }
                let mut outside = false;
                for p in &items {
                    if !c.call("actors", "placeOf", vec![p.clone()])?.truthy() {
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
        let node = match node_of(c, &e.field("target"), &e.field("path"))? {
            Node::Why(why) => {
                drop(why.into());
                continue;
            }
            Node::Id(id) => id,
        };
        // Without a place of its own, an edit lands where its first file is.
        let target = match (&node, &paths) {
            (n, _) if !n.is_nullish() => node.clone(),
            (_, Some(p)) => c.call("actors", "placeOf", vec![p[0].clone()])?,
            _ => Arg::Null,
        };
        if kind == "spawned" && kind_of(&target) == Some("session") && holds_session(c, &target)? == Some(false) {
            drop("that session runs under a process outside this actor's own".into());
            continue;
        }
        if !reference.is_nullish() {
            refs.push(reference.clone());
        }
        let app_given = e.field("app");
        let app = if kind == "spawned" && !app_given.is_nullish() {
            if one_of(&APPS, &app_given) {
                app_given
            } else {
                Arg::text("other")
            }
        } else {
            Arg::Undef
        };
        let platform = if kind == "messaged" {
            let p = e.field("platform");
            if one_of(&PLATFORMS, &p) {
                p
            } else if p.is_nullish() {
                Arg::Undef
            } else {
                Arg::text("other")
            }
        } else {
            Arg::Undef
        };
        let subject = e.field("subject");
        let subject = if one_of(&SUBJECTS, &subject) { subject } else { Arg::Undef };
        let record = Arg::Obj(vec![
            ("kind".into(), Arg::text(kind.clone())),
            ("target".into(), target.clone()),
            ("via".into(), Arg::text("registered")),
            ("at".into(), at.clone()),
            ("ref".into(), reference.or(Arg::Null)),
            ("count".into(), e.field("count")),
            ("paths".into(), paths.map_or(Arg::Undef, Arg::List)),
            ("platform".into(), platform),
            ("subject".into(), subject),
            ("app".into(), app.clone()),
        ]);
        c.call("actors", "record", vec![id.clone(), record])?;
        // A session it started that it didn't name: one that starts under its
        // own processes in that place in the next two minutes is the one.
        if kind == "spawned" && kind_of(&target) != Some("session") && app.as_str() != Some("other") {
            c.call("", "awaitSession", vec![id.clone(), app.or(Arg::Null), target, at])?;
        }
        out.accepted += 1;
    }
    Ok(out)
}

/// What it acts on (POST /api/actors/targets): `targets` merged, or with
/// `replace` the whole list of what an actor may set, and "removeList"
/// taken away. The dropped indexes are into `targets`.
pub fn take_targets(c: &mut dyn Calls, id: &Arg, targets: &[Arg], replace: bool, now: &Arg) -> Result<Taken> {
    let mut out = Taken::default();
    let mut keep: Vec<(Arg, Arg)> = Vec::new();
    for (index, t) in targets.iter().enumerate() {
        if t.is_nullish() || skipped(c, index)? {
            continue;
        }
        let mut drop = |reason: String| out.dropped.push((index, reason));
        let relation = t.field("relation").or(Arg::text("works-on"));
        let name = js_text(&relation);
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
            if !c.call("actors", "can", vec![id.clone(), Arg::text(need)])?.truthy() {
                drop(format!("{name} needs the {need} capability, which it wasn't allowed"));
                continue;
            }
        }
        let node = match node_of(c, &t.field("id"), &t.field("path"))? {
            Node::Why(why) => {
                drop(why.into());
                continue;
            }
            Node::Id(n) if !n.truthy() => {
                drop("a target names a node (id) or a path".into());
                continue;
            }
            Node::Id(n) => n,
        };
        let kind = kind_of(&node);
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
        if name == "spawned" && holds_session(c, &node)? == Some(false) {
            drop("that session runs under a process outside this actor's own".into());
            continue;
        }
        keep.push((node, relation));
    }
    let mine = |relation: &Arg| points_at(&js_text(relation)).is_some();
    if replace {
        let wanted: Vec<String> = keep.iter().map(|(n, r)| format!("{}|{}", js_text(r), js_text(n))).collect();
        for t in list(c.call("actors", "targetsOf", vec![id.clone()])?) {
            let (tid, rel) = (t.field("id"), t.field("relation"));
            if mine(&rel) && !wanted.contains(&format!("{}|{}", js_text(&rel), js_text(&tid))) {
                c.call("actors", "untarget", vec![id.clone(), tid, rel])?;
            }
        }
    }
    for r in list(c.call("", "removeList", vec![])?) {
        let node = match node_of(c, &r.field("id"), &r.field("path"))? {
            Node::Id(n) if n.truthy() => n,
            _ => continue,
        };
        let only = r.field("relation");
        for t in list(c.call("actors", "targetsOf", vec![id.clone()])?) {
            let (tid, rel) = (t.field("id"), t.field("relation"));
            if strict_eq(&tid, &node) && mine(&rel) && (only.is_nullish() || strict_eq(&only, &rel)) {
                c.call("actors", "untarget", vec![id.clone(), tid, rel])?;
            }
        }
    }
    for (node, relation) in &keep {
        c.call("actors", "target", vec![id.clone(), node.clone(), relation.clone(), now.clone()])?;
    }
    out.accepted = keep.len();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_isnt_text_is_refused_as_node_refuses_it() {
        assert_eq!(not_a_path(&Arg::num(5.0)).message, "The \"path\" argument must be of type string. Received type number (5)");
        assert_eq!(not_a_path(&Arg::Null).message, "The \"path\" argument must be of type string. Received null");
    }

    #[test]
    fn each_relation_says_what_it_points_at() {
        let words: Vec<&str> = points_at("watches").unwrap().iter().map(|k| word(Some(k))).collect();
        assert_eq!(words, ["a project", "a project", "a folder", "a session"]);
        assert_eq!(word(None), "undefined");
    }

    #[test]
    fn refs_compare_as_a_set_does() {
        assert!(same_value_zero(&Arg::num(f64::NAN), &Arg::num(f64::NAN)));
        assert!(!same_value_zero(&Arg::num(1.0), &Arg::text("1")));
    }
}
