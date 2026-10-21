//! core/assistant/projection.js: what a remote actor may see of XenoNet's
//! state, as pure functions from a view of the map (source.rs, or the
//! made-up one in fixtures.rs) and a grant's policy to the six tools'
//! data. Every object is made afresh from fields named here, and the
//! service checks each against the strict schemas before it goes.
//!
//! A grant sees only the places it names, by their own ids: a session
//! counts when the place it lives in is one of them, and is never
//! archived. Labels go only when the grant says so, cleaned, and are null
//! otherwise. Never a path, a branch, a remote, a prompt, a message, a
//! question's text, a tool's arguments or output, an account or an error
//! as a source wrote it.
//!
//! A view's places and a policy's places are [key, value] pairs here,
//! where the JS had Maps.

use regex::{Captures, Regex};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use xenonet_app::jsv::truthy;
use xenonet_base::json::number as num;
use xenonet_base::text::{js_string, JS_SPACE};

pub const LABEL_MAX: usize = 240;
pub const LIVE_MS: f64 = 30e3;
pub const WINDOW_MS: f64 = 7.0 * 24.0 * 3600e3;
pub const LOOKBACK_MAX_MS: f64 = 30.0 * 24.0 * 3600e3;
pub const SOURCE_NAMES: [&str; 2] = ["claude", "codex"];

/// A time as an ISO string, or null for one that isn't a time after 1970.
pub fn iso(v: Option<&Value>) -> Value {
    match v.and_then(Value::as_f64) {
        Some(ms) if ms.is_finite() && ms > 0.0 => json!(xenonet_base::time::iso(ms)),
        _ => Value::Null,
    }
}

// ---------- labels ----------

fn control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}' | '\u{2028}' | '\u{2029}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{feff}')
}

struct Res {
    bearer: Regex,
    key: Regex,
    email: Regex,
    user_folder: Regex,
    windows_path: Regex,
    posix_path: Regex,
    run: Regex,
    spaces: Regex,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        // \b and /i as JS has them without /u: ASCII words, ASCII case.
        bearer: Regex::new(&format!(r"(?-u:\b)((?i-u:bearer|token|password|passwd|secret|api[_-]?key))(?-u:\b)[{JS_SPACE}]*[:=][{JS_SPACE}]*[^{JS_SPACE}]+"))
            .expect("a fixed pattern"),
        key: Regex::new(
            r"(?:sk-|sk_|pk_|rk_|ghp_|gho_|ghu_|ghs_|ghr_|github_pat_|glpat-|xox[abprs]-|AKIA|ASIA|AIza|npm_|pypi-|hf_|xenonet_actor_|xenonet_assistant_)[A-Za-z0-9_-]*",
        )
        .expect("a fixed pattern"),
        email: Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("a fixed pattern"),
        // Followed by a separator or the end, checked after (a lookahead).
        user_folder: Regex::new(r#"(?:[A-Za-z]:)?[\\/](?i-u:Users|home)[\\/][^\\/"'`]+"#).expect("a fixed pattern"),
        windows_path: Regex::new(&format!(r#"[A-Za-z]:[\\/][^{JS_SPACE}"'`<>|]*"#)).expect("a fixed pattern"),
        posix_path: Regex::new(r"/(?:[A-Za-z0-9_.@+-]+/)+[A-Za-z0-9_.@+-]*").expect("a fixed pattern"),
        run: Regex::new(r"[A-Za-z0-9+/=_-]{32,}").expect("a fixed pattern"),
        spaces: Regex::new(&format!("[{JS_SPACE}]+")).expect("a fixed pattern"),
    })
}

fn char_len_at(s: &str, i: usize) -> usize {
    s[i..].chars().next().map_or(1, char::len_utf8)
}

// s.replace(re, with) for a /g regex that also has a lookbehind (`before`
// may not be the character before) and a lookahead (`after` must hold of
// the character after, None at the end). A match that fails either is
// tried again a character on, as JS's search would.
fn replace_around(re: &Regex, s: &str, before: impl Fn(char) -> bool, after: impl Fn(Option<char>) -> bool, mut with: impl FnMut(&Captures<'_>) -> String) -> String {
    let mut out = String::with_capacity(s.len());
    let (mut last, mut at) = (0, 0);
    while at <= s.len() {
        let Some(c) = re.captures_at(s, at) else { break };
        let m = c.get(0).expect("group 0 is the whole match");
        let ok = !s[..m.start()].chars().next_back().is_some_and(&before) && after(s[m.end()..].chars().next());
        if !ok {
            if m.start() >= s.len() {
                break;
            }
            at = m.start() + char_len_at(s, m.start());
            continue;
        }
        out.push_str(&s[last..m.start()]);
        out.push_str(&with(&c));
        last = m.end();
        at = if m.end() > m.start() { m.end() } else { m.end() + if m.end() < s.len() { char_len_at(s, m.end()) } else { 1 } };
    }
    out.push_str(&s[last..]);
    out
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

/// A name or a title as it may leave: one line, no control or direction
/// characters, nothing that reads as a key, an email or a person's folder,
/// at most LABEL_MAX characters; None for nothing left. Protection on top
/// of your choice to send labels, not a promise that what's left is
/// harmless.
pub fn clean_label(text: &str) -> Option<String> {
    let r = res();
    let s = icu_normalizer::ComposingNormalizerBorrowed::new_nfc().normalize(text);
    let s: String = s.chars().map(|c| if control(c) { ' ' } else { c }).collect();
    let s = r.bearer.replace_all(&s, "$1: [withheld]").into_owned();
    let s = replace_around(&r.key, &s, alnum, |_| true, |_| "[withheld]".into());
    let s = r.email.replace_all(&s, "[email]").into_owned();
    let s = replace_around(&r.user_folder, &s, |_| false, |c| matches!(c, None | Some('/' | '\\')), |_| "~".into());
    let s = replace_around(&r.windows_path, &s, alnum, |_| true, |_| "[path]".into());
    let s = replace_around(&r.posix_path, &s, |c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '~'), |_| true, |_| "[path]".into());
    let s = r.run.replace_all(&s, |c: &Captures<'_>| {
        let run = &c[0];
        if run.bytes().any(|b| b.is_ascii_alphabetic()) && run.bytes().any(|b| b.is_ascii_digit()) {
            "[withheld]".to_string()
        } else {
            run.to_string()
        }
    });
    let s = r.spaces.replace_all(&s, " ");
    let s = xenonet_base::text::js_trim(&s);
    if s.is_empty() {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    Some(if chars.len() > LABEL_MAX { format!("{}…", chars[..LABEL_MAX - 1].iter().collect::<String>()) } else { s.to_string() })
}

/// clean_label of a JS value: null for one that isn't a string.
pub fn label_of(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => clean_label(s).map_or(Value::Null, Value::String),
        _ => Value::Null,
    }
}

// ---------- sessions ----------

/// A session's status and what it was read from, from what the tailer saw
/// and how its app's source stands. A turn that ended is completed; that
/// says nothing of whether the work it was part of is done.
pub fn session_status(live: Option<&Value>, source_state: &str) -> (&'static str, &'static str) {
    if source_state == "unavailable" {
        return ("unknown", "source_unavailable");
    }
    let Some(live) = live.filter(|l| truthy(Some(l))) else { return ("idle", "observed_inactive") };
    let need = live.get("needsYou").and_then(|n| n.get("kind")).and_then(Value::as_str);
    if matches!(need, Some("permission" | "question")) {
        return ("waiting", "observed_wait");
    }
    if truthy(live.get("stuck")) {
        return ("stuck", "inferred_stall");
    }
    if matches!(live.get("status").and_then(Value::as_str), Some("tool" | "thinking")) {
        return ("working", "observed_turn_active");
    }
    if need == Some("review") {
        return ("completed", "observed_turn_end");
    }
    if truthy(live.get("interruptedAt")) {
        return ("idle", "observed_interrupt");
    }
    ("idle", "observed_idle")
}

/// Why a session looks stuck, as a code: never the reason's words.
pub fn stall_reason(reasons: Option<&Value>) -> &'static str {
    let list: Vec<String> = reasons.and_then(Value::as_array).map(|r| r.iter().map(js_string).collect()).unwrap_or_default();
    if list.iter().any(|r| r.contains("failed tool calls") || r.contains("times in a row")) {
        "repeated_failure"
    } else if list.iter().any(|r| r.starts_with("silent for")) {
        "inactive_mid_turn"
    } else {
        "unknown"
    }
}

/// SESSION_ID in core/schema/assistant.js: `(claude|codex):<uuid>`.
pub fn is_session_id(s: &str) -> bool {
    let Some(u) = s.strip_prefix("claude:").or_else(|| s.strip_prefix("codex:")) else { return false };
    let b = u.as_bytes();
    b.len() == 36 && b.iter().enumerate().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { *c == b'-' } else { matches!(c, b'0'..=b'9' | b'a'..=b'f') })
}

// A key for a Map of JS values: its JSON.
fn key(v: Option<&Value>) -> String {
    v.map_or_else(|| "undefined".into(), |v| xenonet_base::json::stringify(v, 0))
}

/// A view's or policy's pairs, by their first.
pub fn pairs(v: Option<&Value>) -> Vec<(Value, Value)> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(|p| Some((p.get(0)?.clone(), p.get(1).cloned().unwrap_or(Value::Null)))).collect()).unwrap_or_default()
}

/// The places a grant names, the other way: each place to its project.
fn by_place(policy: &Value) -> HashMap<String, Value> {
    pairs(policy.get("places")).into_iter().map(|(project, place)| (key(Some(&place)), project)).collect()
}

/// A session a grant may see, with its project, status and times.
pub struct Visible<'a> {
    pub s: &'a Value,
    pub project: Value,
    pub updated: f64,
    pub source: String,
    pub status: &'static str,
    pub basis: &'static str,
}

fn or0(v: Option<&Value>) -> f64 {
    match v {
        None | Some(Value::Null) => 0.0,
        other => xenonet_base::text::js_number(other),
    }
}

/// The policy's lookback, at most LOOKBACK_MAX_MS.
pub fn lookback(policy: &Value) -> f64 {
    let l = policy.get("lookbackMs").filter(|v| !v.is_null()).map_or(LOOKBACK_MAX_MS, |v| xenonet_base::text::js_number(Some(v)));
    if l.is_nan() {
        f64::NAN
    } else {
        l.min(LOOKBACK_MAX_MS)
    }
}

/// The sessions a grant may see: in a place it names, not archived, with
/// an id of the form the contract allows, and updated within its lookback.
pub fn visible_sessions<'a>(view: &'a Value, policy: &Value, now: f64) -> Vec<Visible<'a>> {
    let places = by_place(policy);
    let oldest = now - lookback(policy);
    let mut out = Vec::new();
    for s in view.get("sessions").and_then(Value::as_array).into_iter().flatten() {
        let Some(project) = places.get(&key(s.get("place"))).filter(|p| truthy(Some(p))) else { continue };
        if truthy(s.get("archived")) || !is_session_id(&s.get("id").map_or_else(|| "undefined".into(), js_string)) {
            continue;
        }
        let updated = or0(s.get("updatedAt")).max(or0(s.get("live").and_then(|l| l.get("lastEventAt"))));
        if updated < oldest || updated.is_nan() {
            continue;
        }
        let app = s.get("app").map_or_else(|| "undefined".into(), js_string);
        let source = view.get("sources").and_then(|x| x.get(&app)).and_then(|x| x.get("state")).filter(|v| !v.is_null()).map_or_else(|| "unavailable".to_string(), js_string);
        let (status, basis) = session_status(s.get("live"), &source);
        out.push(Visible { s, project: project.clone(), updated, source, status, basis });
    }
    out
}

pub fn session_summary(v: &Visible<'_>, policy: &Value) -> Value {
    json!({
        "session_id": v.s.get("id"),
        "project_id": v.project,
        "app": v.s.get("app"),
        "label": if truthy(policy.get("labels")) { label_of(v.s.get("title")) } else { Value::Null },
        "status": v.status,
        "status_basis": v.basis,
        "updated_at": iso(v.s.get("updatedAt")),
        "last_activity_at": iso(Some(&num(v.updated))),
        "source_state": v.source,
    })
}

pub fn empty_counts() -> Map<String, Value> {
    ["working", "waiting", "idle", "completed", "stuck", "unknown"].iter().map(|k| ((*k).to_string(), json!(0))).collect()
}

// ---------- freshness ----------

/// How fresh a view is, from its sources (`names`).
pub struct Freshness {
    pub freshness: &'static str,
    pub observed_at: Option<f64>,
    pub partial: bool,
    pub warnings: Vec<Value>,
}

pub fn freshness_of(view: &Value, now: f64, names: &[String]) -> Freshness {
    let list: Vec<(&String, Option<&Value>)> = names.iter().map(|n| (n, view.get("sources").and_then(|s| s.get(n.as_str())).filter(|s| !s.is_null()))).collect();
    let mut warnings = Vec::new();
    let mut partial = false;
    for (name, s) in &list {
        match s.and_then(|s| s.get("state")).and_then(Value::as_str) {
            _ if s.is_none() => {
                partial = true;
                warnings.push(json!({ "code": "SOURCE_UNAVAILABLE", "source": name }));
            }
            Some("unavailable") => {
                partial = true;
                warnings.push(json!({ "code": "SOURCE_UNAVAILABLE", "source": name }));
            }
            Some("degraded") => {
                partial = true;
                warnings.push(json!({ "code": "SOURCE_DEGRADED", "source": name }));
            }
            _ => {}
        }
    }
    let times: Vec<Option<f64>> = list.iter().map(|(_, s)| s.and_then(|s| s.get("observedAt")).filter(|v| !v.is_null()).map(|v| xenonet_base::text::js_number(Some(v)))).collect();
    if times.iter().any(Option::is_none) {
        return Freshness { freshness: "unknown", observed_at: None, partial, warnings };
    }
    let oldest = times.iter().flatten().fold(f64::INFINITY, |a, b| if b.is_nan() || a.is_nan() { f64::NAN } else { a.min(*b) });
    Freshness { freshness: if now - oldest <= LIVE_MS { "live" } else { "stale" }, observed_at: Some(oldest), partial, warnings }
}

pub fn all_sources() -> Vec<String> {
    SOURCE_NAMES.iter().map(ToString::to_string).collect()
}

fn put(m: &mut Map<String, Value>, k: &str, v: Option<&Value>) {
    if let Some(v) = v {
        m.insert(k.into(), v.clone());
    }
}

pub fn sources_data(view: &Value) -> Value {
    Value::Array(
        SOURCE_NAMES
            .iter()
            .map(|name| match view.get("sources").and_then(|s| s.get(*name)).filter(|s| truthy(Some(s))) {
                Some(s) => {
                    let mut m = Map::new();
                    m.insert("source".into(), json!(name));
                    put(&mut m, "state", s.get("state"));
                    m.insert("observed_at".into(), iso(s.get("observedAt")));
                    put(&mut m, "coverage", s.get("coverage"));
                    put(&mut m, "reason", s.get("reason"));
                    Value::Object(m)
                }
                None => json!({ "source": name, "state": "unavailable", "observed_at": null, "coverage": "unknown", "reason": "not_started" }),
            })
            .collect(),
    )
}

// ---------- attention ----------

/// What needs someone's attention among the sessions a grant sees: one
/// waiting on its user, one that looks stuck, two that edited the same
/// files lately (only when the grant sees both), and a turn that ended.
/// `id_of(parts)` makes an item's id. Nothing here clears anything.
pub fn attention_items(view: &Value, policy: &Value, now: f64, id_of: &dyn Fn(&str) -> String) -> Vec<Value> {
    let visible = visible_sessions(view, policy, now);
    let by_id: HashMap<String, &Visible<'_>> = visible.iter().map(|v| (key(v.s.get("id")), v)).collect();
    let at = view.get("at").cloned().unwrap_or(Value::Null);
    let or_null = |v: Option<&Value>| v.filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
    struct Item {
        kind: &'static str,
        sessions: Vec<Value>,
        project: Value,
        first: Value,
        reason: &'static str,
        files: Option<Value>,
    }
    let mut items: Vec<Item> = Vec::new();
    for v in &visible {
        let Some(live) = v.s.get("live").filter(|l| truthy(Some(l))) else { continue };
        if v.source == "unavailable" {
            continue;
        }
        let need = live.get("needsYou").filter(|n| truthy(Some(n)));
        let kind = need.and_then(|n| n.get("kind")).and_then(Value::as_str);
        let id = v.s.get("id").cloned().unwrap_or(Value::Null);
        if matches!(kind, Some("question" | "permission")) {
            let reason = if kind == Some("permission") { "approval_requested" } else { "input_requested" };
            items.push(Item { kind: "waiting", sessions: vec![id], project: v.project.clone(), first: or_null(need.and_then(|n| n.get("since"))), reason, files: None });
        } else if truthy(live.get("stuck")) {
            let first = or_null(live.get("turnStartedAt").filter(|x| !x.is_null()).or_else(|| live.get("lastEventAt")));
            items.push(Item {
                kind: "stalled",
                sessions: vec![id],
                project: v.project.clone(),
                first,
                reason: stall_reason(live.get("stuck").and_then(|s| s.get("reasons"))),
                files: None,
            });
        } else if kind == Some("review") {
            let first = or_null(live.get("lastTurnEndAt").filter(|x| !x.is_null()).or_else(|| need.and_then(|n| n.get("since"))));
            items.push(Item { kind: "turn_completed", sessions: vec![id], project: v.project.clone(), first, reason: "turn_ended", files: None });
        }
    }
    for c in view.get("collisions").and_then(Value::as_array).into_iter().flatten() {
        let (Some(a), Some(b)) = (by_id.get(&key(c.get("a"))), by_id.get(&key(c.get("b")))) else { continue };
        if a.source == "unavailable" || b.source == "unavailable" {
            continue;
        }
        let mut sessions = vec![c.get("a").cloned().unwrap_or(Value::Null), c.get("b").cloned().unwrap_or(Value::Null)];
        sessions.sort_by(|x, y| js_string(x).encode_utf16().cmp(js_string(y).encode_utf16()));
        items.push(Item {
            kind: "collision",
            sessions,
            project: a.project.clone(),
            first: or_null(c.get("ts")),
            reason: "recent_overlap",
            files: Some(c.get("files").cloned().unwrap_or(Value::Null)),
        });
    }
    items
        .into_iter()
        .map(|i| {
            let mut parts: Vec<String> = vec![i.kind.into()];
            parts.extend(i.sessions.iter().map(js_string));
            parts.push(if i.first.is_null() { String::new() } else { js_string(&i.first) });
            let mut m = Map::new();
            m.insert("attention_id".into(), json!(id_of(&parts.join("|"))));
            m.insert("kind".into(), json!(i.kind));
            m.insert("session_ids".into(), Value::Array(i.sessions));
            m.insert("project_id".into(), i.project);
            m.insert("first_observed_at".into(), iso(Some(&i.first)));
            m.insert("last_observed_at".into(), iso(Some(&at)));
            m.insert("reason_code".into(), json!(i.reason));
            if let Some(f) = i.files {
                let n = if f.is_null() { 0.0 } else { xenonet_base::text::js_number(Some(&f)).floor() };
                m.insert("file_count".into(), num(if n.is_nan() { f64::NAN } else { n.max(0.0) }));
            }
            m.insert("resolved".into(), json!(false));
            Value::Object(m)
        })
        .collect()
}

// ---------- actors ----------

fn presence_basis(a: &Value) -> &'static str {
    if a.get("presence").and_then(Value::as_str) == Some("unknown") {
        return "unknown";
    }
    let via: Vec<&str> = a.get("via").and_then(Value::as_array).map(|v| v.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if via.contains(&"observed") || via.contains(&"attributed") {
        "observed"
    } else if via.contains(&"registered") {
        "reported"
    } else if via.contains(&"remote") {
        "remote_request"
    } else {
        "unknown"
    }
}

/// The actors a grant may see: itself, and others whose targets fall in
/// its places, with only those relations. Never the person at the
/// console, and never an actor that acts only on places the grant doesn't
/// name.
pub fn actor_summaries(view: &Value, policy: &Value, now: f64, project_id: Option<&Value>) -> Vec<Value> {
    let places = by_place(policy);
    let sessions: HashMap<String, Value> = visible_sessions(view, policy, now).into_iter().map(|v| (key(v.s.get("id")), v.project)).collect();
    let me = policy.get("actorId");
    let mut out = Vec::new();
    for a in view.get("actors").and_then(Value::as_array).into_iter().flatten() {
        if a.get("id").and_then(Value::as_str) == Some("actor:you") || a.get("kind").and_then(Value::as_str) == Some("human") {
            continue;
        }
        let is_self = xenonet_app::jsv::strict_eq(a.get("id"), me);
        let mut relations: Vec<Value> = Vec::new();
        for t in a.get("targets").and_then(Value::as_array).into_iter().flatten() {
            let k = key(t.get("id"));
            let project = places.get(&k).or_else(|| sessions.get(&k)).filter(|p| !p.is_null());
            let Some(project) = project.filter(|p| truthy(Some(p))) else { continue };
            if project_id.is_some_and(|p| truthy(Some(p)) && !xenonet_app::jsv::strict_eq(Some(project), Some(p))) {
                continue;
            }
            let target = if places.contains_key(&k) { project.clone() } else { t.get("id").cloned().unwrap_or(Value::Null) };
            let relation = t.get("relation").cloned().unwrap_or(Value::Null);
            if !relations.iter().any(|r| xenonet_app::jsv::strict_eq(r.get("target_id"), Some(&target)) && xenonet_app::jsv::strict_eq(r.get("relation"), Some(&relation))) {
                relations.push(json!({ "target_id": target, "relation": relation }));
            }
        }
        if !is_self && relations.is_empty() {
            continue;
        }
        relations.truncate(16);
        let presence = a.get("presence").and_then(Value::as_str).filter(|p| ["working", "idle", "away", "offline", "unknown"].contains(p)).unwrap_or("unknown");
        let remote = a.get("remote").filter(|r| truthy(Some(r)));
        let connection = match remote {
            Some(r) if r.get("bridge").and_then(Value::as_str) == Some("connected") => "connected",
            Some(_) => "not_connected",
            None => "none",
        };
        out.push(json!({
            "actor_id": a.get("id"),
            "product_id": a.get("productId").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
            "label": if truthy(policy.get("labels")) { label_of(a.get("name")) } else { Value::Null },
            "presence": presence,
            "presence_basis": presence_basis(a),
            "connection_state": connection,
            "account_link_state": remote.and_then(|r| r.get("accountLink")).filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("unverified")),
            "relations": relations,
            "reachable": truthy(a.get("reachable")) && !is_self,
        }));
    }
    out
}

// ---------- stats ----------

/// A session's counts from its stats: fresh input (with what was written
/// to a cache), output, cached input, tool calls and turns. Null for what
/// it doesn't say, never 0 for unknown.
pub fn stats_data(raw: &Value, now: f64) -> Value {
    let n = |v: Option<&Value>| v.and_then(Value::as_f64).filter(|x| x.is_finite() && *x >= 0.0).map(f64::floor);
    let totals = raw.get("totals").filter(|t| !t.is_null());
    let t = |k: &str| n(totals.and_then(|x| x.get(k)));
    let fresh = t("input").map(|i| i + t("cacheWrite").unwrap_or(0.0));
    let tools = raw.get("tools").and_then(Value::as_array).map(|ts| ts.iter().map(|x| n(x.get("calls")).unwrap_or(0.0)).sum::<f64>());
    let turns = raw.get("turns").and_then(Value::as_array).map(Vec::len);
    json!({
        "input_tokens": fresh.map_or(Value::Null, num),
        "output_tokens": t("output").map_or(Value::Null, num),
        "cached_input_tokens": t("cacheRead").map_or(Value::Null, num),
        "tool_call_count": tools.map_or(Value::Null, num),
        "turn_count": turns.map_or(Value::Null, |l| json!(l)),
        "stats_observed_at": xenonet_base::time::iso(now),
        "complete": truthy(Some(raw)) && turns.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_lose_what_reads_as_a_secret_or_a_path() {
        assert_eq!(clean_label("Fix the  build\n now").as_deref(), Some("Fix the build now"));
        assert_eq!(clean_label("token: abc123 and more").as_deref(), Some("token: [withheld] and more"));
        assert_eq!(clean_label("use sk-proj-abcdef here").as_deref(), Some("use [withheld] here"));
        assert_eq!(clean_label("ask sk-x but not task-x or desk_x").as_deref(), Some("ask [withheld] but not task-x or desk_x"));
        assert_eq!(clean_label("mail nova@example.com").as_deref(), Some("mail [email]"));
        assert_eq!(clean_label("in /Users/Jane Doe/Projects/x").as_deref(), Some("in ~/Projects/x"));
        assert_eq!(clean_label(r"C:\Users\nova\src and D:\data\x").as_deref(), Some("~\\src and [path]"));
        assert_eq!(clean_label("see /etc/hosts and ~/x/y and a.b/c/d").as_deref(), Some("see [path] and ~/x/y and a.b/c/d"));
        assert_eq!(clean_label("run abcdefghijklmnopqrstuvwxyz0123456789 ok").as_deref(), Some("run [withheld] ok"));
        assert_eq!(clean_label("\u{202e}\u{200b} "), None);
        assert_eq!(clean_label(&"é".repeat(300)).unwrap().chars().count(), 240);
        assert_eq!(clean_label("e\u{301}").as_deref(), Some("é"));
    }
}
