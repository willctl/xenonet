//! Pages as MCP resources.
//!
//! Every page of every space a caller may read is a resource, `xenolith://<kind>/<name>/pages/<slug>`
//! (xenolith://machine/local/pages/atlas is this machine's page "atlas"), read as text/markdown.
//! Over stdio a client can subscribe to a page and gets notifications/resources/updated when it
//! changes, and notifications/resources/list_changed when pages come or go in the spaces it can
//! read. The HTTP service is stateless, so it offers neither.

use crate::mcp::{Ctx, State};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use xenolith_core::access::{self, Level, Principal};
use xenolith_core::paths::AppPaths;
use xenolith_core::spaces::{self, SpaceId};
use xenolith_core::text::{defang_remote_images, from_ms, local_iso, now_ms};
use xenolith_core::wiki;

pub const SCHEME: &str = "xenolith://";
const PAGE_SIZE: usize = 500;
/// JSON-RPC's code for a resource that doesn't exist (MCP), also given for one the caller may not read.
pub const NOT_FOUND: i64 = -32002;

pub type RpcResult = Result<Value, (i64, String)>;

pub fn page_uri(space: &SpaceId, slug: &str) -> String {
    format!("{SCHEME}{}/{}/pages/{slug}", space.kind, space.name)
}

fn valid_slug(s: &str) -> bool {
    !s.is_empty() && s.len() <= 120 && s.as_bytes()[0].is_ascii_alphanumeric() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// A page's space and slug from its URI.
pub fn parse_uri(uri: &str) -> Option<(SpaceId, String)> {
    let rest = uri.strip_prefix(SCHEME)?;
    let mut parts = rest.split('/');
    let (kind, name, pages, slug) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || pages != "pages" || !valid_slug(slug) {
        return None;
    }
    let id = SpaceId::parse(&format!("{kind}:{name}")).ok()?;
    (id.kind == kind && id.name == name).then(|| (id, slug.to_string()))
}

fn root(st: &State) -> Result<&Path, (i64, String)> {
    st.wiki_dir.as_deref().filter(|_| st.setup_error.is_none()).ok_or_else(|| (-32603, st.setup_error.clone().unwrap_or_else(|| "The wiki folder could not be resolved.".into())))
}

/// The spaces a principal may read now, with their folders.
fn readable(root: &Path, who: &Principal) -> Vec<(SpaceId, std::path::PathBuf)> {
    let policy = access::load(&AppPaths::current());
    let now = now_ms();
    let Some(who) = policy.current(who, now) else { return vec![] };
    spaces::list(root).into_iter().filter(|s| policy.allows(&who, s, Level::Read, now)).map(|s| (s.clone(), spaces::space_dir(root, &s))).collect()
}

/// resources/list: every page the caller may read, a page of PAGE_SIZE at a time.
pub fn list(st: &State, ctx: &Ctx, params: &Value) -> RpcResult {
    let root = root(st)?;
    let start = match params.get("cursor").and_then(Value::as_str) {
        Some(c) => c.parse::<usize>().map_err(|_| (-32602, "Invalid cursor".to_string()))?,
        None => 0,
    };
    let mut all = vec![];
    for (space, dir) in readable(root, &ctx.principal) {
        for p in wiki::list_pages(&dir).unwrap_or_default() {
            let mut r = json!({ "uri": page_uri(&space, &p.slug), "name": format!("{space}/{}", p.slug), "title": p.title, "mimeType": "text/markdown" });
            if !p.summary.is_empty() {
                r["description"] = json!(p.summary);
            }
            if p.time > 0 {
                r["annotations"] = json!({ "lastModified": local_iso(&from_ms(p.time)) });
            }
            all.push(r);
        }
    }
    let page: Vec<Value> = all.iter().skip(start).take(PAGE_SIZE).cloned().collect();
    let mut out = json!({ "resources": page });
    if start + PAGE_SIZE < all.len() {
        out["nextCursor"] = json!((start + PAGE_SIZE).to_string());
    }
    Ok(out)
}

pub fn templates() -> Value {
    json!({ "resourceTemplates": [{
        "uriTemplate": format!("{SCHEME}{{kind}}/{{name}}/pages/{{slug}}"),
        "name": "page",
        "title": "A page in a space",
        "description": "A page of a space, as Markdown: kind is project, identity, actor or machine (machine/local is this machine's own).",
        "mimeType": "text/markdown",
    }] })
}

/// The page a URI names, if the caller may read its space: its folder and slug. A page that
/// doesn't exist and one the caller may not read get the same answer.
fn open(st: &State, ctx: &Ctx, uri: &str) -> Result<(std::path::PathBuf, String), (i64, String)> {
    let root = root(st)?;
    let not_found = || (NOT_FOUND, format!("Resource not found: {uri}"));
    let (space, slug) = parse_uri(uri).ok_or_else(not_found)?;
    let policy = access::load(&AppPaths::current());
    let now = now_ms();
    let who = policy.current(&ctx.principal, now).ok_or_else(not_found)?;
    if !policy.allows(&who, &space, Level::Read, now) || !spaces::exists(root, &space) {
        return Err(not_found());
    }
    Ok((spaces::space_dir(root, &space), slug))
}

/// resources/read: one page's Markdown.
pub fn read(st: &State, ctx: &Ctx, params: &Value) -> RpcResult {
    let uri = params.get("uri").and_then(Value::as_str).ok_or((-32602, "Missing uri".to_string()))?;
    let (dir, slug) = open(st, ctx, uri)?;
    match wiki::read_section(&dir, &slug, None) {
        Ok((rel, text)) if rel == format!("pages/{slug}.md") => Ok(json!({ "contents": [{ "uri": uri, "mimeType": "text/markdown", "text": defang_remote_images(&text) }] })),
        _ => Err((NOT_FOUND, format!("Resource not found: {uri}"))),
    }
}

// ---------------------------------------------------------------- subscriptions (stdio)

/// A connection's subscribed pages, and what it last saw of its spaces' page lists.
#[derive(Default)]
pub struct Subscriptions {
    pages: Mutex<BTreeMap<String, Option<(u64, i128)>>>,
    listing: Mutex<Option<String>>,
}

fn stamp(file: &Path) -> Option<(u64, i128)> {
    let m = std::fs::metadata(file).ok()?;
    let t = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as i128).unwrap_or(0);
    Some((m.len(), t))
}

impl Subscriptions {
    pub fn subscribe(&self, st: &State, ctx: &Ctx, params: &Value) -> RpcResult {
        let uri = params.get("uri").and_then(Value::as_str).ok_or((-32602, "Missing uri".to_string()))?;
        let (dir, slug) = open(st, ctx, uri)?;
        self.pages.lock().unwrap().insert(uri.to_string(), stamp(&dir.join("pages").join(format!("{slug}.md"))));
        Ok(json!({}))
    }

    pub fn unsubscribe(&self, params: &Value) -> RpcResult {
        let uri = params.get("uri").and_then(Value::as_str).ok_or((-32602, "Missing uri".to_string()))?;
        self.pages.lock().unwrap().remove(uri);
        Ok(json!({}))
    }

    /// The notifications due now: an updated page for each subscribed page that changed (and the
    /// caller may still read), and list_changed when the pages of its spaces came or went.
    pub fn poll(&self, st: &State, who: &Principal) -> Vec<Value> {
        let Ok(root) = root(st) else { return vec![] };
        let mut out = vec![];
        let spaces = readable(root, who);
        {
            let mut pages = self.pages.lock().unwrap();
            for (uri, seen) in pages.iter_mut() {
                let Some((space, slug)) = parse_uri(uri) else { continue };
                let Some((_, dir)) = spaces.iter().find(|(s, _)| *s == space) else { continue };
                let now = stamp(&dir.join("pages").join(format!("{slug}.md")));
                if now != *seen {
                    *seen = now;
                    out.push(json!({ "jsonrpc": "2.0", "method": "notifications/resources/updated", "params": { "uri": uri } }));
                }
            }
        }
        let listing: Vec<String> = spaces
            .iter()
            .flat_map(|(space, dir)| {
                let mut names: Vec<String> = std::fs::read_dir(dir.join("pages")).into_iter().flatten().flatten().map(|e| format!("{space}/{}", e.file_name().to_string_lossy())).collect();
                names.sort();
                names
            })
            .collect();
        let sig = listing.join("\n");
        let mut last = self.listing.lock().unwrap();
        if last.as_ref().is_some_and(|l| *l != sig) {
            out.push(json!({ "jsonrpc": "2.0", "method": "notifications/resources/list_changed" }));
        }
        *last = Some(sig);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_uris_round_trip_and_refuse_what_is_not_a_page() {
        let atlas = SpaceId::parse("project:atlas").unwrap();
        assert_eq!(page_uri(&atlas, "db-notes"), "xenolith://project/atlas/pages/db-notes");
        assert_eq!(parse_uri("xenolith://project/atlas/pages/db-notes"), Some((atlas, "db-notes".into())));
        assert_eq!(parse_uri("xenolith://machine/local/pages/home-lab").map(|(s, _)| s.is_local()), Some(true));
        for bad in [
            "xenolith://project/atlas/pages/../x",
            "xenolith://project/atlas/pages/Upper",
            "xenolith://project/atlas/log/2026-10-01",
            "xenolith://project/Atlas/pages/x",
            "xenolith://app/codex/pages/x",
            "xenolith://project/atlas/pages/x/y",
            "file:///etc/passwd",
            "xenolith://project/atlas/pages/",
        ] {
            assert_eq!(parse_uri(bad), None, "{bad}");
        }
    }
}
