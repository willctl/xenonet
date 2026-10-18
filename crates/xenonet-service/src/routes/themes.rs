//! Themes by id (server/routes/themes.js): `GET /api/themes/<id>` returns one
//! whole, as the console draws it; `POST /api/themes/reveal {id}` shows a
//! theme of yours in the file manager (not in the demo). The list, the one
//! in use, switching and copying are server/index.js's (routes/mod.rs).

use crate::cores::CoreError;
use crate::ctx::Ctx;
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::path::Path;

/// `^[a-z0-9][a-z0-9-]{0,40}$`
pub fn theme_id(id: &str) -> bool {
    let b = id.as_bytes();
    (1..=41).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// None when it isn't one of these routes; what it throws goes to the
/// service's own handler, as in JS.
pub async fn route(ctx: &Ctx, req: &mut Req) -> Result<Option<Reply>, CoreError> {
    if req.method == "POST" && req.path == "/api/themes/reveal" {
        // Only a file in the themes folder, named by its id: never a path
        // from the page.
        let id = req.json_body().await.unwrap_or_else(|_| json!({})).get("id").cloned().unwrap_or(Value::Null);
        let shown = match &id {
            Value::Null => "undefined".to_string(),
            v => xenonet_base::text::js_string(v),
        };
        let file = id.as_str().filter(|i| theme_id(i)).map(|i| Path::new(&ctx.themes_dir).join(format!("{i}.json")));
        let Some(file) = file.filter(|f| f.exists()) else {
            return Ok(Some(Reply::error(404, format!("there's no theme file called {shown}"))));
        };
        let file = file.to_string_lossy().into_owned();
        if !ctx.config.demo {
            ctx.cores.opener.reveal(&file)?;
        }
        return Ok(Some(Reply::json(200, json!({ "ok": true, "file": file, "shown": !ctx.config.demo }))));
    }
    let Some(id) = req.path.strip_prefix("/api/themes/").filter(|i| theme_id(i)) else { return Ok(None) };
    if req.method != "GET" {
        return Ok(None);
    }
    let read = ctx.cores.themes.read(&ctx.themes_dir)?;
    Ok(Some(match read.get("themes").and_then(|t| t.get(id)) {
        Some(theme) => Reply::json(200, theme.clone()),
        None => Reply::error(404, format!("there's no theme called {id}")),
    }))
}
