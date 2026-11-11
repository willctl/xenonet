//! The projects folder XenoNet reads (server/routes/workspace.js): GET,
//! POST and DELETE /api/workspace/root, and POST /api/workspace/check.
//! Changing the folder is yours, as connecting agents is; looking and
//! checking are any local surface's (rules.rs).

use crate::cores::CoreError;
use crate::ctx::{pending_reply, Ctx};
use crate::http::{Reply, Req};
use serde_json::Value;

pub async fn route(ctx: &Ctx, req: &mut Req) -> Reply {
    match inner(ctx, req).await {
        Ok(r) => r,
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) => Reply::error(e.status.unwrap_or(400), e.message),
    }
}

async fn inner(ctx: &Ctx, req: &mut Req) -> Result<Reply, CoreError> {
    let state = &ctx.cores.state;
    if req.path == "/api/workspace/root" {
        if req.method == "GET" {
            return Ok(Reply::json(200, state.root_info()?));
        }
        if req.method == "DELETE" {
            return Ok(Reply::json(200, state.set_root(Value::Null).await?));
        }
        if req.method == "POST" {
            let body = req.json_body().await?;
            let Some(path) = body.get("path") else {
                return Ok(Reply::error(400, "say which folder: { path }, or path null for the default"));
            };
            return Ok(Reply::json(200, state.set_root(path.clone()).await?));
        }
    }
    if req.method == "POST" && req.path == "/api/workspace/check" {
        let path = req.json_body().await?.get("path").cloned().unwrap_or(Value::Null);
        return Ok(Reply::json(200, state.check_root(path).await?));
    }
    Ok(Reply::error(404, "not found"))
}
