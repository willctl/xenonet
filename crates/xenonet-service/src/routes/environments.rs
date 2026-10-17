//! The place around the map (server/routes/environments.js): GET
//! /api/environment, and POST to change it, which is yours.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::environments::{environment_of, environments, is_environment};
use crate::http::{Reply, Req};
use serde_json::{json, Value};

pub async fn route(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    if req.path != "/api/environment" {
        return Ok(None);
    }
    inner(ctx, req).await.map(Some)
}

async fn inner(ctx: &Ctx, req: &mut Req) -> Result<Reply, CoreError> {
    let store = &ctx.cores.store;
    match req.method.as_str() {
        "GET" => Ok(Reply::json(200, json!({ "current": environment_of(&store.environment()?), "environments": environments() }))),
        "POST" => {
            if let Some(r) = ctx.yours(req).await {
                return Ok(r);
            }
            let id = req.json_body().await?.get("id").cloned().unwrap_or(Value::Null);
            if !is_environment(&id) {
                return Ok(Reply::error(404, "that environment isn't available"));
            }
            store.set_environment(id.as_str().unwrap_or(""))?;
            ctx.broadcast("environment", &json!({ "id": id }));
            Ok(Reply::json(200, json!({ "id": id })))
        }
        _ => Ok(Reply::error(405, "use GET or POST")),
    }
}
