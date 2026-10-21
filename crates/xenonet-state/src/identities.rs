//! core/identities.js: the accounts sessions ran as.
//!
//! What each app's files say of the accounts work happens under (Claude's
//! and ChatGPT's sign-ins, gh's hosts.yml, ssh aliases, git config), the
//! sign-in timeline, and the links from sessions and projects to them on
//! the map (linkIdentities). Reading a repo's git config means running git,
//! which stays with the caller, as do the lenient checks of what the apps'
//! files say (vendor.js's readers).

use crate::projects::jsre;
use regex::Regex;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::LazyLock;
use xenonet_base::text::{is_js_line_terminator, js_number, js_string, js_trim, js_trim_start, truthy, JS_DOT, JS_SPACE};
use xenonet_base::{json as js_json, jsstr, paths};
use xenonet_sources::lines::{opt_prop, prop, Js, TypeError};

// A sign-in's span is extended this often while it lasts, so state.json
// isn't rewritten on every refresh.
pub const SIGNIN_STEP_MS: f64 = 10.0 * 60e3;
pub const MAX_SPANS: usize = 50;
pub const HEAD_BYTES: u64 = 64 * 1024;

fn re(pattern: &str) -> Regex {
    // JS's \s, \S and `.` (written DOT), as the regex crate has them.
    let p = pattern.replace("\\s", &format!("[{JS_SPACE}]")).replace("\\S", &format!("[^{JS_SPACE}]")).replace("DOT", JS_DOT);
    Regex::new(&p).expect("a regular expression of identities.rs")
}

// /^\s*([^:#\s][^:]*?)\s*:(?:\s+(.*?))?\s*$/
static KEY_LINE: LazyLock<Regex> = LazyLock::new(|| {
    let first = format!("[^:#{JS_SPACE}]");
    re(&format!(r"^\s*({first}[^:]*?)\s*:(?:\s+(DOT*?))?\s*$"))
});
// /^\s*(\w+)\s*(?:=\s*|\s+)(.+?)\s*$/
static SSH_LINE: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([A-Za-z0-9_]+)\s*(?:=\s*|\s+)(DOT+?)\s*$"));
static SPACES: LazyLock<Regex> = LazyLock::new(|| re(r"\s+"));
// /^([a-z][a-z+.-]*):\/\/(?:([^@/:]+)(?::[^@/]*)?@)?([^/:]+)(?::\d+)?\/(.+)$/i
static URL: LazyLock<Regex> = LazyLock::new(|| re(r"^([A-Za-z][A-Za-z+.\-]*)://(?:([^@/:]+)(?::[^@/]*)?@)?([^/:]+)(?::[0-9]+)?/(DOT+)$"));
// /^(https?):\/\/(?:[^@/]+@)?([^/:]+)(?::\d+)?(\/.*)?$/i
static SCOPE: LazyLock<Regex> = LazyLock::new(|| re(r"^([hH][tT][tT][pP][sS]?)://(?:[^@/]+@)?([^/:]+)(?::[0-9]+)?(/DOT*)?$"));
// /\bgh(\.exe)?["']?\s+auth\s+git-credential\b/
static GH_HELPER: LazyLock<Regex> = LazyLock::new(|| re(r#"(?-u:\b)gh(?:\.exe)?["']?\s+auth\s+git-credential(?-u:\b)"#));

/// `typeof v === "string" && v.trim() ? v.trim() : null`
fn str_of(v: Js<'_>) -> Option<String> {
    match v {
        Some(Value::String(s)) if !js_trim(s).is_empty() => Some(js_trim(s).to_string()),
        _ => None,
    }
}

fn nn(v: Js<'_>) -> Js<'_> {
    v.filter(|x| !x.is_null())
}

fn tpl(v: Js<'_>) -> String {
    v.map_or_else(|| "undefined".into(), js_string)
}

fn opt_str(v: Option<String>) -> Value {
    v.map_or(Value::Null, Value::String)
}

// A text split as split(/\r?\n/) splits it.
fn split_lines(s: &str) -> Vec<&str> {
    let parts: Vec<&str> = s.split('\n').collect();
    let last = parts.len() - 1;
    parts.into_iter().enumerate().map(|(i, l)| if i < last { l.strip_suffix('\r').unwrap_or(l) } else { l }).collect()
}

// raw.length - raw.trimStart().length
fn indent_of(raw: &str) -> usize {
    jsstr::len(raw) - jsstr::len(js_trim_start(raw))
}

// v.replace(/^(['"])(.*)\1$/, "$2")
fn unquote(v: &str) -> String {
    let mut cs = v.chars();
    match (cs.next(), cs.next_back()) {
        (Some(a), Some(b)) if a == b && (a == '\'' || a == '"') => {
            let mid = &v[1..v.len() - 1];
            if mid.contains(is_js_line_terminator) {
                v.to_string()
            } else {
                mid.to_string()
            }
        }
        _ => v.to_string(),
    }
}

// ---------- what each app says ----------

fn plan_of(t: Js<'_>) -> Value {
    match t.and_then(Value::as_str) {
        Some("claude_max") => "Max".into(),
        Some("claude_pro") => "Pro".into(),
        Some("claude_team") => "Team".into(),
        Some("claude_enterprise") => "Enterprise".into(),
        _ => Value::Null,
    }
}

/// The account Claude Code is signed in to, from ~/.claude.json's
/// oauthAccount. Never the name fields.
pub fn claude_account(data: Js<'_>) -> Value {
    let a = opt_prop(data, "oauthAccount");
    let get = |k: &str| opt_prop(a, k);
    if str_of(get("accountUuid")).is_none() || str_of(get("organizationUuid")).is_none() {
        return Value::Null;
    }
    let ty = nn(get("organizationType")).map_or_else(String::new, js_string).to_ascii_lowercase();
    let team = ty.contains("team") || ty.contains("enterprise");
    json!({
        "id": format!("claude:{}:{}", tpl(get("accountUuid")), tpl(get("organizationUuid"))),
        "kind": "claude",
        "handle": opt_str(str_of(get("emailAddress"))),
        // A personal organization is named after the email, so only a team's name says anything.
        "detail": { "plan": plan_of(get("organizationType")), "org": if team { opt_str(str_of(get("organizationName"))) } else { Value::Null } },
    })
}

// Buffer.from(s, "base64url"): both alphabets, other characters passed
// over, and the end at the first "=".
fn base64url(s: &str) -> Vec<u8> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = Vec::new();
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    out
}

/// The claims in Codex's ID token (~/.codex/auth.json), decoded without
/// verifying; null when there are none to read. Nothing else in the file
/// is read.
pub fn id_token_claims(auth: Js<'_>) -> Value {
    if !truthy(auth) || !matches!(auth, Some(Value::Object(_) | Value::Array(_))) {
        return Value::Null;
    }
    let token = tpl(opt_prop(opt_prop(auth, "tokens"), "id_token"));
    let Some(part) = token.split('.').nth(1) else { return Value::Null };
    js_json::parse(&String::from_utf8_lossy(&base64url(part))).unwrap_or(Value::Null)
}

/// Codex's sign-in, from auth.json and its ID token's claims (as the lenient
/// reader left them). An API key sign-in says nothing about who.
pub fn chatgpt_account(auth: Js<'_>, claims: Js<'_>) -> Value {
    if !truthy(auth) || !matches!(auth, Some(Value::Object(_) | Value::Array(_))) {
        return Value::Null;
    }
    let empty = Value::Object(Map::new());
    let oa = nn(opt_prop(claims, "https://api.openai.com/auth")).unwrap_or(&empty);
    let oa = Some(oa);
    let tokens = opt_prop(auth, "tokens");
    let workspace = str_of(opt_prop(oa, "chatgpt_account_id")).or_else(|| str_of(opt_prop(tokens, "account_id")));
    let mode = nn(opt_prop(auth, "auth_mode"));
    if let Some(w) = workspace {
        if mode.is_none() || mode.and_then(Value::as_str) == Some("chatgpt") {
            let since = js_number(opt_prop(claims, "auth_time")) * 1000.0;
            let since = if since == 0.0 || since.is_nan() { Value::Null } else { js_json::number(since) };
            return json!({ "id": format!("chatgpt:{w}"), "kind": "chatgpt", "handle": opt_str(str_of(opt_prop(claims, "email"))), "detail": { "plan": opt_str(str_of(opt_prop(oa, "chatgpt_plan_type"))) }, "since": since });
        }
    }
    if truthy(opt_prop(auth, "OPENAI_API_KEY")) || mode.and_then(Value::as_str) == Some("apikey") {
        return json!({ "id": "openai:api-key", "kind": "openai", "handle": null, "detail": { "plan": null } });
    }
    Value::Null
}

/// gh's hosts.yml: each host's signed-in users and the active one. A
/// line-by-line read of the little YAML gh writes; values other than user
/// names are never read, so a token stored in the file stays where it is.
pub fn parse_gh_hosts(text: &str) -> Vec<Value> {
    struct Host {
        name: String,
        users: Vec<String>,
        active: Option<String>,
    }
    let mut hosts: Vec<Host> = Vec::new();
    let mut users_at: i64 = -1;
    let mut user_at: i64 = -1;
    for raw in split_lines(text) {
        let t = js_trim(raw);
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let indent = indent_of(raw) as i64;
        let Some(m) = KEY_LINE.captures(raw) else { continue };
        let key = m.get(1).map_or("", |x| x.as_str());
        let value = m.get(2).map_or("", |x| x.as_str());
        if indent == 0 {
            hosts.push(Host { name: unquote(key).to_lowercase(), users: Vec::new(), active: None });
            users_at = -1;
            user_at = -1;
            continue;
        }
        let Some(host) = hosts.last_mut() else { continue };
        if users_at >= 0 && indent > users_at {
            if user_at < 0 {
                user_at = indent;
            }
            if indent == user_at {
                host.users.push(unquote(key));
            }
            continue;
        }
        users_at = -1;
        user_at = -1;
        if key == "users" {
            users_at = indent;
        } else if key == "user" && !value.is_empty() {
            host.active = Some(unquote(value));
        }
    }
    for h in &mut hosts {
        if let Some(a) = &h.active {
            if !h.users.iter().any(|u| u.to_lowercase() == a.to_lowercase()) {
                h.users.push(a.clone());
            }
        }
    }
    hosts.into_iter().filter(|h| !h.users.is_empty()).map(|h| json!({ "host": h.name, "users": h.users, "active": h.active })).collect()
}

/// hosts.yml as JSON, keys only, for its schema: a key with a value has ""
/// for it, one with keys under it an object, one with a list under it an
/// array, one with nothing null. No value is kept.
pub fn gh_hosts_shape(text: &str) -> Value {
    enum Slot {
        Leaf(Value),
        Node(usize),
    }
    enum Node {
        Obj(Vec<(String, Slot)>),
        Arr,
    }
    struct Frame {
        indent: i64,
        parent: Option<usize>,
        key: String,
        obj: Option<usize>,
    }
    fn put(nodes: &mut [Node], at: usize, key: &str, slot: Slot) {
        if let Node::Obj(kv) = &mut nodes[at] {
            match kv.iter_mut().find(|(k, _)| k == key) {
                Some((_, s)) => *s = slot,
                None => kv.push((key.to_string(), slot)),
            }
        }
    }
    fn value(nodes: &[Node], at: usize) -> Value {
        match &nodes[at] {
            Node::Arr => Value::Array(Vec::new()),
            Node::Obj(kv) => Value::Object(
                kv.iter()
                    .map(|(k, s)| {
                        (
                            k.clone(),
                            match s {
                                Slot::Leaf(v) => v.clone(),
                                Slot::Node(i) => value(nodes, *i),
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }
    let mut nodes = vec![Node::Obj(Vec::new())];
    let mut stack = vec![Frame { indent: -1, parent: None, key: String::new(), obj: Some(0) }];
    for raw in split_lines(text) {
        let line = js_trim(raw);
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let indent = indent_of(raw) as i64;
        while stack.len() > 1 && stack.last().is_some_and(|f| indent <= f.indent) {
            stack.pop();
        }
        let top = stack.len() - 1;
        if line.starts_with('-') {
            if stack[top].obj.is_none() {
                nodes.push(Node::Arr);
                let i = nodes.len() - 1;
                if let Some(p) = stack[top].parent {
                    let key = stack[top].key.clone();
                    put(&mut nodes, p, &key, Slot::Node(i));
                }
                stack[top].obj = Some(i);
            }
            continue;
        }
        let Some(m) = KEY_LINE.captures(line) else { continue };
        if matches!(stack[top].obj.map(|i| &nodes[i]), Some(Node::Arr)) {
            continue;
        }
        let obj = match stack[top].obj {
            Some(i) => i,
            None => {
                nodes.push(Node::Obj(Vec::new()));
                let i = nodes.len() - 1;
                if let Some(p) = stack[top].parent {
                    let key = stack[top].key.clone();
                    put(&mut nodes, p, &key, Slot::Node(i));
                }
                stack[top].obj = Some(i);
                i
            }
        };
        let key = unquote(m.get(1).map_or("", |x| x.as_str()));
        let has_value = m.get(2).is_some_and(|x| !x.as_str().is_empty());
        put(&mut nodes, obj, &key, Slot::Leaf(if has_value { Value::String(String::new()) } else { Value::Null }));
        if !has_value {
            stack.push(Frame { indent, parent: Some(obj), key, obj: None });
        }
    }
    value(&nodes, 0)
}

/// SSH host aliases (`Host gh-work` with `HostName github.com`), so a remote
/// like git@gh-work:me/app counts as GitHub: alias → host, in order.
pub fn parse_ssh_aliases(text: &str) -> Vec<(String, String)> {
    let mut aliases: Vec<(String, String)> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    for raw in split_lines(text) {
        let Some(m) = SSH_LINE.captures(raw) else { continue };
        let key = m[1].to_lowercase();
        let value = &m[2];
        if key == "host" {
            names = SPACES.split(value).filter(|n| !n.contains(['*', '?', '!'])).map(str::to_lowercase).collect();
        } else if key == "match" {
            names.clear();
        } else if key == "hostname" {
            for n in &names {
                if !aliases.iter().any(|(a, _)| a == n) {
                    aliases.push((n.clone(), value.to_lowercase()));
                }
            }
        }
    }
    aliases
}

/// A remote URL's parts, with SSH aliases resolved. `user` is a user name
/// written into an HTTPS URL; anything after it (a token) is dropped.
#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    pub host: String,
    pub owner: String,
    pub repo: String,
    pub https: bool,
    pub user: Option<String>,
}

impl Remote {
    pub fn to_json(&self) -> Value {
        json!({ "host": self.host, "owner": self.owner, "repo": self.repo, "https": self.https, "user": self.user })
    }
}

// /^(?:[^@/]+@)?([^:/\\]{2,}):(?!\/\/)(.+)$/: the scp form, host and path.
fn scp_form(s: &str) -> Option<(String, String)> {
    let at = s.find(['@', '/']).filter(|&i| i > 0 && s.as_bytes()[i] == b'@').map(|i| i + 1);
    for start in [at, Some(0)].into_iter().flatten() {
        let rest = &s[start..];
        let end = rest.find([':', '/', '\\']).unwrap_or(rest.len());
        let host = &rest[..end];
        if jsstr::len(host) < 2 || rest.as_bytes().get(end) != Some(&b':') {
            continue;
        }
        let path = &rest[end + 1..];
        if path.starts_with("//") || path.is_empty() || path.contains(is_js_line_terminator) {
            continue;
        }
        return Some((host.to_string(), path.to_string()));
    }
    None
}

/// parseRemote: a remote URL's host, owner and repo.
pub fn parse_remote(url: &str, aliases: &[(String, String)]) -> Option<Remote> {
    let s = js_trim(url);
    let (scheme, user, host, rest) = match URL.captures(s) {
        Some(m) => (m[1].to_string(), m.get(2).map(|x| x.as_str().to_string()), m[3].to_string(), m[4].to_string()),
        None => {
            let (host, rest) = scp_form(s)?;
            ("ssh".to_string(), None, host, rest)
        }
    };
    let scheme = scheme.to_lowercase();
    let path = rest.trim_start_matches('/');
    let path = path.strip_suffix(".git/").or_else(|| path.strip_suffix(".git")).unwrap_or(path);
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    let lower = host.to_lowercase();
    let host = aliases.iter().find(|(a, _)| *a == lower).map_or(lower, |(_, h)| h.clone());
    Some(Remote { host, owner: owner.into(), repo: repo.into(), https: scheme == "https" || scheme == "http", user: user.filter(|u| scheme.starts_with("http") && !u.is_empty()) })
}

/// `git config --get-regexp` output: the identity commits get, and the
/// credential settings that say who a push signs in as.
pub fn parse_git_config(out: &str) -> Value {
    let (mut name, mut email) = (Value::Null, Value::Null);
    let (mut helpers, mut usernames) = (Vec::new(), Vec::new());
    for line in split_lines(out) {
        if line.is_empty() {
            continue;
        }
        let (key, value) = match line.find(' ') {
            Some(sp) => (&line[..sp], js_trim(&line[sp + 1..])),
            None => (line, ""),
        };
        let lower = key.to_lowercase();
        let some = |v: &str| if v.is_empty() { Value::Null } else { Value::String(v.into()) };
        if lower == "user.name" {
            name = some(value);
        } else if lower == "user.email" {
            email = some(value);
        } else if lower.starts_with("credential.") {
            let v = &lower[lower.rfind('.').map_or(0, |i| i + 1)..];
            let end = jsstr::len(key) as i64 - jsstr::len(v) as i64 - 1;
            let scope = jsstr::slice(key, 11, Some(end));
            let scope = if scope.is_empty() { Value::Null } else { Value::String(scope) };
            if v == "helper" {
                helpers.push(json!({ "scope": scope, "value": value }));
            } else if v == "username" && !value.is_empty() {
                usernames.push(json!({ "scope": scope, "user": value }));
            }
        }
    }
    json!({ "name": name, "email": email, "helpers": helpers, "usernames": usernames })
}

// Does a credential section's URL cover this remote?
fn covers(scope: &str, r: &Remote) -> bool {
    let Some(m) = SCOPE.captures(scope) else { return false };
    if m[2].to_lowercase() != r.host {
        return false;
    }
    let prefix = m.get(3).map_or("", |x| x.as_str()).trim_end_matches('/').to_lowercase();
    prefix.is_empty() || format!("/{}/{}", r.owner, r.repo).to_lowercase().starts_with(&prefix)
}

fn items<'a>(v: Js<'a>, what: &str) -> Result<&'a [Value], TypeError> {
    match v {
        Some(Value::Array(a)) => Ok(a),
        _ => Err(TypeError(format!("{what} is not a function"))),
    }
}

/// Who a push to `url` signs in as: a user name in the URL or in git's
/// credential settings, or, with gh as the credential helper, gh's active
/// account for the host. SSH pushes use a key, which names no account.
pub fn push_account(url: Js<'_>, cfg: Js<'_>, aliases: &[(String, String)]) -> Result<Value, TypeError> {
    let url = nn(url).map_or_else(String::new, js_string);
    let Some(r) = parse_remote(&url, aliases).filter(|r| r.https) else { return Ok(Value::Null) };
    if let Some(u) = &r.user {
        return Ok(json!({ "host": r.host, "user": u, "how": "the remote URL" }));
    }
    let usernames = items(prop(cfg, "usernames")?, "cfg.usernames.filter")?;
    let mut named = None;
    for u in usernames {
        let scope = prop(Some(u), "scope")?;
        if scope.filter(|s| truthy(Some(s))).is_some_and(|s| covers(&js_string(s), &r)) {
            named = Some(u);
        }
    }
    if named.is_none() {
        for u in usernames {
            if !truthy(prop(Some(u), "scope")?) {
                named = Some(u);
            }
        }
    }
    if let Some(u) = named {
        return Ok(json!({ "host": r.host, "user": opt_prop(Some(u), "user").cloned(), "how": "git config" }));
    }
    let mut helpers: Vec<String> = Vec::new();
    for h in items(prop(cfg, "helpers")?, "cfg.helpers")? {
        let scope = prop(Some(h), "scope")?;
        if scope.filter(|s| truthy(Some(s))).is_none_or(|s| covers(&js_string(s), &r)) {
            let value = prop(Some(h), "value")?;
            if let Some(value) = value.filter(|v| truthy(Some(v))) {
                helpers.push(js_string(value));
            } else {
                helpers.clear();
            }
        }
    }
    if helpers.iter().any(|h| GH_HELPER.is_match(h)) {
        return Ok(json!({ "host": r.host, "user": null, "how": "gh" }));
    }
    Ok(Value::Null)
}

// ---------- the sign-in timeline ----------

/// What noting a sign-in does to a timeline.
#[derive(Clone, Debug, PartialEq)]
pub enum Signin {
    /// Nothing changed.
    Same,
    /// The last span now ends at this time.
    Extend(f64),
    /// A new span, and how many of the oldest to drop to keep 50.
    Push { span: Value, drop: usize },
}

// a === b for JSON values (objects are never the same one).
fn strict_eq(a: Js<'_>, b: Js<'_>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(Value::Null), Some(Value::Null)) => true,
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x == y,
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64() == y.as_f64(),
        (Some(Value::String(x)), Some(Value::String(y))) => x == y,
        _ => false,
    }
}

/// Records that `id` is signed in at `now` (noteSignin): `since` is when
/// the app says the sign-in happened, if it says.
pub fn note_signin(spans: &[Value], id: &Value, now: f64, since: Js<'_>) -> Signin {
    let last = spans.last();
    if strict_eq(opt_prop(last, "id"), Some(id)) {
        if now - js_number(opt_prop(last, "to")) < SIGNIN_STEP_MS {
            return Signin::Same;
        }
        return Signin::Extend(now);
    }
    let from = match nn(since) {
        Some(s) => js_number(Some(s)),
        None => now,
    };
    // last?.to ?? -Infinity
    let to = nn(opt_prop(last, "to")).map_or(f64::NEG_INFINITY, |t| js_number(Some(t)));
    let from = if from.is_nan() || to.is_nan() { f64::NAN } else { from.max(to) };
    let span = json!({ "id": id, "from": js_json::number(from), "to": js_json::number(now) });
    let drop = (spans.len() + 1).saturating_sub(MAX_SPANS);
    Signin::Push { span, drop }
}

/// Applies a sign-in to a timeline held here.
fn apply_signin(spans: &mut Vec<Value>, op: &Signin) {
    match op {
        Signin::Same => {}
        Signin::Extend(to) => {
            if let Some(Value::Object(m)) = spans.last_mut() {
                m.insert("to".into(), js_json::number(*to));
            }
        }
        Signin::Push { span, drop } => {
            spans.push(span.clone());
            spans.drain(..*drop);
        }
    }
}

// A value as a Set's or a Map's key.
fn key_of(v: Js<'_>) -> String {
    match v {
        None => "u".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::Bool(b)) => format!("b:{b}"),
        Some(Value::Number(x)) => format!("n:{}", js_json::number_string(x.as_f64().unwrap_or(f64::NAN))),
        Some(Value::String(s)) => format!("s:{s}"),
        Some(o) => format!("o:{}", js_json::stringify(o, 0)),
    }
}

fn link(id: Value, confidence: &str, via: &str) -> Value {
    json!({ "id": id, "confidence": confidence, "via": via })
}

/// Which account an app was signed in to at `t` (accountAt): the current
/// one from when it signed in, an earlier span that covers t, or, failing
/// those, the only account XenoNet knows of for the app.
pub fn account_at(spans: &[Value], current: Js<'_>, t: Js<'_>, known: &[Value]) -> Value {
    if nn(t).is_some() {
        let t = js_number(t);
        let last = spans.last();
        if truthy(current) {
            let since = match nn(opt_prop(current, "since")) {
                Some(s) => js_number(Some(s)),
                None => f64::INFINITY,
            };
            let from = if strict_eq(opt_prop(last, "id"), opt_prop(current, "id")) { js_number(opt_prop(last, "from")) } else { f64::INFINITY };
            let min = if since.is_nan() || from.is_nan() { f64::NAN } else { since.min(from) };
            if t >= min {
                return link(opt_prop(current, "id").cloned().unwrap_or(Value::Null), "likely", "signed in when it ran");
            }
        }
        for i in (0..spans.len()).rev() {
            let s = Some(&spans[i]);
            let next = spans.get(i + 1);
            let from = js_number(opt_prop(s, "from"));
            let inside = t <= js_number(opt_prop(s, "to")) || (next.is_some() && strict_eq(opt_prop(next, "id"), opt_prop(s, "id")) && t < js_number(opt_prop(next, "from")));
            if t >= from && inside {
                return link(opt_prop(s, "id").cloned().unwrap_or(Value::Null), "likely", "signed in when it ran");
            }
        }
    }
    let mut ids: Vec<(String, Value)> = Vec::new();
    let mut add = |v: Js<'_>| {
        let k = key_of(v);
        if !ids.iter().any(|(x, _)| *x == k) {
            ids.push((k, v.cloned().unwrap_or(Value::Null)));
        }
    };
    for k in known {
        add(Some(k));
    }
    for s in spans {
        add(opt_prop(Some(s), "id"));
    }
    if truthy(current) {
        add(opt_prop(current, "id"));
    }
    if ids.len() == 1 {
        link(ids.remove(0).1, "assumed", "the only account XenoNet has seen")
    } else {
        Value::Null
    }
}

// ---------- linking ----------

pub fn empty_saved() -> Value {
    json!({ "names": {}, "hidden": {}, "pins": {}, "known": {}, "signins": {} })
}

pub fn github_id(host: &str, login: &str) -> String {
    format!("github:{host}/{}", login.to_lowercase())
}

/// A git identity's id: a hash of its email, never the email.
pub fn git_id(email: &str) -> String {
    use std::fmt::Write;
    let h = Sha256::digest(js_trim(email).to_lowercase().as_bytes());
    let mut id = String::from("git:");
    for b in &h[..8] {
        let _ = write!(id, "{b:02x}");
    }
    id
}

pub fn claude_id(account: &str, org: &str) -> String {
    format!("claude:{account}:{org}")
}

/// What linkIdentities works from: the map's nodes (each with id, kind,
/// app, claudeAccount, provider, updatedAt, createdAt, origin, key, group
/// and name), its links (source, target, kind), the accounts signed in now,
/// ssh aliases, each repo's git identity and each session's transcript
/// owner (by key, as Maps keep them), the saved state, home, and now.
pub struct LinkInput<'a> {
    pub nodes: &'a [Value],
    pub links: &'a [(Value, Value, Value)],
    pub accounts: &'a Value,
    pub ssh: &'a [(String, String)],
    pub repos: &'a [(Value, Value)],
    pub owners: &'a [(Value, Value)],
    pub saved: &'a Value,
    pub home: Option<&'a str>,
    pub now: f64,
}

/// What it changed: each session's account and each project's identities
/// (by node index), the identities with their counts, the sign-in
/// timelines' changes (app, change), what's remembered of each identity
/// (id, replaced whole or merged in, the fields), and whether the saved
/// state changed.
#[derive(Debug, Default)]
pub struct LinkOutput {
    pub ran_as: Vec<(usize, Value)>,
    pub identities: Vec<(usize, Value)>,
    pub list: Vec<Value>,
    pub signins: Vec<(&'static str, Signin)>,
    pub known: Vec<(String, bool, Value)>,
    pub dirty: bool,
}

// A Map built from pairs: the last value of each key, looked up by key.
fn map_of(pairs: &[(Value, Value)]) -> HashMap<String, &Value> {
    pairs.iter().map(|(k, v)| (key_of(Some(k)), v)).collect()
}

struct Records {
    order: Vec<String>,
    recs: HashMap<String, Map<String, Value>>,
}

impl Records {
    // record(r, src, extra)
    fn record(&mut self, r: &Value, src: Option<&str>, extra: &[(&str, Value)]) {
        let id = r.get("id").cloned().unwrap_or(Value::Null);
        let key = key_of(Some(&id));
        let rec = match self.recs.get_mut(&key) {
            Some(rec) => rec,
            None => {
                let mut m = Map::new();
                m.insert("id".into(), id);
                m.insert("kind".into(), r.get("kind").cloned().unwrap_or(Value::Null));
                m.insert("handle".into(), nn(r.get("handle")).cloned().unwrap_or(Value::Null));
                m.insert("detail".into(), nn(r.get("detail")).cloned().filter(Value::is_object).unwrap_or_else(|| Value::Object(Map::new())));
                m.insert("signedIn".into(), Value::Bool(false));
                m.insert("active".into(), Value::Bool(false));
                m.insert("sources".into(), Value::Array(Vec::new()));
                self.order.push(key.clone());
                self.recs.entry(key).or_insert(m)
            }
        };
        if !truthy(rec.get("handle")) && truthy(r.get("handle")) {
            rec.insert("handle".into(), r["handle"].clone());
        }
        if let Some(Value::Object(d)) = r.get("detail") {
            if let Some(Value::Object(rd)) = rec.get_mut("detail") {
                for (k, v) in d {
                    if !v.is_null() {
                        rd.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        if let Some(src) = src.filter(|s| !s.is_empty()) {
            if let Some(Value::Array(ss)) = rec.get_mut("sources") {
                if !ss.iter().any(|s| s.as_str() == Some(src)) {
                    ss.push(Value::String(src.into()));
                }
            }
        }
        for (k, v) in extra {
            rec.insert((*k).into(), v.clone());
        }
    }

    fn has(&self, id: Js<'_>) -> bool {
        self.recs.contains_key(&key_of(id))
    }
}

// { ...a, ...b } for two details (objects; anything else adds nothing).
fn merged(a: Js<'_>, b: Js<'_>) -> Value {
    let mut m = match a {
        Some(Value::Object(x)) => x.clone(),
        _ => Map::new(),
    };
    if let Some(Value::Object(y)) = b {
        for (k, v) in y {
            m.insert(k.clone(), v.clone());
        }
    }
    Value::Object(m)
}

// saved[part]?.[key], as a property is looked up by a key's string.
fn saved_at<'a>(saved: &'a Value, part: &str, key: Js<'_>) -> Js<'a> {
    opt_prop(opt_prop(Some(saved), part), &tpl(key))
}

fn owner_of(links: &[Value]) -> Option<&Value> {
    links.iter().find(|l| l.get("role").and_then(Value::as_str) == Some("owner")).and_then(|l| l.get("id"))
}

/// Puts identity links on the map (linkIdentities): which account each
/// session ran as, and each project's owner, who pushes and who commits.
pub fn link_identities(input: LinkInput<'_>) -> Result<LinkOutput, TypeError> {
    let LinkInput { nodes, links, accounts, ssh, repos, owners, saved, home, now } = input;
    let mut out = LinkOutput::default();
    let mut recs = Records { order: Vec::new(), recs: HashMap::new() };
    let acc = |k: &str| opt_prop(Some(accounts), k);
    let source = |s: String| match home {
        Some(h) if !s.is_empty() && !h.is_empty() => paths::tilde(&s, h),
        _ => s,
    };

    // Signed in now.
    let gh: &[Value] = match nn(acc("gh")) {
        None => &[],
        Some(Value::Array(a)) => a,
        Some(_) => return Err(TypeError("gh is not iterable".into())),
    };
    let file_of = |k: &str, dflt: &str| nn(acc(k)).map_or_else(|| dflt.to_string(), js_string);
    if let Some(claude) = acc("claude").filter(|v| truthy(Some(v))) {
        recs.record(claude, Some(&source(file_of("claudeFile", "~/.claude.json"))), &[("signedIn", Value::Bool(true))]);
    }
    if let Some(chatgpt) = acc("chatgpt").filter(|v| truthy(Some(v))) {
        recs.record(chatgpt, Some(&source(file_of("chatgptFile", "~/.codex/auth.json"))), &[("signedIn", Value::Bool(true))]);
    }
    let lower = |v: Js<'_>, what: &str| -> Result<String, TypeError> {
        match v {
            Some(Value::String(s)) => Ok(s.to_lowercase()),
            None => Err(TypeError(format!("Cannot read properties of undefined (reading 'toLowerCase'){what}"))),
            Some(Value::Null) => Err(TypeError(format!("Cannot read properties of null (reading 'toLowerCase'){what}"))),
            Some(_) => Err(TypeError("toLowerCase is not a function".into())),
        }
    };
    for h in gh {
        let host = prop(Some(h), "host")?;
        let active = match nn(prop(Some(h), "active")?) {
            None => None,
            a => Some(lower(a, "")?),
        };
        for u in items(prop(Some(h), "users")?, "h.users")? {
            let login = lower(Some(u), "")?;
            let id = github_id(&tpl(host), &js_string(u));
            let r = json!({ "id": id, "kind": "github", "handle": u, "detail": { "host": host.cloned().unwrap_or(Value::Null) } });
            recs.record(&r, Some("gh"), &[("signedIn", Value::Bool(true)), ("active", Value::Bool(Some(&login) == active.as_ref()))]);
        }
    }
    let gh_host = |host: &str| gh.iter().find(|h| h.get("host").and_then(Value::as_str) == Some(host));
    let gh_login = |host: &str, login: &str| -> Option<String> {
        let want = login.to_lowercase();
        gh_host(host)?.get("users")?.as_array()?.iter().filter_map(Value::as_str).find(|u| u.to_lowercase() == want).map(str::to_string)
    };
    let is_forge = |host: &str| host == "github.com" || host.ends_with(".ghe.com") || gh.iter().any(|h| h.get("host").and_then(Value::as_str) == Some(host));

    // The sign-in timelines.
    let empty = Value::Object(Map::new());
    let saved_signins = nn(opt_prop(Some(saved), "signins")).unwrap_or(&empty);
    let mut signins: HashMap<&'static str, Vec<Value>> = HashMap::new();
    for app in ["claude", "chatgpt"] {
        let spans = match nn(opt_prop(Some(saved_signins), app)) {
            None => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(_) => return Err(TypeError("spans.push is not a function".into())),
        };
        signins.insert(app, spans);
    }
    for app in ["claude", "chatgpt"] {
        let a = acc(app);
        if !truthy(a) {
            continue;
        }
        let spans = signins.get_mut(app).expect("both apps' timelines are read above");
        let op = note_signin(spans, a.and_then(|x| x.get("id")).unwrap_or(&Value::Null), now, opt_prop(a, "since"));
        if op != Signin::Same {
            out.dirty = true;
        }
        apply_signin(spans, &op);
        out.signins.push((app, op));
    }

    // Sessions: the account each ran as.
    let owners_by = map_of(owners);
    let repos_by = map_of(repos);
    let mut by_id: HashMap<String, usize> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        by_id.insert(key_of(prop(Some(n), "id")?), i);
    }
    let claude_of = |a: &Value| -> Result<String, TypeError> { Ok(claude_id(&tpl(prop(Some(a), "account")?), &tpl(prop(Some(a), "org")?))) };
    let mut recorded: Vec<String> = Vec::new();
    for (_, o) in owners {
        if truthy(Some(o)) {
            recorded.push(claude_of(o)?);
        }
    }
    let is = |n: &Value, k: &str, v: &str| n.get(k).and_then(Value::as_str) == Some(v);
    for n in nodes {
        if is(n, "kind", "session") && is(n, "app", "claude") && truthy(n.get("claudeAccount")) {
            recorded.push(claude_of(&n["claudeAccount"])?);
        }
    }
    let known_saved = nn(opt_prop(Some(saved), "known")).unwrap_or(&empty);
    let known_of = |kind: &str| -> Result<Vec<Value>, TypeError> {
        let mut ids = Vec::new();
        if let Value::Object(m) = known_saved {
            for (id, k) in m {
                if prop(Some(k), "kind")?.and_then(Value::as_str) == Some(kind) {
                    ids.push(Value::String(id.clone()));
                }
            }
        }
        Ok(ids)
    };
    let mut claude_known = known_of("claude")?;
    claude_known.extend(recorded.into_iter().map(Value::String));
    let mut chatgpt_known = known_of("chatgpt")?;
    chatgpt_known.extend(known_of("openai")?);
    let current = |a: Js<'_>| -> Value {
        if truthy(a) {
            json!({ "id": opt_prop(a, "id").cloned().unwrap_or(Value::Null), "since": nn(opt_prop(a, "since")).cloned().unwrap_or(Value::Null) })
        } else {
            Value::Null
        }
    };
    let mut ran: HashMap<usize, Value> = HashMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if !is(n, "kind", "session") {
            continue;
        }
        let pin = saved_at(saved, "pins", n.get("id"));
        let when = nn(n.get("updatedAt")).or(n.get("createdAt"));
        let l = if truthy(pin) {
            link(prop(pin, "identity")?.cloned().unwrap_or(Value::Null), "yours", "your pin")
        } else if is(n, "app", "claude") {
            if truthy(n.get("claudeAccount")) {
                link(Value::String(claude_of(&n["claudeAccount"])?), "exact", "Claude desktop's session folder")
            } else if let Some(o) = owners_by.get(&key_of(n.get("id"))).filter(|o| truthy(Some(o))) {
                link(Value::String(claude_of(o)?), "exact", "its transcript")
            } else {
                account_at(&signins["claude"], Some(&current(acc("claude"))), when, &claude_known)
            }
        } else if is(n, "app", "codex") && (!truthy(n.get("provider")) || is(n, "provider", "openai")) {
            account_at(&signins["chatgpt"], Some(&current(acc("chatgpt"))), when, &chatgpt_known)
        } else {
            Value::Null
        };
        if !l.is_null() {
            let id = &l["id"];
            let Value::String(ids) = id else { return Err(TypeError("link.id.split is not a function".into())) };
            let kind = ids.split(':').next().unwrap_or("").to_string();
            let handle = nn(opt_prop(saved_at(saved, "known", Some(id)), "handle")).cloned().unwrap_or(Value::Null);
            let src = if l["via"] == "Claude desktop's session folder" { Some("Claude desktop") } else { None };
            recs.record(&json!({ "id": id, "kind": kind, "handle": handle }), src, &[]);
        }
        ran.insert(i, l.clone());
        out.ran_as.push((i, l));
    }

    // Projects: who owns the remote, who pushes, who commits.
    let mut parent: HashMap<String, Value> = HashMap::new();
    for (src, tgt, kind) in links {
        if kind.as_str() != Some("lineage-cross") {
            parent.insert(key_of(Some(tgt)), src.clone());
        }
    }
    let projects: Vec<usize> = (0..nodes.len()).filter(|&i| is(&nodes[i], "kind", "project")).collect();
    let mut ids_of: HashMap<usize, Vec<Value>> = HashMap::new();
    for &pi in &projects {
        let p = &nodes[pi];
        let mut ls = Vec::new();
        let origin = nn(p.get("origin")).map_or_else(String::new, js_string);
        if let Some(remote) = parse_remote(&origin, ssh).filter(|r| is_forge(&r.host)) {
            let login = gh_login(&remote.host, &remote.owner).unwrap_or_else(|| remote.owner.clone());
            let id = github_id(&remote.host, &login);
            recs.record(&json!({ "id": id, "kind": "github", "handle": login, "detail": { "host": remote.host } }), Some("remote URLs"), &[]);
            ls.push(json!({ "id": id, "role": "owner", "confidence": "exact", "via": "its origin remote" }));
        }
        let repo = repos_by.get(&key_of(p.get("key"))).copied();
        if let Some(email) = opt_prop(repo, "email").filter(|e| truthy(Some(e))) {
            let id = git_id(&js_string(email));
            recs.record(
                &json!({ "id": id, "kind": "git", "handle": email, "detail": { "name": nn(opt_prop(repo, "name")).cloned().unwrap_or(Value::Null) } }),
                Some("git config"),
                &[],
            );
            ls.push(json!({ "id": id, "role": "commits-as", "confidence": "exact", "via": "git config" }));
        }
        let push = opt_prop(repo, "push");
        if truthy(push) {
            let user = nn(opt_prop(push, "user"));
            let host = opt_prop(push, "host");
            let host_s = tpl(host);
            let login = user.cloned().or_else(|| host.and_then(Value::as_str).and_then(gh_host).and_then(|h| h.get("active").cloned()));
            if let Some(login) = login.filter(|l| truthy(Some(l))) {
                let login_s = js_string(&login);
                let spelled = gh_login(&host_s, &login_s).map_or(login.clone(), Value::String);
                let id = github_id(&host_s, &js_string(&spelled));
                let mine = truthy(user);
                recs.record(
                    &json!({ "id": id, "kind": "github", "handle": spelled, "detail": { "host": host.cloned().unwrap_or(Value::Null) } }),
                    Some(if mine { "git config" } else { "gh" }),
                    &[],
                );
                let mut l = Map::new();
                l.insert("id".into(), Value::String(id));
                l.insert("role".into(), "pushes-as".into());
                l.insert("confidence".into(), (if mine { "exact" } else { "likely" }).into());
                if mine {
                    // repo.push.how, which JSON leaves out when it isn't there.
                    if let Some(how) = opt_prop(push, "how") {
                        l.insert("via".into(), how.clone());
                    }
                } else {
                    l.insert("via".into(), "gh's active account".into());
                }
                ls.push(Value::Object(l));
            }
        }
        ids_of.insert(pi, ls);
    }
    // A group takes the owner all its projects share, and a project with
    // no remote of its own takes its group's.
    // b.key.length - a.key.length, which throws on a key that isn't there
    // once there are two groups to compare.
    let key_len = |i: usize| -> Result<usize, TypeError> {
        match nodes[i].get("key") {
            Some(Value::String(k)) => Ok(jsstr::len(k)),
            None => Err(TypeError("Cannot read properties of undefined (reading 'length')".into())),
            Some(Value::Null) => Err(TypeError("Cannot read properties of null (reading 'length')".into())),
            Some(_) => Ok(0),
        }
    };
    let group_nodes: Vec<usize> = projects.iter().copied().filter(|&pi| truthy(nodes[pi].get("group"))).collect();
    let mut groups: Vec<(usize, usize)> = Vec::new();
    for &pi in &group_nodes {
        groups.push((pi, if group_nodes.len() > 1 { key_len(pi)? } else { 0 }));
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.1));
    let parent_of = |i: usize| parent.get(&key_of(nodes[i].get("id")));
    for (g, _) in groups {
        let gid = nodes[g].get("id");
        let mut owners_in: Vec<Value> = Vec::new();
        for &p in &projects {
            if p != g && parent_of(p).is_some_and(|x| strict_eq(Some(x), gid)) {
                if let Some(o) = owner_of(&ids_of[&p]).filter(|o| truthy(Some(o))) {
                    if !owners_in.iter().any(|x| key_of(Some(x)) == key_of(Some(o))) {
                        owners_in.push(o.clone());
                    }
                }
            }
        }
        if owners_in.len() == 1 && owner_of(&ids_of[&g]).as_ref().is_none_or(|o| !truthy(Some(o))) {
            ids_of
                .get_mut(&g)
                .expect("every project and group has its list")
                .push(json!({ "id": owners_in[0], "role": "owner", "confidence": "likely", "via": "the projects in it" }));
        }
    }
    // A satellite takes its project's. Two passes, for a satellite of a
    // project in a group.
    for _ in 0..2 {
        for &p in &projects {
            if truthy(nodes[p].get("group")) || owner_of(&ids_of[&p]).is_some_and(|o| truthy(Some(o))) {
                continue;
            }
            let Some(up) = parent_of(p).and_then(|x| by_id.get(&key_of(Some(x)))).copied() else { continue };
            if !is(&nodes[up], "kind", "project") {
                continue;
            }
            let inherited = ids_of.get(&up).and_then(|ls| ls.iter().find(|l| l.get("role").and_then(Value::as_str) == Some("owner"))).cloned();
            if let Some(inh) = inherited {
                let confidence =
                    if inh.get("confidence").and_then(Value::as_str) == Some("exact") { Value::from("likely") } else { inh.get("confidence").cloned().unwrap_or(Value::Null) };
                let name = tpl(nodes[up].get("name"));
                let via = if truthy(nodes[up].get("group")) { format!("its group, {name}") } else { format!("the project it's grouped with, {name}") };
                ids_of
                    .get_mut(&p)
                    .expect("every project and group has its list")
                    .push(json!({ "id": inh.get("id").cloned().unwrap_or(Value::Null), "role": "owner", "confidence": confidence, "via": via }));
            }
        }
    }
    for &p in &projects {
        let pin = saved_at(saved, "pins", nodes[p].get("id"));
        if truthy(pin) {
            let mut ls = vec![json!({ "id": prop(pin, "identity")?.cloned().unwrap_or(Value::Null), "role": "owner", "confidence": "yours", "via": "your pin" })];
            ls.extend(ids_of[&p].iter().filter(|l| l.get("role").and_then(Value::as_str) != Some("owner")).cloned());
            ids_of.insert(p, ls);
        }
    }

    // Remember what was seen, so an account you've switched away from
    // keeps its email, and every identity keeps when XenoNet first saw it.
    let mut known: Map<String, Value> = known_saved.as_object().cloned().unwrap_or_default();
    for key in recs.order.clone() {
        let rec = recs.recs.get_mut(&key).expect("order lists the records' keys");
        let id = tpl(rec.get("id"));
        let k = known.get(&id).cloned();
        let k = k.filter(|k| truthy(Some(k)));
        if !truthy(rec.get("handle")) {
            if let Some(h) = opt_prop(k.as_ref(), "handle").filter(|h| truthy(Some(h))) {
                rec.insert("handle".into(), h.clone());
            }
        }
        if truthy(opt_prop(k.as_ref(), "detail")) {
            let d = merged(opt_prop(k.as_ref(), "detail"), rec.get("detail"));
            rec.insert("detail".into(), d);
        }
        match &k {
            None => {
                let v = json!({ "kind": rec.get("kind"), "handle": rec.get("handle"), "detail": rec.get("detail"), "firstSeen": js_json::number(now), "lastSeen": js_json::number(now) });
                known.insert(id.clone(), v.clone());
                out.known.push((id.clone(), true, v));
                out.dirty = true;
            }
            Some(k) => {
                let handle = rec.get("handle").filter(|h| truthy(Some(h)));
                let changed = handle.is_some_and(|h| !strict_eq(Some(h), k.get("handle")));
                let last = js_number(Some(nn(k.get("lastSeen")).unwrap_or(&Value::from(0))));
                if changed || now - last >= SIGNIN_STEP_MS {
                    let v = json!({ "handle": nn(rec.get("handle")).or(k.get("handle")).cloned(), "detail": merged(k.get("detail"), rec.get("detail")), "lastSeen": js_json::number(now) });
                    if let Some(Value::Object(km)) = known.get_mut(&id) {
                        if let Value::Object(fields) = &v {
                            km.extend(fields.iter().map(|(f, x)| (f.clone(), x.clone())));
                        }
                    }
                    out.known.push((id.clone(), false, v));
                    out.dirty = true;
                }
            }
        }
        match opt_prop(known.get(&id), "firstSeen") {
            Some(f) => {
                rec.insert("firstSeen".into(), f.clone());
            }
            None => {
                rec.shift_remove("firstSeen");
            }
        }
        rec.insert("seenAt".into(), js_json::number(now));
    }
    if let Some(Value::Object(pins)) = nn(opt_prop(Some(saved), "pins")) {
        for pin in pins.values() {
            let identity = prop(Some(pin), "identity")?;
            let k = known.get(&tpl(identity)).filter(|k| truthy(Some(k)));
            let Some(k) = k else { continue };
            if recs.has(identity) {
                continue;
            }
            let mut r = Map::new();
            r.insert("id".into(), identity.cloned().unwrap_or(Value::Null));
            if let Value::Object(km) = k {
                for (f, x) in km {
                    r.insert(f.clone(), x.clone());
                }
            }
            // { firstSeen: known[pin.identity].firstSeen }, left out when
            // it isn't there, as JSON leaves out undefined.
            let first: Vec<(&str, Value)> = k.get("firstSeen").map(|f| ("firstSeen", f.clone())).into_iter().collect();
            recs.record(&Value::Object(r), None, &first);
        }
    }

    // How many sessions and projects each one reaches; a session reaches
    // its project's identities too.
    let home_project = |i: usize| -> Option<usize> {
        let mut cur = parent_of(i).cloned();
        let mut n = 0;
        while let Some(c) = cur.filter(|c| truthy(Some(c))) {
            if n >= 12 {
                break;
            }
            let at = by_id.get(&key_of(Some(&c))).copied();
            if at.is_some_and(|a| is(&nodes[a], "kind", "project")) {
                return at;
            }
            cur = parent.get(&key_of(Some(&c))).cloned();
            n += 1;
        }
        None
    };
    let mut count: HashMap<String, (f64, f64)> = HashMap::new();
    for &p in &projects {
        let mut seen: Vec<String> = Vec::new();
        for l in &ids_of[&p] {
            let k = key_of(l.get("id"));
            if !seen.contains(&k) {
                seen.push(k.clone());
                count.entry(k).or_default().1 += 1.0;
            }
        }
    }
    for (i, n) in nodes.iter().enumerate() {
        if !is(n, "kind", "session") {
            continue;
        }
        let mut ids: Vec<&Value> = Vec::new();
        if let Some(r) = ran.get(&i).and_then(|r| r.get("id")) {
            ids.push(r);
        }
        let home_ids = home_project(i).and_then(|h| ids_of.get(&h));
        ids.extend(home_ids.into_iter().flatten().filter_map(|l| l.get("id")));
        let mut seen: Vec<String> = Vec::new();
        for id in ids.into_iter().filter(|x| truthy(Some(x))) {
            let k = key_of(Some(id));
            if !seen.contains(&k) {
                seen.push(k.clone());
                count.entry(k).or_default().0 += 1.0;
            }
        }
    }

    let mut list: Vec<Value> = recs
        .order
        .iter()
        .map(|key| {
            let mut r = recs.recs[key].clone();
            let id = r.get("id").cloned();
            r.insert("name".into(), nn(saved_at(saved, "names", id.as_ref())).cloned().unwrap_or(Value::Null));
            r.insert("hidden".into(), Value::Bool(truthy(saved_at(saved, "hidden", id.as_ref()))));
            let (s, p) = count.get(key).copied().unwrap_or((0.0, 0.0));
            r.insert("sessions".into(), js_json::number(s));
            r.insert("projects".into(), js_json::number(p));
            Value::Object(r)
        })
        .collect();
    let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    list.sort_by(|a, b| {
        num(b, "sessions")
            .total_cmp(&num(a, "sessions"))
            .then(num(b, "projects").total_cmp(&num(a, "projects")))
            .then_with(|| jsstr::locale_compare(&tpl(a.get("id")), &tpl(b.get("id"))))
    });
    out.list = list;
    out.identities = projects.iter().map(|&p| (p, Value::Array(ids_of.remove(&p).unwrap_or_default()))).collect();
    Ok(out)
}

// ---------- reading it from disk ----------

/// The config file a repo's settings live in: .git/config, or for a
/// worktree or submodule, the one its .git file points at.
pub fn repo_config_file(dir: &str) -> Option<String> {
    let mut git_dir = paths::join(dir, ".git");
    if std::fs::metadata(&git_dir).ok()?.is_file() {
        let text = String::from_utf8_lossy(&std::fs::read(&git_dir).ok()?).into_owned();
        let to = jsre::rest_after(&text, "gitdir:")?;
        git_dir = xenonet_base::nodepath::resolve(&[dir, js_trim(&to)]);
    }
    let common_file = paths::join(&git_dir, "commondir");
    let common = if std::path::Path::new(&common_file).exists() { Some(js_trim(&String::from_utf8_lossy(&std::fs::read(&common_file).ok()?)).to_string()) } else { None };
    let base = match common.filter(|c| !c.is_empty()) {
        Some(c) => xenonet_base::nodepath::resolve(&[git_dir.as_str(), c.as_str()]),
        None => git_dir,
    };
    Some(paths::join(&base, "config"))
}

fn mtime_of(file: &str) -> Option<f64> {
    std::fs::metadata(file).ok().and_then(|m| xenonet_base::fs::mtime_ms_of(&m).ok())
}

/// The mtime of a repo's config file, or None.
pub fn repo_config_mtime(dir: &str) -> Option<f64> {
    repo_config_file(dir).and_then(|f| mtime_of(&f))
}

/// Your git config's files' mtimes, joined: ~/.gitconfig, the XDG one, and
/// the files ~/.gitconfig includes, so a change to any of them reads every
/// repo's identity again.
pub fn global_stamp(home: &str, xdg_config_home: Option<&str>) -> String {
    let main = paths::join(home, ".gitconfig");
    let xdg = xdg_config_home.map_or_else(|| paths::join(home, ".config"), str::to_string);
    let mut files = vec![main.clone(), paths::join_all(&[&xdg, "git", "config"])];
    if let Ok(b) = std::fs::read(&main) {
        for p in jsre::assignments(&String::from_utf8_lossy(&b), "path") {
            // p.replace(/^~(?=[\\/])/, home)
            let p = match p.strip_prefix('~') {
                Some(rest) if rest.starts_with(['/', '\\']) => format!("{home}{rest}"),
                _ => p,
            };
            files.push(p);
        }
    }
    files.iter().map(|f| mtime_of(f).map_or_else(String::new, js_json::number_string)).collect::<Vec<_>>().join("|")
}

/// Why a repo's identity is missing.
pub fn unreadable_config(path: &str, home: &str) -> String {
    format!("git couldn't read the config of {}", paths::tilde(path, home))
}

/// The watch's problem: a file's (with where it is, home as ~), else a
/// repo's. And whether to say it: "ok", "problem", or "" (said already).
pub fn report(file: Option<(&str, &str)>, repo: Option<&str>, home: &str, state: &str, last_error: Option<&str>) -> (&'static str, Option<String>) {
    let problem = match file {
        Some((f, e)) => Some(paths::tilde(&format!("{f}: {e}"), home)),
        None => repo.map(str::to_string),
    };
    match problem.filter(|p| !p.is_empty()) {
        None => ("ok", None),
        Some(p) if state != "degraded" || last_error != Some(p.as_str()) => ("problem", Some(p)),
        Some(p) => ("", Some(p)),
    }
}

/// The environment git config runs in: away from your own home folder (the
/// demo, tests), git reads that folder's config rather than yours. None
/// when it's your own.
pub fn git_env(env: &Map<String, Value>, home: &str, your_home: &str) -> Option<Map<String, Value>> {
    if paths::resolve(home) == paths::resolve(your_home) {
        return None;
    }
    let mut out: Map<String, Value> = env.iter().filter(|(k, _)| !k.starts_with("GIT_CONFIG")).map(|(k, v)| (k.clone(), v.clone())).collect();
    out.insert("HOME".into(), home.into());
    out.insert("USERPROFILE".into(), home.into());
    out.insert("XDG_CONFIG_HOME".into(), paths::join(home, ".config").into());
    out.insert("GIT_CONFIG_NOSYSTEM".into(), "1".into());
    Some(out)
}

/// Where each Claude Code transcript is, by session id, and the account a
/// Remote Control session names in its transcript's first lines.
#[derive(Default)]
pub struct TranscriptOwners {
    dir: String,
    index: Option<HashMap<String, String>>,
    at: f64,
    owners: HashMap<String, (Value, f64)>,
}

impl TranscriptOwners {
    pub fn new(dir: &str) -> Self {
        TranscriptOwners { dir: dir.to_string(), ..Default::default() }
    }

    // Where a session's transcript is: the folder is listed again when
    // the session isn't in it and it's over a minute old.
    fn transcript(&mut self, id: &str, now: f64) -> Option<String> {
        if !self.index.as_ref().is_some_and(|ix| ix.contains_key(id)) && now - self.at > 60e3 {
            let mut ix = HashMap::new();
            self.at = now;
            if let Ok(dirs) = paths::read_dir_sorted(std::path::Path::new(&self.dir)) {
                for d in dirs {
                    let sub = paths::join(&self.dir, &d.file_name().to_string_lossy());
                    let Ok(files) = paths::read_dir_sorted(std::path::Path::new(&sub)) else { continue };
                    for f in files {
                        let name = f.file_name().to_string_lossy().into_owned();
                        if let Some(stem) = name.strip_suffix(".jsonl") {
                            ix.insert(format!("claude:{stem}"), paths::join(&sub, &name));
                        }
                    }
                }
            }
            self.index = Some(ix);
        }
        self.index.as_ref().and_then(|ix| ix.get(id).cloned())
    }

    /// The owner ({ account, org }) a session's transcript names, or null.
    /// Read once per session, again only while the file is still short
    /// enough for the entry to turn up. An error other than a missing file
    /// when it's looked at is thrown, as statSync throws it.
    pub fn owner(&mut self, id: &str, now: f64) -> Result<Value, xenonet_base::fs::FsError> {
        let hit = self.owners.get(id).cloned();
        if let Some((owner, size)) = &hit {
            if !owner.is_null() || *size >= HEAD_BYTES as f64 {
                return Ok(owner.clone());
            }
        }
        let file = self.transcript(id, now);
        let size = match &file {
            None => 0.0,
            Some(f) => match std::fs::metadata(f) {
                Ok(m) => m.len() as f64,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0.0,
                Err(e) => return Err(xenonet_base::fs::FsError::of(e, "stat", f)),
            },
        };
        if hit.as_ref().is_some_and(|(_, s)| *s == size) {
            return Ok(Value::Null);
        }
        let owner = file.and_then(|f| owner_in(&f, (size as u64).min(HEAD_BYTES))).unwrap_or(Value::Null);
        self.owners.insert(id.to_string(), (owner.clone(), size));
        Ok(owner)
    }
}

// The owner the first line naming one says, from a transcript's head.
fn owner_in(file: &str, len: u64) -> Option<Value> {
    use std::io::Read;
    let mut buf = vec![0u8; len as usize];
    let mut f = std::fs::File::open(file).ok()?;
    let mut at = 0;
    while at < buf.len() {
        match f.read(&mut buf[at..]) {
            Ok(0) => break,
            Ok(n) => at += n,
            Err(_) => return None,
        }
    }
    let text = String::from_utf8_lossy(&buf).into_owned();
    for line in text.split('\n') {
        if !line.contains("\"ownerAccountUuid\"") {
            continue;
        }
        let o = js_json::parse(line).ok()?;
        let o = Some(&o);
        if o.is_some_and(Value::is_null) {
            return None;
        }
        let (a, g) = (opt_prop(o, "ownerAccountUuid"), opt_prop(o, "ownerOrganizationUuid"));
        if str_of(a).is_some() && str_of(g).is_some() {
            return Some(json!({ "account": a, "org": g }));
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_apps_files() {
        let a = claude_account(Some(
            &json!({ "oauthAccount": { "accountUuid": "a", "organizationUuid": "o", "emailAddress": " n@x ", "organizationType": "claude_team", "organizationName": "Atlas" } }),
        ));
        assert_eq!(a, json!({ "id": "claude:a:o", "kind": "claude", "handle": "n@x", "detail": { "plan": "Team", "org": "Atlas" } }));
        assert_eq!(claude_account(Some(&json!({}))), Value::Null);
        let claims = json!({ "email": "n@x", "auth_time": 1_700_000_000, "https://api.openai.com/auth": { "chatgpt_account_id": "ws", "chatgpt_plan_type": "plus" } });
        let token = format!("e30.{}.sig", "eyJhIjoxfQ");
        assert_eq!(id_token_claims(Some(&json!({ "tokens": { "id_token": token } }))), json!({ "a": 1 }));
        let auth = json!({ "auth_mode": "chatgpt", "tokens": {} });
        assert_eq!(chatgpt_account(Some(&auth), Some(&claims))["since"], json!(1_700_000_000_000u64));
        assert_eq!(chatgpt_account(Some(&json!({ "auth_mode": "apikey" })), None)["id"], "openai:api-key");
        assert_eq!(base64url("e30=xyz"), b"{}");
        assert_eq!(base64url("e3$0"), b"{}");
    }

    #[test]
    fn reads_gh_and_ssh() {
        let hosts = parse_gh_hosts("github.com:\n    git_protocol: https\n    users:\n        nova-dev:\n        orbitworks:\n            oauth_token: gho_x\n    user: nova-dev\nghe.atlas.example:\n    oauth_token: gho_y\n    user: nova\n");
        assert_eq!(
            hosts,
            [
                json!({ "host": "github.com", "users": ["nova-dev", "orbitworks"], "active": "nova-dev" }),
                json!({ "host": "ghe.atlas.example", "users": ["nova"], "active": "nova" })
            ]
        );
        let shape = gh_hosts_shape("github.com:\n    users:\n        nova:\n            oauth_token: x\n    user: nova\n    list:\n      - a\n");
        assert_eq!(shape, json!({ "github.com": { "users": { "nova": { "oauth_token": "" } }, "user": "", "list": [] } }));
        let aliases = parse_ssh_aliases("Host gh-orbit\n  HostName github.com\nHost *\n  AddKeysToAgent yes\n");
        assert_eq!(aliases, [("gh-orbit".to_string(), "github.com".to_string())]);
        let r = parse_remote("git@gh-orbit:orbitworks/pathfinder.git", &aliases).unwrap();
        assert_eq!((r.host.as_str(), r.owner.as_str(), r.repo.as_str(), r.https), ("github.com", "orbitworks", "pathfinder", false));
        let r = parse_remote("https://orbitworks:ghp_madeup@github.com/orbitworks/sim.git", &[]).unwrap();
        assert_eq!(r.user.as_deref(), Some("orbitworks"));
        assert_eq!(parse_remote("ssh://git@github.com:22/nova-dev/ledger.git", &[]).unwrap().owner, "nova-dev");
        assert_eq!(parse_remote("C:/repos/local", &[]), None);
        assert_eq!(parse_remote("../elsewhere", &[]), None);
    }

    #[test]
    fn says_who_pushes() {
        let cfg = parse_git_config(
            "user.name Nova\ncredential.https://github.com.helper\ncredential.https://github.com.helper !/opt/homebrew/bin/gh auth git-credential\nuser.email hello@x",
        );
        assert_eq!(cfg["email"], "hello@x");
        assert_eq!(push_account(Some(&json!("https://github.com/a/b.git")), Some(&cfg), &[]).unwrap(), json!({ "host": "github.com", "user": null, "how": "gh" }));
        let pinned = parse_git_config("credential.helper osxkeychain\ncredential.https://github.com.username orbitworks");
        assert_eq!(push_account(Some(&json!("https://github.com/o/p")), Some(&pinned), &[]).unwrap()["how"], "git config");
        let cleared = parse_git_config("credential.helper !gh auth git-credential\ncredential.helper\ncredential.https://gitlab.example.helper !gh auth git-credential");
        assert_eq!(push_account(Some(&json!("https://github.com/a/b")), Some(&cleared), &[]).unwrap(), Value::Null);
        assert!(git_id("Nova@Example.com ").starts_with("git:") && git_id("x").len() == 20);
    }

    #[test]
    fn keeps_the_timeline() {
        let h = 3600e3;
        let mut spans: Vec<Value> = Vec::new();
        let a = json!("claude:a");
        for (t, id) in [(10.0 * h, &a), (10.0 * h + 60e3, &a), (12.0 * h, &a), (20.0 * h, &json!("claude:b"))] {
            let op = note_signin(&spans, id, t, None);
            apply_signin(&mut spans, &op);
        }
        assert_eq!(spans.len(), 2);
        let now = json!({ "id": "claude:b", "since": null });
        assert_eq!(account_at(&spans, Some(&now), Some(&json!(11.0 * h)), &[])["id"], "claude:a");
        assert_eq!(account_at(&spans, Some(&now), Some(&json!(15.0 * h)), &[]), Value::Null);
        let one = [json!({ "id": "x", "from": 10.0 * h, "to": 12.0 * h })];
        assert_eq!(account_at(&one, Some(&json!({ "id": "x" })), Some(&json!(h)), &[])["confidence"], "assumed");
    }
}
