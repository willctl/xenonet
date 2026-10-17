//! Drift in the other apps' data XenoNet reads (server/routes/drift.js):
//! GET /api/drift, the drift report (?all=1 lists every record kept), and
//! POST /api/drift/seen { at }, you've looked at the report made then.
//! Marking it seen quiets the doctor's contracts.drift check, so the
//! doctor runs again, a fresh run.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;

pub async fn route(ctx: &Arc<Ctx>, req: &mut Req) -> Result<Reply, CoreError> {
    if req.method == "GET" && req.path == "/api/drift" {
        let mut log = ctx.drift.lock().unwrap();
        return Ok(Reply::json(200, ctx.cores.drift.report(&mut log, req.param("all") == Some("1"), &ctx.config.home, ctx.now())?));
    }
    if req.method == "POST" && req.path == "/api/drift/seen" {
        let body = req.json_body().await?;
        let at = body.get("at").filter(|a| !a.is_null());
        if let Some(at) = at {
            if !at.as_f64().is_some_and(f64::is_finite) {
                return Ok(Reply::error(400, "at is when the report you looked at was made, in epoch milliseconds"));
            }
        }
        // Never past now: drift that turns up later is still new.
        let now = ctx.now();
        let looked = {
            let mut log = ctx.drift.lock().unwrap();
            log.looked(at.and_then(Value::as_f64).unwrap_or(now).min(now));
            log.flush();
            log.view().1
        };
        let c = ctx.clone();
        tokio::spawn(async move {
            let _ = c.cores.doctor.check(true, false).await;
        });
        return Ok(Reply::json(200, json!({ "lookedAt": looked })));
    }
    Ok(Reply::error(404, "not found"))
}
