//! Where the network's news goes: the engine's `Sink`
//! (`xenonet_base::network::Sink`), set once the engine is there. Until
//! then, and in a service without the engine, a `LogSink` writes each call
//! to the log, so nothing that arrives is lost silently.
//!
//! The map's relay bodies go elsewhere, to the `RelaySink`s set here by
//! role: the service's relay (`serve`) and the hub (`hear`). They never
//! reach the engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use xenonet_base::network::{Incoming, RelaySink, Sink};
use xenonet_base::sync::{Locked, ReadWrite};

pub type LogFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// The sink the network calls, which the engine can replace, and the
/// relay sinks beside it.
pub struct SinkSlot {
    inner: RwLock<Arc<dyn Sink>>,
    relays: RwLock<Vec<(&'static str, Arc<dyn RelaySink>)>>,
}

impl SinkSlot {
    pub fn new(sink: Arc<dyn Sink>) -> SinkSlot {
        SinkSlot { inner: RwLock::new(sink), relays: RwLock::new(Vec::new()) }
    }

    /// The engine plugs in here.
    pub fn set(&self, sink: Arc<dyn Sink>) {
        *self.inner.writing() = sink;
    }

    pub fn get(&self) -> Arc<dyn Sink> {
        self.inner.reading().clone()
    }

    /// A relay sink, by its role, in place of one set before with it.
    pub fn set_relay(&self, role: &'static str, sink: Arc<dyn RelaySink>) {
        let mut relays = self.relays.writing();
        relays.retain(|(r, _)| *r != role);
        relays.push((role, sink));
    }

    fn relays(&self) -> Vec<Arc<dyn RelaySink>> {
        self.relays.reading().iter().map(|(_, s)| s.clone()).collect()
    }

    /// A relay body from another host, to each relay sink.
    pub fn relay(&self, incoming: Incoming) {
        for s in self.relays() {
            s.relay(incoming.clone());
        }
    }
}

impl Sink for SinkSlot {
    fn receive(&self, incoming: Incoming) {
        self.get().receive(incoming);
    }
    fn linked(&self, up: bool) {
        self.get().linked(up);
        for s in self.relays() {
            s.linked(up);
        }
    }
    fn removed(&self, host: &str) {
        self.get().removed(host);
        for s in self.relays() {
            s.removed(host);
        }
    }
    fn gap(&self) {
        self.get().gap();
        for s in self.relays() {
            s.gap();
        }
    }
}

/// A sink that only logs: what each call was, never a body's text, and
/// what arrives at most once a minute for each kind and host.
pub struct LogSink {
    log: LogFn,
    said: Mutex<HashMap<String, Instant>>,
}

impl LogSink {
    pub fn new(log: LogFn) -> LogSink {
        LogSink { log, said: Mutex::new(HashMap::new()) }
    }
}

impl Sink for LogSink {
    fn receive(&self, incoming: Incoming) {
        let k = format!("{}|{}", incoming.kind.as_str(), incoming.from);
        {
            let mut said = self.said.locked();
            if said.get(&k).is_some_and(|t| t.elapsed() < Duration::from_secs(60)) {
                return;
            }
            said.insert(k, Instant::now());
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xenonet_base::network::Kind;

    #[derive(Default)]
    struct Heard(Mutex<Vec<String>>);

    impl RelaySink for Heard {
        fn relay(&self, incoming: Incoming) {
            self.0.locked().push(format!("relay {}", incoming.from));
        }
        fn removed(&self, host: &str) {
            self.0.locked().push(format!("removed {host}"));
        }
        fn gap(&self) {
            self.0.locked().push("gap".into());
        }
    }

    #[test]
    fn relay_bodies_go_to_the_relay_sinks_by_role_and_never_the_engine() {
        let engine = Arc::new(LogSink::new(Arc::new(|_: &str, _: &str| {})));
        let slot = SinkSlot::new(engine);
        let (a, b, a2) = (Arc::new(Heard::default()), Arc::new(Heard::default()), Arc::new(Heard::default()));
        slot.set_relay("serve", a.clone());
        slot.set_relay("hear", b.clone());
        slot.set_relay("serve", a2.clone());
        slot.relay(Incoming { from: "laptop".into(), kind: Kind::Relay, body: json!({}), at: 0.0, from_window_host: true });
        slot.removed("desk");
        slot.gap();
        assert!(a.0.locked().is_empty(), "a sink set again by its role replaces the one before");
        assert_eq!(*a2.0.locked(), ["relay laptop", "removed desk", "gap"]);
        assert_eq!(*b.0.locked(), ["relay laptop", "removed desk", "gap"]);
    }
}
