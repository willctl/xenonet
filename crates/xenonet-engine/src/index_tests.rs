//! The index holds to the scan it replaces: after touches, ends, map
//! changes, live moves, actors and delivery's sessions coming and going,
//! in a fixed sequence and in random ones, and through the engine's own
//! calls; and it doesn't go back to the map on every call.

use super::*;
use crate::delivery::policy::WakeAllowed;
use crate::delivery::router::Delivery;
use crate::engine::Engine;
use crate::host::{ActorsView, Clock, Events, Listening, MapReader, Processes, Repo, Repos, StatusSet, Waits};
use serde_json::json;
use xenonet_store::engine::{EngineStore, Shared};

const HOUR: f64 = 3600e3;

/// A made-up machine: its clock, the map's live sessions and nodes (and
/// whether the map says its version), the actors (each with when it was
/// last seen) and the sessions delivery knows.
#[derive(Default)]
struct World {
    now: f64,
    live: Vec<(String, f64)>,
    nodes: Vec<IndexEntry>,
    version: u64,
    versioned: bool,
    actors: Vec<(String, f64)>,
    known: Vec<String>,
    index_reads: u64,
}

struct Fake {
    w: Mutex<World>,
    store: Shared,
    delivery: bool,
}

impl Fake {
    fn new(versioned: bool) -> Fake {
        let w = World { now: 1e12, versioned, version: 1, ..World::default() };
        Fake { w: Mutex::new(w), store: Shared::new(EngineStore::memory(1e12).unwrap()), delivery: true }
    }

    fn c(&self) -> Collaborators<'_> {
        Collaborators {
            store: &self.store,
            clock: self,
            processes: self,
            map: self,
            actors: Some(self),
            delivery: self.delivery.then_some(self as &dyn Delivery),
            policy: None,
            repos: self,
            listening: self,
            waits: self,
            events: self,
            net: None,
        }
    }

    fn now(&self) -> f64 {
        self.w.locked().now
    }

    /// A change on the map, which moves its version.
    fn map(&self, f: impl FnOnce(&mut Vec<IndexEntry>)) {
        let mut w = self.w.locked();
        f(&mut w.nodes);
        w.version += 1;
    }

    fn reads(&self) -> u64 {
        self.w.locked().index_reads
    }
}

impl Clock for Fake {
    fn now(&self) -> f64 {
        Fake::now(self)
    }
    fn time_of(&self, _ms: f64) -> String {
        "9:04 AM".into()
    }
}

impl Processes for Fake {
    fn alive(&self, _pid: f64) -> bool {
        true
    }
}

impl MapReader for Fake {
    fn live_ids(&self) -> Result<Vec<(String, f64)>> {
        Ok(self.w.locked().live.clone())
    }
    fn index(&self) -> Result<Vec<IndexEntry>> {
        let mut w = self.w.locked();
        w.index_reads += 1;
        Ok(w.nodes.clone())
    }
    fn nodes_version(&self) -> Option<u64> {
        let w = self.w.locked();
        w.versioned.then_some(w.version)
    }
    fn node(&self, id: &str) -> Result<Option<Value>> {
        let w = self.w.locked();
        Ok(w.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| json!({ "id": n.id, "kind": n.kind, "title": format!("T {}", n.id), "cwd": "/w/p", "updatedAt": n.updated_at, "archived": n.archived })))
    }
    fn live(&self, id: &str, _now: f64) -> Result<Option<Value>> {
        let w = self.w.locked();
        Ok(w.live.iter().find(|(l, _)| l == id).map(|(_, at)| json!({ "lastEventAt": at, "endedAt": null, "status": "idle", "needsYou": null, "edits": [] })))
    }
    fn observed_live(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }
    fn home_of(&self, _id: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn identities(&self) -> Result<Vec<Value>> {
        Ok(Vec::new())
    }
}

impl ActorsView for Fake {
    fn has(&self, id: &str) -> Result<bool> {
        Ok(self.w.locked().actors.iter().any(|(a, _)| a == id))
    }
    fn is_registered(&self, id: &str) -> Result<bool> {
        ActorsView::has(self, id)
    }
    fn place_of(&self, _dir: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    fn engine_ids(&self, since: f64) -> Result<Vec<String>> {
        Ok(self.w.locked().actors.iter().filter(|(_, seen)| *seen >= since).map(|(a, _)| a.clone()).collect())
    }
    fn engine_view(&self, id: &str, _now: f64) -> Result<Option<Value>> {
        let w = self.w.locked();
        Ok(w.actors.iter().find(|(a, _)| a == id).map(|(a, seen)| json!({ "name": a, "presence": "idle", "lastSeen": seen })))
    }
    fn set_status(&self, _id: &str, _text: &str) -> Result<StatusSet> {
        Ok(StatusSet::Refused("not here".into()))
    }
    fn info(&self, _id: &str) -> Result<Option<Value>> {
        Ok(None)
    }
}

impl Delivery for Fake {
    fn known(&self) -> Vec<String> {
        self.w.locked().known.clone()
    }
    fn reaches(&self, _id: &str) -> bool {
        false
    }
    fn resumable(&self, _view: &Value) -> bool {
        false
    }
    fn wake(&self, _from: &str, _agent: &str) -> WakeAllowed {
        WakeAllowed::No("not here")
    }
    fn way(&self, _me: &str, _recipient: &Value) -> Option<&'static str> {
        None
    }
    fn describe(&self, _id: &str) -> Option<Value> {
        None
    }
}

impl Repos for Fake {
    fn of(&self, _dir: &str) -> Result<Option<Repo>> {
        Ok(None)
    }
}

impl Listening for Fake {
    fn set(&self, _id: &str, _until: f64) {}
    fn delete(&self, _id: &str) {}
    fn has(&self, _id: &str) -> bool {
        false
    }
}

impl Waits for Fake {
    fn wake_waiters(&self, _id: &str) {}
    fn wait(&self, _me: &str, _ms: f64) {}
    fn listen_wait(&self, _me: &str, _ms: f64) {}
    fn aborted(&self) -> bool {
        false
    }
}

impl Events for Fake {
    fn emit(&self, _name: &str, _payload: Value) {}
}

fn session(n: usize) -> String {
    format!("claude:{n:08x}-0000")
}

fn node(id: &str, kind: &str, at: Option<f64>, archived: bool) -> IndexEntry {
    // A project holds the folder every made-up session works in.
    let path = (kind != "session").then(|| "/w/p".to_string());
    IndexEntry { id: id.into(), kind: Some(kind.into()), archived, updated_at: at, name: Some(format!("{id} name")), path }
}

/// A session in touch, through the engine.
fn touch(e: &Engine, f: &Fake, id: &str) {
    e.touch(&f.c(), id, &json!({ "app": "claude", "via": "hook", "cwd": "/w/p" })).unwrap();
}

/// A session's SessionEnd hook, through the engine.
fn end(e: &Engine, f: &Fake, id: &str) {
    let local = id.split_once(':').unwrap().1;
    e.hook(&f.c(), "claude", Some("SessionEnd"), &json!({ "session_id": local })).unwrap();
}

/// The index's candidates against the scan's, for a few windows.
fn same(e: &Engine, f: &Fake, why: &str) {
    let c = f.c();
    for within in [0.0, 60e3, HOUR, 2.0 * HOUR, 7.0 * 24.0 * HOUR, f64::INFINITY] {
        let since = f.now() - within;
        assert_eq!(e.index.candidates(&c, since).unwrap(), SessionIndex::scan(&c, since).unwrap(), "{why}, within {within}");
    }
}

/// What agents() answers, from a long-lived engine and from one made now.
fn same_agents(e: &Engine, f: &Fake, why: &str) {
    let fresh = Engine::default();
    assert_eq!(e.agents(&f.c(), 2.0 * HOUR).unwrap(), fresh.agents(&f.c(), 2.0 * HOUR).unwrap(), "{why}");
}

#[test]
fn the_index_gives_the_scans_candidates_after_each_kind_of_change() {
    let f = Fake::new(true);
    let e = Engine::default();
    let now = f.now();
    f.map(|n| {
        n.push(node("project:p", "project", Some(now), false));
        n.push(node(&session(1), "session", Some(now - 30e3), false));
        n.push(node(&session(2), "session", Some(now - 3.0 * HOUR), false));
        n.push(node(&session(3), "session", Some(now), true));
    });
    f.w.locked().live = vec![(session(4), now - 10e3), (session(1), now - 5.0 * HOUR)];
    same(&e, &f, "built from the scans");
    same_agents(&e, &f, "built from the scans");

    touch(&e, &f, &session(5));
    touch(&e, &f, &session(2));
    same(&e, &f, "two touches");
    end(&e, &f, &session(5));
    same(&e, &f, "an end");
    touch(&e, &f, &session(5));
    same(&e, &f, "in touch again after its end");
    same_agents(&e, &f, "after the touches");

    f.map(|n| {
        n[1].updated_at = Some(now - 4.0 * HOUR);
        n[3].archived = false;
        n.insert(0, node(&session(6), "session", None, false));
    });
    same(&e, &f, "a map change");
    f.w.locked().actors.push(("actor:muse".into(), now - 60e3));
    f.w.locked().known.push(session(7));
    same(&e, &f, "an actor and a known session");
    same_agents(&e, &f, "an actor and a known session");
    f.w.locked().actors.clear();
    same(&e, &f, "the actor gone");
    f.w.locked().live.push((session(8), now));
    same(&e, &f, "a live session");
    f.w.locked().now += 3.0 * HOUR;
    same(&e, &f, "hours later");
    same_agents(&e, &f, "hours later");
}

#[test]
fn a_map_without_a_version_is_read_on_every_call_and_one_with_only_when_it_moves() {
    let f = Fake::new(false);
    let e = Engine::default();
    for _ in 0..5 {
        e.index.candidates(&f.c(), 0.0).unwrap();
    }
    assert_eq!(f.reads(), 5, "no version: every call");

    let f = Fake::new(true);
    let e = Engine::default();
    for _ in 0..5 {
        e.index.candidates(&f.c(), 0.0).unwrap();
    }
    assert_eq!((f.reads(), e.index.stats().rebuilds), (1, 1), "a version that stays: once");
    f.map(|n| n.push(node(&session(1), "session", Some(1e12), false)));
    assert_eq!(e.index.candidates(&f.c(), 0.0).unwrap(), vec![session(1)]);
    assert_eq!(f.reads(), 2, "read again when it moved");
    // A change the map didn't count is seen at the next reconcile.
    f.w.locked().nodes.push(node(&session(2), "session", Some(1e12), false));
    assert_eq!(e.index.candidates(&f.c(), 0.0).unwrap(), vec![session(1)]);
    f.w.locked().now += RECONCILE_MS;
    assert_eq!(e.index.candidates(&f.c(), 0.0).unwrap(), vec![session(1), session(2)]);
    assert_eq!(e.index.stats().rebuilds, 2);
}

#[test]
fn a_write_the_index_cant_be_sure_of_rebuilds_it() {
    let f = Fake::new(true);
    let e = Engine::default();
    e.index.candidates(&f.c(), 0.0).unwrap();
    // A contact written past the engine (another process on the same
    // file): unseen until the index is told it may be wrong.
    f.store.with(|s| s.touch(&session(1), &json!({ "app": "claude", "via": "hook" }), 1e12)).unwrap();
    assert_eq!(e.index.candidates(&f.c(), 0.0).unwrap(), Vec::<String>::new());
    e.index.invalidate();
    assert_eq!(e.index.candidates(&f.c(), 0.0).unwrap(), vec![session(1)]);
    // A touch the store refuses (no app) leaves the index to rebuild.
    let before = e.index.stats().rebuilds;
    assert!(e.touch(&f.c(), &session(2), &json!({ "via": "hook" })).is_err());
    e.index.candidates(&f.c(), 0.0).unwrap();
    assert_eq!(e.index.stats().rebuilds, before + 1);
}

/// xorshift64*: the same random sequence every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn random_updates_keep_the_index_to_the_scan() {
    for seed in 1..=8_u64 {
        let mut r = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let f = Fake::new(seed % 2 == 0);
        let e = Engine::default();
        let pool = 24;
        for step in 0..300 {
            let id = session(r.below(pool) as usize);
            let now = f.now();
            // Times on a coarse grid, so ties (and their order) come up.
            let at = now - (r.below(6) as f64) * HOUR;
            match r.below(11) {
                0 | 1 => touch(&e, &f, &id),
                2 => end(&e, &f, &id),
                3 => {
                    let mut w = f.w.locked();
                    match w.live.iter_mut().find(|(l, _)| *l == id) {
                        Some(l) => l.1 = at,
                        None => w.live.push((id, at)),
                    }
                }
                4 => f.map(|n| match n.iter_mut().find(|x| x.id == id) {
                    Some(x) => {
                        x.updated_at = (r.below(5) > 0).then_some(at);
                        x.archived = r.below(4) == 0;
                    }
                    None => n.insert(r.below(n.len() as u64 + 1) as usize, node(&id, if r.below(5) == 0 { "project" } else { "session" }, Some(at), false)),
                }),
                5 => f.map(|n| n.retain(|x| x.id != id)),
                6 => {
                    let a = format!("actor:a{}", r.below(4));
                    let mut w = f.w.locked();
                    w.actors.retain(|(x, _)| *x != a);
                    if r.below(3) > 0 {
                        w.actors.push((a, at));
                    }
                }
                7 => {
                    let mut w = f.w.locked();
                    match w.known.iter().position(|k| *k == id) {
                        Some(i) => {
                            w.known.remove(i);
                        }
                        None => w.known.push(id),
                    }
                }
                8 => f.w.locked().now += (r.below(4) as f64) * 0.5 * HOUR,
                9 => f.w.locked().now += 1.0,
                _ => f.w.locked().live.retain(|(l, _)| *l != id),
            }
            same(&e, &f, &format!("seed {seed}, step {step}"));
            if step % 50 == 0 {
                same_agents(&e, &f, &format!("seed {seed}, step {step}"));
            }
        }
    }
}

/// A machine with `n` sessions on the map, each in touch, a tenth of them
/// at work in the last hour.
fn crowded(n: usize) -> (Fake, Engine) {
    let f = Fake::new(true);
    let e = Engine::default();
    let now = f.now();
    f.map(|nodes| {
        for i in 0..n {
            let at = if i % 10 == 0 { now - 60e3 } else { now - 30.0 * HOUR };
            nodes.push(node(&session(i), "session", Some(at), false));
        }
    });
    for i in 0..n {
        f.store.with(|s| s.touch(&session(i), &json!({ "app": "claude", "via": "hook" }), if i % 10 == 0 { now - 60e3 } else { now - 30.0 * HOUR })).unwrap();
    }
    (f, e)
}

#[test]
fn a_thousand_sessions_are_read_once_not_on_every_call() {
    let (f, e) = crowded(1000);
    for _ in 0..50 {
        assert_eq!(e.agents(&f.c(), 2.0 * HOUR).unwrap().len(), 100);
    }
    assert_eq!((f.reads(), e.index.stats().rebuilds, e.index.stats().node_reads), (1, 1, 1));
    same(&e, &f, "a thousand sessions");
}

/// What a call costs with the index and with the scan, on a thousand
/// sessions: `cargo test -p xenonet-engine thousand -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: prints its times"]
fn measure_a_thousand_sessions() {
    let (f, e) = crowded(1000);
    let c = f.c();
    let since = f.now() - 2.0 * HOUR;
    let runs = 200;
    e.index.candidates(&c, since).unwrap();
    let t = std::time::Instant::now();
    for _ in 0..runs {
        e.index.candidates(&c, since).unwrap();
    }
    let indexed = t.elapsed() / runs;
    let t = std::time::Instant::now();
    for _ in 0..runs {
        SessionIndex::scan(&c, since).unwrap();
    }
    let scanned = t.elapsed() / runs;
    let t = std::time::Instant::now();
    for _ in 0..runs {
        e.agents(&c, 2.0 * HOUR).unwrap();
    }
    let agents = t.elapsed() / runs;
    println!("1000 sessions, 100 active: candidates {indexed:?} with the index, {scanned:?} by the scan; agents() {agents:?}");
    println!("map reads {}, rebuilds {}", f.reads(), e.index.stats().rebuilds);
}
