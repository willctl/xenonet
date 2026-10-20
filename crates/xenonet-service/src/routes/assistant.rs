//! Remote actors' connections over the API (server/routes/assistant.js;
//! docs/guide/cloud-assistants.md). None of this is reachable from a
//! connection itself. Yours, from the window or your own terminal: the
//! list, places, audit, revoking, turning all off, the tunnels and stopping
//! or forgetting one. The window's alone, since they make or widen access:
//! preview, pairing, a new scope or credential, turning on again, and a
//! tunnel's settings, key, start and check.

use crate::cores::{CoreError, Kind};
use crate::ctx::{fail, pending_reply, Ctx};
use crate::http::{Reply, Req};
use serde_json::{json, Value};
use std::sync::Arc;
use xenonet_base::sync::Locked;

const BODY: usize = 16 << 10;
const WINDOW_MS: f64 = 7.0 * 24.0 * 3600e3;

fn none() -> Value {
    json!({})
}

/// ^gr_[0-9a-f]{24}$
pub fn grant_id(s: &str) -> bool {
    s.len() == 27 && s.starts_with("gr_") && s[3..].bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The products that can be paired (core/actors/adapters.js with
/// products.js), which the actors' slice ports; until then the list is
/// theirs to give.
fn products(ctx: &Ctx) -> Result<Value, CoreError> {
    ctx.cores.assistant_source.as_ref().map_or(Ok(json!([])), |s| s.products())
}

pub struct AssistantRoutes<'a> {
    pub ctx: &'a Arc<Ctx>,
}

impl AssistantRoutes<'_> {
    async fn guard(&self, req: &Req, window: bool) -> Option<Reply> {
        let refused = if window { self.ctx.from_window(req).await } else { self.ctx.from_you(req).await };
        refused.map(|r| fail(403, "yours", r, &[], &none()))
    }

    async fn object_of(&self, req: &mut Req) -> Result<Value, CoreError> {
        let body = req.json(BODY).await?;
        if !body.is_object() {
            return Err(CoreError::of(Kind::Connection, 400, "bad-request", "the body is a JSON object"));
        }
        Ok(body)
    }

    // The projects and folders on the map now, by node id, most active first.
    fn places(&self) -> Result<Vec<Value>, CoreError> {
        let state = &self.ctx.cores.state;
        let t = self.ctx.now();
        let nodes = state.nodes()?;
        let mut counts: Vec<(Value, f64)> = Vec::new();
        for n in &nodes {
            if n["kind"] != "session"
                || xenonet_base::text::truthy(n.get("host"))
                || xenonet_base::text::truthy(n.get("archived"))
                || n["updatedAt"].as_f64().unwrap_or(0.0) < t - WINDOW_MS
            {
                continue;
            }
            let home = state.home_of(n["id"].as_str().unwrap_or(""))?;
            match counts.iter_mut().find(|(h, _)| *h == home) {
                Some(c) => c.1 += 1.0,
                None => counts.push((home, 1.0)),
            }
        }
        let mut places: Vec<Value> = nodes
            .iter()
            .filter(|n| (n["kind"] == "project" || n["kind"] == "zone") && !xenonet_base::text::truthy(n.get("host")))
            .map(|n| {
                let count = counts.iter().find(|(h, _)| *h == n["id"]).map_or(0.0, |c| c.1);
                json!({ "place": n["id"], "name": n.get("name").cloned().unwrap_or(Value::Null), "kind": n["kind"], "sessions7d": xenonet_base::json::number(count), "lastActivity": n.get("lastActivity").cloned().unwrap_or(Value::Null) })
            })
            .collect();
        let name = |v: &Value| xenonet_base::text::js_string(&v["name"]);
        places.sort_by(|a, b| {
            let n = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
            n(b, "sessions7d").total_cmp(&n(a, "sessions7d")).then(n(b, "lastActivity").total_cmp(&n(a, "lastActivity"))).then_with(|| name(a).cmp(&name(b)))
        });
        Ok(places)
    }

    fn place_map(&self) -> Result<Value, CoreError> {
        Ok(Value::Object(self.places()?.into_iter().map(|p| (xenonet_base::text::js_string(&p["place"]), p)).collect()))
    }

    fn identities(&self) -> Result<Vec<Value>, CoreError> {
        Ok(self.ctx.cores.state.graph()?.and_then(|g| g.get("identities").and_then(Value::as_array).cloned()).unwrap_or_default())
    }

    pub async fn route(&self, mut req: Req) -> Result<Reply, CoreError> {
        let Some(connections) = self.ctx.cores.connections.clone() else {
            return Ok(fail(503, "off", "Actors are off here (XENONET_ACTORS=off), and remote actors with them", &[], &none()));
        };
        match self.inner(&mut req, connections).await {
            Ok(r) => Ok(r),
            Err(e) if e.is_pending() => Ok(pending_reply(&e)),
            Err(e) if e.is(Kind::Connection) => {
                Ok(fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("error"), e.message, &[], if e.extra.is_object() { &e.extra } else { &Value::Null }))
            }
            Err(e) if e.is(Kind::Tunnel) => Ok(fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("error"), e.message, &[], &none())),
            Err(e) if e.status == Some(413) => Ok(fail(413, "too-big", "that body is over the size this route takes", &[], &none())),
            Err(e) if e.status == Some(400) => Ok(fail(400, "bad-request", "that body isn't JSON", &[], &none())),
            Err(e) if e.message.contains("couldn't keep the grant's credential private") => Ok(fail(500, "credential", e.message, &[], &none())),
            Err(e) => Err(e),
        }
    }

    async fn inner(&self, req: &mut Req, connections: Arc<dyn crate::cores::Connections>) -> Result<Reply, CoreError> {
        let p = req.path.clone();
        let m = req.method.clone();
        let tunnels = self.ctx.cores.tunnels.clone();
        let no_tunnels = || fail(503, "off", "XenoNet runs no tunnels here: remote actors are off", &[], &none());
        if p == "/api/assistant" && m == "GET" {
            if let Some(r) = self.guard(req, false).await {
                return Ok(r);
            }
            let listening = self.ctx.endpoint.locked().clone().map(|e| e.info());
            let sources = match &self.ctx.cores.assistant_source {
                Some(s) => s.sources_data()?,
                None => Value::Null,
            };
            let diagnostics = json!({
                "service_version": self.ctx.version,
                "assistant_api": 1,
                "endpoint": listening.map(|l| json!({ "listening": l["listening"], "adapters": l["sessions"] })),
                "sources": sources,
            });
            let mut out = connections.list()?;
            out["products"] = products(self.ctx)?;
            out["diagnostics"] = diagnostics;
            return Ok(Reply::json(200, out));
        }
        if p == "/api/assistant/places" && m == "GET" {
            if let Some(r) = self.guard(req, false).await {
                return Ok(r);
            }
            let ids: Vec<Value> = self
                .identities()?
                .into_iter()
                .filter(|i| !xenonet_base::text::truthy(i.get("hidden")))
                .map(|i| json!({ "id": i["id"], "kind": i["kind"], "name": i.get("name").cloned().unwrap_or(Value::Null) }))
                .collect();
            return Ok(Reply::json(200, json!({ "places": self.places()?, "identities": ids })));
        }
        if p == "/api/assistant/audit" && m == "GET" {
            if let Some(r) = self.guard(req, false).await {
                return Ok(r);
            }
            let grant = req.param("grant").filter(|g| !g.is_empty()).map(str::to_string);
            if grant.as_deref().is_some_and(|g| !grant_id(g)) {
                return Ok(fail(400, "bad-request", "grant is a grant's id", &[], &none()));
            }
            let limit = xenonet_base::text::js_number(req.param("limit").map(|s| Value::String(s.into())).as_ref());
            let limit = if limit.is_nan() || limit == 0.0 { 100.0 } else { limit };
            let audit = self.ctx.cores.audit.clone().ok_or_else(|| CoreError::pending("core/assistant/audit.js"))?;
            return Ok(Reply::json(200, json!({ "records": audit.recent(grant.as_deref(), limit)? })));
        }
        if p == "/api/assistant/disable" && m == "POST" {
            if let Some(r) = self.guard(req, false).await {
                return Ok(r);
            }
            return Ok(Reply::json(200, connections.disable()?));
        }
        // Turning access back on widens it again, so it's the window's alone.
        if p == "/api/assistant/enable" && m == "POST" {
            if let Some(r) = self.guard(req, true).await {
                return Ok(r);
            }
            return Ok(Reply::json(200, connections.enable()?));
        }
        if p == "/api/assistant/preview" && m == "POST" {
            if let Some(r) = self.guard(req, true).await {
                return Ok(r);
            }
            let body = self.object_of(req).await?;
            let known = self.place_map()?;
            let source = self.ctx.cores.assistant_source.clone().ok_or_else(|| CoreError::pending("core/assistant/source.js"))?;
            return Ok(Reply::json(200, source.preview(&body, &known).await?));
        }
        if p == "/api/assistant/pair" && m == "POST" {
            if let Some(r) = self.guard(req, true).await {
                return Ok(r);
            }
            let body = self.object_of(req).await?;
            let ids: Vec<String> = self.identities()?.iter().map(|i| xenonet_base::text::js_string(&i["id"])).collect();
            return Ok(Reply::json(201, connections.pair(&body, &self.place_map()?, &ids)?));
        }
        if p == "/api/assistant/tunnels" && m == "GET" {
            if let Some(r) = self.guard(req, false).await {
                return Ok(r);
            }
            let Some(t) = &tunnels else { return Ok(no_tunnels()) };
            return Ok(Reply::json(200, t.status().await?));
        }
        let rest = p.strip_prefix("/api/assistant/").unwrap_or("");
        let parts: Vec<&str> = rest.split('/').collect();
        if m == "POST"
            && parts.len() >= 2
            && parts.len() <= 3
            && grant_id(parts[0])
            && parts[1] == "tunnel"
            && (parts.len() == 2 || ["key", "start", "stop", "check", "forget"].contains(&parts[2]))
        {
            let grant = parts[0];
            let action = parts.get(2).copied();
            let Some(t) = &tunnels else { return Ok(no_tunnels()) };
            // Stopping one, or taking away its key, narrows what reaches this
            // computer: yours from a terminal too. The rest is the window's.
            if matches!(action, Some("stop" | "forget")) {
                if let Some(r) = self.guard(req, false).await {
                    return Ok(r);
                }
                return Ok(Reply::json(200, if action == Some("stop") { t.stop(grant)? } else { t.forget_key(grant).await? }));
            }
            if let Some(r) = self.guard(req, true).await {
                return Ok(r);
            }
            return Ok(Reply::json(
                200,
                match action {
                    Some("start") => t.start(grant).await?,
                    Some("check") => t.check(grant).await?,
                    Some("key") => {
                        let body = self.object_of(req).await?;
                        t.store_key(grant, body.get("key").unwrap_or(&Value::Null)).await?
                    }
                    _ => {
                        let body = self.object_of(req).await?;
                        t.configure(grant, json!({ "tunnelId": body.get("tunnelId"), "keyTarget": body.get("keyTarget"), "autostart": body.get("autostart") }))?
                    }
                },
            ));
        }
        if m == "POST" && (1..=2).contains(&parts.len()) && grant_id(parts[0]) && (parts.len() == 1 || parts[1] == "revoke" || parts[1] == "rotate") {
            let grant = parts[0];
            let action = parts.get(1).copied();
            if action == Some("revoke") {
                if let Some(r) = self.guard(req, false).await {
                    return Ok(r);
                }
                return Ok(Reply::json(200, connections.revoke(grant)?));
            }
            if let Some(r) = self.guard(req, true).await {
                return Ok(r);
            }
            if action == Some("rotate") {
                return Ok(Reply::json(200, connections.rotate(grant)?));
            }
            let body = self.object_of(req).await?;
            return Ok(Reply::json(200, connections.update(grant, body.get("projects").unwrap_or(&Value::Null), body.get("labels").unwrap_or(&Value::Null), &self.place_map()?)?));
        }
        Ok(fail(404, "not-found", "XenoNet has no such route for remote actors", &[], &none()))
    }
}
