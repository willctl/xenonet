//! Actors' HTTP API (server/routes/actors.js; docs/design/actors/
//! interface.md, section 2). Anyone on this machine may read actors; a
//! program asks to be one and then reports with its token, which works
//! once you allow it; XenoNet's own window, with its key, allows, adds and
//! rotates; your terminal or the window removes actors and runs the
//! gateway and wakes. Errors are {"error", "code"}, with a reason for a
//! revoked token, a suggestion for a slug that's taken, and Retry-After
//! over a rate. Nothing here logs an actor's body, status or token.

use crate::actor_caller::{rates, retry_after, ActorCaller, Refusal};
use crate::cores::{CoreError, Kind};
use crate::ctx::{fail, pending_reply, Ctx, YOU_REPORTS};
use crate::http::{decode_uri_component, Reply, Req};
use bytes::Bytes;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use xenonet_actors::tokens::token_in_url;
use xenonet_base::ids::{parse_actor_id, YOU};

const CONSOLE_BODY: usize = 8 << 10;
const REGISTER_BODY: usize = 4 << 10;
const HEARTBEAT_BODY: usize = 4 << 10;
const TARGETS_BODY: usize = 16 << 10;
const EVENTS_BODY: usize = 64 << 10;
const SMALL_BODY: usize = 1 << 10;
const WAKE_BODY: usize = 4 << 10;
// A stream held open says it's still there this often, and checks its
// token still works.
const STREAM_BEAT: Duration = Duration::from_secs(20);
const EVENTS_PER_CALL: usize = 50;

fn none() -> Value {
    json!({})
}

fn not_found() -> Reply {
    fail(404, "not-found", "XenoNet has no such route for actors", &[], &none())
}

fn refusal(r: Refusal) -> Reply {
    fail(r.status, &r.code, r.error, &r.headers, &r.extra)
}

fn bad(error: impl Into<String>) -> Reply {
    fail(400, "bad-request", error, &[], &none())
}

/// A time in epoch milliseconds from ?since=: Ok(None) when there's none,
/// Err when it isn't one.
fn since_of(req: &Req) -> Result<Option<f64>, ()> {
    match req.param("since") {
        None | Some("") => Ok(None),
        Some(raw) => {
            let n = xenonet_base::text::js_number(Some(&Value::String(raw.to_string())));
            if n.is_finite() {
                Ok(Some(n))
            } else {
                Err(())
            }
        }
    }
}

fn limit_of(req: &Req) -> f64 {
    let n = xenonet_base::text::js_number(req.param("limit").map(|s| Value::String(s.to_string())).as_ref());
    if n.is_nan() || n == 0.0 {
        1000.0
    } else {
        n
    }
}

/// An actor's slug from the path: None for one that isn't, or another
/// host's.
fn slug_of(raw: &str) -> Option<String> {
    let id = format!("actor:{}", decode_uri_component(raw)?);
    match parse_actor_id(&id) {
        Some((None, slug)) => Some(slug.to_string()),
        _ => None,
    }
}

fn issue(e: &str) -> String {
    e.to_string()
}

fn from_core(e: CoreError) -> Result<Reply, CoreError> {
    if e.is_pending() {
        return Ok(pending_reply(&e));
    }
    if e.is(Kind::Wake) {
        return Ok(fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("error"), e.message, &[], &none()));
    }
    if e.is(Kind::Actor) {
        let mut extra = e.extra.clone();
        let mut headers = Vec::new();
        if let Value::Object(map) = &mut extra {
            if let Some(ms) = map.remove("retryAfterMs") {
                if !ms.is_null() {
                    headers.push(retry_after(ms.as_f64().unwrap_or(f64::NAN)));
                }
            }
        }
        let extra = if extra.is_object() { extra } else { none() };
        return Ok(fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("error"), e.message, &headers, &extra));
    }
    match e.status {
        Some(413) => Ok(fail(413, "too-big", "that body is over the size this route takes", &[], &none())),
        Some(400) => Ok(fail(400, "bad-request", "that body isn't JSON", &[], &none())),
        _ => Err(e),
    }
}

pub struct ActorRoutes<'a> {
    pub ctx: &'a Arc<Ctx>,
}

impl ActorRoutes<'_> {
    fn caller(&self) -> &ActorCaller {
        self.ctx.caller.as_ref().expect("actors are on")
    }

    async fn object_of(&self, req: &mut Req, limit: usize) -> Result<Value, CoreError> {
        let body = req.json(limit).await?;
        if !body.is_object() {
            return Err(CoreError::of(Kind::Actor, 400, "bad-request", "the body is a JSON object"));
        }
        Ok(body)
    }

    async fn window_only(&self, req: &Req) -> Option<Reply> {
        self.ctx.from_window(req).await.map(|r| fail(403, "yours", r, &[], &none()))
    }

    async fn you(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let actors = self.ctx.cores.actors.as_ref().unwrap();
        // Without a key it's any program asking about you, which gets what
        // the list says.
        if req.method == "GET" && req.header("x-xenonet-window").is_none() {
            return Ok(Reply::json(200, actors.snapshot(YOU, None, true)?.unwrap_or(Value::Null)));
        }
        if !["GET", "POST", "DELETE"].contains(&req.method.as_str()) {
            return Ok(not_found());
        }
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        // Show me on the map turned off: your record goes at once.
        if req.method == "DELETE" {
            actors.you_forget()?;
            return Ok(Reply::json(200, json!({ "deleted": YOU })));
        }
        if req.method == "GET" {
            let Ok(since) = since_of(req) else { return Ok(bad("since is a time, in epoch milliseconds")) };
            return Ok(Reply::json(
                200,
                match since {
                    None => actors.snapshot(YOU, Some("window"), true)?.unwrap_or(Value::Null),
                    Some(s) => actors.events(Some(s), Some(YOU), Some("window"), limit_of(req))?,
                },
            ));
        }
        let wait = self.ctx.you_reports.lock().unwrap().take(1.0, self.ctx.now());
        if wait != 0.0 {
            return Ok(fail(429, "too-many", "the window is reporting faster than XenoNet takes reports", &[retry_after(wait)], &none()));
        }
        let body = req.json(CONSOLE_BODY).await?;
        if let Err(e) = self.ctx.cores.schemas.check("actor/console/v1", &body) {
            return Ok(bad(format!("that isn't a report of you (actor/console/v1): {}", issue(&e))));
        }
        let taken = actors.you_report(&body)?;
        let me = actors.snapshot(YOU, Some("window"), true)?.unwrap_or(Value::Null);
        Ok(Reply::json(
            200,
            json!({ "schema": "actor/me/v1", "id": YOU, "state": "allowed", "presence": me["presence"], "heartbeatMs": me["heartbeatMs"], "unread": 0, "accepted": taken["accepted"], "dropped": taken["dropped"] }),
        ))
    }

    async fn register(&self, req: &mut Req) -> Result<Reply, CoreError> {
        if req.header("authorization").is_some() {
            return Ok(fail(403, "not-allowed", "an actor can't register actors: that's for you, in XenoNet's window or your own terminal", &[], &none()));
        }
        let body = req.json(REGISTER_BODY).await?;
        if let Err(e) = self.ctx.cores.schemas.check("actor/registration/v1", &body) {
            return Ok(bad(format!("that isn't a registration (actor/registration/v1): {}", issue(&e))));
        }
        let product = body.get("product").and_then(|p| p.get("id")).cloned().unwrap_or(Value::Null);
        let pid = body.get("pid").cloned().unwrap_or(Value::Null);
        let verdict = self.ctx.cores.verify.register(req.peer, product, pid).await;
        if !crate::cores::ok(&verdict) {
            return Ok(fail(403, "wrong-process", crate::cores::reason(&verdict), &[], &none()));
        }
        Ok(Reply::json(202, self.registration().register(&body, &verdict)?))
    }

    fn registration(&self) -> &Arc<dyn crate::cores::Registration> {
        self.ctx.cores.registration.as_ref().unwrap()
    }

    fn actors(&self) -> &Arc<dyn crate::cores::Actors> {
        self.ctx.cores.actors.as_ref().unwrap()
    }

    async fn actor_of(&self, req: &Req, waiting: bool) -> Result<Value, Reply> {
        self.caller().authenticate(req, waiting).await.map_err(refusal)
    }

    async fn checked(&self, req: &Req, auth: &Value, pid: Value) -> Result<Value, Reply> {
        self.caller().check(req, auth, pid).await.map_err(|r| fail(r.status, &r.code, r.error, &[], &none()))
    }

    // The processes an actor counts as its own, for the sessions it says it
    // spawned: the ones it named, or else the caller and its parents below
    // the first app.
    fn own_pids(&self, id: &str, verdict: &Value) -> Result<Option<Vec<u32>>, CoreError> {
        let named = self.actors().pids_of(id)?;
        if !named.is_empty() {
            return Ok(Some(named));
        }
        let mine = self.ctx.cores.reports.own_of(verdict.get("chain").unwrap_or(&json!([])));
        Ok((!mine.is_empty()).then_some(mine))
    }

    async fn me(&self, req: &Req) -> Result<Reply, CoreError> {
        let auth = match self.actor_of(req, true).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        if let Err(r) = self.checked(req, &auth, Value::Null).await {
            return Ok(r);
        }
        Ok(Reply::json(200, self.registration().me(&auth, None)?))
    }

    async fn leave(&self, req: &Req) -> Result<Reply, CoreError> {
        let auth = match self.actor_of(req, false).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        if let Err(r) = self.checked(req, &auth, Value::Null).await {
            return Ok(r);
        }
        let slug = auth["slug"].as_str().unwrap_or("").to_string();
        let out = self.registration().remove(&slug, Some("left"))?;
        self.caller().forget(&slug);
        if let Some(w) = &self.ctx.cores.wakes {
            w.forget(&slug).await?;
        }
        Ok(Reply::json(200, out))
    }

    // A report's body, checked against actor/report/v1, the parts this route
    // takes (`parts`) left out of the check to be taken a part at a time.
    async fn report_of(&self, req: &mut Req, limit: usize, parts: &[&str]) -> Result<Result<Value, Reply>, CoreError> {
        let body = self.object_of(req, limit).await?;
        let mut envelope = body.clone();
        for k in parts {
            if envelope.get(*k).is_some() {
                envelope[*k] = json!([]);
            }
        }
        Ok(match self.ctx.cores.schemas.check("actor/report/v1", &envelope) {
            Ok(()) => Ok(body),
            Err(e) => Err(bad(format!("that isn't a report (actor/report/v1): {}", issue(&e)))),
        })
    }

    // A heartbeat's rates, once the process check has passed: the actor's
    // eight a second, then its instance's one, then an event's worth for a
    // new status.
    fn heartbeat_wait(&self, auth: &Value, body: &Value) -> Result<Option<(f64, &'static str)>, CoreError> {
        let slug = auth["slug"].as_str().unwrap_or("");
        let instance = match body.get("instance") {
            Some(Value::Null) | None => "main".to_string(),
            Some(v) => xenonet_base::text::js_string(v),
        };
        let caller = self.caller();
        let mut wait = caller.take(&format!("a:{slug}|hb"), rates::HEARTBEATS, 1.0);
        if wait == 0.0 {
            wait = caller.take(&format!("a:{slug}|hb|{instance}"), rates::HEARTBEAT, 1.0);
        }
        if wait != 0.0 {
            return Ok(Some((wait, "one heartbeat a second per instance, and eight a second per actor, is the most XenoNet takes")));
        }
        let line = self.ctx.cores.reports.status_line(body.get("status").unwrap_or(&Value::Null));
        let id = auth["id"].as_str().unwrap_or("");
        let current = self.actors().snapshot(id, None, false)?.and_then(|s| s.get("status").cloned());
        if line["ok"] != json!(true) || !xenonet_base::text::truthy(line.get("status")) || Some(&line["status"]) == current.as_ref() {
            return Ok(None);
        }
        let busy = caller.take(&format!("a:{slug}|events"), rates::EVENTS, 1.0);
        Ok((busy != 0.0).then_some((busy, "a new status is an event on the feed, and 60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes")))
    }

    async fn heartbeat(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let auth = match self.actor_of(req, false).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        let body = match self.report_of(req, HEARTBEAT_BODY, &[]).await? {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        if let Some(k) = ["events", "targets", "remove", "replace"].iter().find(|k| body.get(**k).is_some()) {
            let to = if *k == "events" { "POST /api/actors/events" } else { "POST /api/actors/targets" };
            return Ok(bad(format!("{k} go to {to}, not to a heartbeat")));
        }
        if let Err(r) = self.checked(req, &auth, body.get("pid").cloned().unwrap_or(Value::Null)).await {
            return Ok(r);
        }
        if let Some((wait, why)) = self.heartbeat_wait(&auth, &body)? {
            return Ok(fail(429, "too-many", why, &[retry_after(wait)], &none()));
        }
        self.ctx.cores.reports.take_heartbeat(auth["id"].as_str().unwrap_or(""), &body)?;
        Ok(Reply::json(200, self.registration().me(&auth, None)?))
    }

    /// The events or targets a call names, each checked on its own: one
    /// that doesn't fit is dropped with why, and the others are taken.
    fn parsed(list: &[Value], parse: impl Fn(&Value) -> Result<Value, String>, what: &str) -> (Vec<Value>, Vec<Value>) {
        let mut invalid = Vec::new();
        let items = list
            .iter()
            .enumerate()
            .map(|(index, e)| match parse(e) {
                Ok(v) => v,
                Err(why) => {
                    invalid.push(json!({ "index": index, "reason": format!("that isn't {what}: {why}").trim().to_string() }));
                    Value::Null
                }
            })
            .collect();
        (items, invalid)
    }

    fn dropped(invalid: Vec<Value>, taken: &Value) -> Vec<Value> {
        let mut all = invalid;
        all.extend(taken["dropped"].as_array().cloned().unwrap_or_default());
        all.sort_by(|a, b| a["index"].as_f64().unwrap_or(0.0).total_cmp(&b["index"].as_f64().unwrap_or(0.0)));
        all
    }

    async fn events(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let auth = match self.actor_of(req, false).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        let body = match self.report_of(req, EVENTS_BODY, &["events"]).await? {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        let Some(list) = body.get("events").and_then(Value::as_array) else { return Ok(bad("events is a list of what it did")) };
        if list.len() > EVENTS_PER_CALL {
            return Ok(fail(413, "too-big", format!("{EVENTS_PER_CALL} events a call at most"), &[], &none()));
        }
        let verdict = match self.checked(req, &auth, Value::Null).await {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let slug = auth["slug"].as_str().unwrap_or("");
        let wait = self.caller().take(&format!("a:{slug}|events"), rates::EVENTS, (list.len() as f64).max(1.0));
        if wait != 0.0 {
            return Ok(fail(429, "too-many", "60 events a minute per actor, in bursts of up to 120, is the most XenoNet takes", &[retry_after(wait)], &none()));
        }
        let reports = &self.ctx.cores.reports;
        let (items, invalid) = Self::parsed(list, |e| reports.parse_event(e), "an event");
        let id = auth["id"].as_str().unwrap_or("");
        let skip = invalid.iter().filter_map(|d| d["index"].as_u64().map(|i| i as usize)).collect();
        let taken = reports.take_events(id, items, self.own_pids(id, &verdict)?, skip).await?;
        let dropped = Self::dropped(invalid, &taken);
        Ok(Reply::json(200, self.registration().me(&auth, Some(json!({ "accepted": taken["accepted"], "dropped": dropped })))?))
    }

    async fn targets(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let auth = match self.actor_of(req, false).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        let body = match self.report_of(req, TARGETS_BODY, &["targets", "remove"]).await? {
            Ok(b) => b,
            Err(r) => return Ok(r),
        };
        for k in ["targets", "remove"] {
            if body.get(k).is_some_and(|v| !v.is_array()) {
                return Ok(bad(format!("{k} is a list")));
            }
        }
        let verdict = match self.checked(req, &auth, Value::Null).await {
            Ok(v) => v,
            Err(r) => return Ok(r),
        };
        let slug = auth["slug"].as_str().unwrap_or("");
        let wait = self.caller().take(&format!("a:{slug}|targets"), rates::TARGETS, 1.0);
        if wait != 0.0 {
            return Ok(fail(429, "too-many", "30 target calls a minute is the most XenoNet takes", &[retry_after(wait)], &none()));
        }
        let reports = &self.ctx.cores.reports;
        let list = body.get("targets").and_then(Value::as_array).cloned().unwrap_or_default();
        let (items, invalid) = Self::parsed(&list, |t| reports.parse_target(t), "a target");
        let remove: Vec<Value> = body.get("remove").and_then(Value::as_array).cloned().unwrap_or_default().into_iter().filter_map(|t| reports.parse_target(&t).ok()).collect();
        let id = auth["id"].as_str().unwrap_or("");
        let skip = invalid.iter().filter_map(|d| d["index"].as_u64().map(|i| i as usize)).collect();
        let taken = reports.take_targets(id, items, remove, body.get("replace") == Some(&Value::Bool(true)), self.own_pids(id, &verdict)?, skip).await?;
        let dropped = Self::dropped(invalid, &taken);
        Ok(Reply::json(200, self.registration().me(&auth, Some(json!({ "accepted": taken["accepted"], "dropped": dropped })))?))
    }

    async fn add(&self, req: &mut Req) -> Result<Reply, CoreError> {
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        let body = req.json(REGISTER_BODY).await?;
        if let Err(e) = self.ctx.cores.schemas.check("actor/registration/v1", &body) {
            return Ok(bad(format!("that isn't an actor to add (actor/registration/v1): {}", issue(&e))));
        }
        Ok(Reply::json(201, self.registration().add(&body)?))
    }

    async fn decide(&self, req: &mut Req, id: &str) -> Result<Reply, CoreError> {
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        let body = self.object_of(req, SMALL_BODY).await?;
        let Some(request) = decode_uri_component(id).filter(|r| !r.is_empty()) else {
            return Ok(fail(404, "not-found", "no registration with that id is waiting", &[], &none()));
        };
        let mut out = self.registration().decide(&request, body.get("decision").unwrap_or(&Value::Null))?;
        // An allowed ask for a gateway token answers the window with the
        // token, once, and the gateway's address to go with it.
        if xenonet_base::text::truthy(out.get("credentials")) {
            out["credentials"]["url"] = self.ctx.gateway.as_ref().map_or(Value::Null, |g| g.url());
        }
        Ok(Reply::json(200, out))
    }

    async fn rotate(&self, req: &Req, raw: &str) -> Result<Reply, CoreError> {
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        let Some(slug) = slug_of(raw) else { return Ok(fail(404, "not-found", "XenoNet has no such actor", &[], &none())) };
        Ok(Reply::json(200, self.registration().rotate(&slug)?))
    }

    async fn remove(&self, req: &Req, raw: &str) -> Result<Reply, CoreError> {
        if let Some(r) = self.ctx.from_you(req).await {
            return Ok(fail(403, "yours", r, &[], &none()));
        }
        let Some(slug) = slug_of(raw) else { return Ok(fail(404, "not-found", "XenoNet has no such actor", &[], &none())) };
        // registration.remove refuses one XenoNet only observes (409
        // observed, with its product), from the window or a program alike.
        let out = self.registration().remove(&slug, None)?;
        self.caller().forget(&slug);
        // One that stays, as XenoNet observes it, keeps no wake either.
        if let Some(w) = &self.ctx.cores.wakes {
            w.forget(&slug).await?;
        }
        Ok(Reply::json(200, out))
    }

    async fn gateway_state(&self, req: &mut Req) -> Result<Reply, CoreError> {
        let Some(gateway) = self.ctx.gateway.clone() else { return Ok(fail(503, "off", "the actor gateway isn't here", &[], &none())) };
        if let Some(r) = self.ctx.from_you(req).await {
            return Ok(fail(403, "yours", r, &[], &none()));
        }
        if req.method == "GET" {
            return Ok(Reply::json(200, gateway.status().await?));
        }
        let body = self.object_of(req, SMALL_BODY).await?;
        // tunnel: null takes it away, an object picks one, anything else
        // leaves it as it is.
        let pick = match body.get("tunnel") {
            Some(Value::Null) => Some(Value::Null),
            Some(t @ Value::Object(_)) => Some(json!({ "kind": t.get("kind"), "name": t.get("name"), "host": t.get("host") })),
            _ => None,
        };
        let on = body.get("on").and_then(Value::as_bool);
        match gateway.set(on, pick, body.get("check") == Some(&Value::Bool(true)), body.get("repair") == Some(&Value::Bool(true))).await {
            Ok(v) => Ok(Reply::json(200, v)),
            Err(e) if e.is_pending() => Ok(pending_reply(&e)),
            Err(e) => Ok(fail(e.status.unwrap_or(400), e.code.as_deref().unwrap_or("bad-request"), e.message, &[], &none())),
        }
    }

    async fn gateway_token(&self, req: &mut Req, raw: &str) -> Result<Reply, CoreError> {
        let Some(slug) = slug_of(raw) else { return Ok(fail(404, "not-found", "XenoNet has no such actor", &[], &none())) };
        if req.method == "DELETE" {
            if let Some(r) = self.ctx.from_you(req).await {
                return Ok(fail(403, "yours", r, &[], &none()));
            }
            return Ok(Reply::json(200, self.registration().drop_gateway(&slug)?));
        }
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        let body = self.object_of(req, SMALL_BODY).await?;
        let caps = body.get("capabilities").filter(|c| c.is_array()).cloned();
        let mut creds = self.registration().gateway(&slug, caps)?;
        creds["url"] = self.ctx.gateway.as_ref().map_or(Value::Null, |g| g.url());
        Ok(Reply::json(201, creds))
    }

    // A session's, or your terminal's, ask for an actor's gateway token. Any
    // program may ask, because the answer holds no token; never an actor's
    // token, and never from under an actor product's processes.
    async fn gateway_ask(&self, req: &mut Req, raw: &str) -> Result<Reply, CoreError> {
        if req.header("authorization").is_some() {
            return Ok(fail(403, "not-allowed", "an actor can't ask for gateway tokens: a session or you can, and the token is made in XenoNet's window", &[], &none()));
        }
        let Some(slug) = slug_of(raw) else { return Ok(fail(404, "not-found", "XenoNet has no such actor", &[], &none())) };
        self.object_of(req, SMALL_BODY).await?;
        let verdict = self.ctx.cores.verify.asker(req.peer).await;
        if !crate::cores::ok(&verdict) {
            return Ok(fail(403, "wrong-process", crate::cores::reason(&verdict), &[], &none()));
        }
        Ok(Reply::json(202, self.registration().ask_gateway(&slug, &verdict)?))
    }

    async fn wakes_state(&self, req: &Req) -> Result<Reply, CoreError> {
        let Some(wakes) = &self.ctx.cores.wakes else { return Ok(fail(503, "off", "waking actors isn't here", &[], &none())) };
        if let Some(r) = self.ctx.from_you(req).await {
            return Ok(fail(403, "yours", r, &[], &none()));
        }
        Ok(Reply::json(200, json!({ "at": xenonet_base::json::number(self.ctx.now()), "wakes": wakes.list()? })))
    }

    async fn wake(&self, req: &mut Req, raw: &str, test: bool) -> Result<Reply, CoreError> {
        let Some(wakes) = self.ctx.cores.wakes.clone() else { return Ok(fail(503, "off", "waking actors isn't here", &[], &none())) };
        let Some(slug) = slug_of(raw).filter(|s| s != "you") else { return Ok(fail(404, "not-found", "XenoNet has no such actor", &[], &none())) };
        if test || req.method == "DELETE" {
            if let Some(r) = self.ctx.from_you(req).await {
                return Ok(fail(403, "yours", r, &[], &none()));
            }
            return Ok(Reply::json(200, if test { wakes.test(&slug).await? } else { wakes.set(&slug, json!({ "kind": "none" })).await? }));
        }
        if let Some(r) = self.window_only(req).await {
            return Ok(r);
        }
        let body = self.object_of(req, WAKE_BODY).await?;
        Ok(Reply::json(200, wakes.set(&slug, json!({ "kind": body.get("kind"), "url": body.get("url"), "key": body.get("key") })).await?))
    }

    // An actor's own stream (text/event-stream): "inbox" with how many
    // messages wait, as each arrives. Its token and process are checked as
    // for its other calls, and its token again every 20 seconds; the stream
    // ends when that stops working.
    async fn stream(&self, req: Req) -> Result<Reply, CoreError> {
        let Some(wakes) = self.ctx.cores.wakes.clone() else { return Ok(fail(503, "off", "waking actors isn't here", &[], &none())) };
        let auth = match self.actor_of(&req, false).await {
            Ok(a) => a,
            Err(r) => return Ok(r),
        };
        if let Err(r) = self.checked(&req, &auth, Value::Null).await {
            return Ok(r);
        }
        let (tx, rx) = mpsc::unbounded_channel::<Bytes>();
        let frame = |event: &str, data: &Value| Bytes::from(format!("event: {event}\ndata: {}\n\n", xenonet_base::json::stringify(data, 0)));
        let _ = tx.send(Bytes::from_static(b"retry: 5000\n\n"));
        let headers = [("content-type", "text/event-stream; charset=utf-8"), ("cache-control", "no-store"), ("x-content-type-options", "nosniff")];
        let id = auth["id"].as_str().unwrap_or("").to_string();
        let mut events = match wakes.subscribe(&id) {
            Ok(e) => e,
            Err(e) => {
                let _ = tx.send(frame("closed", &json!({ "code": e.code.clone().unwrap_or_else(|| "error".into()), "error": e.message })));
                drop(tx);
                return Ok(Reply::stream(200, &headers, rx));
            }
        };
        let ctx = self.ctx.clone();
        tokio::spawn(async move {
            let mut beat = tokio::time::interval_at(tokio::time::Instant::now() + STREAM_BEAT, STREAM_BEAT);
            loop {
                tokio::select! {
                    got = events.recv() => match got {
                        Some((event, data)) => {
                            if tx.send(frame(&event, &data)).is_err() {
                                return;
                            }
                        }
                        None => return,
                    },
                    _ = beat.tick() => {
                        let who = match &ctx.caller {
                            Some(c) => c.authenticate(&req, false).await.map_err(|r| r.code),
                            None => Err("error".to_string()),
                        };
                        match who {
                            Ok(a) if a["id"] == json!(id) => {
                                if tx.send(Bytes::from_static(b": still here\n\n")).is_err() {
                                    return;
                                }
                            }
                            other => {
                                let code = other.err().unwrap_or_else(|| "revoked".into());
                                let _ = tx.send(frame("closed", &json!({ "code": code, "error": "its token stopped working" })));
                                return;
                            }
                        }
                    }
                    _ = tx.closed() => return,
                }
            }
        });
        Ok(Reply::stream(200, &headers, rx))
    }

    async fn observe(&self, req: &mut Req) -> Result<Reply, CoreError> {
        if let Some(r) = self.ctx.from_you(req).await {
            return Ok(fail(403, "yours", r, &[], &none()));
        }
        let body = self.object_of(req, SMALL_BODY).await?;
        Ok(Reply::json(200, self.registration().observe(body.get("product").unwrap_or(&Value::Null), body.get("on").unwrap_or(&Value::Null))?))
    }

    pub async fn route(&self, mut req: Req) -> Result<Reply, CoreError> {
        if self.ctx.cores.actors.is_none() {
            return Ok(fail(503, "off", "Actors are off here (XENONET_ACTORS=off)", &[], &none()));
        }
        if token_in_url(req.params(), &req.path) {
            return Ok(bad("a token goes in the Authorization header, never in a URL"));
        }
        let p = req.path.clone();
        if p == "/api/actors/me/stream" {
            return if req.method == "GET" { self.stream(req).await.or_else(from_core) } else { Ok(not_found()) };
        }
        match self.dispatch(&mut req, &p).await {
            Ok(r) => Ok(r),
            Err(e) => from_core(e),
        }
    }

    async fn dispatch(&self, req: &mut Req, p: &str) -> Result<Reply, CoreError> {
        let m = req.method.clone();
        let m = m.as_str();
        match p {
            "/api/actors" => {
                return match m {
                    "GET" => Ok(Reply::json(200, self.actors().list()?)),
                    "POST" => self.add(req).await,
                    _ => Ok(not_found()),
                }
            }
            "/api/actors/events" => {
                if m == "POST" {
                    return self.events(req).await;
                }
                if m != "GET" {
                    return Ok(not_found());
                }
                let Ok(since) = since_of(req) else { return Ok(bad("since is a time, in epoch milliseconds")) };
                return Ok(Reply::json(200, self.actors().events(since, None, None, limit_of(req))?));
            }
            "/api/actors/you" => return self.you(req).await,
            "/api/actors/register" => return if m == "POST" { self.register(req).await } else { Ok(not_found()) },
            "/api/actors/me" => {
                return match m {
                    "GET" => self.me(req).await,
                    "DELETE" => self.leave(req).await,
                    _ => Ok(not_found()),
                }
            }
            "/api/actors/wakes" => return if m == "GET" { self.wakes_state(req).await } else { Ok(not_found()) },
            "/api/actors/heartbeat" => return if m == "POST" { self.heartbeat(req).await } else { Ok(not_found()) },
            "/api/actors/targets" => return if m == "POST" { self.targets(req).await } else { Ok(not_found()) },
            "/api/actors/requests" => {
                if m != "GET" {
                    return Ok(not_found());
                }
                if let Some(r) = self.window_only(req).await {
                    return Ok(r);
                }
                return Ok(Reply::json(200, self.registration().requests()?));
            }
            "/api/actors/tokens" if m == "GET" => {
                if let Some(r) = self.window_only(req).await {
                    return Ok(r);
                }
                let t = self.ctx.now();
                return Ok(Reply::json(200, json!({ "schema": "actor/tokens/v1", "at": xenonet_base::json::number(t), "tokens": self.actors().tokens(t)? })));
            }
            "/api/actors/observers" => {
                return match m {
                    "GET" => Ok(Reply::json(200, self.registration().observers()?)),
                    "POST" => self.observe(req).await,
                    _ => Ok(not_found()),
                }
            }
            "/api/actors/gateway" => return if m == "GET" || m == "POST" { self.gateway_state(req).await } else { Ok(not_found()) },
            _ => {}
        }
        let rest = p.strip_prefix("/api/actors/").unwrap_or("");
        let parts: Vec<&str> = rest.split('/').collect();
        let one = |s: &str| !s.is_empty();
        match parts.as_slice() {
            [slug, "wake", "test"] if one(slug) => {
                if m == "POST" {
                    self.wake(req, slug, true).await
                } else {
                    Ok(not_found())
                }
            }
            [slug, "wake"] if one(slug) => {
                if m == "PUT" || m == "DELETE" {
                    self.wake(req, slug, false).await
                } else {
                    Ok(not_found())
                }
            }
            [slug, "gateway"] if one(slug) => {
                if m == "POST" || m == "DELETE" {
                    self.gateway_token(req, slug).await
                } else {
                    Ok(not_found())
                }
            }
            [slug, "gateway", "ask"] if one(slug) => {
                if m == "POST" {
                    self.gateway_ask(req, slug).await
                } else {
                    Ok(not_found())
                }
            }
            ["requests", id] if one(id) => {
                if m == "POST" {
                    self.decide(req, id).await
                } else {
                    Ok(not_found())
                }
            }
            [slug, "token"] if one(slug) => {
                if m == "POST" {
                    self.rotate(req, slug).await
                } else {
                    Ok(not_found())
                }
            }
            [slug] if one(slug) && m == "GET" => {
                let found = match slug_of(slug) {
                    Some(s) => self.actors().snapshot(&format!("actor:{s}"), None, true)?,
                    None => None,
                };
                Ok(match found {
                    Some(f) if !f.is_null() => Reply::json(200, f),
                    _ => fail(404, "not-found", "XenoNet has no such actor", &[], &none()),
                })
            }
            [slug] if one(slug) && m == "DELETE" => self.remove(req, slug).await,
            _ => Ok(not_found()),
        }
    }
}

pub fn you_reports_bucket(now: f64) -> crate::buckets::Bucket {
    crate::buckets::Bucket::new(YOU_REPORTS, now)
}
