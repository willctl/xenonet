//! What an MCP server on the map is (server/routes/mcp.js):
//! `GET /api/mcp?server=<name>`, from this machine's sessions, or with
//! `?host=` from another host's catalog. The demo's are made up.

use crate::cores::CoreError;
use crate::ctx::{pending_reply, Ctx};
use crate::http::{Reply, Req};
use crate::routes::hosts::url_search;
use serde_json::{json, Value};

pub async fn route(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    if req.method != "GET" || req.path != "/api/mcp" {
        return Ok(None);
    }
    inner(ctx, req).await.map(Some)
}

async fn inner(ctx: &Ctx, req: &mut Req) -> Result<Reply, CoreError> {
    let server = req.param("server").unwrap_or("").to_string();
    if server.is_empty() || server.encode_utf16().count() > 160 || server.chars().any(|c| (c as u32) < 0x20) {
        return Ok(Reply::error(400, "say which MCP server to inspect"));
    }
    if let Some(host) = req.param("host").filter(|h| !h.is_empty()) {
        let Some(hub) = &ctx.cores.hub else { return Ok(Reply::error(404, "that host isn't on this map")) };
        // Another machine's reads are the window's alone.
        if !ctx.window_only(req) {
            return Ok(crate::routes::hosts::window_only_reply());
        }
        return Ok(match hub.get(host, &format!("/api/mcp?{}", url_search(&[("server", &server)]))).await {
            Ok(r) => Reply::json(r.status, r.body),
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(_) => Reply::error(503, "this host's MCP catalog is unavailable"),
        });
    }
    let state = &ctx.cores.state;
    let nodes = state.nodes()?;
    let known = nodes
        .iter()
        .any(|n| (n["kind"] == "resource" && n["name"] == json!(server)) || (n["kind"] == "wiki" && server == n.get("server").and_then(Value::as_str).unwrap_or("agent-wiki")));
    if !known {
        return Ok(Reply::error(404, "that MCP server isn't on this map"));
    }
    let live = state.live_snapshot()?;
    let sessions: Vec<Value> = live["sessions"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut s| {
            let node = s["id"].as_str().and_then(|id| state.node(id).ok().flatten());
            s["cwd"] = node.as_ref().and_then(|n| n.get("cwd").cloned()).unwrap_or(Value::Null);
            s["title"] = node.as_ref().and_then(|n| n.get("title").cloned()).unwrap_or(Value::Null);
            s
        })
        .collect();
    let info = if ctx.config.demo {
        let mut info = ctx.cores.mcp_info.demo(&server)?;
        info["sessions"] = Value::Array(
            sessions
                .iter()
                .filter(|s| s["mcp"].as_array().is_some_and(|m| m.iter().any(|m| m["server"] == json!(server))))
                .map(|s| json!({ "id": s["id"], "app": s["app"], "title": s["title"], "status": s["status"] }))
                .collect(),
        );
        info
    } else {
        ctx.cores.mcp_info.get(&server, &Value::Array(sessions), req.param("refresh") == Some("1")).await?
    };
    Ok(Reply::json(200, info))
}
