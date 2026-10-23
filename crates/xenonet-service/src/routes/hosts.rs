//! Hosts over the API (server/routes/hosts.js; docs/design/hosts.md). Being
//! a host needs no flag (GET and POST /api/hosts/self); hosts over SSH are
//! behind XENONET_HOSTS=1, without which /api/hosts answers 404. A remote
//! session's transcript and stats, and a collision on one host, are read
//! through its relay (over SSH or the network), and only by XenoNet's own
//! window: with the window key, never an agent's or an actor's request,
//! so a session here can't read another machine's transcripts. Every other
//! route that takes an id refuses remote ones, since hosts are read-only.
//! `route` is None when it isn't one of these.

use crate::cores::{ok, reason, CoreError, HostsConfig, Hub, Verify};
use crate::ctx::pending_reply;
use crate::http::{decode_uri_component, Reply, Req};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::sync::Arc;
use xenonet_base::ids::{is_host_id, parse_qualified};

const OFF: &str = "hosts are off in this XenoNet: start it with XENONET_HOSTS=1 to show other machines' XenoNets (a prototype)";

/// The refusal of another machine's transcript, stats or collision to
/// anything but the window.
pub fn window_only_reply() -> Reply {
    Reply::send(403, &json!({ "error": "another machine's transcripts, stats and diffs open only in XenoNet's own window", "code": "window" }), &[])
}

pub struct HostRoutes<'a> {
    pub hosts: Arc<dyn HostsConfig>,
    pub hub: Option<Arc<dyn Hub>>,
    pub verify: Arc<dyn Verify>,
    pub home: &'a str,
    /// Whether the request is the window's (`Ctx::window_only`): its key,
    /// and no token.
    pub window: bool,
}

// ^(?:claude|codex):[0-9A-Za-z][0-9A-Za-z._-]{0,127}$
fn is_session(local: &str) -> bool {
    let Some(rest) = local.strip_prefix("claude:").or_else(|| local.strip_prefix("codex:")) else { return false };
    let b = rest.as_bytes();
    (1..=128).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn remote_session(id: Option<&str>) -> Option<(String, String)> {
    let (host, local) = parse_qualified(id?)?;
    let host = host?;
    is_session(local).then(|| (host.to_string(), local.to_string()))
}

fn limit_ok(v: &str) -> bool {
    (1..=4).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit())
}

fn cursor_ok(v: &str) -> bool {
    (1..=200).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// `{ ...a, ...b }` for objects.
fn merged(a: Value, b: Value) -> Value {
    let mut out = a;
    if let (Value::Object(o), Value::Object(more)) = (&mut out, b) {
        for (k, v) in more {
            o.insert(k, v);
        }
    }
    out
}

/// URLSearchParams.toString() of these pairs.
fn search(pairs: &[(&str, &str)]) -> String {
    let enc = |s: &str| {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(b as char),
                b' ' => out.push('+'),
                _ => {
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
        out
    };
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

pub fn url_search(pairs: &[(&str, &str)]) -> String {
    search(pairs)
}

impl HostRoutes<'_> {
    async fn yours(&self, req: &Req) -> Option<Reply> {
        let you = self.verify.person(req.peer).await;
        (!ok(&you)).then(|| Reply::error(403, reason(&you)))
    }

    pub async fn route(&self, req: &mut Req) -> Option<Reply> {
        match self.inner(req).await {
            Ok(r) => r,
            Err(e) if e.is_pending() => Some(pending_reply(&e)),
            Err(e) => Some(Reply::error(e.status.unwrap_or(400), e.message)),
        }
    }

    async fn inner(&self, req: &mut Req) -> Result<Option<Reply>, CoreError> {
        let p = req.path.clone();
        if p == "/api/hosts/self" {
            if req.method == "GET" {
                return Ok(Some(Reply::json(200, merged(self.hosts.self_info()?, json!({ "home": self.home })))));
            }
            if req.method == "POST" {
                if let Some(r) = self.yours(req).await {
                    return Ok(Some(r));
                }
                let body = req.json_body().await?;
                let Some(id) = body.get("id") else {
                    return Ok(Some(Reply::error(400, "say what to call this XenoNet: { id }, or id null to take its name away")));
                };
                return Ok(Some(Reply::json(200, merged(self.hosts.set_self(id)?, json!({ "home": self.home })))));
            }
            return Ok(Some(Reply::error(405, "GET or POST")));
        }

        // A remote session's transcript or stats, through its host's relay.
        if let Some(rest) = p.strip_prefix("/api/session/") {
            if let Some((raw, what)) = rest.split_once('/') {
                if !raw.is_empty() && (what == "transcript" || what == "stats") && req.method == "GET" {
                    let Some(id) = decode_uri_component(raw) else { return Ok(None) };
                    let Some(hub) = self.hub.as_ref().filter(|_| id.starts_with('@')) else { return Ok(None) };
                    if !self.window {
                        return Ok(Some(window_only_reply()));
                    }
                    let Some((host, local)) = remote_session(Some(&id)).filter(|(h, _)| hub.has(h)) else {
                        return Ok(Some(Reply::error(404, "XenoNet doesn't know that session")));
                    };
                    let mut params: Vec<(&str, String)> = Vec::new();
                    for (k, test) in [("limit", limit_ok as fn(&str) -> bool), ("cursor", cursor_ok)] {
                        if let Some(v) = req.param(k) {
                            if what == "transcript" {
                                if !test(v) {
                                    return Ok(Some(Reply::error(400, format!("{k} isn't one XenoNet reads"))));
                                }
                                params.push((k, v.to_string()));
                            }
                        }
                    }
                    let pairs: Vec<(&str, &str)> = params.iter().map(|(k, v)| (*k, v.as_str())).collect();
                    let q = if pairs.is_empty() { String::new() } else { format!("?{}", search(&pairs)) };
                    let r = hub.get(&host, &format!("/api/session/{local}/{what}{q}")).await?;
                    let body = if r.body.is_object() {
                        let mut b = r.body.clone();
                        if b.get("id").is_some_and(Value::is_string) {
                            b["id"] = json!(id);
                        }
                        b["host"] = json!(host);
                        b
                    } else {
                        r.body
                    };
                    return Ok(Some(Reply::json(r.status, body)));
                }
            }
        }

        // A collision on one host, with both sides' diffs, through its relay.
        if p == "/api/collision" && req.method == "GET" {
            if let Some(hub) = self.hub.as_ref() {
                let (qa, qb) = (req.param("a").map(str::to_string), req.param("b").map(str::to_string));
                if [&qa, &qb].iter().any(|x| x.as_deref().is_some_and(|s| s.starts_with('@'))) {
                    if !self.window {
                        return Ok(Some(window_only_reply()));
                    }
                    let (a, b) = (remote_session(qa.as_deref()), remote_session(qb.as_deref()));
                    let (Some(a), Some(b)) = (a, b) else {
                        return Ok(Some(Reply::error(400, "collisions across machines come later: both sessions have to be on one host")));
                    };
                    if a.0 != b.0 {
                        return Ok(Some(Reply::error(400, "collisions across machines come later: both sessions have to be on one host")));
                    }
                    if !hub.has(&a.0) {
                        return Ok(Some(Reply::error(404, format!("there's no host called {}", a.0))));
                    }
                    let r = hub.get(&a.0, &format!("/api/collision?a={}&b={}", a.1, b.1)).await?;
                    let body = if r.body.is_object() {
                        let mut out = r.body.clone();
                        out["a"] = merged(r.body.get("a").cloned().unwrap_or(json!({})), json!({ "id": qa }));
                        out["b"] = merged(r.body.get("b").cloned().unwrap_or(json!({})), json!({ "id": qb }));
                        out["host"] = json!(a.0);
                        out
                    } else {
                        r.body
                    };
                    return Ok(Some(Reply::json(r.status, body)));
                }
            }
        }

        if p != "/api/hosts" && !p.starts_with("/api/hosts/") {
            return Ok(None);
        }
        let Some(hub) = self.hub.as_ref().filter(|h| h.ssh()) else { return Ok(Some(Reply::error(404, OFF))) };
        if p == "/api/hosts" {
            if req.method == "GET" {
                return Ok(Some(Reply::json(200, json!({ "self": self.hosts.self_info()?, "sshConfig": self.hosts.ssh_config()?, "hosts": hub.list()? }))));
            }
            if req.method == "POST" {
                if let Some(r) = self.yours(req).await {
                    return Ok(Some(r));
                }
                let body = req.json_body().await?;
                let replace = body.get("replace") == Some(&Value::Bool(true));
                return Ok(Some(Reply::json(200, hub.add(&body, replace)?)));
            }
            return Ok(Some(Reply::error(405, "GET or POST")));
        }
        let rest = &p["/api/hosts/".len()..];
        let (id, action) = match rest.split_once('/') {
            Some((id, a @ ("reconnect" | "test"))) => (id, Some(a)),
            Some(_) => return Ok(Some(Reply::error(404, "not found"))),
            None => (rest, None),
        };
        if id.is_empty() || !is_host_id(id) {
            return Ok(Some(Reply::error(404, "not found")));
        }
        match (action, req.method.as_str()) {
            (None, "GET") => Ok(Some(Reply::json(200, hub.detail(id)?))),
            (None, "DELETE") => {
                if let Some(r) = self.yours(req).await {
                    return Ok(Some(r));
                }
                Ok(Some(Reply::json(200, hub.remove(id, req.param("forget") == Some("1"))?)))
            }
            (Some("reconnect"), "POST") => Ok(Some(Reply::json(200, hub.reconnect(id)?))),
            (Some("test"), "POST") => {
                if !hub.has(id) {
                    return Ok(Some(Reply::error(404, format!("there's no host called {id}"))));
                }
                Ok(Some(Reply::json(200, hub.test(id).await?)))
            }
            _ => Ok(Some(Reply::error(405, "not that way"))),
        }
    }
}
