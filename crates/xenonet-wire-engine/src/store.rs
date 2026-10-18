//! The engine's database (core/engine/store.js over engine.sqlite,
//! crates/xenonet-engine/src/store.rs): its calls by their JS names, for
//! the cores that call "engine.store", "actors.store" and the like, and the
//! service's own reads of it (the EngineStore trait).

use crate::js::{at, rows};
use serde_json::Value;
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::sql::{self, Arg, SqlError};
use xenonet_engine::store::{EngineStore, Shared};
use xenonet_service::cores::{CoreError, EngineStore as Trait, R};

fn row(r: sql::Result<Option<sql::Row>>) -> sql::Result<Arg> {
    r.map(|r| r.as_ref().map_or(Arg::Null, crate::js::row))
}

fn many(r: sql::Result<Vec<sql::Row>>) -> sql::Result<Arg> {
    r.map(|r| rows(&r))
}

fn unit(r: sql::Result<()>) -> sql::Result<Arg> {
    r.map(|_| Arg::Undef)
}

fn json(r: sql::Result<Value>) -> sql::Result<Arg> {
    r.map(|v| crate::js::arg(&v))
}

/// One of the store's calls, by its JS name, with the JS defaults the twin
/// fills in (`now` is Date.now() where the JS left it out).
fn dispatch(s: &mut EngineStore, name: &str, v: &[Arg]) -> sql::Result<Arg> {
    let x = |i| at(v, i);
    let now = |i: usize| match at(v, i) {
        Arg::Undef => Arg::Num(crate::js::now_ms()),
        a => a,
    };
    match name {
        "prune" => s.prune(now(0).as_f64().unwrap_or(f64::NAN)).map(|_| Arg::Undef),
        "pruneAudit" => unit(s.prune_audit(now(0).as_f64().unwrap_or(f64::NAN))),
        "touch" => unit(s.touch(x(0), &x(1), now(2))),
        "ended" => unit(s.ended(x(0), now(1))),
        "contact" => row(s.contact(x(0))),
        "contacts" => many(s.contacts(x(0))),
        "pins" => many(s.pins(x(0))),
        "pinnedTo" => many(s.pinned_to(x(0))),
        "pin" => unit(s.pin(x(0), &x(1), x(2), now(3))),
        "dropGuesses" => unit(s.drop_guesses(x(0))),
        "unpin" => unit(s.unpin(x(0), &x(1))),
        "send" => s.send(&x(0), now(1)).map(Arg::Num),
        "deliver" => unit(s.deliver(x(0), x(1))),
        "scoped" => many(s.scoped(x(0), x(1))),
        "message" => row(s.message(x(0))),
        "unseen" => many(s.unseen(x(0))),
        "markSeen" => unit(s.mark_seen(x(0), &x(1), now(2))),
        "markUnseen" => unit(s.mark_unseen(x(0), &x(1))),
        "deliveries" => many(s.deliveries(x(0))),
        "recent" => many(s.recent(x(0))),
        "claim" => unit(s.claim(x(0), x(1), x(2), x(3), now(4))),
        "release" => {
            let p = match x(1) {
                Arg::Undef => Arg::Null,
                p => p,
            };
            s.release(x(0), &p).map(Arg::Num)
        }
        "claims" => many(s.claims(now(0))),
        "setFocus" => s
            .set_focus(x(0), x(1), now(2))
            .map(|c| c.map_or(Arg::Undef, |c| crate::js::obj(vec![("lastInsertRowid", Arg::Num(c.last_insert_rowid as f64)), ("changes", Arg::Num(c.changes as f64))]))),
        "focus" => row(s.focus(x(0))),
        "actorRows" => many(s.actor_rows()),
        "actorRow" => row(s.actor_row(x(0))),
        "saveActor" => unit(s.save_actor(&x(0))),
        "removeActor" => unit(s.remove_actor(x(0))),
        "forgetActor" => unit(s.forget_actor(x(0))),
        "addActorEvent" => unit(s.add_actor_event(&x(0))),
        "lastActorEvents" => many(s.last_actor_events(
            x(0),
            match x(1) {
                Arg::Undef => Arg::Num(20.0),
                l => l,
            },
        )),
        "actorEventsSince" => many(s.actor_events_since(
            x(0),
            &match x(1) {
                Arg::Undef => Arg::Obj(vec![]),
                o => o,
            },
        )),
        "actorEventCounts" => many(s.actor_event_counts(x(0))),
        "actorSpawns" => many(s.actor_spawns(x(0))),
        "hasActorRef" => s.has_actor_ref(x(0), x(1), x(2)).map(Arg::Bool),
        "actorTargets" => many(s.actor_targets()),
        "saveActorTarget" => unit(s.save_actor_target(&x(0))),
        "dropActorTarget" => unit(s.drop_actor_target(x(0), x(1), x(2))),
        "actorFactsSince" => many(s.actor_facts_since(x(0), x(1), x(2))),
        "setActorToken" => unit(s.set_actor_token(x(0), x(1), x(2))),
        "dropActorToken" => unit(s.drop_actor_token(x(0))),
        "touchActorToken" => unit(s.touch_actor_token(x(0), x(1))),
        "actorByToken" => row(s.actor_by_token(x(0))),
        "actorTokenTimes" => s.actor_token_times().map(|r| Arg::List(r.iter().map(crate::js::arg).collect())),
        "actorToken" => row(s.actor_token(x(0))),
        "staleActorTokens" => many(s.stale_actor_tokens(x(0))),
        "expiredActorTokens" => s.expired_actor_tokens().map(|r| Arg::List(r.iter().map(crate::js::arg).collect())),
        "addActorRequest" => unit(s.add_actor_request(&x(0))),
        "actorRequest" => row(s.actor_request(x(0))),
        "actorRequestByToken" => row(s.actor_request_by_token(x(0))),
        "waitingActorRequests" => many(s.waiting_actor_requests(x(0))),
        "lapsedActorRequests" => many(s.lapsed_actor_requests(x(0))),
        "decideActorRequest" => s.decide_actor_request(x(0), x(1), x(2)).map(Arg::Bool),
        "actorRequestTimes" => s.actor_request_times(x(0)).map(|r| Arg::List(r.iter().map(crate::js::arg).collect())),
        "setGatewayToken" => unit(s.set_gateway_token(x(0), x(1), x(2))),
        "dropGatewayToken" => unit(s.drop_gateway_token(x(0))),
        "touchGatewayToken" => unit(s.touch_gateway_token(x(0), x(1))),
        "actorByGatewayToken" => row(s.actor_by_gateway_token(x(0))),
        "gatewayToken" => row(s.gateway_token(x(0))),
        "staleGatewayTokens" => many(s.stale_gateway_tokens(x(0))),
        "revokeActorToken" => unit(s.revoke_actor_token(&x(0))),
        "revokedActorToken" => row(s.revoked_actor_token(x(0))),
        "meta" => json(s.meta(x(0))),
        "setMeta" => unit(s.set_meta(x(0), x(1))),
        "projectIdFor" => json(s.project_id_for(x(0))),
        "placeOfProject" => json(s.place_of_project(x(0))),
        "addConnection" => unit(s.add_connection(&x(0))),
        "connection" => row(s.connection(x(0))),
        "connectionsOf" => many(s.connections_of(x(0))),
        "connections" => many(s.connections()),
        "setTransportBinding" => unit(s.set_transport_binding(x(0), x(1))),
        "endConnection" => unit(s.end_connection(x(0), x(1))),
        "addGrant" => unit(s.add_grant(&x(0))),
        "grant" => row(s.grant(x(0))),
        "grants" => many(s.grants()),
        "changeGrant" => s.change_grant(x(0), &x(1)).map(Arg::Bool),
        "revokeGrant" => s.revoke_grant(x(0), x(1), x(2)).map(Arg::Bool),
        "touchGrant" => unit(s.touch_grant(x(0), x(1))),
        "addAccountLink" => unit(s.add_account_link(&x(0))),
        "accountLinks" => many(s.account_links(x(0))),
        "dropAccountLink" => unit(s.drop_account_link(x(0))),
        "expireAccountLink" => unit(s.expire_account_link(x(0), x(1))),
        "addExternalRef" => unit(s.add_external_ref(&x(0))),
        "externalRef" => row(s.external_ref(&x(0))),
        "externalRefs" => many(s.external_refs(x(0))),
        "addAudit" => unit(s.add_audit(&x(0))),
        "audit" => many(s.audit(&match x(0) {
            Arg::Undef => Arg::Obj(vec![]),
            o => o,
        })),
        other => Err(SqlError::Other(format!("store.{other} is not a function"))),
    }
}

/// `store.<name>(...args)`, as a core's call to it.
pub fn call(store: &Shared, name: &str, args: &[Arg]) -> Result<Arg> {
    store.with(|s| dispatch(s, name, args)).map_err(Thrown::from)
}

/// The store's calls, by their JS names.
const CALLS: &[&str] = &[
    "prune",
    "pruneAudit",
    "touch",
    "ended",
    "contact",
    "contacts",
    "pins",
    "pinnedTo",
    "pin",
    "dropGuesses",
    "unpin",
    "send",
    "deliver",
    "scoped",
    "message",
    "unseen",
    "markSeen",
    "markUnseen",
    "deliveries",
    "recent",
    "claim",
    "release",
    "claims",
    "setFocus",
    "focus",
    "actorRows",
    "actorRow",
    "saveActor",
    "removeActor",
    "forgetActor",
    "addActorEvent",
    "lastActorEvents",
    "actorEventsSince",
    "actorEventCounts",
    "actorSpawns",
    "hasActorRef",
    "actorTargets",
    "saveActorTarget",
    "dropActorTarget",
    "actorFactsSince",
    "setActorToken",
    "dropActorToken",
    "touchActorToken",
    "actorByToken",
    "actorTokenTimes",
    "actorToken",
    "staleActorTokens",
    "expiredActorTokens",
    "addActorRequest",
    "actorRequest",
    "actorRequestByToken",
    "waitingActorRequests",
    "lapsedActorRequests",
    "decideActorRequest",
    "actorRequestTimes",
    "setGatewayToken",
    "dropGatewayToken",
    "touchGatewayToken",
    "actorByGatewayToken",
    "gatewayToken",
    "staleGatewayTokens",
    "revokeActorToken",
    "revokedActorToken",
    "meta",
    "setMeta",
    "projectIdFor",
    "placeOfProject",
    "addConnection",
    "connection",
    "connectionsOf",
    "connections",
    "setTransportBinding",
    "endConnection",
    "addGrant",
    "grant",
    "grants",
    "changeGrant",
    "revokeGrant",
    "touchGrant",
    "addAccountLink",
    "accountLinks",
    "dropAccountLink",
    "expireAccountLink",
    "addExternalRef",
    "externalRef",
    "externalRefs",
    "addAudit",
    "audit",
];

/// The kind of `store.<name>`, as a core asks before it calls one: the
/// store itself is an object, and each of its calls a function.
pub fn kind(name: &str) -> &'static str {
    if name.is_empty() {
        "object"
    } else if CALLS.contains(&name) {
        "function"
    } else {
        "undefined"
    }
}

fn core_error(e: SqlError) -> CoreError {
    CoreError::new(e.message().to_string())
}

/// The service's own reads of the engine's database.
pub struct Store(pub Shared);

impl Trait for Store {
    fn meta(&self, key: &str) -> R<Option<String>> {
        let v = self.0.with(|s| s.meta(Arg::text(key))).map_err(core_error)?;
        Ok(match v {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(xenonet_base::text::js_string(&other)),
        })
    }
    fn set_meta(&self, key: &str, value: &str) -> R<()> {
        self.0.with(|s| s.set_meta(Arg::text(key), Arg::text(value))).map_err(core_error)
    }
    fn actor_token_times(&self) -> R<Vec<Value>> {
        self.0.with(|s| s.actor_token_times()).map_err(core_error)
    }
    fn expired_actor_tokens(&self) -> R<Value> {
        self.0.with(|s| s.expired_actor_tokens()).map(Value::Array).map_err(core_error)
    }
    fn waiting_actor_requests(&self, now: f64) -> R<Vec<Value>> {
        self.0.with(|s| s.waiting_actor_requests(Arg::Num(now))).map(|r| r.iter().map(|r| crate::js::json(&crate::js::row(r))).collect()).map_err(core_error)
    }
    fn native(&self) -> Option<std::sync::Arc<dyn std::any::Any + Send + Sync>> {
        Some(std::sync::Arc::new(self.0.clone()))
    }
    fn connection(&self, id: &str) -> R<Option<Value>> {
        self.0.with(|s| s.connection(Arg::text(id))).map(|r| r.map(|r| crate::js::json(&crate::js::row(&r)))).map_err(core_error)
    }
}

/// A building block's health, as /api/health lists it: its name and kind,
/// then what its lifecycle says.
pub fn block_health(name: &str, kind: &str, l: &xenonet_blocks::lifecycle::Lifecycle) -> Value {
    let n = xenonet_base::json::number;
    let counters: serde_json::Map<String, Value> = l.counters.iter().map(|(k, v)| (k.to_string(), n(v))).collect();
    serde_json::json!({
        "name": name,
        "kind": kind,
        "state": l.state.as_str(),
        "since": n(l.since),
        "lastOk": l.last_ok.map(n),
        "lastError": l.last_error.as_ref().map(|e| serde_json::json!({ "at": n(e.at), "message": e.message })),
        "counters": counters,
        "budget": null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js::obj;

    #[test]
    fn calls_go_by_their_js_names_with_the_js_defaults() {
        let s = Shared::new(EngineStore::open(":memory:", 1000.0).unwrap());
        let m = obj(vec![("sender", Arg::text("claude:a")), ("target", Arg::text("codex:b")), ("body", Arg::text("hi")), ("recipients", Arg::List(vec![Arg::text("codex:b")]))]);
        assert_eq!(call(&s, "send", &[m, Arg::Num(2000.0)]).unwrap(), Arg::Num(1.0));
        let unseen = crate::js::list(call(&s, "unseen", &[Arg::text("codex:b")]).unwrap());
        assert_eq!(unseen.len(), 1);
        // An integer column reads as a number, as node:sqlite reads it.
        assert_eq!(unseen[0].field("created_at"), Arg::Num(2000.0));
        assert_eq!(call(&s, "message", &[Arg::Num(9.0)]).unwrap(), Arg::Null);
        assert_eq!(call(&s, "release", &[Arg::text("codex:b")]).unwrap(), Arg::Num(0.0));
        assert!(call(&s, "nope", &[]).unwrap_err().message.contains("is not a function"));
        assert_eq!((kind("unseen"), kind("nope")), ("function", "undefined"));
        let t = Store(s.clone());
        t.set_meta("gateway", "{}").unwrap();
        assert_eq!(t.meta("gateway").unwrap().as_deref(), Some("{}"));
        assert_eq!(t.meta("none").unwrap(), None);
        assert!(t.waiting_actor_requests(0.0).unwrap().is_empty());
    }
}
