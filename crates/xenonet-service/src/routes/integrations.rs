//! Integrations (server/routes/integrations.js): yours to read, the owner's
//! (the window's, with its process) to change. Each route goes to the
//! integration that answers it; their own errors keep their status and
//! code, a body that's too big or isn't JSON is a bad request, and anything
//! else is the integration failing.

use crate::cores::{ok, reason, Kind};
use crate::ctx::{pending_reply, Ctx};
use crate::http::{Reply, Req};
use serde_json::{json, Value};

// The routes: method, path, the integrations' method it calls (as
// integrations.<name> in JS), the body's limit (None for a GET, which takes
// the query), and the arguments it passes, from the body or the query.
type Args = fn(&Value) -> Vec<Value>;
const ROUTES: [(&str, &str, &str, Option<usize>, Args); 17] = [
    ("GET", "", "status", None, |_| vec![]),
    ("GET", "/history", "history", None, |q| vec![q.clone()]),
    ("GET", "/pi", "pi", None, |q| vec![q.clone()]),
    ("GET", "/sandbox/review", "shell.review", None, |q| vec![field(q, "id")]),
    ("POST", "/route/preview", "routing.preview", Some(8192), |b| vec![b.clone()]),
    ("POST", "/route/send", "routing.route", Some(1024), |b| vec![field(b, "ticket")]),
    ("POST", "/route/outcome", "routing.outcome", Some(1024), |b| vec![field(b, "id"), field(b, "outcome")]),
    ("POST", "/sandbox/check", "shell.status", Some(1024), |b| vec![b.clone()]),
    ("POST", "/sandbox/preview", "shell.preview", Some(8192), |b| vec![b.clone()]),
    ("POST", "/sandbox/launch", "shell.launch", Some(1024), |b| vec![field(b, "ticket")]),
    ("POST", "/sandbox/inspect", "shell.inspect", Some(1024), |b| vec![field(b, "id")]),
    ("POST", "/sandbox/retrieve", "shell.retrieve", Some(1024), |b| vec![field(b, "id")]),
    ("POST", "/sandbox/export", "shell.exportReview", Some(1024), |b| vec![field(b, "id"), field(b, "revision")]),
    ("POST", "/sandbox/forget-review", "shell.forgetReview", Some(1024), |b| vec![field(b, "id")]),
    ("POST", "/sandbox/delete", "shell.remove", Some(1024), |b| vec![field(b, "id")]),
    ("POST", "/pi/enable", "enablePi", Some(1024), |b| vec![field(b, "on")]),
    ("POST", "/pi/import", "importPi", Some(1024 * 1024), |b| vec![b.clone()]),
];

/// `body?.k`: undefined (null here) for a body that isn't an object.
fn field(v: &Value, k: &str) -> Value {
    v.get(k).cloned().unwrap_or(Value::Null)
}

async fn owner(ctx: &Ctx, req: &Req) -> Option<Reply> {
    if !ctx.window_key.matches(req.header("x-xenonet-window")) {
        return Some(Reply::coded(403, "yours", "Use XenoNet's own window to send decisions or launch jobs."));
    }
    let seen = ctx.cores.verify.window(req.peer).await;
    (!ok(&seen)).then(|| Reply::coded(403, "yours", reason(&seen)))
}

pub async fn route(ctx: &Ctx, req: &mut Req) -> Option<Reply> {
    if req.path != "/api/integrations" && !req.path.starts_with("/api/integrations/") {
        return None;
    }
    if req.header("authorization").is_some() {
        return Some(Reply::coded(403, "yours", "Integration data belongs to the owner."));
    }
    if let Some(r) = ctx.yours(req).await {
        return Some(r);
    }
    if req.method != "GET" {
        if let Some(r) = owner(ctx, req).await {
            return Some(r);
        }
    }
    let route = req.path["/api/integrations".len()..].to_string();
    let Some((_, _, name, limit, args)) = ROUTES.iter().find(|(m, r, ..)| *m == req.method && *r == route) else {
        return Some(Reply::error(404, "No such integration route."));
    };
    let input = match limit {
        None => params_object(&req.query),
        Some(n) => match req.json(*n).await {
            Ok(b) => b,
            Err(e) => return Some(Reply::send(e.status, &json!({ "error": e.message, "code": "bad-request" }), &[])),
        },
    };
    Some(match ctx.cores.integrations.call(name, args(&input)).await {
        Ok(v) => Reply::json(200, v),
        Err(e) if e.is_pending() => pending_reply(&e),
        Err(e) if e.is(Kind::Integration) => Reply::send(e.status.unwrap_or(500), &json!({ "error": e.message, "code": e.code }), &[]),
        Err(_) => Reply::coded(502, "integration-failed", "The integration could not complete this operation. Check its configuration and gateway, then refresh."),
    })
}

/// `Object.fromEntries(url.searchParams)`: the last value of each key, in
/// the order the keys first came.
pub fn params_object(query: &[(String, String)]) -> Value {
    let mut out = serde_json::Map::new();
    for (k, v) in query {
        out.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(out)
}
