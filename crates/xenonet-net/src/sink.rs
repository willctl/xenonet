//! Where the network's news goes: the engine's `Sink`
//! (`xenonet_base::network::Sink`), set once the engine is there. Until
//! then, and in a service without the engine, a `LogSink` writes each call
//! to the log, so nothing that arrives is lost silently.

use std::sync::{Arc, RwLock};
use xenonet_base::network::{Incoming, Sink};
use xenonet_base::sync::ReadWrite;

pub type LogFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// The sink the network calls, which the engine can replace.
pub struct SinkSlot {
    inner: RwLock<Arc<dyn Sink>>,
}

impl SinkSlot {
    pub fn new(sink: Arc<dyn Sink>) -> SinkSlot {
        SinkSlot { inner: RwLock::new(sink) }
    }

    /// The engine plugs in here.
    pub fn set(&self, sink: Arc<dyn Sink>) {
        *self.inner.writing() = sink;
    }

    pub fn get(&self) -> Arc<dyn Sink> {
        self.inner.reading().clone()
    }
}

impl Sink for SinkSlot {
    fn receive(&self, incoming: Incoming) {
        self.get().receive(incoming);
    }
    fn linked(&self, up: bool) {
        self.get().linked(up);
    }
    fn removed(&self, host: &str) {
        self.get().removed(host);
    }
    fn gap(&self) {
        self.get().gap();
    }
}

/// A sink that only logs: what each call was, never a body's text.
pub struct LogSink {
    pub log: LogFn,
}

impl Sink for LogSink {
    fn receive(&self, incoming: Incoming) {
        (self.log)("net.receive", &format!("a {} frame from {}, with no engine to take it", incoming.kind.as_str(), incoming.from));
    }
    fn linked(&self, up: bool) {
        (self.log)("net.link", if up { "the link to the network's gateway is up" } else { "the link to the network's gateway is down" });
    }
    fn removed(&self, host: &str) {
        (self.log)("net.removed", &format!("{host} was removed from the network"));
    }
    fn gap(&self) {
        (self.log)("net.gap", "the gateway lost frames for this host or from it");
    }
}
