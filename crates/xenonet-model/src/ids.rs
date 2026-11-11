//! Ids, as docs/design/contracts.md fixes them: typed, so a host, an app,
//! an agent, an actor and a repo can't be mixed up, and so a schema can
//! say which it takes.
//!
//! - A host id is a name a person chooses for a machine when hosts are
//!   first connected (`studio`, `vm1`): `^[a-z][a-z0-9-]{1,31}$`.
//! - An id from another host is qualified, `@<host>:<local id>`; an
//!   unqualified id is this XenoNet's own host's.
//! - An agent is a harness session, `<app>:<the run's id>`. Which apps
//!   there are is the adapter registry's to say; here an app is any
//!   lowercase name, and the run's id a letter or digit then six or more
//!   letters, digits and dashes.
//! - An actor is `actor:<slug>`; the person at the console is `actor:you`.
//! - A repo is a project merged across hosts by its git remote,
//!   `repo:<host>/<path>`.

use schemars::{json_schema, JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

pub const HOST_ID: &str = "^[a-z][a-z0-9-]{1,31}$";
pub const ACTOR_SLUG: &str = "^[a-z0-9][a-z0-9-]{0,31}$";
pub const YOU: &str = "actor:you";

fn lower_dash(b: &[u8]) -> bool {
    b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Why a text isn't an id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotAnId(pub String);

impl fmt::Display for NotAnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not an id: {}", self.0)
    }
}

impl std::error::Error for NotAnId {}

// The error for a string read as an id that isn't one: what it is, and
// the pattern an id matches.
fn refused<E: serde::de::Error>(s: &str, pattern: &str) -> E {
    E::invalid_value(serde::de::Unexpected::Str(crate::schema::shown(s)), &format!("a string matching {pattern}").as_str())
}

macro_rules! string_id {
    ($(#[$m:meta])* $name:ident, $pattern:expr, $check:expr) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// The id, when the text is one.
            pub fn parse(s: &str) -> Result<Self, NotAnId> {
                if $check(s) {
                    Ok($name(s.to_string()))
                } else {
                    Err(NotAnId(s.to_string()))
                }
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = NotAnId;
            fn from_str(s: &str) -> Result<Self, NotAnId> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <Cow<'de, str>>::deserialize(d)?;
                Self::parse(&s).map_err(|_| refused(&s, $pattern))
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

/// Whether `host` is a host id.
pub fn is_host_id(host: &str) -> bool {
    let b = host.as_bytes();
    (2..=32).contains(&b.len()) && b[0].is_ascii_lowercase() && lower_dash(&b[1..])
}

/// Whether `slug` is an actor's slug.
pub fn is_actor_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    (1..=32).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && lower_dash(b)
}

/// Whether `app` could be an app's id: lowercase letters, digits and
/// dashes, starting with a letter. The registry says which exist.
pub fn is_app_id(app: &str) -> bool {
    let b = app.as_bytes();
    !b.is_empty() && b[0].is_ascii_lowercase() && lower_dash(b)
}

/// The run part of a session id: a letter or digit, then six or more
/// letters, digits and dashes.
pub fn is_run_id(run: &str) -> bool {
    let b = run.as_bytes();
    b.len() >= 7 && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

string_id! {
    /// A machine's name in a network of hosts.
    HostId, HOST_ID, is_host_id
}
string_id! {
    /// An actor's slug: the part of `actor:<slug>` after the colon.
    ActorSlug, ACTOR_SLUG, is_actor_slug
}
string_id! {
    /// An app the registry may know: `claude`, `codex`, `grok`…
    AppId, "^[a-z][a-z0-9-]*$", is_app_id
}

/// A local id and the host it's from: `@<host>:<local>`, or the local id
/// alone for this XenoNet's own host.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Qualified<'a> {
    pub host: Option<&'a str>,
    pub local: &'a str,
}

/// `@vm1:claude:4f1c…` → host vm1, local `claude:4f1c…`; an unqualified id
/// → no host. None for what isn't an id: empty, or an `@` without a valid
/// host and a local id after it.
pub fn parse_qualified(id: &str) -> Option<Qualified<'_>> {
    if id.is_empty() {
        return None;
    }
    let Some(rest) = id.strip_prefix('@') else { return Some(Qualified { host: None, local: id }) };
    let (host, local) = rest.split_once(':').unwrap_or(("", ""));
    (is_host_id(host) && !local.is_empty() && !local.starts_with('@')).then_some(Qualified { host: Some(host), local })
}

/// An id as one XenoNet writes it: a local id from `host` gets the host's
/// prefix, unless `host` is None or this XenoNet's own (`self_host`), whose
/// ids stay as they are. An id that's already qualified keeps its host,
/// except that this XenoNet's own comes off.
pub fn qualify(id: &str, host: Option<&str>, self_host: Option<&str>) -> Result<String, QualifyError> {
    let q = parse_qualified(id).ok_or(QualifyError::NotAnId)?;
    if host.is_some_and(|h| !is_host_id(h)) {
        return Err(QualifyError::NotAHost);
    }
    Ok(match q.host.or(host) {
        None => q.local.to_string(),
        Some(f) if Some(f) == self_host => q.local.to_string(),
        Some(f) => format!("@{f}:{}", q.local),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualifyError {
    NotAnId,
    NotAHost,
}

/// `actor:<slug>` or `@<host>:actor:<slug>` → (host, slug); None for any other id.
pub fn parse_actor_id(id: &str) -> Option<(Option<&str>, &str)> {
    let q = parse_qualified(id)?;
    let slug = q.local.strip_prefix("actor:")?;
    is_actor_slug(slug).then_some((q.host, slug))
}

/// An actor of this machine, never you.
pub fn is_actor_agent(id: &str) -> bool {
    parse_actor_id(id).is_some_and(|(host, _)| host.is_none()) && id != YOU
}

/// A harness session: `<app>:<run>`, on this host or another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentId {
    pub host: Option<HostId>,
    pub app: AppId,
    pub run: String,
}

impl AgentId {
    /// `claude:4f1c…` or `@vm1:claude:4f1c…`.
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(id.to_string());
        let q = parse_qualified(id).ok_or_else(bad)?;
        let (app, run) = q.local.split_once(':').ok_or_else(bad)?;
        if app == "actor" || !is_app_id(app) || !is_run_id(run) {
            return Err(bad());
        }
        Ok(AgentId { host: q.host.map(|h| HostId(h.to_string())), app: AppId(app.to_string()), run: run.to_string() })
    }

    /// The id without its host.
    pub fn local(&self) -> String {
        format!("{}:{}", self.app, self.run)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Some(h) => write!(f, "@{h}:{}:{}", self.app, self.run),
            None => write!(f, "{}:{}", self.app, self.run),
        }
    }
}

/// An actor: `actor:<slug>`, on this host or another.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActorId {
    pub host: Option<HostId>,
    pub slug: ActorSlug,
}

impl ActorId {
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(id.to_string());
        let q = parse_qualified(id).ok_or_else(bad)?;
        let slug = q.local.strip_prefix("actor:").ok_or_else(bad)?;
        Ok(ActorId { host: q.host.map(|h| HostId(h.to_string())), slug: ActorSlug::parse(slug)? })
    }

    /// The person at the console, who is never on another host.
    pub fn you() -> Self {
        ActorId { host: None, slug: ActorSlug("you".into()) }
    }

    pub fn is_you(&self) -> bool {
        self.host.is_none() && self.slug.as_str() == "you"
    }

    pub fn local(&self) -> String {
        format!("actor:{}", self.slug)
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.host {
            Some(h) => write!(f, "@{h}:actor:{}", self.slug),
            None => write!(f, "actor:{}", self.slug),
        }
    }
}

/// Who acts: a session, or an actor (the person among them).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Principal {
    Agent(AgentId),
    Actor(ActorId),
}

impl Principal {
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        ActorId::parse(id).map(Principal::Actor).or_else(|_| AgentId::parse(id).map(Principal::Agent))
    }

    pub fn host(&self) -> Option<&HostId> {
        match self {
            Principal::Agent(a) => a.host.as_ref(),
            Principal::Actor(a) => a.host.as_ref(),
        }
    }

    pub fn is_you(&self) -> bool {
        matches!(self, Principal::Actor(a) if a.is_you())
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Principal::Agent(a) => a.fmt(f),
            Principal::Actor(a) => a.fmt(f),
        }
    }
}

macro_rules! text_serde {
    ($name:ident, $pattern:expr) => {
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_string())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <Cow<'de, str>>::deserialize(d)?;
                Self::parse(&s).map_err(|_| refused(&s, $pattern))
            }
        }
        impl FromStr for $name {
            type Err = NotAnId;
            fn from_str(s: &str) -> Result<Self, NotAnId> {
                Self::parse(s)
            }
        }
        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

text_serde!(AgentId, "^(@[a-z][a-z0-9-]{1,31}:)?[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{6,}$");
text_serde!(ActorId, "^(@[a-z][a-z0-9-]{1,31}:)?actor:[a-z0-9][a-z0-9-]{0,31}$");
text_serde!(Principal, "^(@[a-z][a-z0-9-]{1,31}:)?[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{0,}$");

// ---------- repos ----------

/// A remote as its host and path, the same for every way of writing it:
/// `git@github.com:willctl/xenonet.git` and `https://user@github.com/willctl/xenonet`
/// are both github.com/willctl/xenonet.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.to_lowercase();
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some(i) = rest.find(':').filter(|&i| i > 0) {
            s = format!("{}/{}", &rest[..i], &rest[i + 1..]);
        }
    }
    if let Some(i) = s.find("://") {
        if i > 0 && s[..i].bytes().all(|c| c.is_ascii_lowercase() || c == b'+') {
            s = s[i + 3..].to_string();
        }
    }
    if let Some(i) = s.find(['@', '/']) {
        if i > 0 && s.as_bytes()[i] == b'@' {
            s = s[i + 1..].to_string();
        }
    }
    s.strip_suffix(".git").map(str::to_string).unwrap_or(s)
}

/// A remote's host and the repo's path, as repoKey reads them: a URL
/// (`scheme://[user@]host[:port][/path]`) or the scp form (`[user@]host:path`).
/// None for a remote that's a folder on disk (file://, /srv/git/x), which
/// means something only on its own machine, or that has no path.
pub fn remote_parts(url: &str) -> Option<(String, String)> {
    let s = xenonet_text::text::trim(url);
    let (host, rest) = match url_form(s) {
        UrlForm::Remote(host, rest) => (host, rest),
        UrlForm::File => return None,
        UrlForm::No => scp_form(s)?,
    };
    let repo = rest.trim_start_matches('/').trim_end_matches('/');
    // The last four bytes, when they're whole characters: a repo whose name
    // ends inside one (gh:éabc) has no ".git" to take off.
    let repo = match repo.len().checked_sub(4).and_then(|i| Some((i, repo.get(i..)?))) {
        Some((i, end)) if end.eq_ignore_ascii_case(".git") => &repo[..i],
        _ => repo,
    };
    (!repo.is_empty()).then(|| (host.to_lowercase(), repo.to_string()))
}

// What url_form makes of a remote: no URL, a file:// URL, which is no
// remote, or a URL's host and path.
enum UrlForm<'a> {
    No,
    File,
    Remote(&'a str, &'a str),
}

// /^([a-z][a-z0-9+.-]*):\/\/(?:[^@/]*@)?(\[[^\]/]+\]|[^/:]+)(?::\d*)?(\/.*)?$/i
fn url_form(s: &str) -> UrlForm<'_> {
    let Some((scheme, after)) = s.split_once("://") else {
        return UrlForm::No;
    };
    let b = scheme.as_bytes();
    if b.is_empty() || !b[0].is_ascii_alphabetic() || !b.iter().all(|c| c.is_ascii_alphanumeric() || b"+.-".contains(c)) {
        return UrlForm::No;
    }
    // The user part, when there's an @ before the first /; else none.
    let tries: Vec<&str> = match after.find(['@', '/']) {
        Some(i) if after.as_bytes()[i] == b'@' => vec![&after[i + 1..], after],
        _ => vec![after],
    };
    for t in tries {
        let host_end =
            if t.starts_with('[') { t.find(']').map(|i| i + 1).filter(|&i| !t[1..i - 1].contains('/') && i > 2) } else { t.find(['/', ':']).or(Some(t.len())).filter(|&i| i > 0) };
        let Some(h) = host_end else { continue };
        let (host, mut rest) = t.split_at(h);
        if let Some(port) = rest.strip_prefix(':') {
            let digits = port.len() - port.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            rest = &port[digits..];
        }
        if !(rest.is_empty() || rest.starts_with('/')) || rest.contains(xenonet_text::text::is_line_terminator) {
            continue;
        }
        if scheme.eq_ignore_ascii_case("file") {
            return UrlForm::File;
        }
        return UrlForm::Remote(host, rest);
    }
    UrlForm::No
}

// /^(?:[^@/]+@)?([^:/\\]{2,}):(?!\/\/)(.+)$/
fn scp_form(s: &str) -> Option<(&str, &str)> {
    let mut tries = Vec::new();
    if let Some(i) = s.find(['@', '/']).filter(|&i| i > 0 && s.as_bytes()[i] == b'@') {
        tries.push(&s[i + 1..]);
    }
    tries.push(s);
    for t in tries {
        let end = t.find([':', '/', '\\']).unwrap_or(t.len());
        if end < 2 || t.as_bytes().get(end) != Some(&b':') || t[..end].chars().count() < 2 {
            continue;
        }
        let rest = &t[end + 1..];
        if rest.starts_with("//") || rest.is_empty() || rest.contains(xenonet_text::text::is_line_terminator) {
            continue;
        }
        return Some((&t[..end], rest));
    }
    None
}

/// A project merged across hosts by its git remote: `repo:github.com/willctl/xenonet`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RepoId(String);

impl RepoId {
    /// From a remote, however it's written; None for an empty one.
    pub fn of_remote(remote: &str) -> Option<Self> {
        let r = normalize_remote(remote);
        (!r.is_empty()).then_some(RepoId(r))
    }

    /// `repo:<remote>`.
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        match id.strip_prefix("repo:") {
            Some(r) if !r.is_empty() && !r.contains(['\n', '\r', '\u{2028}', '\u{2029}']) => Ok(RepoId(r.to_string())),
            _ => Err(NotAnId(id.to_string())),
        }
    }

    /// The normalized remote.
    pub fn remote(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "repo:{}", self.0)
    }
}

text_serde!(RepoId, "^repo:.+$");

// ---------- schemas ----------

/// A schema's id: its path (`live/snapshot`) and major version; written
/// short (`live/snapshot/v1`) in data, and as a URI
/// (`https://xenonet.dev/schemas/live/snapshot/v1.json`) in `$id`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SchemaId {
    pub path: String,
    pub major: u64,
}

fn segment(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && lower_dash(b)
}

fn version_segment(s: &str) -> bool {
    s.strip_prefix('v').is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
}

/// A schema path is lowercase segments of letters, digits and dashes, the
/// last of which isn't a version.
pub fn is_schema_path(path: &str) -> bool {
    path.split('/').all(segment) && !path.rsplit('/').next().is_some_and(version_segment)
}

impl SchemaId {
    /// Either form.
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(id.to_string());
        let base = format!("{}/", crate::schema::BASE);
        let s = id.strip_prefix(&base).and_then(|r| r.strip_suffix(".json")).unwrap_or(id);
        let (path, v) = s.rsplit_once("/v").ok_or_else(bad)?;
        if path.is_empty() || v.is_empty() || v.starts_with('0') || !v.bytes().all(|c| c.is_ascii_digit()) || !is_schema_path(path) {
            return Err(bad());
        }
        Ok(SchemaId { path: path.to_string(), major: v.parse().map_err(|_| bad())? })
    }

    /// `<path>/v<major>`.
    pub fn short(&self) -> String {
        format!("{}/v{}", self.path, self.major)
    }

    /// The `$id`.
    pub fn uri(&self) -> String {
        format!("{}/{}.json", crate::schema::BASE, self.short())
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

text_serde!(SchemaId, "^[a-z0-9][a-z0-9-]*(/[a-z0-9][a-z0-9-]*)*/v[1-9][0-9]*$");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_and_qualified_ids() {
        assert!(is_host_id("vm1") && is_host_id("studio-2") && !is_host_id("x") && !is_host_id("Vm1") && !is_host_id("1vm"));
        assert_eq!(parse_qualified("@vm1:claude:x"), Some(Qualified { host: Some("vm1"), local: "claude:x" }));
        assert_eq!(parse_qualified("claude:x"), Some(Qualified { host: None, local: "claude:x" }));
        assert_eq!(parse_qualified("@vm1:@vm2:x"), None);
        assert_eq!(parse_qualified("@x:y"), None, "a host needs two characters");
        assert_eq!(qualify("claude:x", Some("vm1"), None).unwrap(), "@vm1:claude:x");
        assert_eq!(qualify("@vm1:claude:x", None, Some("vm1")).unwrap(), "claude:x");
        assert_eq!(qualify("claude:x", Some("VM"), None), Err(QualifyError::NotAHost));
        assert_eq!(qualify("", None, None), Err(QualifyError::NotAnId));
    }

    #[test]
    fn agents_and_actors() {
        let a = AgentId::parse("claude:4f1c2a3b-1").unwrap();
        assert_eq!((a.app.as_str(), a.run.as_str(), a.host.is_none()), ("claude", "4f1c2a3b-1", true));
        assert_eq!(a.to_string(), "claude:4f1c2a3b-1");
        let r = AgentId::parse("@vm1:codex:0199abcdef").unwrap();
        assert_eq!(r.host.as_ref().map(HostId::as_str), Some("vm1"));
        assert_eq!(r.to_string(), "@vm1:codex:0199abcdef");
        assert_eq!(r.local(), "codex:0199abcdef");
        assert!(AgentId::parse("claude:short").is_err(), "a run's id has seven characters or more");
        assert!(AgentId::parse("actor:kite-bot").is_err(), "an actor isn't an agent");
        assert!(AgentId::parse("Claude:abcdefgh").is_err());
        let k = ActorId::parse("actor:kite").unwrap();
        assert_eq!((k.slug.as_str(), k.to_string().as_str()), ("kite", "actor:kite"));
        assert!(ActorId::parse("actor:you").unwrap().is_you());
        assert!(!ActorId::parse("@vm1:actor:you").unwrap().is_you(), "you are only ever here");
        assert!(ActorId::parse("actor:Kite").is_err() && ActorId::parse("actor:").is_err());
        assert!(matches!(Principal::parse("actor:kite").unwrap(), Principal::Actor(_)));
        assert!(matches!(Principal::parse("grok:abcdefg").unwrap(), Principal::Agent(_)));
        assert!(Principal::parse("project:/x").is_err());
        let json = serde_json::to_string(&Principal::parse("@vm1:actor:kite").unwrap()).unwrap();
        assert_eq!(json, "\"@vm1:actor:kite\"");
        assert!(serde_json::from_str::<AgentId>("\"nope\"").is_err());
    }

    #[test]
    fn repos() {
        assert_eq!(normalize_remote("git@github.com:willctl/XenoNet.git"), "github.com/willctl/xenonet");
        assert_eq!(normalize_remote("https://user@github.com/willctl/xenonet"), "github.com/willctl/xenonet");
        assert_eq!(RepoId::of_remote("ssh://git@host:22/a/b.git").unwrap().to_string(), "repo:host:22/a/b");
        assert!(RepoId::of_remote("").is_none());
        assert_eq!(RepoId::parse("repo:github.com/a/b").unwrap().remote(), "github.com/a/b");
        assert!(RepoId::parse("repo:").is_err() && RepoId::parse("project:/x").is_err());
    }

    #[test]
    fn schema_ids() {
        let s = SchemaId::parse("https://xenonet.dev/schemas/live/snapshot/v1.json").unwrap();
        assert_eq!((s.path.as_str(), s.major), ("live/snapshot", 1));
        assert_eq!(s.short(), "live/snapshot/v1");
        assert_eq!(SchemaId::parse("live/snapshot/v1").unwrap().uri(), "https://xenonet.dev/schemas/live/snapshot/v1.json");
        assert!(SchemaId::parse("session/v0").is_err() && SchemaId::parse("Session/v1").is_err() && SchemaId::parse("v1/v2").is_err());
        assert!(is_schema_path("a/v2/b"), "only the last segment can't be a version");
    }

    #[test]
    fn actors_and_remotes() {
        assert_eq!(parse_actor_id("@vm1:actor:grok-bot"), Some((Some("vm1"), "grok-bot")));
        assert_eq!(parse_actor_id("actor:you"), Some((None, "you")));
        assert_eq!(parse_actor_id("claude:4f1c-000"), None);
        assert_eq!(remote_parts("ssh://git@github.com:22/willctl/xenonet.git"), Some(("github.com".into(), "willctl/xenonet".into())));
        assert_eq!(remote_parts("git@gh-work:willctl/xenonet"), Some(("gh-work".into(), "willctl/xenonet".into())));
        assert_eq!(remote_parts("file:///srv/git/x"), None);
        assert_eq!(remote_parts("/srv/git/x"), None);
        assert_eq!(remote_parts("C:/x"), None, "a drive letter is no host");
        assert_eq!(remote_parts("gh:éabc"), Some(("gh".into(), "éabc".into())), "a name that ends inside a character has no .git");
        assert_eq!(remote_parts("gh:dépôt.GIT"), Some(("gh".into(), "dépôt".into())));
    }
}
