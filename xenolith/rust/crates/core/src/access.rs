//! Who may do what in which space, and deny by default.
//!
//! - Levels: read < append < write < admin. Read is context_start, context_search and
//!   context_read; append is context_log, and context_upsert_page in append mode; write is
//!   context_upsert_page in replace mode; admin creates spaces and manages the rest.
//! - Selectors: key:value terms, all of which must hold, separated by spaces
//!   or "+"; "all" is the empty selector; a value may end in * for a prefix. For spaces the keys
//!   are project, identity (an identity's space, and the projects that belong to it), machine and
//!   by (an actor's own space: by:<slug> names the actor itself).
//! - Scopes: `ctx:<level>` or `ctx:<level>:<selector>`, written as OAuth scope strings, so terms
//!   join with "+": `ctx:read:project:atlas+identity:adal`. A level includes the ones below it.
//! - Grants: what you allow a subject (actor:<slug>, client:<id>): a scope until it expires. A
//!   subject can hold several.
//! - Tokens: Biscuits rooted at this Xenolith's token root (grants.rs), `xenolith_grant_…`.
//!   access.json keeps no token, only each one's revocation id with its row: its subject, the
//!   scopes it asks for, its label and end; and the revocation ids of revoked tokens. What a token
//!   may do in a space is the lower of its own scopes and its subject's live grants there, and
//!   never more than its Biscuit's rights: a token can't reach past what you granted, and a grant
//!   does nothing without a token that asks for it. Every decision is xenonet-authz's.
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
use crate::text::now_ms;
#[cfg(test)]
use crate::text::random_hex;
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

/// The selector keys that describe a space.
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

/// A selector over the space keys: None when the text is not a selector.
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

/// "all", or the terms separated by spaces.
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

/// Whether everything `inner` can select lies inside `outer` (every term of outer
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

/// A token's handle for listing and revoking: `tok_` and the first 12 hex digits of its revocation
/// id.
pub fn handle_of(revocation: &str) -> String {
    format!("tok_{}", &revocation[..revocation.len().min(12)])
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

/// What access.json keeps of a token: never the token, only its revocation id and its row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenRec {
    /// The handle (`tok_…`).
    pub id: String,
    /// Its Biscuit's first revocation id, in hex.
    pub revocation: String,
    pub subject: String,
    /// What it may select: the row decisions read (grants.rs).
    pub scopes: Vec<Scope>,
    pub label: String,
    pub at: i64,
    pub expires_at: i64,
}

/// A token, checked: it stands for the holder of a `Principal::Token`, so a decision can show it
/// to xenonet-authz. Two are the same when their revocation ids are.
#[derive(Clone)]
pub struct Proof(pub std::sync::Arc<xenonet_authz::Token>);

impl PartialEq for Proof {
    fn eq(&self, other: &Proof) -> bool {
        self.0.id() == other.0.id()
    }
}

impl Eq for Proof {}

impl std::fmt::Debug for Proof {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Who is asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// You, or a session on this machine (stdio, or HTTP from this machine without a token).
    Local,
    /// A bearer token's holder.
    Token { id: String, subject: String, scopes: Vec<Scope>, proof: Proof },
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

/// Subjects a grant or token can name: an actor, or an OAuth client.
pub fn valid_subject(s: &str) -> bool {
    let Some((k, v)) = s.split_once(':') else { return false };
    matches!(k, "actor" | "client") && !v.is_empty() && v.len() <= 80 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

pub const GRANT_MS: i64 = 30 * 24 * 3600 * 1000;

#[derive(Clone, Debug, Default)]
pub struct Policy {
    /// What you and this machine's sessions may do; None takes even that away.
    pub local: Option<Level>,
    pub grants: Vec<Grant>,
    pub tokens: Vec<TokenRec>,
    /// The revocation ids (hex) of tokens revoked: a token holding one is no token.
    pub revoked: Vec<String>,
    pub spaces: BTreeMap<String, Facts>,
    /// Set when access.json could not be read: tokens are then refused, and /status says why.
    pub problem: Option<String>,
    /// This Xenolith, as decisions name it, and the token roots it made: from the state folder,
    /// by `load`.
    pub me: crate::grants::Me,
    pub roots: Vec<xenonet_authz::RootCertificate>,
}

pub fn policy_file(paths: &AppPaths) -> PathBuf {
    paths.config_dir.join("access.json")
}

fn scopes_of(v: &Value) -> Vec<Scope> {
    v.as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(parse_scope).collect()
}

fn is_hex_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
                let revocation = t["revocation"].as_str().filter(|h| is_hex_id(h))?.to_string();
                Some(TokenRec {
                    id: handle_of(&revocation),
                    revocation,
                    subject: t["subject"].as_str().filter(|s| valid_subject(s))?.to_string(),
                    scopes: scopes_of(&t["scopes"]),
                    label: t["label"].as_str().unwrap_or("").to_string(),
                    at: t["at"].as_i64().unwrap_or(0),
                    expires_at: t["expiresAt"].as_i64()?,
                })
            })
            .collect();
        let revoked = v["revoked"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|h| is_hex_id(h)).map(String::from).collect();
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
        Policy { local, grants, tokens, revoked, spaces, ..Policy::new() }
    }

    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("version".into(), json!(2));
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
                        json!({
                            "revocation": t.revocation,
                            "subject": t.subject,
                            "scopes": t.scopes.iter().map(format_scope).collect::<Vec<_>>(),
                            "label": t.label,
                            "at": t.at,
                            "expiresAt": t.expires_at,
                        })
                    })
                    .collect(),
            ),
        );
        o.insert("revoked".into(), json!(self.revoked));
        o.insert("spaces".into(), Value::Object(self.spaces.iter().map(|(k, f)| (k.clone(), json!({ "identities": f.identities }))).collect()));
        Value::Object(o)
    }

    /// Writes the policy for you alone (and, on Windows, the service, which checks tokens with it).
    pub fn save(&self, file: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.to_json()).unwrap_or_default();
        crate::wiki::write_private(file, &format!("{text}\n"), &crate::paths::service_readers())?;
        Ok(())
    }

    pub fn facts(&self, space: &SpaceId) -> Facts {
        self.spaces.get(&space.to_string()).cloned().unwrap_or_default()
    }

    /// The live row of a token, by its revocation id.
    fn row(&self, revocation: &str, now: i64) -> Option<&TokenRec> {
        self.tokens.iter().find(|t| t.revocation == revocation && t.expires_at > now && !self.revoked.contains(&t.revocation))
    }

    /// The principal a bearer token stands for: one of this Xenolith's tokens, checked against its
    /// roots and revocations, with a live row here. None otherwise.
    pub fn principal_for(&self, text: &str, now: i64) -> Option<Principal> {
        if self.problem.is_some() || !crate::grants::looks_like_token(text) {
            return None;
        }
        let token = crate::grants::parse(self, text, now)?;
        let rec = self.row(&token.id(), now)?;
        if token.bearer().slug.as_str() != rec.subject {
            return None;
        }
        Some(Principal::Token { id: rec.id.clone(), subject: rec.subject.clone(), scopes: rec.scopes.clone(), proof: Proof(std::sync::Arc::new(token)) })
    }

    /// A principal as this policy sees it now: a token's holder again from its live row, so a
    /// revoked or expired token stops working at once, even on a connection opened before.
    pub fn current(&self, who: &Principal, now: i64) -> Option<Principal> {
        match who {
            Principal::Local => Some(Principal::Local),
            Principal::Token { proof, .. } => {
                if self.problem.is_some() {
                    return None;
                }
                let t = self.row(&proof.0.id(), now)?;
                Some(Principal::Token { id: t.id.clone(), subject: t.subject.clone(), scopes: t.scopes.clone(), proof: proof.clone() })
            }
        }
    }

    fn grant_level(&self, subject: &str, space: &SpaceId, facts: &Facts, now: i64) -> Option<Level> {
        self.grants.iter().filter(|g| g.subject == subject && g.expires_at > now && g.scope.covers(space, facts)).map(|g| g.scope.level).max()
    }

    /// What a token's row reaches in a space: the lower of its own scopes and its subject's live
    /// grants there. grants.rs reads it for xenonet-authz, which also holds it to the token's
    /// rights.
    pub fn row_level(&self, subject: &str, scopes: &[Scope], space: &SpaceId, now: i64) -> Option<Level> {
        let facts = self.facts(space);
        let asked = scopes.iter().filter(|s| s.covers(space, &facts)).map(|s| s.level).max()?;
        let granted = self.grant_level(subject, space, &facts, now)?;
        Some(asked.min(granted))
    }

    /// The most a principal may do in a space, as xenonet-authz decides it, or None: deny by
    /// default.
    pub fn level(&self, who: &Principal, space: &SpaceId, now: i64) -> Option<Level> {
        match who {
            Principal::Local => crate::grants::level(self, None, space, now),
            Principal::Token { subject, scopes, proof, .. } => {
                if self.problem.is_some() {
                    return None;
                }
                crate::grants::level(self, Some((&proof.0, subject, scopes)), space, now)
            }
        }
    }

    pub fn allows(&self, who: &Principal, space: &SpaceId, need: Level, now: i64) -> bool {
        match who {
            Principal::Local => crate::grants::decide(self, None, space, need, now).is_ok(),
            Principal::Token { subject, scopes, proof, .. } => self.problem.is_none() && crate::grants::decide(self, Some((&proof.0, subject, scopes)), space, need, now).is_ok(),
        }
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
            return wiki_err(format!("\"{subject}\" is not a subject: use actor:<slug> or client:<id>."));
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

    /// Records a token minted for `subject` (its revocation id, from grants::mint) with the row it
    /// selects by. Returns the record; the token itself is never kept.
    pub fn record(&mut self, subject: &str, scopes: Vec<Scope>, label: &str, revocation: &str, until: i64, now: i64) -> Result<TokenRec> {
        if !valid_subject(subject) {
            return wiki_err(format!("\"{subject}\" is not a subject: use actor:<slug> or client:<id>."));
        }
        if scopes.is_empty() {
            return wiki_err("A token needs at least one scope, such as ctx:read.");
        }
        if !is_hex_id(revocation) {
            return wiki_err("That isn't a token's revocation id.");
        }
        let rec = TokenRec {
            id: handle_of(revocation),
            revocation: revocation.into(),
            subject: subject.into(),
            scopes,
            label: crate::text::one_line(label).chars().take(80).collect(),
            at: now,
            expires_at: until,
        };
        self.tokens.push(rec.clone());
        Ok(rec)
    }

    /// Revokes tokens by handle (tok_…) or by subject (every token it holds): their revocation ids
    /// join `revoked`, so the tokens are no tokens even if a row came back. Returns the revocation
    /// ids.
    pub fn revoke_token(&mut self, id_or_subject: &str) -> Vec<String> {
        let gone: Vec<String> = self.tokens.iter().filter(|t| t.id == id_or_subject || t.subject == id_or_subject).map(|t| t.revocation.clone()).collect();
        self.tokens.retain(|t| !gone.contains(&t.revocation));
        for r in &gone {
            if !self.revoked.contains(r) {
                self.revoked.push(r.clone());
            }
        }
        gone
    }

    /// Drops expired tokens and grants (kept a day after they lapse, so a listing can show them).
    pub fn prune(&mut self, now: i64) {
        self.grants.retain(|g| g.expires_at + 86_400_000 > now);
        self.tokens.retain(|t| t.expires_at + 86_400_000 > now);
    }
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

/// The policy, with this Xenolith's own id and its token roots, which decisions need.
pub fn load(paths: &AppPaths) -> Policy {
    let mut p = Policy::load(&policy_file(paths));
    p.me = crate::grants::Me::load(paths);
    p.roots = crate::grants::roots(paths);
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grants::{self, Me};
    use xenonet_authz::TokenRoot;
    use xenonet_id::sign::{KeyHome, SeedSigner};

    fn sp(s: &str) -> SpaceId {
        SpaceId::parse(s).unwrap()
    }
    fn sc(s: &str) -> Scope {
        parse_scope(s).unwrap()
    }

    /// A Xenolith's root and a policy that knows it.
    fn world(now: i64) -> (TokenRoot, Policy) {
        let key = SeedSigner::from_seed(&[5; 32], KeyHome::File).unwrap();
        let me = Me::of(&key).unwrap();
        let root = TokenRoot::new(me.machine.clone(), &key, now - 1000, now + grants::ROOT_MS).unwrap();
        let p = Policy { me, roots: vec![root.certificate().clone()], ..Policy::new() };
        (root, p)
    }

    /// A token minted and recorded in `p`: its text and its principal.
    fn issue(root: &TokenRoot, p: &mut Policy, subject: &str, scopes: &[&str], until: i64, now: i64) -> (String, Principal) {
        let scopes: Vec<Scope> = scopes.iter().map(|s| sc(s)).collect();
        let (token, text) = grants::mint(root, subject, &scopes, until).unwrap();
        p.record(subject, scopes, "", &token.id(), until, now).unwrap();
        let who = p.principal_for(&text, now).unwrap();
        (text, who)
    }

    #[test]
    fn selectors_follow_their_grammar_with_space_keys() {
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
    fn bearer_headers_and_token_shapes() {
        assert_eq!(bearer_of(None), None);
        assert_eq!(bearer_of(Some("Basic abc")), Some(None));
        assert_eq!(bearer_of(Some("Bearer")), Some(None));
        assert_eq!(bearer_of(Some("bearer  abc ")), Some(Some("abc".into())));
        assert_eq!(bearer_of(Some("Bearer a b")), Some(None));
        assert!(!grants::looks_like_token(&format!("xenolith_{}", "0".repeat(64))), "the old form is no token");
        assert_eq!(handle_of("0123456789abcdef"), "tok_0123456789ab");
    }

    #[test]
    fn deny_by_default_and_the_lower_of_scope_and_grant() {
        let now = 1_800_000_000_000;
        let (root, mut p) = world(now);
        let atlas = sp("project:atlas");
        assert_eq!(p.level(&Principal::Local, &atlas, now), Some(Level::Admin), "you have admin everywhere");
        let (_, who) = issue(&root, &mut p, "actor:hermes", &["ctx:write:project:atlas", "ctx:append:by:hermes"], now + GRANT_MS * 2, now);
        assert_eq!(p.level(&who, &atlas, now), None, "a token asks, a grant allows: nothing granted yet");
        p.grant("actor:hermes", sc("ctx:read:project:atlas"), GRANT_MS, "you", now).unwrap();
        assert_eq!(p.level(&who, &atlas, now), Some(Level::Read), "the grant caps what the token asks");
        assert!(p.allows(&who, &atlas, Level::Read, now) && !p.allows(&who, &atlas, Level::Append, now));
        assert_eq!(p.level(&who, &SpaceId::local(), now), None, "nothing granted on this machine's space");
        p.grant("actor:hermes", sc("ctx:admin:by:hermes"), GRANT_MS, "you", now).unwrap();
        assert_eq!(p.level(&who, &sp("actor:hermes"), now), Some(Level::Append), "the token asks only append there");
        assert_eq!(p.level(&who, &atlas, now + GRANT_MS + 1), None, "grants expire");
        assert_eq!(p.revoke("actor:hermes", None), 2);
        assert_eq!(p.level(&who, &atlas, now), None, "revoked at once");
        assert_eq!(Policy::scope_ceiling(&who), Some(Level::Write));
        p.local = None;
        assert_eq!(p.level(&Principal::Local, &atlas, now), None);
        assert!(p.grant("hermes", sc("ctx:read"), GRANT_MS, "you", now).is_err(), "subjects name their kind");
        assert!(p.grant("machine:laptop", sc("ctx:read"), GRANT_MS, "you", now).is_err(), "machines join by a share now, not a grant");
    }

    #[test]
    fn a_token_reaches_this_machines_space_only_through_a_grant_that_names_it() {
        let now = 1_800_000_000_000;
        let local = SpaceId::local();
        for grant in ["ctx:read", "ctx:read:all", "ctx:read:machine:*", "ctx:read:machine:loc*", "ctx:admin"] {
            let (root, mut p) = world(now);
            p.grant("client:phone", sc(grant), GRANT_MS, "you", now).unwrap();
            let (_, named) = issue(&root, &mut p, "client:phone", &["ctx:read:machine:local"], now + GRANT_MS, now);
            let (_, all) = issue(&root, &mut p, "client:phone", &["ctx:read"], now + GRANT_MS, now);
            assert_eq!(p.level(&named, &local, now), None, "{grant} does not name machine:local");
            assert_eq!(p.level(&all, &sp("project:atlas"), now).is_some(), !grant.contains("machine"), "{grant} still reaches other spaces");
        }
        let (root, mut p) = world(now);
        p.grant("client:phone", sc("ctx:read:machine:local"), GRANT_MS, "you", now).unwrap();
        let (_, named) = issue(&root, &mut p, "client:phone", &["ctx:read:machine:local"], now + GRANT_MS, now);
        let (_, all) = issue(&root, &mut p, "client:phone", &["ctx:read"], now + GRANT_MS, now);
        assert_eq!(p.level(&named, &local, now), Some(Level::Read), "named exactly, by the grant and the token");
        assert_eq!(p.level(&all, &local, now), None, "the token's own scopes must name it too");
        assert_eq!(p.level(&named, &sp("machine:studio"), now), None);
        assert_eq!(p.level(&Principal::Local, &local, now), Some(Level::Admin), "this machine's own processes are not tokens");
    }

    #[test]
    fn identities_of_a_project_come_from_the_policy() {
        let now = 1_800_000_000_000;
        let (root, mut p) = world(now);
        p.spaces.insert("project:atlas".into(), Facts { identities: vec!["adal".into()] });
        p.grant("client:phone", sc("ctx:read:identity:adal"), GRANT_MS, "you", now).unwrap();
        let (_, who) = issue(&root, &mut p, "client:phone", &["ctx:read"], now + GRANT_MS, now);
        assert_eq!(p.level(&who, &sp("project:atlas"), now), Some(Level::Read));
        assert_eq!(p.level(&who, &sp("project:other"), now), None);
        assert_eq!(p.level(&who, &sp("identity:adal"), now), Some(Level::Read));
    }

    #[test]
    fn the_policy_file_round_trips_and_a_broken_one_refuses_tokens() {
        let dir = std::env::temp_dir().join(format!("xenolith-access-{}", random_hex(8)));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("access.json");
        let now = 1_800_000_000_000;
        let (root, mut p) = world(now);
        p.grant("actor:hermes", sc("ctx:append:by:hermes"), GRANT_MS, "you", now).unwrap();
        let (secret, who) = issue(&root, &mut p, "actor:hermes", &["ctx:append:by:hermes", "ctx:read"], now + GRANT_MS, now);
        p.spaces.insert("project:atlas".into(), Facts { identities: vec!["adal".into()] });
        p.revoked.push("ab".repeat(32));
        p.save(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains(&secret) && !text.contains(&secret[grants::PREFIX.len()..][..40]), "never the token");
        let q = Policy { me: p.me.clone(), roots: p.roots.clone(), ..Policy::load(&file) };
        assert_eq!(q.grants, p.grants);
        assert_eq!(q.tokens, p.tokens);
        assert_eq!(q.spaces, p.spaces);
        assert_eq!(q.revoked, p.revoked);
        assert_eq!(q.principal_for(&secret, now).unwrap(), who);
        assert_eq!(q.level(&who, &sp("actor:hermes"), now), Some(Level::Append));
        // Minted by this root but never recorded here: nobody.
        let (_, stray) = grants::mint(&root, "actor:hermes", &[sc("ctx:read")], now + 1000).unwrap();
        assert!(q.principal_for(&stray, now).is_none(), "a token without its row reaches nothing");
        std::fs::write(&file, "{ not json").unwrap();
        let broken = Policy { me: p.me.clone(), roots: p.roots.clone(), ..Policy::load(&file) };
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
    fn tokens_expire_and_revoke_by_handle_or_subject() {
        let now = 1_800_000_000_000;
        let (root, mut p) = world(now);
        let (a, wa) = issue(&root, &mut p, "client:phone", &["ctx:read"], now + 3_600_000, now);
        let (b, _) = issue(&root, &mut p, "client:phone", &["ctx:read"], now + GRANT_MS, now);
        assert!(p.principal_for(&a, now + 3_600_001).is_none(), "expired");
        let Principal::Token { id, .. } = &wa else { unreachable!() };
        let gone = p.revoke_token(id);
        assert_eq!(gone.len(), 1);
        assert!(p.principal_for(&a, now).is_none() && p.principal_for(&b, now).is_some());
        assert!(p.revoked.contains(&gone[0]), "its revocation id is kept");
        assert_eq!(p.revoke_token("client:phone").len(), 1);
        assert!(p.tokens.is_empty());
        assert!(p.record("client:phone", vec![], "", "ab", now, now).is_err());
        assert!(p.record("client:phone", vec![sc("ctx:read")], "", "not hex", now, now).is_err());
    }

    #[test]
    fn a_principal_is_checked_again_against_the_live_policy() {
        let now = 1_800_000_000_000;
        let (root, mut p) = world(now);
        let (_, who) = issue(&root, &mut p, "actor:hermes", &["ctx:read"], now + 1_000_000, now);
        assert_eq!(p.current(&who, now), Some(who.clone()));
        assert_eq!(p.current(&who, now + 1_000_001), None, "expired since the connection opened");
        let Principal::Token { id, .. } = &who else { unreachable!() };
        p.revoke_token(id);
        assert_eq!(p.current(&who, now), None, "revoked since the connection opened");
        assert_eq!(p.current(&Principal::Local, now), Some(Principal::Local));
    }
}
