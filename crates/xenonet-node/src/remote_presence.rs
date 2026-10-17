//! xenonet-actors' remote presence for the twin of
//! core/actors/remote-presence.js: what a remote actor's reads say of it.

use crate::args::JsArg;
use crate::peer::{rethrow, settle, SyncCalls};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::cell::RefCell;
use xenonet_actors::remote_presence::{remote_instance, RemotePresence};
use xenonet_engine::peer::list;

#[napi(js_name = "remoteInstance")]
pub fn instance(connection_id: JsArg) -> String {
    remote_instance(&connection_id.0)
}

#[napi]
pub struct RemotePresenceCore {
    inner: RefCell<RemotePresence>,
}

#[napi]
impl RemotePresenceCore {
    #[napi(constructor)]
    pub fn new() -> Self {
        RemotePresenceCore { inner: RefCell::new(RemotePresence::default()) }
    }

    /// A read went out: `deps` has the actors and onRead.
    #[napi]
    #[allow(clippy::too_many_arguments)]
    pub fn read(&self, env: Env, deps: Object, actor_id: JsArg, connection_id: JsArg, targets: JsArg, reach: JsArg, at: f64, on_read: bool) -> Result<()> {
        let mut p = self.inner.try_borrow_mut().map_err(|_| Error::new(Status::GenericFailure, "the remote presence was asked again while it was busy".to_string()))?;
        let out = p.read(&mut SyncCalls::new(&env, &deps), &actor_id.0, &connection_id.0, &list(targets.0), &list(reach.0), at, on_read);
        let out = out.map_err(|t| rethrow(&env, t));
        settle(&env);
        out
    }
}
