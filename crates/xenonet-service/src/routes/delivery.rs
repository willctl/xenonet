//! Delivery's controls (server/routes/delivery.js), yours only, never an
//! agent's or an actor's: the pause switch, scopes granted to actors'
//! tokens, consent to wake or type into a session, and the audit log. GET
//! /api/delivery reads them; the POSTs change them.

use crate::borrowed::from_opencode_id;
use crate::cores::{ok, reason, CoreError, Engine, Policy, Router, Verify};
use crate::ctx::pending_reply;
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_base::ids::is_session_id;
use xenonet_base::text::{js_number, js_string, truthy};

pub struct DeliveryRoutes {
    pub policy: Arc<dyn Policy>,
    pub router: Arc<dyn Router>,
    pub engine: Arc<dyn Engine>,
    pub verify: Arc<dyn Verify>,
}

// Consent is kept under the id the router looks it up by, so an OpenCode
// session's own ses_… becomes XenoNet's opencode:ses-…, and anything that
// isn't a session's id is refused rather than kept where nothing would
// ever read it.
fn session_of(text: &Value) -> Result<String, CoreError> {
    let id = from_opencode_id(text);
    if !is_session_id(&id) {
        let shown = if text.is_null() { String::new() } else { js_string(text) };
        return Err(CoreError::new(format!("\"{shown}\" isn't a session's id: give its id, such as opencode:ses-… or claude:…, which xenonet agents --json lists")));
    }
    Ok(id)
}

fn str_of(v: &Value) -> String {
    if v.is_null() {
        String::new()
    } else {
        js_string(v)
    }
}

/// Hours as milliseconds, a minute to 90 days, `d` when it isn't one.
fn hours(h: Option<&Value>, d: f64) -> f64 {
    let n = js_number(h);
    let n = if n.is_nan() || n == 0.0 { d } else { n };
    (1.0 / 60.0f64).max(n.min(24.0 * 90.0)) * 3600e3
}

impl DeliveryRoutes {
    pub async fn route(&self, req: &mut Req) -> Reply {
        match self.inner(req).await {
            Ok(r) => r,
            Err(e) if e.is_pending() => pending_reply(&e),
            Err(e) => Reply::error(400, e.message),
        }
    }

    async fn inner(&self, req: &mut Req) -> Result<Reply, CoreError> {
        if req.header("authorization").is_some() {
            return Ok(Reply::coded(403, "yours", "delivery's controls are yours, and an actor can't change them"));
        }
        let route = req.path.clone();
        if req.method == "GET" && route == "/api/delivery" {
            let usage: serde_json::Map<String, Value> = ["claude", "codex"].iter().map(|app| (app.to_string(), self.router.usage(app).unwrap_or(Value::Null))).collect();
            let listening: Vec<Value> = self.engine.listening().unwrap_or_default().into_iter().map(|(agent, until)| json!({ "agent": agent, "until": until })).collect();
            let audit = js_number(req.param("audit").map(|s| Value::String(s.to_string())).as_ref());
            let audit = if audit.is_nan() || audit == 0.0 { 50.0 } else { audit };
            return Ok(Reply::json(
                200,
                json!({
                    "paused": self.policy.paused()?,
                    "grants": self.policy.grants()?,
                    "consent": self.policy.consent()?,
                    "usage": usage,
                    "listening": listening,
                    "recent": self.router.list(50).unwrap_or_else(|_| json!([])),
                    "audit": self.policy.read_audit(audit)?,
                }),
            ));
        }
        if req.method != "POST" {
            return Ok(Reply::error(404, "not found"));
        }
        let you = self.verify.person(req.peer).await;
        if !ok(&you) {
            return Ok(Reply::error(403, reason(&you)));
        }
        let b = req.json_body().await?;
        let ans = |v: Value| Ok(Reply::json(200, v));
        match route.as_str() {
            "/api/delivery/pause" => ans(self.policy.set_paused(b.get("paused") != Some(&Value::Bool(false)))?),
            "/api/delivery/grant" => ans(self.policy.grant(&str_of(&b["actor"]), &str_of(&b["scope"]), hours(b.get("hours"), 24.0 * 30.0))?),
            "/api/delivery/revoke" => ans(self.policy.revoke(&str_of(&b["actor"]))?),
            "/api/delivery/allow" => {
                let session = session_of(b.get("session").unwrap_or(&Value::Null))?;
                let mut out = json!({ "session": session });
                if let Value::Object(more) = self.policy.allow(&session, truthy(b.get("remote")), hours(b.get("hours"), 8.0))? {
                    for (k, v) in more {
                        out[k] = v;
                    }
                }
                ans(out)
            }
            // Taking consent back works for any key, so one kept before this
            // check can still go.
            "/api/delivery/disallow" => {
                self.policy.disallow(&from_opencode_id(b.get("session").unwrap_or(&Value::Null)))?;
                ans(json!({ "ok": true }))
            }
            _ => Ok(Reply::error(404, "not found")),
        }
    }
}
