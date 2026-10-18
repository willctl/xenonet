//! core/hosts/relay.js: xenonet relay, this XenoNet's map for a console on
//! another machine, over the ssh connection the console opened. It reads
//! the map, the live state and the engine's from 127.0.0.1, follows the
//! event stream, and writes it all as frames on its standard output; on
//! its standard input it reads only the console's requests, and answers
//! them from an allowlist of GET routes. It never opens a port, never
//! writes to XenoNet, and never sends the home folder: every path under it
//! goes as ~, and so does the name of any other folder named for a person
//! that the map's places are in.
//!
//! What crosses by default is titles, states and counts: prompts,
//! messages, a waiting question's text, tool details, subagents' tasks and
//! the files sessions touch stay here unless the console asked for
//! --detail. Only this XenoNet's own view leaves: anything qualified with
//! another host is dropped, so data never passes from one machine to the
//! next.
//!
//! This has the shaping, the allowlist and the relay's decisions; the
//! twin wires the streams, the HTTP calls and the timers.

use crate::merge::repo_key;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use xenonet_app::jstext::utf16_len;
use xenonet_app::jsv::truthy;
use xenonet_base::paths::path_key;
use xenonet_base::text::{js_number, js_string};

pub const PING_MS: f64 = 15e3;
pub const DEFAULT_SINCE_MS: f64 = 7.0 * 86400e3;
pub const GET_TIMEOUT_MS: f64 = 20e3;
pub const MAX_GETS: usize = 4;
/// What waits while the console reads slowly, besides the newest map and
/// live state: other events up to this many bytes.
pub const QUEUE_BYTES: usize = 4 << 20;
pub const EXIT_DONE: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_NO_XENONET: i32 = 3;
pub const EXIT_XENONET_GONE: i32 = 4;
pub const EXIT_OLD_XENONET: i32 = 5;

/// "7d" → 604800000; None for anything else.
pub fn parse_since(text: &str) -> Option<f64> {
    let (digits, unit) = text.split_at(text.len().checked_sub(1)?);
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let ms = match unit {
        "m" => 60e3,
        "h" => 3600e3,
        "d" => 86400e3,
        _ => return None,
    };
    Some(digits.parse::<f64>().ok()? * ms)
}

/// A key restricted to the relay in authorized_keys runs that command
/// whatever the console asked for, and ssh puts what it asked for in
/// SSH_ORIGINAL_COMMAND. Only --since is taken from there.
pub fn from_original_command(text: &str) -> Option<f64> {
    let words: Vec<&str> = xenonet_base::text::js_trim(text).split(|c: char| xenonet_base::text::is_js_space(c)).filter(|w| !w.is_empty()).collect();
    let at = words.iter().position(|w| *w == "--since")?;
    parse_since(words.get(at + 1).copied().unwrap_or("")).filter(|s| *s != 0.0)
}

// ---------- the home folder as ~ ----------

fn word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

// One character as /i compares them.
fn fold(c: char) -> char {
    let mut l = c.to_lowercase();
    match (l.next(), l.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}

// Where `form` matches `s` at byte `at`, case aside: the byte it ends at.
fn starts(s: &str, at: usize, form: &str) -> Option<usize> {
    let mut it = s[at..].char_indices();
    for f in form.chars() {
        let (_, c) = it.next()?;
        if fold(c) != fold(f) {
            return None;
        }
    }
    Some(it.next().map_or(s.len(), |(i, _)| at + i))
}

/// One of the scrubber's swaps: what it matches, and what may not follow.
enum Swap {
    /// Any of `forms` (in order), not followed by a stop: becomes ~.
    Whole { forms: Vec<String>, stops: fn(char) -> bool },
    /// One of `parents`, then `sep` (a separator, or none), then `name`,
    /// not followed by a stop: the parent part stays, the name becomes ~.
    Named { parents: Vec<String>, seps: &'static [char], name: String, stops: fn(char) -> bool },
}

fn stop_wdd(c: char) -> bool {
    word(c) || c == '.' || c == '-'
}

fn stop_wd(c: char) -> bool {
    word(c) || c == '.'
}

impl Swap {
    // The match at `at`: (where it ends, what it becomes).
    fn at(&self, s: &str, at: usize) -> Option<(usize, String)> {
        let free = |end: usize, stops: fn(char) -> bool| s[end..].chars().next().is_none_or(|c| !stops(c));
        match self {
            Swap::Whole { forms, stops } => forms.iter().find_map(|f| starts(s, at, f).filter(|e| free(*e, *stops)).map(|e| (e, "~".to_string()))),
            Swap::Named { parents, seps, name, stops } => parents.iter().find_map(|p| {
                let mut e = starts(s, at, p)?;
                if !seps.is_empty() {
                    let c = s[e..].chars().next()?;
                    if !seps.contains(&c) {
                        return None;
                    }
                    e += c.len_utf8();
                }
                let kept = s[at..e].to_string();
                let end = starts(s, e, name)?;
                free(end, *stops).then(|| (end, format!("{kept}~")))
            }),
        }
    }

    fn replace(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut i = 0;
        while let Some(c) = s[i..].chars().next() {
            if let Some((end, to)) = self.at(s, i) {
                out.push_str(&to);
                if end == i {
                    out.push(c);
                    i += c.len_utf8();
                } else {
                    i = end;
                }
                continue;
            }
            out.push(c);
            i += c.len_utf8();
        }
        out
    }
}

/// A copy of a value with the home folder written as ~ in every string and
/// every key, in each form it takes here: as XenoNet was given it, as a
/// path key, and as Claude Code names its project folders
/// (-Users-nova-Projects-atlas). `also` lists other folders named for a
/// person, which go with ~ in place of the name alone.
pub struct Scrubber {
    swaps: Vec<Swap>,
    shortest: usize,
}

fn dashed(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\\' | '/' | '.' | ':') { '-' } else { c }).collect()
}

fn unique(items: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

impl Scrubber {
    pub fn new(home: Option<&str>, also: &[String]) -> Self {
        let h = home.unwrap_or("").trim_end_matches(['\\', '/']).to_string();
        let mut swaps = Vec::new();
        let mut lengths = Vec::new();
        if utf16_len(&h) >= 2 {
            let mut forms = vec![h.clone(), h.replace('\\', "/")];
            if let Some(k) = path_key(&h) {
                forms.push(k);
            }
            let forms = unique(forms);
            lengths.extend(forms.iter().map(|f| utf16_len(f)));
            swaps.push(Swap::Whole { forms, stops: stop_wdd });
            swaps.push(Swap::Whole { forms: vec![dashed(&h)], stops: stop_wd });
        }
        for folder in also {
            let f = folder.trim_end_matches(['\\', '/']);
            let cut = f.rfind(['/', '\\']);
            let Some(cut) = cut.filter(|c| *c >= 1 && *c != f.len() - 1) else { continue };
            let (parent, name) = (&f[..cut], &f[cut + 1..]);
            let parents = unique(vec![parent.to_string(), parent.replace('\\', "/")]);
            swaps.push(Swap::Named { parents, seps: &['\\', '/'], name: name.to_string(), stops: stop_wdd });
            swaps.push(Swap::Named { parents: vec![format!("{}-", dashed(parent))], seps: &[], name: name.to_string(), stops: stop_wd });
            lengths.push(utf16_len(f));
        }
        let shortest = if swaps.is_empty() { usize::MAX } else { lengths.into_iter().min().unwrap_or(usize::MAX) };
        Scrubber { swaps, shortest }
    }

    pub fn text(&self, s: &str) -> String {
        if utf16_len(s) < self.shortest {
            return s.to_string();
        }
        self.swaps.iter().fold(s.to_string(), |t, sw| sw.replace(&t))
    }

    pub fn walk(&self, v: &Value) -> Value {
        match v {
            Value::String(s) => Value::String(self.text(s)),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.walk(x)).collect()),
            Value::Object(m) => {
                let mut out = Map::new();
                for (k, x) in m {
                    out.insert(self.text(k), self.walk(x));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// The folder named for a person that a path is in, when it isn't the home
/// folder: /Users/<name> or /home/<name>, after a drive if there is one
/// (C:\Users\nova, /c/Users/nova, or WSL's /mnt/c/Users/nova).
pub fn user_folder_of(p: Option<&Value>, home: Option<&str>) -> Option<String> {
    let s = p.and_then(Value::as_str).unwrap_or("");
    let found = user_folder(s)?;
    let key = path_key(&found).unwrap_or_default();
    let home_key = home.and_then(path_key).filter(|k| !k.is_empty());
    match home_key {
        Some(h) if key == h || key.starts_with(&format!("{h}/")) => None,
        _ => Some(found),
    }
}

fn sep(c: Option<char>) -> bool {
    matches!(c, Some('/' | '\\'))
}

// ^((?:[A-Za-z]:|\/mnt\/[A-Za-z]|\/[A-Za-z](?=[\\/]))?[\\/](?:Users|home)[\\/][^\\/]+)(?=[\\/]|$)/i
fn user_folder(s: &str) -> Option<String> {
    let b: Vec<char> = s.chars().collect();
    let at = |i: usize| b.get(i).copied();
    let letter = |i: usize| at(i).is_some_and(|c| c.is_ascii_alphabetic());
    let mut prefixes = Vec::new();
    if letter(0) && at(1) == Some(':') {
        prefixes.push(2);
    }
    if at(0) == Some('/') && starts(s, 1, "mnt/").is_some() && letter(5) {
        prefixes.push(6);
    }
    if at(0) == Some('/') && letter(1) && sep(at(2)) {
        prefixes.push(2);
    }
    prefixes.push(0);
    for p in prefixes {
        if !sep(at(p)) {
            continue;
        }
        let rest: String = b[p + 1..].iter().collect();
        let word_len = ["Users", "home"].iter().find_map(|w| starts(&rest, 0, w).map(|_| w.chars().count()));
        let Some(w) = word_len else { continue };
        let i = p + 1 + w;
        if !sep(at(i)) {
            continue;
        }
        let mut j = i + 1;
        while j < b.len() && !sep(at(j)) {
            j += 1;
        }
        if j == i + 1 {
            continue;
        }
        return Some(b[..j].iter().collect());
    }
    None
}

fn qualified(id: Option<&Value>) -> bool {
    id.and_then(Value::as_str).is_some_and(|s| s.starts_with('@'))
}

/// The folders named for a person, besides the home folder, that the
/// map's places are in, one each however it's written, sorted.
pub fn user_folders(graph: &Value, home: Option<&str>) -> Vec<String> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut add = |p: Option<&Value>| {
        if let Some(f) = user_folder_of(p, home) {
            let k = path_key(&f).unwrap_or_default();
            if !found.iter().any(|(fk, _)| *fk == k) {
                found.push((k, f));
            }
        }
    };
    if let Some(Value::Array(nodes)) = graph.get("nodes") {
        for n in nodes {
            if qualified(n.get("id")) {
                continue;
            }
            match n.get("kind").and_then(Value::as_str) {
                Some("core" | "zone") => add(n.get("path")),
                Some("native") => {
                    if let Some(Value::Array(roots)) = n.get("roots") {
                        for r in roots {
                            add(Some(r));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut out: Vec<String> = found.into_iter().map(|(_, f)| f).collect();
    out.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
    out
}

// ---------- titles, states and counts only ----------

const SUBAGENT: &str = "subagent";

// A stuck reason without the command in it.
fn plain_reason(r: &Value) -> Value {
    let s = js_string(r);
    if let Some(rest) = s.strip_prefix("ran ") {
        if let Some(tail) = rest.strip_suffix(" times in a row") {
            if let Some(sp) = tail.rfind(' ') {
                let (cmd, count) = (&tail[..sp], &tail[sp + 1..]);
                if !cmd.is_empty() && !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit()) && !cmd.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
                    return json!(format!("ran the same command {count} times in a row"));
                }
            }
        }
    }
    Value::String(s)
}

fn obj_mut(v: &mut Value) -> Option<&mut Map<String, Value>> {
    v.as_object_mut()
}

pub fn metadata_graph(mut g: Value) -> Value {
    if let Some(Value::Array(nodes)) = g.get_mut("nodes") {
        for n in nodes.iter_mut() {
            let kind = n.get("kind").and_then(Value::as_str).map(String::from);
            let Some(m) = obj_mut(n) else { continue };
            if kind.as_deref() == Some("session") && m.contains_key("preview") {
                m.insert("preview".into(), Value::Null);
            }
            if kind.as_deref() == Some("page") {
                m.insert("summary".into(), json!(""));
            }
        }
    }
    g
}

fn spread_with(v: &Value, k: &str, x: Value) -> Value {
    let mut m = v.as_object().cloned().unwrap_or_default();
    m.insert(k.into(), x);
    Value::Object(m)
}

pub fn metadata_live(mut l: Value) -> Value {
    if let Some(Value::Array(sessions)) = l.get_mut("sessions") {
        for s in sessions.iter_mut() {
            let Some(m) = obj_mut(s) else { continue };
            m.insert("lastMessage".into(), Value::Null);
            if let Some(nv) = m.get("needsYou").filter(|v| truthy(Some(v))).cloned() {
                m.insert("needsYou".into(), spread_with(&nv, "text", json!("")));
            }
            if let Some(t) = m.get("lastTool").filter(|v| truthy(Some(v))).cloned() {
                m.insert("lastTool".into(), spread_with(&t, "detail", json!("")));
            }
            m.insert("files".into(), json!([]));
            if let Some(st) = m.get("stuck").filter(|v| truthy(v.get("reasons"))).cloned() {
                let reasons: Vec<Value> = st["reasons"].as_array().map(|r| r.iter().map(plain_reason).collect()).unwrap_or_default();
                m.insert("stuck".into(), spread_with(&st, "reasons", Value::Array(reasons)));
            }
            if let Some(Value::Array(subs)) = m.get("subagents") {
                let subs: Vec<Value> = subs.iter().map(|a| spread_with(a, "label", json!(SUBAGENT))).collect();
                m.insert("subagents".into(), Value::Array(subs));
            }
        }
    }
    for k in ["collisions", "collab"] {
        if let Some(Value::Array(cs)) = l.get_mut(k) {
            for c in cs.iter_mut() {
                if let Some(m) = obj_mut(c) {
                    m.insert("files".into(), json!([]));
                    if k == "collisions" {
                        m.insert("paths".into(), json!([]));
                    }
                }
            }
        }
    }
    l
}

pub fn metadata_activity(mut a: Value) -> Value {
    let kind = a.get("kind").and_then(Value::as_str).map(String::from);
    if let Some(m) = obj_mut(&mut a) {
        if matches!(kind.as_deref(), Some("tool" | "prompt" | "message" | "needs-you")) {
            m.insert("detail".into(), json!(""));
        }
        if matches!(kind.as_deref(), Some("subagent-start" | "subagent-stop")) {
            m.insert("label".into(), json!(SUBAGENT));
        }
        if m.get("files").is_some_and(Value::is_array) {
            m.insert("files".into(), json!([]));
        }
    }
    a
}

pub fn metadata_engine(mut e: Value) -> Value {
    if let Some(Value::Array(ms)) = e.get_mut("messages") {
        for x in ms.iter_mut() {
            if let Some(m) = obj_mut(x) {
                m.insert("text".into(), json!(""));
            }
        }
    }
    if let Some(Value::Array(agents)) = e.get_mut("agents") {
        for a in agents.iter_mut() {
            let claims: Vec<Value> = a
                .get("claims")
                .and_then(Value::as_array)
                .map(|cs| cs.iter().map(|c| spread_with(&spread_with(c, "pattern", json!("")), "note", Value::Null)).collect())
                .unwrap_or_default();
            if let Some(m) = obj_mut(a) {
                m.insert("claims".into(), Value::Array(claims));
            }
        }
    }
    e
}

pub fn metadata_message(mut m: Value) -> Value {
    if let Some(o) = obj_mut(&mut m) {
        o.insert("text".into(), json!(""));
    }
    m
}

// ---------- the shapes that leave ----------

fn list(v: Option<&Value>) -> Vec<Value> {
    match v {
        Some(Value::Array(a)) => a.clone(),
        _ => Vec::new(),
    }
}

fn id_of(v: Option<&Value>) -> String {
    v.map(js_string).unwrap_or_default()
}

/// The map as it leaves: without another host's nodes, without sessions
/// older than `since_ms` or archived (a session kept under one that went
/// moves up to the nearest place kept), each project with its repo key, and
/// titles, states and counts only unless `detail`; and which sessions it
/// left out. What's kept is copied once, by `copy` (the relay's
/// scrubber, which writes the home folder as ~ as it copies).
pub fn shape_graph(
    input: &Value,
    since_ms: Option<f64>,
    now: f64,
    aliases: &HashMap<String, String>,
    detail: bool,
    copy: &mut dyn FnMut(&Value) -> Value,
) -> (Value, HashSet<String>) {
    let nodes = list(input.get("nodes"));
    let cutoff = since_ms.map_or(f64::NEG_INFINITY, |s| now - s);
    let mut dropped = HashSet::new();
    let mut gone = HashSet::new();
    for x in &nodes {
        let id = id_of(x.get("id"));
        if qualified(x.get("id")) || x.get("kind") == Some(&json!("host")) {
            gone.insert(id);
        } else if x.get("kind") == Some(&json!("session")) {
            let updated = x.get("updatedAt").filter(|v| !v.is_null()).map_or(0.0, |v| js_number(Some(v)));
            if truthy(x.get("archived")) || updated < cutoff {
                dropped.insert(id);
            }
        }
    }
    let out = |id: Option<&Value>| qualified(id) || gone.contains(&id_of(id)) || dropped.contains(&id_of(id));
    let by_id: HashMap<String, &Value> = nodes.iter().map(|x| (id_of(x.get("id")), x)).collect();
    let mut parent: HashMap<String, Value> = HashMap::new();
    for l in list(input.get("links")) {
        if l.get("kind") != Some(&json!("lineage-cross")) {
            parent.insert(id_of(l.get("target")), l.get("source").cloned().unwrap_or(Value::Null));
        }
    }
    let mut links = Vec::new();
    for l in list(input.get("links")) {
        if out(l.get("target")) {
            continue;
        }
        if !out(l.get("source")) {
            links.push(l);
            continue;
        }
        if l.get("kind") == Some(&json!("lineage-cross")) || gone.contains(&id_of(l.get("source"))) || qualified(l.get("source")) {
            continue;
        }
        let mut up = l.get("source").cloned();
        for _ in 0..64 {
            match &up {
                Some(u) if truthy(Some(u)) && out(Some(u)) => up = parent.get(&js_string(u)).cloned(),
                _ => break,
            }
        }
        if let Some(u) = up.filter(|u| truthy(Some(u)) && !out(Some(u))) {
            let kind = if by_id.get(&js_string(&u)).and_then(|x| x.get("kind")) == Some(&json!("session")) { "lineage" } else { "session" };
            links.push(json!({ "source": u, "target": l.get("target"), "kind": kind }));
        }
    }
    let mut kept = Vec::new();
    for x in &nodes {
        if out(x.get("id")) {
            continue;
        }
        let key = if x.get("kind") == Some(&json!("project")) { repo_key(x.get("git").and_then(|g| g.get("origin")), aliases) } else { None };
        kept.push(match key {
            Some(k) => {
                let mut m = x.as_object().cloned().unwrap_or_default();
                m.insert("repoKey".into(), json!(k));
                Value::Object(m)
            }
            None => x.clone(),
        });
    }
    let mut top = input.as_object().cloned().unwrap_or_default();
    top.insert("nodes".into(), Value::Array(kept));
    top.insert("links".into(), Value::Array(links));
    top.insert("relations".into(), Value::Array(list(input.get("relations")).into_iter().filter(|r| !out(r.get("a")) && !out(r.get("b"))).collect()));
    top.insert("identities".into(), Value::Array(list(input.get("identities")).into_iter().filter(|i| !qualified(i.get("id"))).collect()));
    top.shift_remove("hosts");
    top.shift_remove("self");
    let mut g = copy(&Value::Object(top));
    if let Some(Value::Object(stats)) = g.get("stats").cloned().as_ref().filter(|s| truthy(Some(s))) {
        let ns = list(g.get("nodes"));
        let sessions: Vec<&Value> = ns.iter().filter(|x| x.get("kind") == Some(&json!("session"))).collect();
        let mut st = stats.clone();
        st.insert("sessions".into(), json!(sessions.len()));
        st.insert("claude".into(), json!(sessions.iter().filter(|x| x.get("app") == Some(&json!("claude"))).count()));
        st.insert("codex".into(), json!(sessions.iter().filter(|x| x.get("app") == Some(&json!("codex"))).count()));
        st.insert("active".into(), json!(sessions.iter().filter(|x| x.get("activity") == Some(&json!("active"))).count()));
        st.insert("archived".into(), json!(0));
        g["stats"] = Value::Object(st);
    }
    (if detail { g } else { metadata_graph(g) }, dropped)
}

/// The live state as it leaves, without another host's sessions or the
/// sessions the map left out.
pub fn shape_live(input: &Value, dropped: &HashSet<String>, detail: bool, copy: &mut dyn FnMut(&Value) -> Value) -> Value {
    let out = |id: Option<&Value>| qualified(id) || dropped.contains(&id_of(id));
    let mut l = input.as_object().cloned().unwrap_or_default();
    l.insert("sessions".into(), Value::Array(list(input.get("sessions")).into_iter().filter(|x| !out(x.get("id"))).collect()));
    for k in ["collisions", "collab"] {
        l.insert(k.into(), Value::Array(list(input.get(k)).into_iter().filter(|c| !out(c.get("a")) && !out(c.get("b"))).collect()));
    }
    let mut l = copy(&Value::Object(l));
    if let Some(Value::Object(limits)) = l.get_mut("limits") {
        limits.shift_remove("others");
    }
    if detail {
        l
    } else {
        metadata_live(l)
    }
}

pub fn shape_engine(input: &Value, detail: bool, copy: &mut dyn FnMut(&Value) -> Value) -> Value {
    let e = if input.is_null() { json!({}) } else { input.clone() };
    let mut m = e.as_object().cloned().unwrap_or_default();
    m.insert("agents".into(), Value::Array(list(e.get("agents")).into_iter().filter(|a| !qualified(a.get("id"))).collect()));
    let messages: Vec<Value> = list(e.get("messages"))
        .into_iter()
        .filter(|x| !qualified(x.get("from")) && !list(x.get("to")).iter().any(|d| qualified(d.get("id").filter(|v| !v.is_null()).or(Some(d)))))
        .collect();
    m.insert("messages".into(), Value::Array(messages));
    let c = copy(&Value::Object(m));
    if detail {
        c
    } else {
        metadata_engine(c)
    }
}

/// One of the host's events as it leaves, or None for one that stays.
pub fn shape_event(name: &str, data: &Value, dropped: &HashSet<String>, detail: bool, copy: &mut dyn FnMut(&Value) -> Value) -> Option<Value> {
    let out = |id: Option<&Value>| qualified(id) || dropped.contains(&id_of(id));
    if !data.is_object() && !data.is_array() {
        return None;
    }
    match name {
        "activity" => {
            if !truthy(data.get("sessionId")) || out(data.get("sessionId")) || qualified(data.get("projectId")) {
                return None;
            }
            let c = copy(data);
            Some(if detail { c } else { metadata_activity(c) })
        }
        "tokens" => (truthy(data.get("sessionId")) && !out(data.get("sessionId"))).then(|| copy(data)),
        "pulse" => (truthy(data.get("nodeId")) && !out(data.get("nodeId"))).then(|| copy(data)),
        "agent-message" => {
            if qualified(data.get("from")) || list(data.get("to")).iter().any(|x| qualified(Some(x))) {
                return None;
            }
            let c = copy(data);
            Some(if detail { c } else { metadata_message(c) })
        }
        "doctor" => {
            // How the host's own checks went, for its state on the console.
            let problems: Vec<Value> = list(data.get("results"))
                .into_iter()
                .filter(|r| matches!(r.get("status").and_then(Value::as_str), Some("warn" | "fail")))
                .map(|r| {
                    let mut m = Map::new();
                    for k in ["id", "title", "status"] {
                        if let Some(v) = r.get(k) {
                            m.insert(k.into(), v.clone());
                        }
                    }
                    if let Some(v) = r.get("detail") {
                        m.insert("detail".into(), v.clone());
                    }
                    Value::Object(m)
                })
                .collect();
            let mut m = Map::new();
            if let Some(at) = data.get("at") {
                m.insert("at".into(), at.clone());
            }
            if let Some(s) = data.get("summary") {
                m.insert("summary".into(), s.clone());
            }
            m.insert("problems".into(), Value::Array(problems));
            Some(copy(&Value::Object(m)))
        }
        _ => None,
    }
}

// ---------- the console's reads ----------

/// A route a console may read through the relay: its shape on the way out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    None,
    Graph,
    Live,
    Engine,
}

fn session_ok(s: &str) -> bool {
    let rest = s.strip_prefix("claude:").or_else(|| s.strip_prefix("codex:"));
    rest.is_some_and(|r| {
        let b = r.as_bytes();
        !b.is_empty() && b.len() <= 128 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || b"._-".contains(c))
    })
}

fn chars_ok(v: &str, extra: &[u8], min: usize, max: usize) -> bool {
    (min..=max).contains(&v.len()) && v.bytes().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

// Whether a query parameter (key, value) may come with a route.
type QueryOk = fn(&str, &str) -> bool;

// The route a path is, and whether a query parameter may come with it.
fn route_of(path: &str) -> Option<(Shape, QueryOk)> {
    let session_route = |p: &str, end: &str| p.strip_prefix("/api/session/").and_then(|r| r.strip_suffix(end)).is_some_and(session_ok);
    if path == "/api/mcp" {
        return Some((Shape::None, |k, v| k == "server" && chars_ok(v, b"._:-", 1, 160)));
    }
    if session_route(path, "/transcript") {
        return Some((Shape::None, |k, v| {
            (k == "cursor" && chars_ok(v, b"._:-", 1, 200)) || (k == "limit" && (1..=4).contains(&v.len()) && v.bytes().all(|c| c.is_ascii_digit()))
        }));
    }
    if session_route(path, "/stats") {
        return Some((Shape::None, |_, _| false));
    }
    if path == "/api/collision" {
        return Some((Shape::None, |k, v| (k == "a" || k == "b") && session_ok(v)));
    }
    match path {
        "/api/graph" => Some((Shape::Graph, |_, _| false)),
        "/api/live" => Some((Shape::Live, |_, _| false)),
        "/api/engine/state" => Some((Shape::Engine, |_, _| false)),
        _ => None,
    }
}

/// A get's path against the allowlist: its shape and the path to read, or
/// why not. Only plain paths pass: no dots, nothing encoded, no
/// backslashes or doubled slashes, nothing a URL parser would read
/// differently.
pub fn allowed(raw: Option<&Value>) -> Result<(Shape, String), String> {
    let Some(raw) = raw.and_then(Value::as_str).filter(|r| !r.is_empty() && utf16_len(r) <= 2048) else { return Err("not a path".into()) };
    let plain = "only plain paths can be read through the relay";
    if raw.bytes().any(|c| !(0x21..=0x7e).contains(&c)) || raw.contains(['%', '\\', '#']) || raw.contains("..") || raw.contains("//") {
        return Err(plain.into());
    }
    let (pathname, search) = raw.split_once('?').unwrap_or((raw, ""));
    let base = url::Url::parse("http://relay.invalid").expect("a fixed URL");
    let Ok(u) = base.join(raw) else { return Err("not a path".into()) };
    if u.origin().ascii_serialization() != "http://relay.invalid" || u.path() != pathname {
        return Err(plain.into());
    }
    let Some((shape, takes)) = route_of(pathname) else { return Err(format!("{pathname} can't be read through the relay")) };
    // URLSearchParams: & between pairs, = between name and value, + as a
    // space (% was refused above).
    let mut pairs = Vec::new();
    for part in search.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let (k, v) = (k.replace('+', " "), v.replace('+', " "));
        if !takes(&k, &v) {
            return Err(format!("{pathname} doesn't take {k}={}", xenonet_app::jstext::slice16(&v, 40)));
        }
        pairs.push((k, v));
    }
    let query = xenonet_app::jstext::search_params(&pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect::<Vec<_>>());
    Ok((shape, if search.is_empty() { pathname.to_string() } else { format!("{pathname}?{query}") }))
}

/// What a get reads: graph and live are asked for this XenoNet's own view.
pub fn route_to_read(shape: Shape, path: &str) -> String {
    match shape {
        Shape::Graph | Shape::Live => format!("{path}{}hosts=0", if path.contains('?') { "&" } else { "?" }),
        _ => path.to_string(),
    }
}

// ---------- what goes out, and when ----------

/// The relay's way out: frames go as they come while the console keeps
/// up; while it doesn't, the newest map and live state wait (each replacing
/// the last), other events up to QUEUE_BYTES, and past that a fresh
/// snapshot goes once it catches up.
#[derive(Default)]
pub struct Outbox {
    pub busy: bool,
    queue: Vec<String>,
    queued: usize,
    latest: Vec<(String, String)>,
    gap: bool,
    pub last_write: f64,
}

/// What sending does: write these now (each one returns whether the
/// output took it), or nothing.
impl Outbox {
    /// A frame's text to go now, or None while busy (it waits, or is
    /// dropped and remembered as a gap).
    pub fn send(&mut self, text: String, coalesce: Option<&str>, now: f64) -> Option<String> {
        if self.busy {
            match coalesce {
                Some(c) => match self.latest.iter_mut().find(|(k, _)| k == c) {
                    Some(slot) => slot.1 = text,
                    None => self.latest.push((c.to_string(), text)),
                },
                None if self.queued + utf16_len(&text) <= QUEUE_BYTES => {
                    self.queued += utf16_len(&text);
                    self.queue.push(text);
                }
                None => self.gap = true,
            }
            return None;
        }
        self.last_write = now;
        Some(text)
    }

    /// The output said it can take no more for now.
    pub fn full(&mut self) {
        self.busy = true;
    }

    /// A frame went to the output now.
    pub fn wrote(&mut self, now: f64) {
        self.last_write = now;
    }

    /// The output drained: what waited, in order, to write while it takes
    /// them; the twin calls `wrote` before each, and `left` with the rest
    /// once the output is full again.
    pub fn drain(&mut self) -> Vec<String> {
        self.busy = false;
        let mut waiting = std::mem::take(&mut self.queue);
        waiting.extend(std::mem::take(&mut self.latest).into_iter().map(|(_, t)| t));
        self.queued = 0;
        waiting
    }

    /// What a drain couldn't write, back in the queue.
    pub fn left(&mut self, rest: Vec<String>) {
        for t in rest {
            self.queued += utf16_len(&t);
            self.queue.push(t);
        }
    }

    /// Whether a fresh snapshot is owed, now that the output keeps up.
    pub fn take_gap(&mut self) -> bool {
        if !self.busy && self.gap {
            self.gap = false;
            return true;
        }
        false
    }

    pub fn ping_due(&self, now: f64) -> bool {
        now - self.last_write >= PING_MS
    }
}

/// What the relay says when it stops for want of a XenoNet here, or one
/// that can't say which host it is.
pub fn self_bye(status: f64, body: &Value, up_version: Option<&Value>) -> (i32, Value) {
    if status == 404.0 {
        let v = up_version.filter(|v| !v.is_null()).map_or_else(|| "an older version".to_string(), js_string);
        return (EXIT_OLD_XENONET, json!({ "code": "old-xenonet", "message": format!("the XenoNet here ({v}) can't be a host yet; update it") }));
    }
    let why = body.get("error").filter(|e| !e.is_null()).map_or_else(|| xenonet_base::json::number_string(status), js_string);
    (EXIT_FAILED, json!({ "code": "no-self", "message": format!("the XenoNet here couldn't say which host it is ({why})") }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_folder_goes_as_tilde() {
        let s = Scrubber::new(Some("/Users/nova"), &["/mnt/c/Users/nova2".into()]);
        assert_eq!(s.text("/Users/nova/Projects/atlas"), "~/Projects/atlas");
        assert_eq!(s.text("/USERS/NOVA/x"), "~/x");
        assert_eq!(s.text("/Users/novak/x"), "/Users/novak/x", "another name that starts the same stays");
        assert_eq!(s.text("-Users-nova-Projects-atlas"), "~-Projects-atlas");
        assert_eq!(s.text("/mnt/c/Users/nova2/Projects"), "/mnt/c/Users/~/Projects");
        assert_eq!(s.walk(&json!({ "/Users/nova/x": ["/Users/nova"] })), json!({ "~/x": ["~"] }));
        assert_eq!(user_folder_of(Some(&json!("/mnt/c/Users/nova/Projects")), Some("/home/nova")).as_deref(), Some("/mnt/c/Users/nova"));
        assert_eq!(user_folder_of(Some(&json!("/home/nova/Projects")), Some("/home/nova")), None);
        assert_eq!(user_folder_of(Some(&json!("C:\\Users\\nova\\x")), None).as_deref(), Some("C:\\Users\\nova"));
        assert_eq!(user_folder_of(Some(&json!("/c/Users/nova")), None).as_deref(), Some("/c/Users/nova"));
    }

    #[test]
    fn only_the_allowlist_is_read() {
        assert_eq!(allowed(Some(&json!("/api/session/claude:abc/stats"))).unwrap(), (Shape::None, "/api/session/claude:abc/stats".into()));
        assert_eq!(allowed(Some(&json!("/api/mcp?server=agent-wiki"))).unwrap().1, "/api/mcp?server=agent-wiki");
        for bad in ["/api/../etc", "/api//x", "/api/open", "/api/mcp?server=a%20b", "/api/mcp?evil=1", "/api/mcp?constructor=1", "http://x/api/graph"] {
            assert!(allowed(Some(&json!(bad))).is_err(), "{bad}");
        }
        assert_eq!(route_to_read(Shape::Graph, "/api/graph"), "/api/graph?hosts=0");
        assert_eq!(parse_since("90m"), Some(5_400_000.0));
        assert_eq!(parse_since("7x"), None);
        assert_eq!(from_original_command("xenonet relay --since 12h"), Some(43_200_000.0));
    }
}
