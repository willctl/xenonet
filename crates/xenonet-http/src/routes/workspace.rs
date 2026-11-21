//! Where XenoNet looks for projects: the projects folder (GET, POST and
//! DELETE /api/workspace/root, and POST /api/workspace/check), and the
//! other roots you named beside it (GET and PUT /api/workspace/roots).
//! Changing them is yours, as connecting agents is; looking and checking
//! are any local surface's (rules.rs).

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
    if req.path == "/api/workspace/roots" {
        if req.method == "GET" {
            return Ok(Reply::json(200, state.roots()?));
        }
        if req.method == "PUT" {
            let body = req.json_body().await?;
            let Some(roots) = body.get("roots").filter(|r| r.is_array()) else {
                return Ok(Reply::error(400, "say which folders: { roots: [path] }"));
            };
            return Ok(Reply::json(200, state.set_roots(roots.clone()).await?));
        }
    }
    if req.path == "/api/workspace/app-roots" {
        if req.method == "GET" {
            return Ok(Reply::json(200, state.app_roots()?));
        }
        if req.method == "PUT" {
            let body = req.json_body().await?;
            let (Some(app), Some(roots)) = (body.get("app").filter(|a| a.is_string()), body.get("roots").filter(|r| r.is_array())) else {
                return Ok(Reply::error(400, "say which app and its folders: { app, roots: [path] }"));
            };
            // The folders she had named for the app, to tell which this takes away.
            let named = |v: &Value| -> Vec<String> {
                let of = v.get("apps").and_then(Value::as_array).into_iter().flatten().find(|a| a.get("app") == Some(app));
                of.and_then(|a| a.get("roots"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|r| r.get("named") == Some(&Value::Bool(true)))
                    .filter_map(|r| r.get("root")?.as_str().map(str::to_string))
                    .collect()
            };
            let before = named(&state.app_roots()?);
            let mut after = state.set_app_roots(app.clone(), roots.clone()).await?;
            // A folder taken away keeps no hooks of XenoNet's: they would go on
            // reporting its sessions, and XenoNet would no longer read it.
            let kept = named(&after);
            let mut taken = Vec::new();
            for root in before.iter().filter(|r| !kept.contains(r)) {
                taken.push(match ctx.cores.connect.remove_root_hooks(app.as_str().unwrap_or_default(), root).await {
                    Ok(done) => done,
                    Err(e) if e.is_pending() => continue,
                    Err(e) => serde_json::json!({ "app": app, "root": root, "hooks": "kept", "why": e.message }),
                });
            }
            if !taken.is_empty() {
                after["hooks"] = Value::Array(taken);
            }
            return Ok(Reply::json(200, after));
        }
    }
    if req.method == "POST" && req.path == "/api/workspace/check" {
        let path = req.json_body().await?.get("path").cloned().unwrap_or(Value::Null);
        return Ok(Reply::json(200, state.check_root(path).await?));
    }
    Ok(Reply::error(404, "not found"))
}
