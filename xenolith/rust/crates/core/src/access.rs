//! Who may do what in which space, in XenoNet's scope model (core/delivery/selector.js and
//! policy.js there), and deny by default.
//!
//! - Levels: read < append < write < admin. Read is context_start, context_search and
//!   context_read; append is context_log, and context_upsert_page in append mode; write is
//!   context_upsert_page in replace mode; admin creates spaces and manages the rest.
//! - Selectors: XenoNet's grammar. key:value terms, all of which must hold, separated by spaces
//!   or "+"; "all" is the empty selector; a value may end in * for a prefix. For spaces the keys
//!   are project, identity (an identity's space, and the projects that belong to it), machine and
//!   by (an actor's own space, as by:<slug> names the actor itself in a XenoNet grant).
//! - Scopes: `ctx:<level>` or `ctx:<level>:<selector>`, written as OAuth scope strings, so terms
//!   join with "+": `ctx:read:project:atlas+identity:adal`. A level includes the ones below it.
//! - Grants: what you allow a subject (actor:<slug>, client:<id>, machine:<name>): a scope until
//!   it expires. A subject can hold several.
//! - Tokens: `xenolith_<64 hex digits>`, made the way XenoNet makes actor tokens; Xenolith keeps
//!   only the SHA-256. Each names its subject and the scopes it asks for. What a token may do in a
//!   space is the lower of its own scopes and its subject's live grants there: a token can't
//!   reach past what you granted, and a grant does nothing without a token that asks for it.
//!
//! Local callers (stdio, and HTTP from a process on this machine without a token and without a
//! proxy's Forwarded, X-Forwarded-* or Via header) have admin everywhere by default. Local is any
//! process on this machine, not only the owner's own apps: tokens scope remote and other apps'
//! access. `"local"` in access.json lowers that (read, append, write) or takes it away ("none");
//! a value Xenolith doesn't know counts as "none".
//! The policy is access.json in Xenolith's config folder, never in the wiki, whose files are
//! data that sessions and models write.

use crate::paths::AppPaths;
use crate::spaces::SpaceId;
use crate::text::{now_ms, random_hex, sha256_hex};
use crate::wiki::{self, Result, wiki_err};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- levels

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Read = 1,
    Append = 2,
    Write = 3,
    Admin = 4,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Read, Level::Append, Level::Write, Level::Admin];

    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Level::Read),
            "append" => Some(Level::Append),
            "write" => Some(Level::Write),
            "admin" => Some(Level::Admin),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Read => "read",
            Level::Append => "append",
            Level::Write => "write",
            Level::Admin => "admin",
        }
    }
}

// ---------------------------------------------------------------- selectors

/// The selector keys that describe a space (XenoNet's app and status describe sessions only).
pub const KEYS: [&str; 4] = ["project", "identity", "machine", "by"];

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Selector {
    pub terms: Vec<(String, String)>,
}

impl Selector {
    pub fn all() -> Selector {
        Selector { terms: vec![] }
    }
}

/// XenoNet's parseSelector, with the space keys: None when the text is not a selector.
pub fn parse_selector(text: &str) -> Option<Selector> {
    let s = text.trim();
    if s.is_empty() {
        return None;
    }
    if s == "all" {
        return Some(Selector::all());
    }
    let mut terms = vec![];
    for part in s.split(|c: char| c == '+' || c.is_whitespace()).filter(|p| !p.is_empty()) {
        let (k, v) = part.split_once(':')?;
        let (k, v) = (k.to_ascii_lowercase(), v.to_lowercase());
        if !KEYS.contains(&k.as_str()) || v.is_empty() || v.contains(':') {
            return None;
        }
        terms.push((k, v));
    }
    Some(Selector { terms })
}

/// "all", or the terms separated by spaces (XenoNet's formatSelector).
pub fn format_selector(sel: &Selector) -> String {
    if sel.terms.is_empty() { "all".into() } else { sel.terms.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join(" ") }
}

fn like(value: &str, want: &str) -> bool {
    let v = value.to_lowercase();
    match want.strip_suffix('*') {
        Some(prefix) => v.starts_with(prefix),
        None => v == want,
    }
}

/// What a selector is matched against besides the space's own id: the identities a project
/// space belongs to (set by an admin, in access.json).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub identities: Vec<String>,
}

/// Whether a space falls inside a selector: every term holds.
pub fn matches(sel: &Selector, space: &SpaceId, facts: &Facts) -> bool {
    sel.terms.iter().all(|(k, v)| match k.as_str() {
        "project" => space.kind == "project" && like(&space.name, v),
        "identity" => (space.kind == "identity" && like(&space.name, v)) || (space.kind == "project" && facts.identities.iter().any(|i| like(i, v))),
        "machine" => space.kind == "machine" && like(&space.name, v),
        "by" => space.kind == "actor" && like(&space.name, v),
        _ => false,
    })
}

/// XenoNet's within: everything `inner` can select lies inside `outer` (every term of outer
/// appears in inner, or a prefix of it does).
pub fn within(inner: &Selector, outer: &Selector) -> bool {
    outer.terms.iter().all(|(ok, ov)| inner.terms.iter().any(|(ik, iv)| ik == ok && (iv == ov || ov.strip_suffix('*').is_some_and(|p| iv.starts_with(p)))))
}

// ---------------------------------------------------------------- scopes

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scope {
    pub level: Level,
    pub selector: Selector,
}

impl Scope {
    /// Whether the scope reaches a space. This machine's own space (machine:local) holds the
    /// owner's notes and the questions asked in the window: only a selector that names it exactly
    /// reaches it, never "all", a wildcard or a selector about something else.
    pub fn covers(&self, space: &SpaceId, facts: &Facts) -> bool {
        if space.is_local() && !names_local(&self.selector) {
            return false;
        }
        matches(&self.selector, space, facts)
    }
}

/// Whether a selector names this machine's own space exactly: machine:local, no wildcard.
fn names_local(sel: &Selector) -> bool {
    sel.terms.iter().any(|(k, v)| k == "machine" && v == "local")
}

/// `ctx:<level>` or `ctx:<level>:<selector>` ("+" or spaces between terms). None when it isn't one.
pub fn parse_scope(text: &str) -> Option<Scope> {
    let rest = text.trim().strip_prefix("ctx:")?;
    let (level, sel) = match rest.split_once(':') {
        Some((l, s)) => (l, s),
        None => (rest, "all"),
    };
    Some(Scope { level: Level::parse(level)?, selector: parse_selector(sel)? })
}

/// The scope as an OAuth scope string: `ctx:read`, or `ctx:read:project:atlas+identity:adal`.
pub fn format_scope(s: &Scope) -> String {
    if s.selector.terms.is_empty() {
        format!("ctx:{}", s.level.as_str())
    } else {
        format!("ctx:{}:{}", s.level.as_str(), s.selector.terms.iter().map(|(k, v)| format!("{k}:{v}")).collect::<Vec<_>>().join("+"))
    }
}

/// The scopes a server advertises (RFC 9728 scopes_supported): one per level, for every space.
pub fn scopes_supported() -> Vec<String> {
    Level::ALL.iter().map(|l| format!("ctx:{}", l.as_str())).collect()
}

/// A space-separated scope string (OAuth `scope`) as scopes; an error names the one that isn't.
pub fn parse_scope_list(text: &str) -> Result<Vec<Scope>> {
    text.split_whitespace()
        .map(|s| {
            parse_scope(s)
                .ok_or_else(|| wiki::Error::Wiki(format!("\"{s}\" is not a scope: use ctx:read, ctx:append, ctx:write or ctx:admin, optionally with :<selector> such as ctx:read:project:atlas")))
        })
        .collect()
}

// ---------------------------------------------------------------- tokens

pub const TOKEN_PREFIX: &str = "xenolith_";

pub fn is_token(s: &str) -> bool {
    s.len() == TOKEN_PREFIX.len() + 64 && s.starts_with(TOKEN_PREFIX) && s[TOKEN_PREFIX.len()..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn hash_token(token: &str) -> String {
    sha256_hex(token.as_bytes())
}

/// A new token and the hash Xenolith keeps of it. Shown once; it travels only as a Bearer header.
pub fn new_token() -> (String, String) {
    let token = format!("{TOKEN_PREFIX}{}", random_hex(64));
    let hash = hash_token(&token);
    (token, hash)
}

/// The token an Authorization header carries (RFC 6750): None without the header, Some(None) when
/// it isn't a Bearer, Some(Some(token)) otherwise (which may still be one Xenolith doesn't know).
pub fn bearer_of(header: Option<&str>) -> Option<Option<String>> {
    let raw = header?.trim();
    let mut parts = raw.splitn(2, [' ', '\t']);
    let scheme = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    if !scheme.eq_ignore_ascii_case("bearer") || rest.is_empty() || rest.contains([' ', '\t']) {
        return Some(None);
    }
    Some(Some(rest.to_string()))
}

// ---------------------------------------------------------------- the policy

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub subject: String,
    pub scope: Scope,
    pub at: i64,
    pub expires_at: i64,
    pub by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRec {
    /// A short handle for listing and revoking: "tok_" and the hash's first 12 hex digits.
    pub id: String,
    pub hash: String,
    pub subject: String,
    pub scopes: Vec<Scope>,
    pub label: String,
    pub at: i64,
    /// None: until revoked.
    pub expires_at: Option<i64>,
    /// An OAuth access token's resource (RFC 8707): the server it was issued for.
    pub resource: Option<String>,
}

/// Who is asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// You, or a session on this machine (stdio, or HTTP from this machine without a token).
    Local,
    /// A bearer token's holder.
    Token { id: String, subject: String, scopes: Vec<Scope> },
}

impl Principal {
    /// How the audit log and refusals name it.
    pub fn name(&self) -> String {
        match self {
            Principal::Local => "you".into(),
            Principal::Token { id, subject, .. } => format!("{subject} ({id})"),
        }
    }
}

/// Subjects a grant or token can name: an actor, an OAuth client, another machine (sync).
pub fn valid_subject(s: &str) -> bool {
    let Some((k, v)) = s.split_once(':') else { return false };
    matches!(k, "actor" | "client" | "machine") && !v.is_empty() && v.len() <= 80 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub const GRANT_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// What you and this machine's sessions may do; None takes even that away.
    pub local: Option<Level>,
    pub grants: Vec<Grant>,
    pub tokens: Vec<TokenRec>,
    pub spaces: BTreeMap<String, Facts>,
    /// Set when access.json could not be read: tokens are then refused, and /status says why.
    pub problem: Option<String>,
}

pub fn policy_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("access.json")
}

fn scopes_of(v: &Value) -> Vec<Scope> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(parse_scope).collect()
}

impl Policy {
    pub fn new() -> Policy {
        Policy { local: Some(Level::Admin), ..Default::default() }
    }

    /// access.json as a policy. Missing: the default (you have admin, nobody else has anything).
    /// Unreadable: tokens get nothing, you keep admin so you can fix it, and `problem` says why.
    pub fn load(file: &Path) -> Policy {
        let raw = match std::fs::read_to_string(file) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Policy::new(),
            Err(e) => return Policy { problem: Some(format!("{} could not be read: {e}", wiki::disp(file))), ..Policy::new() },
        };
        let v: Value = match serde_json::from_str(raw.trim_start_matches('\u{feff}')) {
            Ok(v @ Value::Object(_)) => v,
            _ => return Policy { problem: Some(format!("{} is not a JSON object; tokens are refused until it is fixed", wiki::disp(file))), ..Policy::new() },
        };
        // Missing: admin, the default. Anything but a level ("none", a typo, null) is no access:
        // a policy that can't be read right must not hand this machine's processes more.
        let local = match v.get("local") {
            None => Some(Level::Admin),
            Some(l) => l.as_str().and_then(Level::parse),
        };
        let grants = v["grants"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| {
                Some(Grant {
                    subject: g["subject"].as_str().filter(|s| valid_subject(s))?.to_string(),
                    scope: parse_scope(g["scope"].as_str()?)?,
                    at: g["at"].as_i64().unwrap_or(0),
                    expires_at: g["expiresAt"].as_i64()?,
                    by: g["by"].as_str().unwrap_or("you").to_string(),
                })
            })
            .collect();
        let tokens = v["tokens"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| {
                let hash = t["hash"].as_str().filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))?.to_string();
                Some(TokenRec {
                    id: format!("tok_{}", &hash[..12]),
                    hash,
                    subject: t["subject"].as_str().filter(|s| valid_subject(s))?.to_string(),
                    scopes: scopes_of(&t["scopes"]),
                    label: t["label"].as_str().unwrap_or("").to_string(),
                    at: t["at"].as_i64().unwrap_or(0),
                    expires_at: t["expiresAt"].as_i64(),
                    resource: t["resource"].as_str().map(String::from),
                })
            })
            .collect();
        let spaces = v["spaces"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, f)| {
                let id = SpaceId::parse(k).ok()?;
                let identities = f["identities"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_lowercase).collect();
                Some((id.to_string(), Facts { identities }))
            })
            .collect();
        Policy { local, grants, tokens, spaces, problem: None }
    }

    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("version".into(), json!(1));
        o.insert("local".into(), json!(self.local.map_or("none", Level::as_str)));
        o.insert(
            "grants".into(),
            Value::Array(self.grants.iter().map(|g| json!({ "subject": g.subject, "scope": format_scope(&g.scope), "at": g.at, "expiresAt": g.expires_at, "by": g.by })).collect()),
        );
        o.insert(
            "tokens".into(),
            Value::Array(
                self.tokens
                    .iter()
                    .map(|t| {
                        let mut m = Map::new();
                        m.insert("hash".into(), json!(t.hash));
                        m.insert("subject".into(), json!(t.subject));
                        m.insert("scopes".into(), json!(t.scopes.iter().map(format_scope).collect::<Vec<_>>()));
                        m.insert("label".into(), json!(t.label));
                        m.insert("at".into(), json!(t.at));
                        if let Some(e) = t.expires_at {
                            m.insert("expiresAt".into(), json!(e));
                        }
                        if let Some(r) = &t.resource {
                            m.insert("resource".into(), json!(r));
                        }
                        Value::Object(m)
                    })
                    .collect(),
            ),
        );
        o.insert("spaces".into(), Value::Object(self.spaces.iter().map(|(k, f)| (k.clone(), json!({ "identities": f.identities }))).collect()));
        Value::Object(o)
    }

    pub fn save(&self, file: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default();
        wiki::atomic_write(file, &format!("{text}\n"), None, None)?;
        restrict(file);
        Ok(())
    }

    pub fn facts(&self, space: &SpaceId) -> Facts {
        self.spaces.get(&space.to_string()).cloned().unwrap_or_default()
    }

    /// The principal a bearer token stands for; None when it is not a live token of this policy.
    pub fn principal_for(&self, token: &str, now: i64) -> Option<Principal> {
        if self.problem.is_some() || !is_token(token) {
            return None;
        }
        let hash = hash_token(token);
        let t = self.tokens.iter().find(|t| t.hash == hash)?;
        if t.expires_at.is_some_and(|e| e <= now) {
            return None;
        }
        Some(Principal::Token { id: t.id.clone(), subject: t.subject.clone(), scopes: t.scopes.clone() })
    }

    /// The live record of a token, by the token itself.
    pub fn token(&self, secret: &str) -> Option<&TokenRec> {
        if !is_token(secret) {
            return None;
        }
        let hash = hash_token(secret);
        self.tokens.iter().find(|t| t.hash == hash)
    }

    /// A principal as this policy sees it now: a token's holder again from its live record, so a
    /// revoked or expired token stops working at once, even on a connection opened before.
    pub fn current(&self, who: &Principal, now: i64) -> Option<Principal> {
        match who {
            Principal::Local => Some(Principal::Local),
            Principal::Token { id, .. } => {
                if self.problem.is_some() {
                    return None;
                }
                let t = self.tokens.iter().find(|t| t.id == *id && t.expires_at.is_none_or(|e| e > now))?;
                Some(Principal::Token { id: t.id.clone(), subject: t.subject.clone(), scopes: t.scopes.clone() })
            }
        }
    }

    fn grant_level(&self, subject: &str, space: &SpaceId, facts: &Facts, now: i64) -> Option<Level> {
        self.grants.iter().filter(|g| g.subject == subject && g.expires_at > now && g.scope.covers(space, facts)).map(|g| g.scope.level).max()
    }

    /// The most a principal may do in a space, or None: deny by default.
    pub fn level(&self, who: &Principal, space: &SpaceId, now: i64) -> Option<Level> {
        match who {
            Principal::Local => self.local,
            Principal::Token { subject, scopes, .. } => {
                if self.problem.is_some() {
                    return None;
                }
                let facts = self.facts(space);
                let asked = scopes.iter().filter(|s| s.covers(space, &facts)).map(|s| s.level).max()?;
                let granted = self.grant_level(subject, space, &facts, now)?;
                Some(asked.min(granted))
            }
        }
    }

    pub fn allows(&self, who: &Principal, space: &SpaceId, need: Level, now: i64) -> bool {
        self.level(who, space, now).is_some_and(|l| l >= need)
    }

    /// The highest level a token's own scopes reach in any space: below what a call needs, the
    /// answer is OAuth's insufficient_scope (ask for more), not "no such space".
    pub fn scope_ceiling(who: &Principal) -> Option<Level> {
        match who {
            Principal::Local => Some(Level::Admin),
            Principal::Token { scopes, .. } => scopes.iter().map(|s| s.level).max(),
        }
    }

    /// Grants `scope` to `subject` for `ms` (at least a minute), replacing a grant of the same
    /// scope. Returns the grant.
    pub fn grant(&mut self, subject: &str, scope: Scope, ms: i64, by: &str, now: i64) -> Result<Grant> {
        if !valid_subject(subject) {
            return wiki_err(format!("\"{subject}\" is not a subject: use actor:<slug>, client:<id> or machine:<name>."));
        }
        let g = Grant { subject: subject.into(), scope, at: now, expires_at: now + ms.max(60_000), by: by.into() };
        self.grants.retain(|x| !(x.subject == g.subject && x.scope == g.scope));
        self.grants.push(g.clone());
        Ok(g)
    }

    /// Takes away a subject's grants: all of them, or the ones of one scope. Returns how many.
    pub fn revoke(&mut self, subject: &str, scope: Option<&Scope>) -> usize {
        let before = self.grants.len();
        self.grants.retain(|g| !(g.subject == subject && scope.is_none_or(|s| *s == g.scope)));
        before - self.grants.len()
    }

    /// Makes a token for `subject` asking for `scopes`. Returns the token (shown once) and its record.
    pub fn issue(&mut self, subject: &str, scopes: Vec<Scope>, label: &str, ttl_ms: Option<i64>, resource: Option<String>, now: i64) -> Result<(String, TokenRec)> {
        if !valid_subject(subject) {
            return wiki_err(format!("\"{subject}\" is not a subject: use actor:<slug>, client:<id> or machine:<name>."));
        }
        if scopes.is_empty() {
            return wiki_err("A token needs at least one scope, such as ctx:read.");
        }
        let (token, hash) = new_token();
        let rec = TokenRec {
            id: format!("tok_{}", &hash[..12]),
            hash,
            subject: subject.into(),
            scopes,
            label: crate::text::one_line(label).chars().take(80).collect(),
            at: now,
            expires_at: ttl_ms.map(|t| now + t.max(60_000)),
            resource,
        };
        self.tokens.push(rec.clone());
        Ok((token, rec))
    }

    /// Revokes tokens by id (tok_…) or by subject (every token it holds). Returns how many.
    pub fn revoke_token(&mut self, id_or_subject: &str) -> usize {
        let before = self.tokens.len();
        self.tokens.retain(|t| t.id != id_or_subject && t.subject != id_or_subject);
        before - self.tokens.len()
    }

    /// Drops expired tokens and grants (kept a day after they lapse, so a listing can show them).
    pub fn prune(&mut self, now: i64) {
        self.grants.retain(|g| g.expires_at + 86_400_000 > now);
        self.tokens.retain(|t| t.expires_at.is_none_or(|e| e + 86_400_000 > now));
    }
}

/// Owner-only, where the OS has that notion: the file holds token hashes and the grants.
fn restrict(file: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
    }
    let _ = file;
}

/// Changes the policy under a lock in the config folder, so two processes never lose each other's
/// change. Refuses to touch a policy file it cannot read.
pub fn update<T>(paths: &AppPaths, f: impl FnOnce(&mut Policy) -> Result<T>) -> Result<T> {
    let file = policy_file(paths);
    std::fs::create_dir_all(&paths.config_dir)?;
    let mut guard = match crate::lock::acquire(&paths.config_dir, "access", &crate::lock::LockOpts::default()) {
        Ok(g) => g,
        Err(crate::lock::AcquireError::Busy(b)) => return wiki_err(format!("The access policy is busy ({}). Try again shortly.", b.0)),
        Err(crate::lock::AcquireError::Io(e)) => return Err(e.into()),
    };
    let mut p = Policy::load(&file);
    if let Some(problem) = &p.problem {
        guard.release();
        return wiki_err(format!("Not changed: {problem}."));
    }
    let r = f(&mut p);
    if r.is_ok() {
        p.prune(now_ms());
        p.save(&file)?;
    }
    guard.release();
    r
}

pub fn load(paths: &AppPaths) -> Policy {
    Policy::load(&policy_file(paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sp(s: &str) -> SpaceId {
        SpaceId::parse(s).unwrap()
    }
    fn sc(s: &str) -> Scope {
        parse_scope(s).unwrap()
    }
    fn tok(scopes: &[&str]) -> Principal {
        Principal::Token { id: "tok_x".into(), subject: "actor:hermes".into(), scopes: scopes.iter().map(|s| sc(s)).collect() }
    }

    #[test]
    fn selectors_follow_xenonet_grammar_with_space_keys() {
        assert_eq!(parse_selector("all"), Some(Selector::all()));
        let s = parse_selector("project:Atlas+identity:adal").unwrap();
        assert_eq!(format_selector(&s), "project:atlas identity:adal");
        assert_eq!(parse_selector("project:atlas identity:adal"), Some(s));
        for bad in ["", "atlas", "app:codex", "status:idle", "project:", "project:a:b", "repo"] {
            assert_eq!(parse_selector(bad), None, "{bad}");
        }
        let atlas = sp("project:atlas");
        let adal = Facts { identities: vec!["adal".into()] };
        assert!(matches(&parse_selector("project:atl*").unwrap(), &atlas, &Facts::default()));
        assert!(matches(&parse_selector("identity:adal").unwrap(), &atlas, &adal), "a project belongs to its identities");
        assert!(matches(&parse_selector("identity:adal").unwrap(), &sp("identity:adal"), &Facts::default()));
        assert!(!matches(&parse_selector("identity:adal").unwrap(), &atlas, &Facts::default()));
        assert!(!matches(&parse_selector("project:atlas identity:bob").unwrap(), &atlas, &adal), "all terms must hold");
        assert!(matches(&parse_selector("by:hermes").unwrap(), &sp("actor:hermes"), &Facts::default()));
        assert!(!matches(&parse_selector("by:hermes").unwrap(), &sp("project:hermes"), &Facts::default()));
        assert!(matches(&parse_selector("machine:local").unwrap(), &SpaceId::local(), &Facts::default()));
        assert!(!matches(&parse_selector("machine:local").unwrap(), &atlas, &Facts::default()), "machine names a machine's own space only");
        assert!(matches(&Selector::all(), &atlas, &Facts::default()));
        assert!(within(&parse_selector("project:atlas identity:adal").unwrap(), &parse_selector("identity:adal").unwrap()));
        assert!(within(&parse_selector("project:atlas").unwrap(), &parse_selector("project:at*").unwrap()));
        assert!(!within(&parse_selector("project:atlas").unwrap(), &parse_selector("identity:adal").unwrap()));
    }

    #[test]
    fn scopes_read_as_oauth_scope_strings() {
        assert_eq!(sc("ctx:read"), Scope { level: Level::Read, selector: Selector::all() });
        assert_eq!(sc("ctx:read:all"), sc("ctx:read"));
        let s = sc("ctx:write:project:atlas+identity:adal");
        assert_eq!(s.level, Level::Write);
        assert_eq!(format_scope(&s), "ctx:write:project:atlas+identity:adal");
        assert_eq!(format_scope(&sc("ctx:admin")), "ctx:admin");
        for bad in ["read", "ctx:", "ctx:own", "ctx:read:app:codex", "scope:read"] {
            assert!(parse_scope(bad).is_none(), "{bad}");
        }
        assert_eq!(parse_scope_list("ctx:read ctx:append:by:hermes").unwrap().len(), 2);
        assert!(parse_scope_list("ctx:read nope").unwrap_err().to_string().contains("\"nope\" is not a scope"));
        assert_eq!(scopes_supported(), ["ctx:read", "ctx:append", "ctx:write", "ctx:admin"]);
    }

    #[test]
    fn tokens_are_prefixed_hex_and_kept_as_hashes() {
        let (t, h) = new_token();
        assert!(is_token(&t) && t.starts_with("xenolith_") && t.len() == 73);
        assert_eq!(h, hash_token(&t));
        assert!(!is_token("xenolith_XYZ") && !is_token(&t.to_uppercase()));
        assert_eq!(bearer_of(None), None);
        assert_eq!(bearer_of(Some("Basic abc")), Some(None));
        assert_eq!(bearer_of(Some("Bearer")), Some(None));
        assert_eq!(bearer_of(Some("bearer  abc ")), Some(Some("abc".into())));
        assert_eq!(bearer_of(Some("Bearer a b")), Some(None));
    }

    #[test]
    fn deny_by_default_and_the_lower_of_scope_and_grant() {
        let now = 1_000_000;
        let mut p = Policy::new();
        let atlas = sp("project:atlas");
        assert_eq!(p.level(&Principal::Local, &atlas, now), Some(Level::Admin), "you have admin everywhere");
        let who = tok(&["ctx:write:project:atlas", "ctx:append:by:hermes"]);
        assert_eq!(p.level(&who, &atlas, now), None, "a token asks, a grant allows: nothing granted yet");
        p.grant("actor:hermes", sc("ctx:read:project:atlas"), GRANT_MS, "you", now).unwrap();
        assert_eq!(p.level(&who, &atlas, now), Some(Level::Read), "the grant caps what the token asks");
        assert!(p.allows(&who, &atlas, Level::Read, now) && !p.allows(&who, &atlas, Level::Append, now));
        assert_eq!(p.level(&who, &SpaceId::local(), now), None, "nothing granted on this machine's space");
        p.grant("actor:hermes", sc("ctx:admin:by:hermes"), GRANT_MS, "you", now).unwrap();
        assert_eq!(p.level(&who, &sp("actor:hermes"), now), Some(Level::Append), "the token asks only append there");
        assert_eq!(p.level(&tok(&["ctx:read"]), &atlas, now), Some(Level::Read));
        assert_eq!(p.level(&who, &atlas, now + GRANT_MS + 1), None, "grants expire");
        assert_eq!(p.revoke("actor:hermes", None), 2);
        assert_eq!(p.level(&who, &atlas, now), None, "revoked at once");
        assert_eq!(Policy::scope_ceiling(&who), Some(Level::Write));
        p.local = None;
        assert_eq!(p.level(&Principal::Local, &atlas, now), None);
        assert!(p.grant("hermes", sc("ctx:read"), GRANT_MS, "you", now).is_err(), "subjects name their kind");
    }

    #[test]
    fn a_token_reaches_this_machines_space_only_through_a_grant_that_names_it() {
        let now = 1_000;
        let local = SpaceId::local();
        let reader = |scope: &str| Principal::Token { id: "tok_l".into(), subject: "client:phone".into(), scopes: vec![sc(scope)] };
        for grant in ["ctx:read", "ctx:read:all", "ctx:read:machine:*", "ctx:read:machine:loc*", "ctx:admin"] {
            let mut p = Policy::new();
            p.grant("client:phone", sc(grant), GRANT_MS, "you", now).unwrap();
            assert_eq!(p.level(&reader("ctx:read:machine:local"), &local, now), None, "{grant} does not name machine:local");
            assert_eq!(p.level(&reader("ctx:read"), &sp("project:atlas"), now).is_some(), !grant.contains("machine"), "{grant} still reaches other spaces");
        }
        let mut p = Policy::new();
        p.grant("client:phone", sc("ctx:read:machine:local"), GRANT_MS, "you", now).unwrap();
        assert_eq!(p.level(&reader("ctx:read:machine:local"), &local, now), Some(Level::Read), "named exactly, by the grant and the token");
        assert_eq!(p.level(&reader("ctx:read"), &local, now), None, "the token's own scopes must name it too");
        assert_eq!(p.level(&reader("ctx:read:machine:local"), &sp("machine:studio"), now), None);
        assert_eq!(p.level(&Principal::Local, &local, now), Some(Level::Admin), "this machine's own processes are not tokens");
    }

    #[test]
    fn identities_of_a_project_come_from_the_policy() {
        let now = 5;
        let mut p = Policy::new();
        p.spaces.insert("project:atlas".into(), Facts { identities: vec!["adal".into()] });
        p.grant("client:phone", sc("ctx:read:identity:adal"), GRANT_MS, "you", now).unwrap();
        let who = Principal::Token { id: "tok_y".into(), subject: "client:phone".into(), scopes: vec![sc("ctx:read")] };
        assert_eq!(p.level(&who, &sp("project:atlas"), now), Some(Level::Read));
        assert_eq!(p.level(&who, &sp("project:other"), now), None);
        assert_eq!(p.level(&who, &sp("identity:adal"), now), Some(Level::Read));
    }

    #[test]
    fn the_policy_file_round_trips_and_a_broken_one_refuses_tokens() {
        let dir = std::env::temp_dir().join(format!("xenolith-access-{}", random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("access.json");
        let now = 10_000;
        let mut p = Policy::new();
        p.grant("actor:hermes", sc("ctx:append:by:hermes"), GRANT_MS, "you", now).unwrap();
        let (secret, rec) = p.issue("actor:hermes", vec![sc("ctx:append:by:hermes"), sc("ctx:read")], "Hermes", None, None, now).unwrap();
        p.spaces.insert("project:atlas".into(), Facts { identities: vec!["adal".into()] });
        p.save(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains(&secret), "only the hash is kept");
        let q = Policy::load(&file);
        assert_eq!(q.grants, p.grants);
        assert_eq!(q.tokens, p.tokens);
        assert_eq!(q.spaces, p.spaces);
        let who = q.principal_for(&secret, now).unwrap();
        assert_eq!(who, Principal::Token { id: rec.id.clone(), subject: "actor:hermes".into(), scopes: rec.scopes.clone() });
        assert_eq!(q.level(&who, &sp("actor:hermes"), now), Some(Level::Append));
        assert!(q.principal_for(&new_token().0, now).is_none(), "an unknown token is nobody");
        std::fs::write(&file, "{ not json").unwrap();
        let broken = Policy::load(&file);
        assert!(broken.problem.is_some());
        assert!(broken.principal_for(&secret, now).is_none(), "a policy that can't be read refuses every token");
        assert_eq!(broken.level(&Principal::Local, &SpaceId::local(), now), Some(Level::Admin), "you keep admin to fix it");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_unknown_local_value_means_none() {
        let dir = std::env::temp_dir().join(format!("xenolith-access-local-{}", random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("access.json");
        for (text, want) in [
            (r#"{}"#, Some(Level::Admin)),
            (r#"{"local":"read"}"#, Some(Level::Read)),
            (r#"{"local":"Write"}"#, Some(Level::Write)),
            (r#"{"local":"none"}"#, None),
            (r#"{"local":"admn"}"#, None),
            (r#"{"local":"everything"}"#, None),
            (r#"{"local":null}"#, None),
            (r#"{"local":4}"#, None),
        ] {
            std::fs::write(&file, text).unwrap();
            let p = Policy::load(&file);
            assert_eq!(p.local, want, "{text}");
            assert!(p.problem.is_none(), "{text}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn tokens_expire_and_revoke_by_id_or_subject() {
        let now = 100_000;
        let mut p = Policy::new();
        let (a, ra) = p.issue("client:phone", vec![sc("ctx:read")], "", Some(3_600_000), None, now).unwrap();
        let (b, _) = p.issue("client:phone", vec![sc("ctx:read")], "", None, None, now).unwrap();
        assert!(p.principal_for(&a, now).is_some());
        assert!(p.principal_for(&a, now + 3_600_001).is_none(), "expired");
        assert_eq!(p.revoke_token(&ra.id), 1);
        assert!(p.principal_for(&a, now).is_none() && p.principal_for(&b, now).is_some());
        assert_eq!(p.revoke_token("client:phone"), 1);
        assert!(p.tokens.is_empty());
        assert!(p.issue("client:phone", vec![], "", None, None, now).is_err());
    }

    #[test]
    fn a_principal_is_checked_again_against_the_live_policy() {
        let now = 7;
        let mut p = Policy::new();
        let (secret, rec) = p.issue("actor:hermes", vec![sc("ctx:read")], "", Some(1_000_000), None, now).unwrap();
        let who = p.principal_for(&secret, now).unwrap();
        assert_eq!(p.current(&who, now), Some(who.clone()));
        assert_eq!(p.current(&who, now + 1_000_001), None, "expired since the connection opened");
        assert_eq!(p.token(&secret).map(|t| t.id.clone()), Some(rec.id.clone()));
        p.revoke_token(&rec.id);
        assert_eq!(p.current(&who, now), None, "revoked since the connection opened");
        assert_eq!(p.current(&Principal::Local, now), Some(Principal::Local));
    }
}
