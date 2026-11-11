//! Registering actors (crates/xenonet-actors/src/registration.rs): their
//! requests, tokens, the gateway's asks and the limits. The registry and
//! the clock answer its host, and so does xenonet-authz: tokens are
//! minted from this start's token root and decided on the engine's
//! ground; its rows are the engine store's; what it
//! tells (requests, asked, asked-gateway) goes to the page once the call
//! is done.

use crate::actors::{row_of, rows_json, thrown, Rows};
use crate::runtime::Runtime;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use xenonet_actors::registration::{Checked, Fail, Host, Registration as Core, Store};
use xenonet_actors::store as rows;
use xenonet_api::cores::{CoreError, Kind, Registration, R};
use xenonet_api::From;
use xenonet_authz::{decide, Act, Action, Door, Principal, Request, Resource, Token};
use xenonet_engine::engine::Thrown;
use xenonet_os::sync::Locked;
use xenonet_store::engine::Shared;
use xenonet_text::text::display;
use xenonet_text::time::now_ms;

/// What registering asks of the world: the registry, the clock, and its
/// own observing().
struct RegHost<'a> {
    w: &'a Runtime,
}

/// A registry failure, as a registration call carries it on.
fn registry_fail(t: Thrown) -> Fail {
    Fail::Error(rows::Error { message: t.message, code: t.code })
}

impl RegHost<'_> {
    fn actors(&self) -> Option<&crate::actors::ActorSide> {
        self.w.actors.as_ref()
    }
    /// A read of the registry, or `none` without one.
    fn read<T>(&self, none: T, f: impl FnOnce(&xenonet_actors::registry::Actors) -> T) -> T {
        self.actors().map_or(none, |a| a.read(f))
    }
    /// A change to the registry, with what it told told once it's done.
    fn change<T>(&self, f: impl FnOnce(&mut xenonet_actors::registry::Actors, &mut crate::actors::ActorHost<'_>) -> xenonet_actors::registry::Result<T>) -> Result<T, Fail> {
        let Some(a) = self.actors() else { return Err(Fail::Error(rows::Error::new("no actors"))) };
        a.with(self.w, f).map_err(registry_fail)
    }
}

impl Host for RegHost<'_> {
    fn now(&mut self) -> f64 {
        now_ms()
    }
    fn demo(&mut self) -> bool {
        self.actors().is_some_and(|a| a.is_demo)
    }
    fn observing(&mut self, product: &str) -> Result<bool, Fail> {
        Core::observing(self, product)
    }
    fn info(&mut self, id: &str) -> Value {
        self.read(Value::Null, |r| r.info(id))
    }
    fn is_registered(&mut self, id: &str) -> bool {
        self.read(false, |r| r.is_registered(id))
    }
    fn registered_count(&mut self) -> usize {
        self.read(0, xenonet_actors::registry::Actors::registered_count)
    }
    fn upsert(&mut self, def: &Value) -> Result<(), Fail> {
        self.change(|r, h| r.upsert(def, h))
    }
    fn watch_pid(&mut self, id: &str, pid: &Value) -> Result<(), Fail> {
        let Some(a) = self.actors() else { return Err(Fail::Error(rows::Error::new("no actors"))) };
        a.reg.locked().watch_pid(id, pid).map_err(|f| registry_fail(thrown(f)))
    }
    fn snapshot(&mut self, id: &str) -> Result<Value, Fail> {
        self.change(|r, h| r.snapshot(id, "public", Some(now_ms()), None, h))
    }
    fn allow(&mut self, id: &str, at: f64) -> Result<(), Fail> {
        self.change(|r, _| r.allow(id, at))
    }
    fn has(&mut self, id: &str) -> bool {
        self.read(false, |r| r.has(id))
    }
    fn only_observed(&mut self, id: &str) -> bool {
        self.read(false, |r| r.only_observed(id))
    }
    fn unregister(&mut self, id: &str, keep_remote: bool) -> Result<(), Fail> {
        let Some(a) = self.actors() else { return Err(Fail::Error(rows::Error::new("no actors"))) };
        a.unregister(self.w, id, keep_remote).map_err(registry_fail)
    }
    fn remove(&mut self, id: &str) -> Result<(), Fail> {
        let Some(a) = self.actors() else { return Err(Fail::Error(rows::Error::new("no actors"))) };
        a.remove(self.w, id).map_err(registry_fail)
    }
    fn of_product(&mut self, product: &str) -> Vec<String> {
        self.read(Vec::new(), |r| r.of_product(Some(&Value::String(product.into()))))
    }
    fn enabled(&mut self, id: &str) -> bool {
        self.read(false, |r| r.enabled(id))
    }
    fn set_enabled(&mut self, id: &str, on: bool) -> Result<(), Fail> {
        self.change(|r, _| r.set_enabled(id, on))
    }
    fn present(&mut self, slug: &str, capabilities: &[String], gateway: bool, reason: &str) -> Result<(), Fail> {
        let mut rights = xenonet_actors::tokens::rights(capabilities);
        if gateway {
            rights.push(Action::UseGateway);
        }
        // Only the window's routes issue tokens.
        crate::ground::present(self.w, true, slug, rights, crate::grants::scopes_hash(&json!({})), reason)
            .map_err(|why| Fail::Actor(xenonet_actors::registration::ActorError { status: 403.0, code: "presence", message: why, extra: json!({}) }))
    }
    fn mint(&mut self, slug: &str, capabilities: &[String], gateway: bool) -> Result<(String, String), Fail> {
        let mut rights = xenonet_actors::tokens::rights(capabilities);
        let (door, prefix) = if gateway {
            rights.push(Action::UseGateway);
            (Some(Door::Gateway), xenonet_authz::GATEWAY_PREFIX)
        } else {
            (None, xenonet_authz::ACTOR_PREFIX)
        };
        let token = crate::ground::mint(self.w, slug, rights, None, None, door).map_err(|why| Fail::Error(rows::Error::new(why)))?;
        let text = token.encode(prefix).map_err(|e| Fail::Error(rows::Error::new(e.message)))?;
        Ok((text, token.id()))
    }
    fn verify(&mut self, text: &str, gateway: bool) -> Checked {
        let Some(g) = crate::ground::ground(self.w, None) else { return Checked::Unknown };
        let (prefix, door) = if gateway { (xenonet_authz::GATEWAY_PREFIX, Door::Gateway) } else { (xenonet_authz::ACTOR_PREFIX, Door::Local) };
        let token = match Token::parse(text, prefix, &g, g.facts.now) {
            Ok(t) => t,
            Err(e) if e.code == "revoked" => return Checked::Revoked(Token::ids_of(text, prefix).unwrap_or_default()),
            Err(_) => return Checked::Unknown,
        };
        let own = g.facts.machine.clone();
        let (slug, machine) = (&token.bearer().slug, &token.bearer().machine);
        if machine != &own {
            return Checked::Unknown;
        }
        // What it may do at its door, decided now: the acts the routes ask
        // of a token (reads, messages, handoffs). Starts are decided with
        // the run they start, and claims in each project their paths are
        // in: here, a claim only needs its token to hold the right.
        let principal = Principal::Actor { slug: slug.clone(), machine: own.clone() };
        let mut allows: Vec<String> = [Act::Read { scope: xenonet_authz::ReadScope::Map, filters: None }, Act::Message, Act::Handoff]
            .into_iter()
            .filter(|act| decide(Some(&token), &Request::new(principal.clone(), Resource::Machine(own.clone()), act.clone(), door, g.facts.now, None), &g, None).is_allow())
            .map(|act| act.action().as_str().to_string())
            .collect();
        if token.rights().contains(&Action::Claim) {
            allows.push(Action::Claim.as_str().to_string());
        }
        Checked::Valid { slug: slug.as_str().to_string(), id: token.id(), allows }
    }
}

impl Store for Rows {
    fn waiting_actor_requests(&self, now: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.waiting_actor_requests(now)))
    }
    fn lapsed_actor_requests(&self, now: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.lapsed_actor_requests(now)))
    }
    fn actor_request(&self, id: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.actor_request(id)))
    }
    fn actor_request_by_token(&self, id: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.actor_request_by_token(id)))
    }
    fn add_actor_request(&self, r: &Value) -> rows::Result<()> {
        self.direct(|s| s.add_actor_request(r))
    }
    fn decide_actor_request(&self, id: &Value, decision: &str, at: f64) -> rows::Result<bool> {
        self.direct(|s| s.decide_actor_request(&display(id), decision, at))
    }
    fn actor_request_times(&self, since: f64) -> rows::Result<Vec<Value>> {
        self.direct(|s| s.actor_request_times(since))
    }
    fn revoke_token(&self, r: &Value) -> rows::Result<()> {
        self.direct(|s| s.revoke_token(r))
    }
    fn set_actor_token(&self, slug: &str, id: &Value, rights: &str, at: f64) -> rows::Result<()> {
        self.direct(|s| s.set_actor_token(slug, &display(id), rights, at))
    }
    fn drop_actor_token(&self, slug: &str) -> rows::Result<()> {
        self.direct(|s| s.drop_actor_token(slug))
    }
    fn touch_actor_token(&self, slug: &str, at: f64) -> rows::Result<()> {
        self.direct(|s| s.touch_actor_token(slug, at))
    }
    fn actor_by_token(&self, id: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.actor_by_token(id)))
    }
    fn actor_token(&self, slug: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.actor_token(slug)))
    }
    fn stale_actor_tokens(&self, before: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.stale_actor_tokens(before)))
    }
    fn revoked(&self, id: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.revoked(id)))
    }
    fn set_gateway_token(&self, slug: &str, id: &str, at: f64) -> rows::Result<()> {
        self.direct(|s| s.set_gateway_token(slug, id, at))
    }
    fn drop_gateway_token(&self, slug: &str) -> rows::Result<()> {
        self.direct(|s| s.drop_gateway_token(slug))
    }
    fn touch_gateway_token(&self, slug: &str, at: f64) -> rows::Result<()> {
        self.direct(|s| s.touch_gateway_token(slug, at))
    }
    fn actor_by_gateway_token(&self, id: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.actor_by_gateway_token(id)))
    }
    fn gateway_token(&self, slug: &str) -> rows::Result<Option<Value>> {
        row_of(self.0.with(|s| s.gateway_token(slug)))
    }
    fn stale_gateway_tokens(&self, before: f64) -> rows::Result<Vec<Value>> {
        rows_json(self.0.with(|s| s.stale_gateway_tokens(before)))
    }
    fn release_claims(&self, agent: &str) -> rows::Result<()> {
        self.direct(|s| s.release(agent, None)).map(|_| ())
    }
    fn clear_focus(&self, agent: &str, at: f64) -> rows::Result<()> {
        self.direct(|s| s.set_focus(agent, "", at)).map(|_| ())
    }
    fn unseen_count(&self, agent: &str) -> rows::Result<usize> {
        self.direct(|s| s.unseen(agent)).map(|r| r.len())
    }
}

/// An ActorError as the routes answer it: its status, code and extra
/// fields; anything else an Error.
pub fn actor_error(f: Fail) -> CoreError {
    match f {
        Fail::Actor(e) => CoreError::of(Kind::Actor, e.status as u16, e.code, e.message).with_extra(e.extra),
        Fail::Error(e) => CoreError { code: e.code, ..CoreError::new(e.message) },
    }
}

pub struct RegSide {
    core: Mutex<Core>,
}

impl RegSide {
    pub fn new(store: Shared, home: &str) -> RegSide {
        RegSide { core: Mutex::new(Core::new(Arc::new(Rows(store)), home)) }
    }

    /// A call that holds what's kept, with what it told told once it's done.
    fn with<T>(&self, w: &Runtime, f: impl FnOnce(&mut Core, &mut RegHost<'_>) -> Result<T, Fail>) -> Result<T, Fail> {
        let (out, emits) = {
            let mut core = self.core.locked();
            let out = f(&mut core, &mut RegHost { w });
            (out, core.take_emits())
        };
        for (name, v) in emits {
            w.emit(From::Registration, name, v);
        }
        out
    }

    /// A call that needs none of what's kept (observers asks observing
    /// while another runs).
    fn free<T>(w: &Runtime, f: impl FnOnce(&mut RegHost<'_>) -> Result<T, Fail>) -> Result<T, Fail> {
        f(&mut RegHost { w })
    }

    pub fn tick(&self, w: &Runtime) {
        let _ = self.with(w, |r, _| r.tick(now_ms()));
    }

    /// Whether the observer of a product is on, as the observers ask.
    pub fn observing(&self, w: &Runtime, product: &str) -> bool {
        Self::free(w, |c| Core::observing(c, product)).unwrap_or(true)
    }
}

/// `{ checked = false, chain = null, cwd = null }` of a verdict.
fn verdict_parts(v: &Value) -> (bool, Vec<Value>, Value) {
    let checked = xenonet_text::text::truthy(v.get("checked"));
    let chain = v.get("chain").and_then(Value::as_array).cloned().unwrap_or_default();
    let cwd = v.get("cwd").cloned().unwrap_or(Value::Null);
    (checked, chain, cwd)
}

pub struct RegistrationDriver(pub Arc<Runtime>);

impl RegistrationDriver {
    fn side(&self) -> R<&RegSide> {
        self.0.registration.as_ref().ok_or_else(|| CoreError::pending("core/actors/registration.js"))
    }

    fn run(&self, f: impl FnOnce(&mut Core, &mut RegHost<'_>) -> Result<Value, Fail>) -> R<Value> {
        self.side()?.with(&self.0, f).map_err(actor_error)
    }
}

/// What authenticate gives for a call that failed: a refusal, never a pass.
fn refusal(f: Fail) -> Value {
    match f {
        Fail::Actor(e) => json!({ "ok": false, "status": xenonet_text::json::number(e.status), "code": e.code, "error": e.message }),
        Fail::Error(e) => json!({ "ok": false, "status": 500, "code": "error", "error": e.message }),
    }
}

impl Registration for RegistrationDriver {
    fn authenticate(&self, token: &str) -> Value {
        match self.side() {
            Ok(s) => s.with(&self.0, |r, c| r.authenticate(c, token)).unwrap_or_else(refusal),
            Err(e) => json!({ "ok": false, "code": "pending", "error": e.message }),
        }
    }
    fn authenticate_gateway(&self, token: &str) -> Value {
        match self.side() {
            Ok(s) => s.with(&self.0, |r, c| r.authenticate_gateway(c, token)).unwrap_or_else(refusal),
            Err(e) => json!({ "ok": false, "status": 501, "code": "pending", "error": e.message }),
        }
    }
    fn product_of(&self, auth: &Value) -> Value {
        match self.side() {
            Ok(_) => RegSide::free(&self.0, |c| Core::product_of(c, auth)).unwrap_or(Value::Null),
            Err(_) => Value::Null,
        }
    }
    fn register(&self, body: &Value, verdict: &Value) -> R<Value> {
        let (checked, chain, cwd) = verdict_parts(verdict);
        self.run(|r, c| r.register(c, body, checked, &chain, &cwd))
    }
    fn me(&self, auth: &Value, extra: Option<Value>) -> R<Value> {
        let extra = extra.unwrap_or_else(|| json!({}));
        self.run(|r, c| r.me(c, auth, &extra))
    }
    fn remove(&self, slug: &str, reason: Option<&str>) -> R<Value> {
        let reason = reason.unwrap_or("removed");
        self.run(|r, c| r.remove(c, slug, reason))
    }
    fn add(&self, body: &Value) -> R<Value> {
        self.run(|r, c| r.add(c, body))
    }
    fn decide(&self, request: &str, decision: &Value) -> R<Value> {
        self.run(|r, c| r.decide(c, request, decision))
    }
    fn rotate(&self, slug: &str) -> R<Value> {
        self.run(|r, c| r.rotate(c, slug))
    }
    fn drop_gateway(&self, slug: &str) -> R<Value> {
        self.run(|r, c| r.drop_gateway(c, slug))
    }
    fn gateway(&self, slug: &str, capabilities: Option<Value>) -> R<Value> {
        let body = match capabilities {
            Some(c) => json!({ "capabilities": c }),
            None => json!({}),
        };
        self.run(|r, c| r.gateway(c, slug, &body))
    }
    fn ask_gateway(&self, slug: &str, verdict: &Value) -> R<Value> {
        let (checked, chain, cwd) = verdict_parts(verdict);
        self.run(|r, c| r.ask_gateway(c, slug, checked, &chain, &cwd))
    }
    fn requests(&self) -> R<Value> {
        self.run(|r, c| r.requests(c))
    }
    fn observers(&self) -> R<Value> {
        self.side()?;
        RegSide::free(&self.0, |c| Core::observers(c)).map_err(actor_error)
    }
    fn observe(&self, product: &Value, on: &Value) -> R<Value> {
        self.side()?;
        RegSide::free(&self.0, |c| Core::observe(c, product, on)).map_err(actor_error)
    }
    fn waiting(&self) -> usize {
        self.side().ok().and_then(|s| s.with(&self.0, |r, _| r.waiting(now_ms())).ok()).unwrap_or(0)
    }
    fn simulate(&self, kind: &str, a: Option<String>) -> R<Option<Value>> {
        let v = self.run(|r, c| r.simulate(c, kind, a.as_deref()))?;
        Ok((!v.is_null()).then_some(v))
    }
    fn tick(&self) {
        if let Ok(s) = self.side() {
            s.tick(&self.0);
        }
    }
}
