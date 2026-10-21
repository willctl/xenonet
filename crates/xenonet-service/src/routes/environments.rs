//! The place around the map (server/routes/environments.js): GET
//! /api/environment, and POST to change it, which is yours.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use xenonet_app::environments::{environment_of, ENVIRONMENTS};

pub async fn route(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    if req.path != "/api/environment" {
        return Ok(None);
    }
    inner(ctx, req).await.map(Some)
}

// ENVIRONMENTS as core/environments.js exports it.
fn environments() -> Value {
    Value::Array(ENVIRONMENTS.iter().map(|e| json!({ "id": e.id, "name": e.name, "about": e.about, "shape": e.shape, "index": e.index })).collect())
}

fn is_environment(id: &Value) -> bool {
    id.as_str().is_some_and(|id| ENVIRONMENTS.iter().any(|e| e.id == id))
}

async fn inner(ctx: &Ctx, req: &mut Req) -> Result<Reply, CoreError> {
    let store = &ctx.cores.store;
    match req.method.as_str() {
        "GET" => Ok(Reply::json(200, json!({ "current": environment_of(store.environment()?.as_str()).id, "environments": environments() }))),
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
