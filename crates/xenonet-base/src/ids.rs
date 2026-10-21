//! Ids, as docs/design/contracts.md fixes them (core/schema/ids.js):
//!
//! - a schema's $id, `https://xenonet.dev/schemas/<path>/v<major>.json`, and
//!   its short form, `<path>/v<major>`, which data carries as "schema";
//! - host-qualified ids: an unqualified id is this XenoNet's own host's,
//!   and one from another host is `@<host>:<local id>`;
//! - sessions, `<app>:<the run's id>`;
//! - actors, `actor:<slug>` (the person at the console is actor:you), and
//!   projects merged across hosts by their git remote, `repo:<remote>`.

use std::sync::OnceLock;

pub const SCHEMA_BASE: &str = "https://xenonet.dev/schemas";

// The regular expressions the JS exports, for its twin.
pub const HOST_ID: &str = "^[a-z][a-z0-9-]{1,31}$";
pub const ACTOR_SLUG: &str = "^[a-z0-9][a-z0-9-]{0,31}$";
pub const YOU: &str = "actor:you";

fn segment(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

// A segment that's a version: v and digits.
fn version_segment(s: &str) -> bool {
    s.strip_prefix('v').is_some_and(|d| !d.is_empty() && d.bytes().all(|c| c.is_ascii_digit()))
}

/// A schema path is lowercase segments of letters, digits and dashes
/// (session, live/snapshot, vendor/openai/codex/rollout-line), the last of
/// which isn't a version (v and digits).
pub fn is_schema_path(path: &str) -> bool {
    path.split('/').all(segment) && !path.rsplit('/').next().is_some_and(version_segment)
}

/// "`<path>/v<major>`", the form data and XenoNet's registry use.
pub fn short_id(path: &str, major: f64) -> Result<String, String> {
    if !is_schema_path(path) {
        return Err(format!("not a schema path: {path}"));
    }
    if major.fract() != 0.0 || major < 1.0 || !major.is_finite() {
        return Err(format!("not a major version: {}", crate::json::number_string(major)));
    }
    Ok(format!("{path}/v{}", major as u64))
}

/// A schema id in either form: its short form, path and major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaId {
    pub id: String,
    pub path: String,
    pub major: u64,
}

/// Either form, parsed; None when it's neither.
pub fn parse_schema_id(id: &str) -> Option<SchemaId> {
    let base = format!("{SCHEMA_BASE}/");
    let s = match id.strip_prefix(&base).and_then(|r| r.strip_suffix(".json")) {
        Some(inner) => inner,
        None => id,
    };
    // /^(.+)\/v([1-9]\d*)$/
    let (path, v) = s.rsplit_once("/v")?;
    if path.is_empty() || v.is_empty() || v.starts_with('0') || !v.bytes().all(|c| c.is_ascii_digit()) || !is_schema_path(path) {
        return None;
    }
    Some(SchemaId { id: s.to_string(), path: path.to_string(), major: v.parse().ok()? })
}

/// The $id of a schema, from its short form.
pub fn schema_uri(short: &str) -> Result<String, String> {
    let parsed = parse_schema_id(short).ok_or_else(|| format!("not a schema id: {short}"))?;
    Ok(format!("{SCHEMA_BASE}/{}.json", parsed.id))
}

// ---------- hosts ----------

/// A host id is a name you choose when hosts are first connected (studio,
/// vm1), never the machine's own name, which often holds a person's.
pub fn is_host_id(host: &str) -> bool {
    let b = host.as_bytes();
    (2..=32).contains(&b.len()) && b[0].is_ascii_lowercase() && b[1..].iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// "@vm1:claude:4f1c…" → (Some("vm1"), "claude:4f1c…"), and an unqualified
/// id → (None, id). The host runs from the @ to the first colon. None for
/// something that isn't an id: empty, or an @ without a valid host and a
/// local id after it.
pub fn parse_qualified(id: &str) -> Option<(Option<&str>, &str)> {
    if id.is_empty() {
        return None;
    }
    let Some(rest) = id.strip_prefix('@') else { return Some((None, id)) };
    let (host, local) = rest.split_once(':').unwrap_or(("", ""));
    (is_host_id(host) && !local.is_empty() && !local.starts_with('@')).then_some((Some(host), local))
}

/// An id as this XenoNet writes it: a local id from `host` gets the host's
/// prefix, unless `host` is None or this XenoNet's own (`self_host`), whose
/// ids stay as they are. An id that's already qualified keeps its host,
/// except that this XenoNet's own comes off. `shown` is how an error writes
/// the id and the host, as JSON.stringify did.
pub fn qualify(id: &str, host: Option<&str>, self_host: Option<&str>) -> Result<String, QualifyError> {
    let (h, local) = parse_qualified(id).ok_or(QualifyError::NotAnId)?;
    if let Some(host) = host {
        if !is_host_id(host) {
            return Err(QualifyError::NotAHost);
        }
    }
    let from = h.or(host);
    Ok(match from {
        None => local.to_string(),
        Some(f) if Some(f) == self_host => local.to_string(),
        Some(f) => format!("@{f}:{local}"),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualifyError {
    NotAnId,
    NotAHost,
}

// ---------- sessions ----------

/// The apps XenoNet knows sessions of, and what each is called, in the
/// order core/schema/ids.js lists them: the app registry's (`apps`).
pub fn session_apps() -> &'static [(&'static str, &'static str)] {
    static A: OnceLock<Vec<(&'static str, &'static str)>> = OnceLock::new();
    A.get_or_init(|| crate::apps::apps().iter().map(|a| (a.id.as_str(), a.name.as_str())).collect())
}

pub fn is_session_app(app: &str) -> bool {
    session_apps().iter().any(|(a, _)| *a == app)
}

/// The app a session id begins with, when it's one XenoNet knows.
pub fn app_of_session(id: &str) -> Option<&'static str> {
    let (app, _) = id.split_once(':')?;
    session_apps().iter().find(|(a, _)| *a == app).map(|(a, _)| *a)
}

/// A session's id, unqualified: the app, then the run's own id (a letter or
/// digit, then six or more letters, digits and dashes).
pub fn is_session_id(id: &str) -> bool {
    let Some(app) = app_of_session(id) else { return false };
    let run = &id.as_bytes()[app.len() + 1..];
    run.len() >= 7 && run[0].is_ascii_alphanumeric() && run.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
}

/// The alternation of the apps, longest first, as the JS's SESSION_LOCAL
/// and SESSION_PREFIX are made from it.
pub fn session_app_alternation() -> String {
    let mut apps: Vec<&str> = session_apps().iter().map(|(a, _)| *a).collect();
    apps.sort_by_key(|a| std::cmp::Reverse(a.len()));
    apps.join("|")
}

// ---------- actors ----------

pub fn is_actor_slug(slug: &str) -> bool {
    let b = slug.as_bytes();
    (1..=32).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// `actor:<slug>` or `@<host>:actor:<slug>` → (host, slug); None for any other id.
pub fn parse_actor_id(id: &str) -> Option<(Option<&str>, &str)> {
    let (host, local) = parse_qualified(id)?;
    let slug = local.strip_prefix("actor:")?;
    is_actor_slug(slug).then_some((host, slug))
}

// ---------- repos ----------

/// A remote as its host and path, the same for every way of writing it:
/// `git@github.com:willctl/xenonet.git` and `https://user@github.com/willctl/xenonet`
/// are both github.com/willctl/xenonet.
pub fn normalize_remote(url: &str) -> String {
    let mut s = url.to_lowercase();
    // ^git@([^:]+): → $1/
    if let Some(rest) = s.strip_prefix("git@") {
        if let Some(i) = rest.find(':').filter(|&i| i > 0) {
            s = format!("{}/{}", &rest[..i], &rest[i + 1..]);
        }
    }
    // ^[a-z+]+:// → ""
    if let Some(i) = s.find("://") {
        if i > 0 && s[..i].bytes().all(|c| c.is_ascii_lowercase() || c == b'+') {
            s = s[i + 3..].to_string();
        }
    }
    // ^[^@/]+@ → ""
    if let Some(i) = s.find(['@', '/']) {
        if i > 0 && s.as_bytes()[i] == b'@' {
            s = s[i + 1..].to_string();
        }
    }
    s.strip_suffix(".git").map(str::to_string).unwrap_or(s)
}

/// A project merged across hosts by its git remote:
/// repo:github.com/willctl/xenonet.
pub fn repo_id(remote: &str) -> Option<String> {
    let r = normalize_remote(remote);
    (!r.is_empty()).then(|| format!("repo:{r}"))
}

/// A remote's host and the repo's path, as repoKey reads them: a URL
/// (`scheme://[user@]host[:port][/path]`) or the scp form (`[user@]host:path`).
/// None for a remote that's a folder on disk (file://, /srv/git/x), which
/// means something only on its own machine, or that has no path.
pub fn remote_parts(url: &str) -> Option<(String, String)> {
    let s = crate::text::js_trim(url);
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
        if !(rest.is_empty() || rest.starts_with('/')) || rest.contains(crate::text::is_js_line_terminator) {
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
        if rest.starts_with("//") || rest.is_empty() || rest.contains(crate::text::is_js_line_terminator) {
            continue;
        }
        return Some((&t[..end], rest));
    }
    None
}

/// `repo:<remote>` → the normalized remote; None for any other id.
pub fn parse_repo_id(id: &str) -> Option<&str> {
    id.strip_prefix("repo:").filter(|r| !r.is_empty() && !r.starts_with(crate::text::is_js_line_terminator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_ids() {
        assert_eq!(parse_schema_id("https://xenonet.dev/schemas/live/snapshot/v1.json").map(|s| (s.id, s.major)), Some(("live/snapshot/v1".into(), 1)));
        assert!(parse_schema_id("session/v0").is_none() && parse_schema_id("Session/v1").is_none() && parse_schema_id("v1/v2").is_none());
        assert_eq!(short_id("session", 2.0).unwrap(), "session/v2");
        assert!(short_id("session/v1", 2.0).is_err() && short_id("session", 1.5).is_err());
        assert!(is_schema_path("a/v2/b"), "only the last segment can't be a version");
    }

    #[test]
    fn qualified_ids() {
        assert_eq!(parse_qualified("@vm1:claude:x"), Some((Some("vm1"), "claude:x")));
        assert_eq!(parse_qualified("claude:x"), Some((None, "claude:x")));
        assert_eq!(parse_qualified("@vm1:@vm2:x"), None);
        assert_eq!(parse_qualified("@x:y"), None, "a host needs two characters");
        assert_eq!(qualify("claude:x", Some("vm1"), None).unwrap(), "@vm1:claude:x");
        assert_eq!(qualify("@vm1:claude:x", None, Some("vm1")).unwrap(), "claude:x");
        assert_eq!(qualify("claude:x", Some("Bad"), None), Err(QualifyError::NotAHost));
    }

    #[test]
    fn sessions_actors_and_repos() {
        assert!(is_session_id("claude:0190d3e0-0000") && is_session_id("devin-cli:abcdefg") && !is_session_id("claude:abc") && !is_session_id("nope:abcdefgh"));
        assert_eq!(parse_actor_id("@vm1:actor:grok-bot"), Some((Some("vm1"), "grok-bot")));
        assert_eq!(normalize_remote("git@github.com:willctl/xenonet.git"), "github.com/willctl/xenonet");
        assert_eq!(normalize_remote("https://user@github.com/willctl/xenonet"), "github.com/willctl/xenonet");
        assert_eq!(remote_parts("ssh://git@github.com:22/willctl/xenonet.git"), Some(("github.com".into(), "willctl/xenonet".into())));
        assert_eq!(remote_parts("git@gh-work:willctl/xenonet"), Some(("gh-work".into(), "willctl/xenonet".into())));
        assert_eq!(remote_parts("file:///srv/git/x"), None);
        assert_eq!(remote_parts("/srv/git/x"), None);
        assert_eq!(remote_parts("C:/x"), None, "a drive letter is no host");
        assert_eq!(remote_parts("gh:éabc"), Some(("gh".into(), "éabc".into())), "a name that ends inside a character has no .git");
        assert_eq!(remote_parts("gh:dépôt.GIT"), Some(("gh".into(), "dépôt".into())));
    }
}
