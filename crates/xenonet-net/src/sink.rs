//! Where the network's news goes: the engine's `Sink`
//! (`xenonet_model::ops::Sink`), set once the engine is there. Until
//! then, and in a service without the engine, a `LogSink` writes each call
//! to the log, so nothing that arrives is lost silently.
//!
//! Entries of other members' feeds (messages, receipts, handoffs) go to
//! the `Feeds` set here: the engine's, which applies each once in its
//! store, in the transaction that writes what it does with it. Until one is
//! set, and in a service without the engine, the slot's own feeds in
//! memory take them, once each and in order, and hand what they apply to
//! the sink as `Incoming` bodies.

use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use xenonet_log::feed::{Feed, Kind as EntryKind, MemoryStore, Offer, Verified, VerifiedCompacted};
use xenonet_model::ops::{Incoming, Kind, Sink};
use xenonet_os::sync::{Locked, ReadWrite};

/// An entry of another member's feed, as this machine took it: its author's
/// signature checked against the key the network's chain lists for the
/// member, which is `host`.
#[derive(Debug, Clone)]
pub struct Arrived {
    pub host: String,
    pub entry: Verified,
    /// What its act rests on, beside the entry in its frame (`{}` for none).
    pub authority: Value,
    /// Whether the chain makes `host` a window host.
    pub window: bool,
}

/// A compacted range of another member's feed, as this machine took it:
/// the author's answer to a Want below its floor, from the author or a
/// holder, every entry of it checked against the key the network's chain
/// lists for the member, which is `host`.
#[derive(Debug, Clone)]
pub struct CompactedIn {
    pub host: String,
    pub compacted: VerifiedCompacted,
    /// Whether the chain makes `host` a window host.
    pub window: bool,
}

/// Where other members' feed entries go.
pub trait Feeds: Send + Sync {
    /// An entry to offer to the author's feed here and apply.
    fn entry(&self, arrived: Arrived);
    /// A compacted range to offer to the author's feed here
    /// (`Feed::offer_compacted`): the live entries it lacks applied, what
    /// it held there that isn't live dropped, and what the compact entry
    /// leaves out ended.
    fn compacted(&self, arrived: CompactedIn) {
        let _ = arrived;
    }
    /// Takes `author`'s feed in `network` again after a fork, keeping the
    /// entry this machine had: the person's alone, from the window. How
    /// many held entries it applied.
    fn unfreeze(&self, author: &str, network: &str) -> Result<usize, String>;
}

/// The engine's kind for an entry's, for the ones it takes.
pub fn kind_of(k: EntryKind) -> Option<Kind> {
    match k {
        EntryKind::Message => Some(Kind::Message),
        EntryKind::Receipt => Some(Kind::Receipt),
        EntryKind::Handoff => Some(Kind::Handoff),
        EntryKind::Claim => Some(Kind::Claims),
        EntryKind::Intent => Some(Kind::Intent),
        EntryKind::Assign => Some(Kind::Assign),
        EntryKind::Report => Some(Kind::Report),
        EntryKind::Compact => None,
        // Xenolith's, in a space's feed: never the engine's.
        EntryKind::CtxNote | EntryKind::CtxPropose | EntryKind::CtxPage | EntryKind::CtxRemove | EntryKind::CtxLog | EntryKind::CtxForget | EntryKind::CtxSnapshot => None,
    }
}

/// An entry's kind for the engine's.
pub fn entry_kind(k: Kind) -> Option<EntryKind> {
    match k {
        Kind::Message => Some(EntryKind::Message),
        Kind::Receipt => Some(EntryKind::Receipt),
        Kind::Handoff => Some(EntryKind::Handoff),
        Kind::Claims => Some(EntryKind::Claim),
        Kind::Intent => Some(EntryKind::Intent),
        Kind::Assign => Some(EntryKind::Assign),
        Kind::Report => Some(EntryKind::Report),
        _ => None,
    }
}

pub type LogFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// The sink the network calls, which the engine can replace.
pub struct SinkSlot {
    inner: RwLock<Arc<dyn Sink>>,
    feeds: RwLock<Option<Arc<dyn Feeds>>>,
    /// The feeds in memory, until the engine's are set.
    memory: Mutex<Feed<MemoryStore>>,
}

impl SinkSlot {
    pub fn new(sink: Arc<dyn Sink>) -> SinkSlot {
        SinkSlot { inner: RwLock::new(sink), feeds: RwLock::new(None), memory: Mutex::new(Feed::new(MemoryStore::new())) }
    }

    /// The engine's feeds plug in here.
    pub fn set_feeds(&self, feeds: Arc<dyn Feeds>) {
        *self.feeds.writing() = Some(feeds);
    }

    /// An entry of another member's feed: to the engine's feeds, or, with
    /// none set, applied in memory and handed to the sink.
    pub fn entry(&self, a: Arrived) {
        let feeds = self.feeds.reading().clone();
        if let Some(f) = feeds {
            f.entry(a);
            return;
        }
        let offered = self.memory.locked().offer(a.entry);
        if let Ok(Offer::New(entries)) = offered {
            for e in entries {
                let Some(kind) = kind_of(e.kind) else { continue };
                self.receive(Incoming { network: e.group.clone(), from: a.host.clone(), kind, body: e.body, at: e.at as f64, from_window_host: a.window });
            }
        }
    }

    /// A compacted range of another member's feed: to the engine's feeds,
    /// or, with none set, applied in memory, its live entries handed to the
    /// sink.
    pub fn compacted(&self, a: CompactedIn) {
        let feeds = self.feeds.reading().clone();
        if let Some(f) = feeds {
            f.compacted(a);
            return;
        }
        let offered = self.memory.locked().offer_compacted(a.compacted);
        if let Ok(Offer::New(entries)) = offered {
            for e in entries {
                let Some(kind) = kind_of(e.kind) else { continue };
                self.receive(Incoming { network: e.group.clone(), from: a.host.clone(), kind, body: e.body, at: e.at as f64, from_window_host: a.window });
            }
        }
    }

    /// Unfreezes `author`'s feed in `network`, in the engine's feeds or in
    /// memory.
    pub fn unfreeze(&self, author: &str, network: &str) -> Result<usize, String> {
        let feeds = self.feeds.reading().clone();
        match feeds {
            Some(f) => f.unfreeze(author, network),
            None => Ok(self.memory.locked().unfreeze(author, network).map_or(0, |v| v.len())),
        }
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
        (self.log)("net.link", if up { "this machine reaches the network's machines" } else { "this machine reaches none of the network's machines" });
    }
    fn removed(&self, host: &str) {
        (self.log)("net.removed", &format!("{host} was removed from the network"));
    }
    fn gap(&self) {
        (self.log)("net.gap", "entries for this host or from it were lost on the way");
    }
}
