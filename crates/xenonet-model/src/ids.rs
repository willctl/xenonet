//! Ids, as docs/design/contracts.md fixes them: typed, so a host, an app,
//! an agent, an actor and a repo can't be mixed up, and so a schema can
//! say which it takes.
//!
//! - A host id is a name a person chooses for a machine when hosts are
//!   first connected (`studio`, `vm1`): `^[a-z][a-z0-9-]{1,31}$`.
//! - An agent is a harness session, `<app>:<the run's id>`, on its own
//!   machine. Which apps there are is the adapter registry's to say; here
//!   an app is any lowercase name, and the run's id a letter or digit then
//!   six or more letters, digits and dashes. Another machine's agent (or
//!   actor) is named by its `agt_`, which its machine made, with the
//!   machine it's on beside it where a person needs to see which.
//! - An actor is `actor:<slug>`; the person at the console is `actor:you`.
//! - A repo is a project merged across hosts by its git remote,
//!   `repo:<host>/<path>`.
//! - The store's ids, `prj_`, `chk_`, `agt_` and `int_`, are below
//!   ("stored ids"), with a shared-context space's `spc_` and the
//!   `EntityRef` that says what a space is about.

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

/// `actor:<slug>` → its slug; None for any other id.
pub fn parse_actor_id(id: &str) -> Option<&str> {
    id.strip_prefix("actor:").filter(|slug| is_actor_slug(slug))
}

/// An actor of this machine, never you.
pub fn is_actor_agent(id: &str) -> bool {
    parse_actor_id(id).is_some() && id != YOU
}

/// A harness session as its app names it on its machine: `<app>:<run>`.
/// The store's own id for it is an `AgentId`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId {
    pub app: AppId,
    pub run: String,
}

impl SessionId {
    /// `claude:4f1c…`.
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(id.to_string());
        let (app, run) = id.split_once(':').ok_or_else(bad)?;
        if app == "actor" || !is_app_id(app) || !is_run_id(run) {
            return Err(bad());
        }
        Ok(SessionId { app: AppId(app.to_string()), run: run.to_string() })
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.app, self.run)
    }
}

/// An actor of this machine: `actor:<slug>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ActorId {
    pub slug: ActorSlug,
}

impl ActorId {
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(id.to_string());
        let slug = id.strip_prefix("actor:").ok_or_else(bad)?;
        Ok(ActorId { slug: ActorSlug::parse(slug)? })
    }

    /// The person at the console.
    pub fn you() -> Self {
        ActorId { slug: ActorSlug("you".into()) }
    }

    pub fn is_you(&self) -> bool {
        self.slug.as_str() == "you"
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "actor:{}", self.slug)
    }
}

/// Who acts: a session, or an actor (the person among them).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Principal {
    Agent(SessionId),
    Actor(ActorId),
}

impl Principal {
    pub fn parse(id: &str) -> Result<Self, NotAnId> {
        ActorId::parse(id).map(Principal::Actor).or_else(|_| SessionId::parse(id).map(Principal::Agent))
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

text_serde!(SessionId, "^[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{6,}$");
text_serde!(ActorId, "^actor:[a-z0-9][a-z0-9-]{0,31}$");
text_serde!(Principal, "^[a-z][a-z0-9-]*:[A-Za-z0-9][A-Za-z0-9-]{0,}$");

// ---------- repos ----------

/// A remote as its host and path, the same for every way of writing it:
/// `git@github.com:willctl/xenonet.git`, `https://user@github.com/willctl/xenonet`
/// and `ssh://git@github.com:22/willctl/xenonet` are all
/// github.com/willctl/xenonet. Lowercased, without a scheme, a user, the
/// scheme's own port (ssh's 22, https's 443, http's 80, git's 9418) or
/// `.git`; another port stays (`host:2222/x`). The scp form
/// (`[user@]host:path`) names its host by more than one letter, so a
/// Windows path (`c:/repo`) stays as it is.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.to_lowercase();
    let mut scheme = None;
    if let Some(i) = s.find("://") {
        if i > 0 && s[..i].bytes().all(|c| c.is_ascii_lowercase() || c == b'+') {
            scheme = Some(s[..i].to_string());
            s = s[i + 3..].to_string();
        }
    }
    // A user before the host: what's before an @ ahead of the path.
    let host_end = s.find('/').unwrap_or(s.len());
    if let Some(i) = s[..host_end].rfind('@') {
        s = s[i + 1..].to_string();
    }
    let host_end = s.find('/').unwrap_or(s.len());
    if let Some(c) = s[..host_end].find(':') {
        match &scheme {
            Some(scheme) => {
                let port = &s[c + 1..host_end];
                let default = match scheme.as_str() {
                    "ssh" | "git+ssh" | "ssh+git" => "22",
                    "https" => "443",
                    "http" => "80",
                    "git" => "9418",
                    _ => "",
                };
                if !default.is_empty() && port == default {
                    s = format!("{}{}", &s[..c], &s[host_end..]);
                }
            }
            None if c > 1 => s = format!("{}/{}", &s[..c], s[c + 1..].trim_start_matches('/')),
            None => {}
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

// ---------- stored ids ----------
//
// The ids the store keeps (docs/design/direction.md, "Domain model"): each
// a prefix and the base32 of 20 bytes, as a person's `prs_` is.
//
// - A project is `prj_`: the HMAC-SHA256 of its normalized git remote
//   under its owner's salt, which their person chain holds, so one repo
//   is one project on every machine of theirs, and its id names no repo
//   to anyone without the chain. A project with no remote gets one at
//   random.
// - A checkout is `chk_`, at random: one per folder on one machine.
// - An agent is `agt_`, at random, made the first time XenoNet sees the
//   session; the store keeps which app's session it is.
// - An intent is `int_`: the first 20 bytes of the hash of the act that
//   made it.
// - A space (a layer of shared context, xenonet-log's `xenolith-space/1`
//   chain) is `spc_`: the first 20 bytes of SHA-256 over
//   `xenonet-space/1`, a zero byte and the hash of the chain's entry 0, so
//   every holder of the chain derives the same id and nobody can pick one.
//
// Random ones come from an `Ids`, which the demo and the golden files seed
// so the same run makes the same ids.

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// The bytes a stored id carries.
pub const STORED_BYTES: usize = 20;
/// The base32 characters those bytes take.
const STORED_CHARS: usize = 32;

/// RFC 4648 base32, lowercase and unpadded, of 20 bytes: 32 characters.
fn base32(bytes: &[u8; STORED_BYTES]) -> String {
    let mut out = String::with_capacity(STORED_CHARS);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in bytes {
        buffer = (buffer << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
        buffer &= (1 << bits) - 1;
    }
    out
}

fn is_base32(s: &str) -> bool {
    s.len() == STORED_CHARS && s.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

fn sha256(parts: &[&[u8]]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// HMAC-SHA256 (RFC 2104) of `msg` under `key`.
fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&sha256(&[key]));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut inner, mut outer) = ([0x36u8; BLOCK], [0x5cu8; BLOCK]);
    for i in 0..BLOCK {
        inner[i] ^= k[i];
        outer[i] ^= k[i];
    }
    sha256(&[&outer, &sha256(&[&inner, msg])])
}

fn first20(bytes: &[u8; 32]) -> [u8; STORED_BYTES] {
    let mut out = [0u8; STORED_BYTES];
    out.copy_from_slice(&bytes[..STORED_BYTES]);
    out
}

macro_rules! stored_id {
    ($(#[$m:meta])* $name:ident, $prefix:literal, $pattern:literal) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// The id of these 20 bytes.
            pub fn from_bytes(bytes: &[u8; STORED_BYTES]) -> Self {
                $name(format!("{}{}", $prefix, base32(bytes)))
            }

            /// The id, when the text is one.
            pub fn parse(s: &str) -> Result<Self, NotAnId> {
                match s.strip_prefix($prefix) {
                    Some(rest) if is_base32(rest) => Ok($name(s.to_string())),
                    _ => Err(NotAnId(s.to_string())),
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

        text_serde!($name, $pattern);
    };
}

stored_id!(
    /// A project: `prj_` and 32 base32 characters.
    ProjectId,
    "prj_",
    "^prj_[a-z2-7]{32}$"
);
stored_id!(
    /// A checkout of a project, one folder on one machine: `chk_` and 32 base32 characters.
    CheckoutId,
    "chk_",
    "^chk_[a-z2-7]{32}$"
);
stored_id!(
    /// An agent, a harness's session as the store knows it: `agt_` and 32 base32 characters.
    AgentId,
    "agt_",
    "^agt_[a-z2-7]{32}$"
);
stored_id!(
    /// An intent: `int_` and 32 base32 characters.
    IntentId,
    "int_",
    "^int_[a-z2-7]{32}$"
);
stored_id!(
    /// A space of shared context, by its chain: `spc_` and 32 base32 characters.
    SpaceId,
    "spc_",
    "^spc_[a-z2-7]{32}$"
);

/// A person's salt for their projects' ids: 32 bytes their person chain
/// holds from its founding.
pub type ProjectSalt = [u8; 32];

impl ProjectId {
    /// The project of a repo, by its normalized remote (`normalize_remote`),
    /// for the person whose salt this is.
    pub fn of_repo(salt: &ProjectSalt, repo_key: &str) -> Self {
        Self::from_bytes(&first20(&hmac_sha256(salt, repo_key.as_bytes())))
    }
}

impl IntentId {
    /// The intent an act made, by the act's hash.
    pub fn of_act(hash: &[u8; 32]) -> Self {
        Self::from_bytes(&first20(hash))
    }
}

impl SpaceId {
    /// What a space's id is made under, before the zero byte.
    pub const DOMAIN: &'static str = "xenonet-space/1";

    /// The space whose chain's entry 0 hashes to `genesis` (the SHA-256 of
    /// its canonical JSON with its signers, xenonet-log's `Entry::hash`).
    pub fn of_genesis(genesis: &[u8; 32]) -> Self {
        Self::from_bytes(&first20(&sha256(&[Self::DOMAIN.as_bytes(), b"\0", genesis])))
    }
}

// ---------- what a space is about ----------
//
// A space is about one entity, named by its id: a project (`prj_`), a
// person (`prs_`), a machine by its signing key (`xns1_`), an actor of a
// machine (`actor:<slug>@xns1_…`, since a slug means something only on its
// machine), or an account (`identity:<its id>`, as `identity/v1` writes
// it). A space made before it's tied to one names it by kind and name,
// `name:<kind>:<name>`, as Xenolith's folders do (`name:project:atlas`);
// xenonet-log's `bind` ties it to an id once. Orgs come later.

/// The kinds a space not yet tied to an entity names it by.
pub const NAME_KINDS: [&str; 4] = ["project", "identity", "actor", "machine"];
/// The longest account id a space names.
const IDENTITY_MAX: usize = 160;
/// The longest name an unbound space names.
const NAME_MAX: usize = 64;

/// The text of an `EntityRef` (its schema's pattern). A name may not hold
/// `..`, which `EntityRef::parse` refuses and the pattern doesn't say.
pub const ENTITY_REF: &str = "^(prj_[a-z2-7]{32}|prs_[a-z2-7]{32}|xns1_[a-z2-7]{32}|actor:[a-z0-9][a-z0-9-]{0,31}@xns1_[a-z2-7]{32}|identity:[a-z][a-z0-9-]{0,31}:[!-~]{1,160}|name:(project|identity|actor|machine):[a-z0-9][a-z0-9._-]{0,63})$";

fn prefixed_base32(s: &str, prefix: &str) -> bool {
    s.strip_prefix(prefix).is_some_and(is_base32)
}

/// Whether `id` is a signing key's id: `xns1_` and 32 base32 characters.
pub fn is_signing_key_id(id: &str) -> bool {
    prefixed_base32(id, "xns1_")
}

/// Whether `id` is a person's id: `prs_` and 32 base32 characters.
pub fn is_person_id(id: &str) -> bool {
    prefixed_base32(id, "prs_")
}

// An account's id as identity/v1 has it: a lowercase kind, a colon, then
// printable ASCII with no space (`github:github.com/willctl`).
fn is_identity_id(id: &str) -> bool {
    let Some((kind, rest)) = id.split_once(':') else { return false };
    let k = kind.as_bytes();
    (1..=32).contains(&k.len()) && k[0].is_ascii_lowercase() && lower_dash(k) && (1..=IDENTITY_MAX).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_graphic())
}

/// Whether `name` is a space's name as Xenolith's folders have them:
/// lowercase letters, digits, `.`, `_` and `-`, starting with a letter or
/// digit, 64 at most, never `..`.
pub fn is_space_name(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=NAME_MAX).contains(&b.len())
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(c))
        && !name.contains("..")
}

/// What a space is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityRef {
    Project(ProjectId),
    /// A person, by their `prs_`.
    Person(String),
    /// A machine, by its signing key's `xns1_`.
    Machine(String),
    /// An actor of the machine whose signing key is `machine`.
    Actor {
        slug: ActorSlug,
        machine: String,
    },
    /// An account, by its identity/v1 id.
    Identity(String),
    /// Not tied to an entity yet: a kind of `NAME_KINDS` and a name.
    Name {
        kind: String,
        name: String,
    },
}

impl EntityRef {
    pub fn parse(s: &str) -> Result<Self, NotAnId> {
        let bad = || NotAnId(s.to_string());
        if let Ok(p) = ProjectId::parse(s) {
            return Ok(EntityRef::Project(p));
        }
        if is_person_id(s) {
            return Ok(EntityRef::Person(s.to_string()));
        }
        if is_signing_key_id(s) {
            return Ok(EntityRef::Machine(s.to_string()));
        }
        if let Some(rest) = s.strip_prefix("actor:") {
            let (slug, machine) = rest.split_once('@').ok_or_else(bad)?;
            if !is_signing_key_id(machine) {
                return Err(bad());
            }
            return Ok(EntityRef::Actor { slug: ActorSlug::parse(slug).map_err(|_| bad())?, machine: machine.to_string() });
        }
        if let Some(id) = s.strip_prefix("identity:") {
            return if is_identity_id(id) { Ok(EntityRef::Identity(id.to_string())) } else { Err(bad()) };
        }
        if let Some(rest) = s.strip_prefix("name:") {
            let (kind, name) = rest.split_once(':').ok_or_else(bad)?;
            if NAME_KINDS.contains(&kind) && is_space_name(name) {
                return Ok(EntityRef::Name { kind: kind.to_string(), name: name.to_string() });
            }
        }
        Err(bad())
    }

    /// `project`, `person`, `machine`, `actor`, `identity` or `name`.
    pub fn kind(&self) -> &'static str {
        match self {
            EntityRef::Project(_) => "project",
            EntityRef::Person(_) => "person",
            EntityRef::Machine(_) => "machine",
            EntityRef::Actor { .. } => "actor",
            EntityRef::Identity(_) => "identity",
            EntityRef::Name { .. } => "name",
        }
    }

    /// Whether it names an entity by its id, rather than a kind and a name.
    pub fn is_bound(&self) -> bool {
        !matches!(self, EntityRef::Name { .. })
    }
}

impl fmt::Display for EntityRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityRef::Project(p) => p.fmt(f),
            EntityRef::Person(id) | EntityRef::Machine(id) => f.write_str(id),
            EntityRef::Actor { slug, machine } => write!(f, "actor:{slug}@{machine}"),
            EntityRef::Identity(id) => write!(f, "identity:{id}"),
            EntityRef::Name { kind, name } => write!(f, "name:{kind}:{name}"),
        }
    }
}

text_serde!(EntityRef, ENTITY_REF);

/// Where random ids come from: the system's random numbers, or a seed, so
/// the demo and the golden files make the same ids each run. A seeded
/// source's n-th id is the first 20 bytes of SHA-256 over its seed, its
/// kind's prefix and n, counted for all kinds together.
pub struct Ids(Source);

enum Source {
    Random,
    Seeded { seed: Vec<u8>, next: std::sync::atomic::AtomicU64 },
}

impl Ids {
    /// From the system's random numbers.
    pub fn random() -> Self {
        Ids(Source::Random)
    }

    /// From `seed`, the same ids in the same order every time.
    pub fn seeded(seed: &str) -> Self {
        Ids(Source::Seeded { seed: seed.as_bytes().to_vec(), next: std::sync::atomic::AtomicU64::new(0) })
    }

    fn bytes(&self, prefix: &str) -> [u8; STORED_BYTES] {
        match &self.0 {
            Source::Random => {
                let mut b = [0u8; STORED_BYTES];
                // The system's random numbers fail only on a broken system,
                // where no id XenoNet could make would be safe either.
                getrandom::fill(&mut b).expect("the system's random numbers");
                b
            }
            Source::Seeded { seed, next } => {
                let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                first20(&sha256(&[seed, b"\0", prefix.as_bytes(), &n.to_be_bytes()]))
            }
        }
    }

    /// A project with no remote.
    pub fn project(&self) -> ProjectId {
        ProjectId::from_bytes(&self.bytes(ProjectId::PREFIX))
    }

    pub fn checkout(&self) -> CheckoutId {
        CheckoutId::from_bytes(&self.bytes(CheckoutId::PREFIX))
    }

    pub fn agent(&self) -> AgentId {
        AgentId::from_bytes(&self.bytes(AgentId::PREFIX))
    }
}

impl fmt::Debug for Ids {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Source::Random => "Ids::random()",
            Source::Seeded { .. } => "Ids::seeded(…)",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_as_rfc_4231_has_it() {
        let hex = |b: &[u8]| b.iter().fold(String::new(), |out, x| out + &format!("{x:02x}"));
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        let long = hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First");
        assert_eq!(hex(&long), "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54", "a key longer than a block is hashed first");
    }

    #[test]
    fn hosts() {
        assert!(is_host_id("vm1") && is_host_id("studio-2") && !is_host_id("x") && !is_host_id("Vm1") && !is_host_id("1vm"));
    }

    #[test]
    fn agents_and_actors() {
        let a = SessionId::parse("claude:4f1c2a3b-1").unwrap();
        assert_eq!((a.app.as_str(), a.run.as_str()), ("claude", "4f1c2a3b-1"));
        assert_eq!(a.to_string(), "claude:4f1c2a3b-1");
        assert!(SessionId::parse("@vm1:codex:0199abcdef").is_err(), "another machine's agent is its agt_");
        assert!(SessionId::parse("claude:short").is_err(), "a run's id has seven characters or more");
        assert!(SessionId::parse("actor:kite-bot").is_err(), "an actor isn't an agent");
        assert!(SessionId::parse("Claude:abcdefgh").is_err());
        let k = ActorId::parse("actor:kite").unwrap();
        assert_eq!((k.slug.as_str(), k.to_string().as_str()), ("kite", "actor:kite"));
        assert!(ActorId::parse("actor:you").unwrap().is_you());
        assert!(ActorId::parse("@vm1:actor:kite").is_err(), "another machine's actor is its agt_");
        assert_eq!(parse_actor_id("actor:kite"), Some("kite"));
        assert_eq!(parse_actor_id("actor:Kite"), None);
        assert!(is_actor_agent("actor:kite") && !is_actor_agent(YOU));
        assert!(ActorId::parse("actor:Kite").is_err() && ActorId::parse("actor:").is_err());
        assert!(matches!(Principal::parse("actor:kite").unwrap(), Principal::Actor(_)));
        assert!(matches!(Principal::parse("grok:abcdefg").unwrap(), Principal::Agent(_)));
        assert!(Principal::parse("project:/x").is_err());
        let json = serde_json::to_string(&Principal::parse("actor:kite").unwrap()).unwrap();
        assert_eq!(json, "\"actor:kite\"");
        assert!(serde_json::from_str::<SessionId>("\"nope\"").is_err());
    }

    #[test]
    fn repos() {
        assert_eq!(normalize_remote("git@github.com:willctl/XenoNet.git"), "github.com/willctl/xenonet");
        assert_eq!(normalize_remote("https://user@github.com/willctl/xenonet"), "github.com/willctl/xenonet");
        assert_eq!(normalize_remote("ssh://git@github.com:22/willctl/xenonet"), "github.com/willctl/xenonet", "ssh's own port");
        assert_eq!(normalize_remote("https://github.com:443/willctl/xenonet.git"), "github.com/willctl/xenonet", "https's own port");
        assert_eq!(normalize_remote("ssh://git@host:2222/a/b"), "host:2222/a/b", "another port stays");
        assert_eq!(normalize_remote("alice@host.example:team/app"), "host.example/team/app", "the scp form, any user");
        assert_eq!(normalize_remote("host.example:/team/app.git"), "host.example/team/app");
        assert_eq!(normalize_remote("c:/repos/app"), "c:/repos/app", "a Windows path isn't a host");
        assert_eq!(normalize_remote("/srv/git/app.git"), "/srv/git/app");
        assert_eq!(RepoId::of_remote("ssh://git@host:22/a/b.git").unwrap().to_string(), "repo:host/a/b");
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
        assert_eq!(parse_actor_id("@vm1:actor:grok-bot"), None, "another machine's actor is its agt_");
        assert_eq!(parse_actor_id("actor:you"), Some("you"));
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
