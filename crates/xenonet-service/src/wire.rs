//! What the binary's drivers for the cores are given (crates/xenonet-wire-*):
//! the service's settings, its version, the drift log and vendor readers
//! they share, and the way their events and log lines reach the service,
//! which exists only once the cores do.

use crate::ctx::Ctx;
use crate::glue::{self, From};
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use xenonet_schema::drift::DriftLog;
use xenonet_schema::vendor::Readers;

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

    /// Ties the emitter to the service once it's up: `xenonet serve` does,
    /// and the tests that read the events.
    pub fn bind(&self, ctx: &Arc<Ctx>) {
        let _ = self.0.set(Arc::downgrade(ctx));
    }
}

/// The lenient readers of the vendors' sources, one for each source
/// (core/vendor.js's one `readers` Map). The map's driver makes them, over
/// XenoNet's schemas folder, and the actors' observers check what they read
/// through the same ones, so the drift report counts what each has read.
#[derive(Clone, Default)]
pub struct VendorReaders(Arc<OnceLock<Arc<Readers>>>);

impl VendorReaders {
    /// The readers, made with `make` by whichever driver asks first (the
    /// map's, in the service; another driver's when it runs without one).
    pub fn get_or_make(&self, make: impl FnOnce() -> Readers) -> Arc<Readers> {
        self.0.get_or_init(|| Arc::new(make())).clone()
    }
}

/// What every driver is given beside the cores and the settings.
#[derive(Clone)]
pub struct WireEnv {
    pub version: String,
    pub emitter: Emitter,
    /// The drift log (core/drift.js's one log): the readers record what
    /// they find in it, and the service's routes and the doctor read it.
    /// A driver that knows the cache folder first opens it there
    /// (`cache_dir/drift.json`); the service opens it otherwise.
    pub drift: Arc<Mutex<DriftLog>>,
    /// The vendor readers, which record in `drift`.
    pub readers: VendorReaders,
}
