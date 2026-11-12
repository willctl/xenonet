//! Reads of another machine of the network (docs/design/network.md,
//! section 7): a remote session's transcript and stats, over svc/2 under
//! the window's key, only by XenoNet's own window (rules.rs: those rows
//! are the window's when they name another machine), so a session here
//! can't read another machine's transcripts.
//! Every other route that takes an id refuses remote ones, since other
//! machines' maps are read-only here. `route` is None when it isn't one of
//! these.

use crate::cores::CoreError;
use crate::ctx::pending_reply;
use crate::http::{decode_uri_component, Reply, Req};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::sync::Arc;
use xenonet_model::ids::{parse_qualified, Qualified};

/// The refusal of another machine's transcript, stats or collision to
/// anything but the window.
pub fn window_only_reply() -> Reply {
    Reply::send(403, &json!({ "error": "another machine's transcripts, stats and diffs open only in XenoNet's own window", "code": "window" }), &[])
}

pub struct HostRoutes {
    pub net: Option<Arc<dyn crate::parts::Network>>,
}

// ^(?:claude|codex):[0-9A-Za-z][0-9A-Za-z._-]{0,127}$
fn is_session(local: &str) -> bool {
    let Some(rest) = local.strip_prefix("claude:").or_else(|| local.strip_prefix("codex:")) else { return false };
    let b = rest.as_bytes();
    (1..=128).contains(&b.len()) && b[0].is_ascii_alphanumeric() && b.iter().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

fn remote_session(id: Option<&str>) -> Option<(String, String)> {
    let Qualified { host, local } = parse_qualified(id?)?;
    let host = host?;
    is_session(local).then(|| (host.to_string(), local.to_string()))
}

fn limit_ok(v: &str) -> bool {
    (1..=4).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_digit())
}

fn cursor_ok(v: &str) -> bool {
    (1..=200).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
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

impl HostRoutes {
    pub async fn route(&self, req: &mut Req) -> Option<Reply> {
        match self.inner(req).await {
            Ok(r) => r,
            Err(e) if e.is_pending() => Some(pending_reply(&e)),
            Err(e) => Some(Reply::error(e.status.unwrap_or(400), e.message)),
        }
    }

    async fn inner(&self, req: &mut Req) -> Result<Option<Reply>, CoreError> {
        let p = req.path.clone();
        // A remote session's transcript or stats, over svc/2.
        if let Some(rest) = p.strip_prefix("/api/session/") {
            if let Some((raw, what)) = rest.split_once('/') {
                if !raw.is_empty() && (what == "transcript" || what == "stats") && req.method == "GET" {
                    let Some(id) = decode_uri_component(raw) else { return Ok(None) };
                    if !id.starts_with('@') {
                        return Ok(None);
                    }
                    let Some((host, local)) = remote_session(Some(&id)) else {
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
                    let path = format!("/api/session/{local}/{what}{q}");
                    let read = match &self.net {
                        Some(net) => net.window_read(&host, &path).await,
                        None => None,
                    };
                    let Some((status, body)) = read else {
                        return Ok(Some(Reply::error(503, format!("this machine's endpoint isn't running, so {host}'s sessions can't be read from here"))));
                    };
                    let body = if body.is_object() {
                        let mut b = body;
                        if b.get("id").is_some_and(Value::is_string) {
                            b["id"] = json!(id);
                        }
                        b["host"] = json!(host);
                        b
                    } else {
                        body
                    };
                    return Ok(Some(Reply::json(status, body)));
                }
            }
        }

        Ok(None)
    }
}
