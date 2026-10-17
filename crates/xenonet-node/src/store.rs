//! xenonet-engine's store for the twin of core/engine/store.js: the
//! store's calls by their JS names, its database as a DatabaseSync of
//! node:sqlite's kind (for the tests that look into it), and the migration
//! steps as the JS ran them on a database of its own.

use crate::args::{throw_sql, JsArg, JsArgs, Out};
use crate::blocks::HasLife;
use crate::js::Json;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::{json, Value};
use xenonet_engine::sql::{self, Arg, SqlError};
use xenonet_engine::store::{engine_step, EngineStore, Shared, ENGINE_VERSION};

#[napi]
pub struct EngineStoreCore {
    pub(crate) inner: Shared,
}

fn a(args: &[Arg], i: usize) -> Arg {
    args.get(i).cloned().unwrap_or(Arg::Undef)
}

fn rows(r: sql::Result<Vec<sql::Row>>) -> sql::Result<Out> {
    r.map(Out::Rows)
}

fn row(r: sql::Result<Option<sql::Row>>) -> sql::Result<Out> {
    r.map(Out::Row)
}

fn unit(r: sql::Result<()>) -> sql::Result<Out> {
    r.map(|_| Out::Undef)
}

fn dispatch(s: &mut EngineStore, name: &str, v: &[Arg]) -> sql::Result<Out> {
    let x = |i| a(v, i);
    match name {
        "pruneAudit" => unit(s.prune_audit(x(0).as_f64().unwrap_or(f64::NAN))),
        "touch" => unit(s.touch(x(0), &x(1), x(2))),
        "ended" => unit(s.ended(x(0), x(1))),
        "contact" => row(s.contact(x(0))),
        "contacts" => rows(s.contacts(x(0))),
        "pins" => rows(s.pins(x(0))),
        "pinnedTo" => rows(s.pinned_to(x(0))),
        "pin" => unit(s.pin(x(0), &x(1), x(2), x(3))),
        "dropGuesses" => unit(s.drop_guesses(x(0))),
        "unpin" => unit(s.unpin(x(0), &x(1))),
        "send" => s.send(&x(0), x(1)).map(Out::Num),
        "deliver" => unit(s.deliver(x(0), x(1))),
        "scoped" => rows(s.scoped(x(0), x(1))),
        "message" => row(s.message(x(0))),
        "unseen" => rows(s.unseen(x(0))),
        "markSeen" => unit(s.mark_seen(x(0), &x(1), x(2))),
        "markUnseen" => unit(s.mark_unseen(x(0), &x(1))),
        "deliveries" => rows(s.deliveries(x(0))),
        "recent" => rows(s.recent(x(0))),
        "claim" => unit(s.claim(x(0), x(1), x(2), x(3), x(4))),
        "release" => s.release(x(0), &x(1)).map(Out::Num),
        "claims" => rows(s.claims(x(0))),
        "setFocus" => s.set_focus(x(0), x(1), x(2)).map(|c| c.map_or(Out::Undef, Out::Changes)),
        "focus" => row(s.focus(x(0))),
        "actorRows" => rows(s.actor_rows()),
        "actorRow" => row(s.actor_row(x(0))),
        "saveActor" => unit(s.save_actor(&x(0))),
        "removeActor" => unit(s.remove_actor(x(0))),
        "forgetActor" => unit(s.forget_actor(x(0))),
        "addActorEvent" => unit(s.add_actor_event(&x(0))),
        "lastActorEvents" => rows(s.last_actor_events(x(0), x(1))),
        "actorEventsSince" => rows(s.actor_events_since(x(0), &x(1))),
        "actorEventCounts" => rows(s.actor_event_counts(x(0))),
        "actorSpawns" => rows(s.actor_spawns(x(0))),
        "hasActorRef" => s.has_actor_ref(x(0), x(1), x(2)).map(Out::Bool),
        "actorTargets" => rows(s.actor_targets()),
        "saveActorTarget" => unit(s.save_actor_target(&x(0))),
        "dropActorTarget" => unit(s.drop_actor_target(x(0), x(1), x(2))),
        "actorFactsSince" => rows(s.actor_facts_since(x(0), x(1), x(2))),
        "setActorToken" => unit(s.set_actor_token(x(0), x(1), x(2))),
        "dropActorToken" => unit(s.drop_actor_token(x(0))),
        "touchActorToken" => unit(s.touch_actor_token(x(0), x(1))),
        "actorByToken" => row(s.actor_by_token(x(0))),
        "actorTokenTimes" => s.actor_token_times().map(|r| Out::Json(Value::Array(r))),
        "actorToken" => row(s.actor_token(x(0))),
        "staleActorTokens" => rows(s.stale_actor_tokens(x(0))),
        "expiredActorTokens" => s.expired_actor_tokens().map(|r| Out::Json(Value::Array(r))),
        "addActorRequest" => unit(s.add_actor_request(&x(0))),
        "actorRequest" => row(s.actor_request(x(0))),
        "actorRequestByToken" => row(s.actor_request_by_token(x(0))),
        "waitingActorRequests" => rows(s.waiting_actor_requests(x(0))),
        "lapsedActorRequests" => rows(s.lapsed_actor_requests(x(0))),
        "decideActorRequest" => s.decide_actor_request(x(0), x(1), x(2)).map(Out::Bool),
        "actorRequestTimes" => s.actor_request_times(x(0)).map(|r| Out::Json(Value::Array(r))),
        "setGatewayToken" => unit(s.set_gateway_token(x(0), x(1), x(2))),
        "dropGatewayToken" => unit(s.drop_gateway_token(x(0))),
        "touchGatewayToken" => unit(s.touch_gateway_token(x(0), x(1))),
        "actorByGatewayToken" => row(s.actor_by_gateway_token(x(0))),
        "gatewayToken" => row(s.gateway_token(x(0))),
        "staleGatewayTokens" => rows(s.stale_gateway_tokens(x(0))),
        "revokeActorToken" => unit(s.revoke_actor_token(&x(0))),
        "revokedActorToken" => row(s.revoked_actor_token(x(0))),
        "meta" => s.meta(x(0)).map(Out::Json),
        "setMeta" => unit(s.set_meta(x(0), x(1))),
        "projectIdFor" => s.project_id_for(x(0)).map(Out::Json),
        "placeOfProject" => s.place_of_project(x(0)).map(Out::Json),
        "addConnection" => unit(s.add_connection(&x(0))),
        "connection" => row(s.connection(x(0))),
        "connectionsOf" => rows(s.connections_of(x(0))),
        "connections" => rows(s.connections()),
        "setTransportBinding" => unit(s.set_transport_binding(x(0), x(1))),
        "endConnection" => unit(s.end_connection(x(0), x(1))),
        "addGrant" => unit(s.add_grant(&x(0))),
        "grant" => row(s.grant(x(0))),
        "grants" => rows(s.grants()),
        "changeGrant" => s.change_grant(x(0), &x(1)).map(Out::Bool),
        "revokeGrant" => s.revoke_grant(x(0), x(1), x(2)).map(Out::Bool),
        "touchGrant" => unit(s.touch_grant(x(0), x(1))),
        "addAccountLink" => unit(s.add_account_link(&x(0))),
        "accountLinks" => rows(s.account_links(x(0))),
        "dropAccountLink" => unit(s.drop_account_link(x(0))),
        "expireAccountLink" => unit(s.expire_account_link(x(0), x(1))),
        "addExternalRef" => unit(s.add_external_ref(&x(0))),
        "externalRef" => row(s.external_ref(&x(0))),
        "externalRefs" => rows(s.external_refs(x(0))),
        "addAudit" => unit(s.add_audit(&x(0))),
        "audit" => rows(s.audit(&x(0))),
        other => Err(SqlError::Other(format!("the store has no call {other}"))),
    }
}

#[napi]
impl EngineStoreCore {
    /// Opens (and migrates and prunes) `file`.
    #[napi(constructor)]
    pub fn new(file: String, now: f64) -> Result<Self> {
        EngineStore::open(&file, now).map(|s| EngineStoreCore { inner: Shared::new(s) }).map_err(|e| Error::new(Status::GenericFailure, e))
    }

    /// {from, version, newer}.
    #[napi(getter)]
    pub fn version(&self) -> Json {
        Json(self.inner.with(|s| s.version()).map_or(Value::Null, |v| json!({ "from": v.from, "version": v.version, "newer": v.newer })))
    }

    #[napi(getter, js_name = "backupFile")]
    pub fn backup_file(&self) -> Option<String> {
        self.inner.with(|s| s.block.backup_file.clone())
    }

    #[napi(getter)]
    pub fn file(&self) -> String {
        self.inner.with(|s| s.block.file.clone())
    }

    #[napi(getter, js_name = "isOpen")]
    pub fn is_open(&self) -> bool {
        self.inner.with(|s| s.is_open())
    }

    #[napi(getter, js_name = "inTransaction")]
    pub fn in_transaction(&self) -> bool {
        self.inner.with(|s| s.in_transaction())
    }

    /// One of the store's calls, by its JS name, with its JS arguments.
    #[napi]
    pub fn call(&self, env: Env, name: String, args: JsArgs) -> Result<Out> {
        self.inner.with(|s| dispatch(s, &name, &args.0)).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn prune(&self, env: Env, now: f64) -> Result<()> {
        self.inner.with(|s| s.prune(now)).map_err(|e| throw_sql(&env, &e))
    }

    /// `fn` in a transaction of its own, unless one is under way already.
    #[napi]
    pub fn transaction<'a>(&self, env: Env, f: Function<'a, (), Unknown<'a>>) -> Result<Unknown<'a>> {
        let mut thrown: Option<Error> = None;
        let out = self.inner.transaction(|| -> std::result::Result<Option<Unknown<'a>>, SqlError> {
            match f.call(()) {
                Ok(v) => Ok(Some(v)),
                Err(e) => {
                    thrown = Some(e);
                    Err(SqlError::Other("thrown".into()))
                }
            }
        });
        match (out, thrown) {
            (_, Some(e)) => Err(e),
            (Ok(Some(v)), None) => Ok(v),
            (Ok(None), None) => unreachable!(),
            (Err(e), None) => Err(throw_sql(&env, &e)),
        }
    }

    /// A place's own id, made by `make` the first time it's asked for.
    #[napi(js_name = "projectIdOf")]
    pub fn project_id_of(&self, env: Env, place: JsArg, make: Function<(), JsArg>, now: JsArg) -> Result<Json> {
        match self.inner.with(|s| s.project_id_for(place.0.clone())) {
            Ok(Value::Null) => {}
            Ok(v) => return Ok(Json(v)),
            Err(e) => return Err(throw_sql(&env, &e)),
        }
        let id = make.call(()).map(|v| v.0)?;
        self.inner.with(|s| s.project_id_of::<SqlError>(place.0, &mut || Ok(id.clone()), now.0)).map(Json).map_err(|e| throw_sql(&env, &e))
    }

    /// Checks a statement prepares (DatabaseSync.prepare).
    #[napi]
    pub fn prepare(&self, env: Env, sql_text: String) -> Result<()> {
        self.inner.with(|s| s.db().and_then(|db| sql::prepare(db, &sql_text))).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn all(&self, env: Env, sql_text: String, args: JsArgs) -> Result<Out> {
        self.inner.with(|s| s.db().and_then(|db| sql::all(db, &sql_text, &args.0))).map(Out::Rows).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn get(&self, env: Env, sql_text: String, args: JsArgs) -> Result<Out> {
        self.inner.with(|s| s.db().and_then(|db| sql::get(db, &sql_text, &args.0))).map(Out::First).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn run(&self, env: Env, sql_text: String, args: JsArgs) -> Result<Out> {
        self.inner.with(|s| s.db().and_then(|db| sql::run(db, &sql_text, &args.0))).map(Out::Changes).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn exec(&self, env: Env, sql_text: String) -> Result<()> {
        self.inner.with(|s| s.db().and_then(|db| sql::exec(db, &sql_text))).map_err(|e| throw_sql(&env, &e))
    }

    #[napi]
    pub fn close(&self, now: f64) {
        self.inner.with(|s| s.close(now));
    }
}

#[napi(js_name = "engineVersion")]
pub fn engine_version() -> u32 {
    ENGINE_VERSION as u32
}

/// The statements migration step `i` runs, asking `has_column(table,
/// column)` of the database it's for.
#[napi(js_name = "engineStep")]
pub fn engine_step_sql(i: u32, has_column: Function<FnArgs<(String, String)>, bool>) -> Result<Vec<&'static str>> {
    engine_step(i as usize, &mut |t, c| has_column.call(FnArgs::from((t.to_string(), c.to_string()))))
}

/// The store's block, for the list of blocks and their health: its
/// lifecycle as opening it left it.
#[napi]
pub struct EngineBlockCore {
    life: xenonet_blocks::lifecycle::Lifecycle,
}

impl crate::blocks::HasLife for EngineBlockCore {
    fn life(&self) -> &xenonet_blocks::lifecycle::Lifecycle {
        &self.life
    }
    fn life_mut(&mut self) -> &mut xenonet_blocks::lifecycle::Lifecycle {
        &mut self.life
    }
}

crate::blocks::lifecycle_api!(EngineBlockCore);

#[napi]
impl EngineStoreCore {
    #[napi]
    pub fn block(&self) -> EngineBlockCore {
        EngineBlockCore { life: self.inner.with(|s| s.block.life.clone()) }
    }
}
