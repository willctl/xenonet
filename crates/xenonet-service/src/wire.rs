//! What the binary's drivers for the cores are given (crates/xenonet-wire-*):
//! the service's settings, its version, and the way their events and log
//! lines reach the service, which exists only once the cores do.

use crate::ctx::Ctx;
use crate::glue::{self, From};
use serde_json::Value;
use std::sync::{Arc, OnceLock, Weak};

/// Where a core's events go: server/index.js's handlers (glue.rs), once the
/// service is up. An event before that has no page to reach and is let go.
#[derive(Clone, Default)]
pub struct Emitter(Arc<OnceLock<Weak<Ctx>>>);

impl Emitter {
    pub fn new() -> Emitter {
        Emitter::default()
    }

    pub fn emit(&self, from: From, name: &str, data: Value) {
        if let Some(ctx) = self.ctx() {
            glue::event(&ctx, from, name, data);
        }
    }

    /// A line in the service's error log (level error, warn or info).
    pub fn log(&self, level: &str, code: &str, message: &str) {
        if let Some(ctx) = self.ctx() {
            ctx.log(level, code, message, None, None);
        }
    }

    /// The service, once it's up: for a core that needs more of it (the
    /// pages' count, the desktop's notifications).
    pub fn ctx(&self) -> Option<Arc<Ctx>> {
        self.0.get().and_then(Weak::upgrade)
    }

    pub(crate) fn bind(&self, ctx: &Arc<Ctx>) {
        let _ = self.0.set(Arc::downgrade(ctx));
    }
}

/// What every driver is given beside the cores and the settings.
#[derive(Clone)]
pub struct WireEnv {
    pub version: String,
    pub emitter: Emitter,
}
