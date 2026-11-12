//! What an MCP server on the map is (server/routes/mcp.js):
//! `GET /api/mcp?server=<name>`, from this machine's sessions. The demo's
//! are made up.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
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
    // Another machine's MCP servers stay on it: the window reads only its
    // sessions' transcripts and stats (section 7).
    if req.param("host").is_some_and(|h| !h.is_empty()) {
        return Ok(Reply::error(404, "another machine's MCP servers are read on that machine"));
    }
    let state = &ctx.cores.state;
    let nodes = state.nodes()?;
    let known = nodes.iter().any(|n| {
        (n["kind"] == "resource" && n.get("server").unwrap_or(&n["name"]) == &json!(server))
            || (n["kind"] == "wiki" && server == n.get("server").and_then(Value::as_str).unwrap_or("agent-wiki"))
    });
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
