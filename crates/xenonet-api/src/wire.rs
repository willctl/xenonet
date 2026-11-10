//! What the crates that implement the cores are given at composition:
//! the service's settings, its version, the drift log and vendor readers
//! they share, and the way their events and log lines reach the service,
//! which exists only once the cores do.

use crate::cores::{Decider, R};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use xenonet_schema::drift::DriftLog;
use xenonet_schema::vendor::Readers;

/// The events the cores send, by where they come from.
pub enum From {
    /// XenoNetState: pulse, activity, refusal, tokens, graph, live.
    State,
    /// The engine: message.
    Engine,
    /// The delivery router: delivery, error.
    Delivery,
    /// Actors: actors, event.
    Actors,
    /// Registration: requests, asked, asked-gateway.
    Registration,
    /// Wakes: mail (a message for an actor nothing can wake).
    Wakes,
    /// The hub: graph, live, activity, tokens, pulse, agent-message, hosts.
    Hub,
    /// The assistant's tunnels: changed.
    Tunnels,
    /// The doctor: result, problems.
    Doctor,
}

/// Where a driver's events and log lines reach the service once it's up.
/// The service implements it; the drivers see only this, so they never
/// depend on the service itself.
pub trait EventSink: Send + Sync {
    /// A core's event, as server/index.js's handlers take it: to the pages,
    /// and the desktop's notifications.
    fn event(&self, from: From, name: &str, data: Value);
    /// A line in the service's error log (level error, warn or info).
    fn log(&self, level: &str, code: &str, message: &str);
    /// An event for every page, as it is, for what no core's handler says
    /// (a remote actor's read, a theme's edit).
    fn broadcast(&self, event: &str, data: &Value);
}

/// The actor gateway's status, for the doctor.
#[async_trait]
pub trait GatewayStatus: Send + Sync {
    async fn status(&self) -> R<Value>;
}

/// A part the composition sets once it has it, after the driver that reads
/// it is made: empty until then.
pub type Slot<T> = Arc<OnceLock<T>>;

/// Where a core's events go: the service's sink, once the service is up.
/// An event before that has no page to reach and is let go.
#[derive(Clone, Default)]
pub struct Emitter(Arc<OnceLock<Weak<dyn EventSink>>>);

impl Emitter {
    pub fn new() -> Emitter {
        Emitter::default()
    }

    pub fn emit(&self, from: From, name: &str, data: Value) {
        if let Some(sink) = self.sink() {
            sink.event(from, name, data);
        }
    }

    /// A line in the service's error log (level error, warn or info).
    pub fn log(&self, level: &str, code: &str, message: &str) {
        if let Some(sink) = self.sink() {
            sink.log(level, code, message);
        }
    }

    /// The service's sink, once it's up.
    pub fn sink(&self) -> Option<Arc<dyn EventSink>> {
        self.0.get().and_then(Weak::upgrade)
    }

    /// Ties the emitter to the service once it's up: `xenonet serve` does,
    /// and the tests that read the events.
    pub fn bind<S: EventSink + 'static>(&self, sink: &Arc<S>) {
        let weak: Weak<S> = Arc::downgrade(sink);
        let _ = self.0.set(weak);
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

/// What every driver is given beside the cores and its settings.
#[derive(Clone, Default)]
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
    /// The decider, which the app's driver makes after the map's, whose
    /// project picks ask it: set once every driver has run.
    pub decider: Slot<Arc<dyn Decider>>,
    /// The actor gateway's status, set once the service has made the
    /// gateway.
    pub gateway: Slot<Arc<dyn GatewayStatus>>,
}

impl WireEnv {
    /// A driver's environment at `version`, its slots empty.
    pub fn new(version: &str) -> WireEnv {
        WireEnv { version: version.to_string(), ..WireEnv::default() }
    }
}
