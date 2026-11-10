//! The engine's session index: the agents it knows of, by id, with what
//! `Engine::agents()` picks its candidates by, kept current as things change
//! instead of read again from every source on every call.
//!
//! It keeps two columns. The map's sessions (kind session, not archived,
//! and when each last changed) are read again only when the map's version
//! moves (`MapReader::nodes_version`: the map's refresh bumps it), and the
//! map's places (projects and zones) a view's folder is matched against
//! with them; a map reader that has no version is read on every call, as
//! before. The store's
//! contacts (when each agent was last in touch, and whether its SessionEnd
//! hook said it ended) are pushed by the engine's own writes (`touched`,
//! `ended`), under the store's lock so they land in the store's order.
//!
//! The live tailer's ids are read on every call (the tailer keeps them in
//! memory, and nothing tells the engine when one moves yet), and so are the
//! actors' and delivery's (in-memory lists, whose rules of who is listed
//! depend on the time and live in their own crates).
//!
//! Correctness never depends on every push arriving: the index is built
//! from the scans on its first read, again once a minute (`RECONCILE_MS`),
//! and on the next read after a write whose effect it can't be sure of
//! (`invalidate`). `scan` is the pass the index replaces, kept as the
//! reference the tests hold the index to.

use crate::engine::Result;
use crate::host::{Collaborators, IndexEntry};
use crate::store::row_json;
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};
use xenonet_os::sync::Locked;
use xenonet_text::text::{display, truthy};

/// How often the index is rebuilt from the scans, whatever was pushed.
pub const RECONCILE_MS: f64 = 60e3;

/// A time as the columns order it. NaN is never stored, and -0 is stored
/// as 0, so the order agrees with `>=`.
#[derive(Clone, Copy, Debug)]
struct At(f64);

impl PartialEq for At {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}
impl Eq for At {}
impl PartialOrd for At {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for At {
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// One agent in a column: its place in the source's own order, when it
/// last moved, and whether it counts (a contact that ended doesn't).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slot {
    pos: u64,
    at: f64,
    counts: bool,
}

/// One source's agents, by id and by time.
#[derive(Clone, Debug, Default)]
struct Column {
    slots: HashMap<String, Slot>,
    by_time: BTreeSet<(At, u64, String)>,
    next: u64,
}

impl Column {
    /// An agent at `pos` in the source's order.
    fn place(&mut self, id: &str, pos: u64, at: f64, counts: bool) {
        if let Some(old) = self.slots.remove(id) {
            self.by_time.remove(&(At(old.at), old.pos, id.to_string()));
        }
        let at = at + 0.0;
        if counts && !at.is_nan() {
            self.by_time.insert((At(at), pos, id.to_string()));
        }
        self.slots.insert(id.to_string(), Slot { pos, at, counts });
        self.next = self.next.max(pos + 1);
    }

    /// An agent heard of again keeps its place; a new one goes last, as a
    /// new row goes last in the store.
    fn set(&mut self, id: &str, at: f64, counts: bool) {
        let pos = self.slots.get(id).map_or(self.next, |s| s.pos);
        self.place(id, pos, at, counts);
    }

    /// An agent that stops counting, when the column has it.
    fn stop(&mut self, id: &str) {
        if let Some(s) = self.slots.get(id).copied() {
            self.place(id, s.pos, s.at, false);
        }
    }

    /// The agents that count and moved at `since` or later, in the
    /// source's order.
    fn since(&self, since: f64) -> Vec<String> {
        if since.is_nan() {
            return Vec::new();
        }
        let from = (At(since + 0.0), 0, String::new());
        let mut hits: Vec<(u64, &str)> = self.by_time.range(from..).map(|(_, pos, id)| (*pos, id.as_str())).collect();
        hits.sort_unstable();
        hits.into_iter().map(|(_, id)| id.to_string()).collect()
    }

    /// The map's sessions, in the map's order.
    fn of_nodes(nodes: &[IndexEntry]) -> Column {
        let mut c = Column::default();
        for (pos, n) in nodes.iter().enumerate() {
            if n.kind.as_deref() == Some("session") && !n.archived {
                c.place(&n.id, pos as u64, n.updated_at.unwrap_or(0.0), true);
            }
        }
        c
    }

    /// The store's contacts, in the store's order.
    fn of_contacts(rows: &[Value]) -> Column {
        let mut c = Column::default();
        for (pos, r) in rows.iter().enumerate() {
            let at = r.get("at").and_then(Value::as_f64).unwrap_or(f64::NAN);
            c.place(&agent_of(r), pos as u64, at, !truthy(r.get("ended_at")));
        }
        c
    }
}

/// A contact's agent, as the engine writes it in a template.
fn agent_of(contact: &Value) -> String {
    contact.get("agent").map_or_else(|| "undefined".into(), display)
}

/// How often the index went back to its sources, for the measurements.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IndexStats {
    /// Full rebuilds from the scans.
    pub rebuilds: u64,
    /// Reads of the map's nodes, the rebuilds' included.
    pub node_reads: u64,
}

#[derive(Default)]
struct Inner {
    /// When it was last built from the scans, by the engine's clock.
    built: Option<f64>,
    /// A write it can't be sure of: the next read rebuilds.
    stale: bool,
    /// The map's version its sessions were read at.
    version: Option<u64>,
    nodes: Column,
    /// The map's projects and zones that have a path, in the map's order.
    places: Arc<Vec<IndexEntry>>,
    contacts: Column,
    stats: IndexStats,
}

#[derive(Default)]
pub struct SessionIndex {
    inner: Mutex<Inner>,
}

impl SessionIndex {
    /// The agents `agents()` builds views of for the window from `since`:
    /// the live sessions that moved since, the map's sessions that changed
    /// since, the agents in touch since whose sessions haven't ended, the
    /// actors at work since, and the sessions delivery knows, each once, in
    /// that order. The same list `scan` gives.
    pub fn candidates(&self, c: &Collaborators<'_>, since: f64) -> Result<Vec<String>> {
        let live = c.map.live_ids()?;
        self.fresh(c)?;
        let (nodes, contacts) = {
            let ix = self.inner.locked();
            (ix.nodes.since(since), ix.contacts.since(since))
        };
        let mut out = Ordered::default();
        for (id, at) in live {
            if at >= since {
                out.add(id);
            }
        }
        out.extend(nodes);
        out.extend(contacts);
        Self::others(c, since, &mut out)?;
        Ok(out.ids)
    }

    /// The pass the index replaces: every source read again.
    pub fn scan(c: &Collaborators<'_>, since: f64) -> Result<Vec<String>> {
        let mut out = Ordered::default();
        for (id, at) in c.map.live_ids()? {
            if at >= since {
                out.add(id);
            }
        }
        for n in c.map.index()? {
            if n.kind.as_deref() == Some("session") && !n.archived && n.updated_at.unwrap_or(0.0) >= since {
                out.add(n.id);
            }
        }
        for contact in c.store.with(|s| s.contacts(since))?.iter().map(row_json) {
            if !truthy(contact.get("ended_at")) {
                out.add(agent_of(&contact));
            }
        }
        Self::others(c, since, &mut out)?;
        Ok(out.ids)
    }

    /// The actors at work since `since`, and the sessions delivery knows.
    fn others(c: &Collaborators<'_>, since: f64, out: &mut Ordered) -> Result<()> {
        if let Some(actors) = c.actors {
            out.extend(actors.engine_ids(since)?);
        }
        if let Some(delivery) = c.delivery {
            out.extend(delivery.known());
        }
        Ok(())
    }

    /// Rebuilt when it's due, else the map's sessions read again when the
    /// map changed.
    fn fresh(&self, c: &Collaborators<'_>) -> Result<()> {
        let now = c.clock.now();
        let version = c.map.nodes_version();
        let (due, moved) = {
            let ix = self.inner.locked();
            let due = ix.stale || ix.built.is_none_or(|b| !(now - b < RECONCILE_MS && now >= b));
            (due, version.is_none() || version != ix.version)
        };
        if due {
            return self.reconcile(c);
        }
        if moved {
            // The version first: a refresh between the two reads leaves
            // the older number, so the next read looks again.
            let (nodes, places) = Self::read_nodes(c)?;
            let mut ix = self.inner.locked();
            ix.nodes = nodes;
            ix.places = places;
            ix.version = version;
            ix.stats.node_reads += 1;
        }
        Ok(())
    }

    /// Builds the index again from the scans and puts it in place of what
    /// was pushed. The contacts are read and put in place under the store's
    /// lock, so no write lands between the two.
    pub fn reconcile(&self, c: &Collaborators<'_>) -> Result<()> {
        let now = c.clock.now();
        let version = c.map.nodes_version();
        let (nodes, places) = Self::read_nodes(c)?;
        c.store.with(|s| -> Result<()> {
            let rows: Vec<Value> = s.contacts(f64::MIN)?.iter().map(row_json).collect();
            let mut ix = self.inner.locked();
            ix.nodes = nodes;
            ix.places = places;
            ix.version = version;
            ix.contacts = Column::of_contacts(&rows);
            ix.built = Some(now);
            ix.stale = false;
            ix.stats.rebuilds += 1;
            ix.stats.node_reads += 1;
            Ok(())
        })
    }

    /// The map's nodes, as its sessions and its places.
    fn read_nodes(c: &Collaborators<'_>) -> Result<(Column, Arc<Vec<IndexEntry>>)> {
        let all = c.map.index()?;
        let nodes = Column::of_nodes(&all);
        let places = all.into_iter().filter(|n| matches!(n.kind.as_deref(), Some("project" | "zone")) && n.path.as_deref().is_some_and(|p| !p.is_empty())).collect();
        Ok((nodes, Arc::new(places)))
    }

    /// The map's projects and zones that have a path, in the map's order,
    /// for a view's place: read with the map's sessions.
    pub fn places(&self, c: &Collaborators<'_>) -> Result<Arc<Vec<IndexEntry>>> {
        self.fresh(c)?;
        Ok(self.inner.locked().places.clone())
    }

    /// An agent's contact was written at `at`: it counts again, ended or
    /// not before. Called under the store's lock, after the write.
    pub fn touched(&self, id: &str, at: f64) {
        self.inner.locked().contacts.set(id, at, true);
    }

    /// An agent's SessionEnd hook said it ended. Called under the store's
    /// lock, after the write.
    pub fn ended(&self, id: &str) {
        self.inner.locked().contacts.stop(id);
    }

    /// A write the index can't be sure of (it failed partway): the next
    /// read rebuilds.
    pub fn invalidate(&self) {
        self.inner.locked().stale = true;
    }

    pub fn stats(&self) -> IndexStats {
        self.inner.locked().stats
    }
}

/// Ids in the order first heard, each once.
#[derive(Default)]
struct Ordered {
    ids: Vec<String>,
    seen: HashSet<String>,
}

impl Ordered {
    fn add(&mut self, id: String) {
        if self.seen.insert(id.clone()) {
            self.ids.push(id);
        }
    }

    fn extend(&mut self, ids: impl IntoIterator<Item = String>) {
        for id in ids {
            self.add(id);
        }
    }
}

#[cfg(test)]
#[path = "index_tests.rs"]
mod tests;
