//! Provider refusals XenoNet kept (server/routes/refusals.js), yours only:
//! the list with its filters, whether their context is kept, and one's
//! saved request.

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
use crate::routes::integrations::params_object;
use serde_json::{json, Value};
use std::path::Path;

// ^\/api\/refusals\/\d{8}-[0-9a-f]{8}\/request$
fn request_id(route: &str) -> Option<&str> {
    let id = route.strip_prefix("/api/refusals/")?.strip_suffix("/request")?;
    let (day, hex) = id.split_once('-')?;
    (day.len() == 8 && day.bytes().all(|b| b.is_ascii_digit()) && hex.len() == 8 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))).then_some(id)
}

pub async fn route(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    let route = req.path.clone();
    if route != "/api/refusals" && !route.starts_with("/api/refusals/") {
        return Ok(None);
    }
    if let Some(r) = ctx.yours(req).await {
        return Ok(Some(r));
    }
    let refusals = &ctx.cores.refusals;
    let demo = if ctx.config.demo { Some(refusals.demo_records()?) } else { None };
    if route == "/api/refusals/context" && req.method == "POST" {
        let on = req.json(1024).await?.get("on") == Some(&Value::Bool(true));
        return Ok(Some(Reply::json(200, json!({ "context": refusals.set_context(on)? }))));
    }
    if route == "/api/refusals" && req.method == "GET" {
        let records = match &demo {
            Some(d) => d.clone(),
            None => refusals.list()?,
        };
        let mut out = json!({ "context": refusals.context_on()? });
        if let Value::Object(q) = refusals.query(&records, &params_object(&req.query))? {
            for (k, v) in q {
                out[k] = v;
            }
        }
        return Ok(Some(Reply::json(200, out)));
    }
    if let (Some(id), "GET") = (request_id(&route), req.method.as_str()) {
        let record = match &demo {
            Some(d) => d.as_array().and_then(|l| l.iter().find(|r| r["id"] == json!(id)).cloned()),
            None => refusals.get(id)?,
        };
        let Some(record) = record.filter(|r| xenonet_base::text::truthy(r.get("context"))) else {
            return Ok(Some(Reply::error(404, "No saved request for this record.")));
        };
        if demo.is_some() {
            return Ok(Some(Reply::json(
                200,
                json!({ "request": { "provider": record["provider"], "model": record["signal"]["model"], "messages": [{ "role": "user", "content": "A made-up demo request." }] } }),
            )));
        }
        // Fixed paths, never a filename a record or caller supplied.
        let file = Path::new(&refusals.dir()).join(id).join("request.json");
        let read = || -> Option<Result<Value, ()>> {
            let meta = std::fs::symlink_metadata(&file).ok()?;
            if !meta.is_file() {
                return None;
            }
            if std::fs::metadata(&file).ok()?.len() > 1024 * 1024 {
                return Some(Err(()));
            }
            let text = std::fs::read_to_string(&file).ok()?;
            xenonet_base::json::parse(&text).ok().map(Ok)
        };
        return Ok(Some(match read() {
            Some(Ok(request)) => Reply::json(200, json!({ "request": request })),
            Some(Err(())) => Reply::error(413, "This request is too large for the panel. Use xenonet refusals show --context or export --with-context."),
            None => Reply::error(404, "The saved request is missing or unreadable."),
        }));
    }
    Ok(Some(Reply::error(404, "No such refusal route.")))
}
