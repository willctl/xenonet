//! The place around the map, chosen apart from its theme
//! (server/routes/environments.js): GET /api/environment, and POST to
//! change it, which is yours.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
use serde_json::{json, Value};

pub struct Environment {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub shape: &'static str,
    pub index: u32,
}

pub const DEFAULT_ENVIRONMENT: &str = "nebula";

pub const ENVIRONMENTS: [Environment; 4] = [
    Environment { id: "nebula", name: "Nebula", about: "The original drifting clouds and star field.", shape: "clouds", index: 0 },
    Environment { id: "aurora", name: "Aurora", about: "Slow curtains of light across a quiet atmospheric horizon.", shape: "curtains", index: 1 },
    Environment { id: "trench", name: "Trench", about: "A deep-sea trench with rippling caustics and distant light shafts.", shape: "water", index: 2 },
    Environment { id: "vault", name: "Vault", about: "A monumental hall of repeating arches and illuminated ribs.", shape: "arches", index: 3 },
];

/// The place with this id, or the first one.
pub fn environment_of(id: Option<&str>) -> &'static Environment {
    id.and_then(|id| ENVIRONMENTS.iter().find(|e| e.id == id)).unwrap_or(&ENVIRONMENTS[0])
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_place_or_the_first() {
        assert_eq!(environment_of(Some("vault")).name, "Vault");
        assert_eq!(environment_of(Some("nowhere")).id, DEFAULT_ENVIRONMENT);
        assert_eq!(environment_of(None).id, "nebula");
        assert!(ENVIRONMENTS.iter().enumerate().all(|(i, e)| e.index as usize == i));
    }
}
